pub mod executor;
pub mod flash_loan;
pub mod instructions;
pub mod profitability;
pub mod reserve;

use anyhow::{Context, Result};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::Signer,
};

use crate::config::AppConfig;
use crate::db::{self, SupabaseClient, NewLiquidationRecord, UpdateLiquidationResult, LiquidationStatus};
use crate::obligation::{health::HealthResult, positions};

/// Execute a flash-loan-based liquidation for an underwater obligation.
/// Logs all attempts (profitable or not, success or failure) to Supabase.
pub async fn execute_liquidation(
    config: &AppConfig,
    obligation_pubkey: &Pubkey,
    health: &HealthResult,
    supabase: Option<&SupabaseClient>,
) -> Result<()> {
    tracing::info!(
        obligation = %obligation_pubkey,
        ltv = %health.current_ltv,
        "preparing flash loan liquidation"
    );

    let rpc = RpcClient::new_with_commitment(
        config.rpc_url.clone(),
        CommitmentConfig::confirmed(),
    );

    let obligation_account = rpc
        .get_account(obligation_pubkey)
        .context("failed to fetch obligation")?;

    let obligation_positions = positions::parse_positions(&obligation_account.data)?;

    if obligation_positions.borrows.is_empty() || obligation_positions.deposits.is_empty() {
        tracing::debug!(obligation = %obligation_pubkey, "no borrows or deposits — skipping");
        return Ok(());
    }

    let repay_pos = obligation_positions.borrows.iter()
        .max_by_key(|b| b.market_value_sf).unwrap();
    let withdraw_pos = obligation_positions.deposits.iter()
        .max_by_key(|d| d.market_value_sf).unwrap();

    let repay_reserve_acct = rpc.get_account(&repay_pos.reserve)
        .context("failed to fetch repay reserve")?;
    let repay_reserve = reserve::parse_reserve(&repay_pos.reserve, &repay_reserve_acct.data)?;

    let withdraw_reserve_acct = rpc.get_account(&withdraw_pos.reserve)
        .context("failed to fetch withdraw reserve")?;
    let withdraw_reserve = reserve::parse_reserve(&withdraw_pos.reserve, &withdraw_reserve_acct.data)?;

    let market_acct = rpc.get_account(&obligation_positions.lending_market)
        .context("failed to fetch lending market")?;
    let market = reserve::parse_lending_market(&market_acct.data)?;

    let sf_shift: u128 = 1u128 << 60;
    let borrowed_amount = (repay_pos.borrowed_amount_sf / sf_shift) as u64;
    let close_factor = market.liquidation_max_debt_close_factor_pct as u64;
    let repay_amount = (borrowed_amount * close_factor / 100)
        .min(repay_reserve.available_liquidity);

    if repay_amount == 0 {
        tracing::debug!(obligation = %obligation_pubkey, "repay amount is zero — skipping");
        return Ok(());
    }

    let profit = profitability::estimate_profit(
        repay_amount,
        &repay_reserve,
        &withdraw_reserve,
        config.min_profit_lamports,
    );

    // Build the DB record (used for both profitable and skipped)
    let record_id = db::new_record_id();
    let db_record = NewLiquidationRecord {
        id: record_id.clone(),
        obligation_pubkey: obligation_pubkey.to_string(),
        obligation_owner: obligation_positions.owner.to_string(),
        lending_market: obligation_positions.lending_market.to_string(),
        repay_reserve: repay_pos.reserve.to_string(),
        repay_mint: repay_reserve.accounts.liquidity_mint.to_string(),
        withdraw_reserve: withdraw_pos.reserve.to_string(),
        withdraw_mint: withdraw_reserve.accounts.liquidity_mint.to_string(),
        ltv_at_detection: health.current_ltv,
        unhealthy_ltv: health.unhealthy_ltv,
        repay_amount: repay_amount as i64,
        liquidation_bonus_bps: profit.liquidation_bonus_bps as i32,
        flash_loan_fee_fraction: profit.flash_loan_fee_fraction,
        estimated_gross_profit_usd: profit.gross_profit_usd,
        estimated_net_profit_usd: profit.net_profit_usd,
        status: if profit.is_profitable {
            LiquidationStatus::Pending.to_string()
        } else {
            LiquidationStatus::Skipped.to_string()
        },
        error_message: if !profit.is_profitable {
            Some(format!(
                "not profitable: net=${:.4} (bonus={}bps, fee={:.4})",
                profit.net_profit_usd, profit.liquidation_bonus_bps, profit.flash_loan_fee_fraction
            ))
        } else {
            None
        },
    };

    // Insert the record (fire-and-forget on error — don't block liquidation)
    if let Some(sb) = supabase {
        if let Err(e) = sb.insert_liquidation(&db_record).await {
            tracing::warn!(error = %e, "failed to insert liquidation record to supabase");
        }
    }

    if !profit.is_profitable {
        tracing::info!(
            obligation = %obligation_pubkey,
            net_profit_usd = %format!("{:.4}", profit.net_profit_usd),
            "liquidation not profitable — skipped (logged to DB)"
        );
        return Ok(());
    }

    tracing::info!(
        obligation = %obligation_pubkey,
        net_profit_usd = %format!("{:.4}", profit.net_profit_usd),
        repay_amount = repay_amount,
        "profitable liquidation — building tx"
    );

    // Build and submit the transaction
    let (tx, liquidator_keypair) =
        flash_loan::build_liquidation_tx(config, obligation_pubkey, health).await?;

    // Update DB: submitted
    if let Some(sb) = supabase {
        let _ = sb.update_liquidation(&record_id, &UpdateLiquidationResult {
            status: LiquidationStatus::Submitted.to_string(),
            updated_at: db::now_iso(),
            tx_signature: None,
            error_message: None,
            actual_profit_usd: None,
            sol_fee_paid: None,
            slot_submitted: None,
            slot_confirmed: None,
        }).await;
    }

    match rpc.send_and_confirm_transaction(&tx) {
        Ok(signature) => {
            tracing::info!(
                obligation = %obligation_pubkey,
                signature = %signature,
                liquidator = %liquidator_keypair.pubkey(),
                net_profit_usd = %format!("{:.4}", profit.net_profit_usd),
                "liquidation confirmed"
            );

            // Update DB: confirmed
            if let Some(sb) = supabase {
                let _ = sb.update_liquidation(&record_id, &UpdateLiquidationResult {
                    status: LiquidationStatus::Confirmed.to_string(),
                    updated_at: db::now_iso(),
                    tx_signature: Some(signature.to_string()),
                    error_message: None,
                    actual_profit_usd: Some(profit.net_profit_usd), // TODO: calculate actual from on-chain
                    sol_fee_paid: None, // TODO: fetch from tx meta
                    slot_submitted: None,
                    slot_confirmed: None,
                }).await;
            }
        }
        Err(e) => {
            tracing::error!(
                obligation = %obligation_pubkey,
                error = %e,
                "liquidation transaction failed"
            );

            // Update DB: failed
            if let Some(sb) = supabase {
                let _ = sb.update_liquidation(&record_id, &UpdateLiquidationResult {
                    status: LiquidationStatus::Failed.to_string(),
                    updated_at: db::now_iso(),
                    tx_signature: None,
                    error_message: Some(e.to_string()),
                    actual_profit_usd: None,
                    sol_fee_paid: None,
                    slot_submitted: None,
                    slot_confirmed: None,
                }).await;
            }

            return Err(e.into());
        }
    }

    Ok(())
}
