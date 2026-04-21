//! Unified cross-protocol liquidation executor.
//!
//! Selects the cheapest flash-loan provider for each liquidation via the
//! [`FlashLoanProvider`] trait, asks the active [`LendingProtocol`] to build
//! its liquidation instruction, assembles the full transaction, and submits
//! it as a Jito bundle.
//!
//! Transaction layout:
//!   `[ATA setup]` → `flash_borrow` → `liquidate` → `[swap]` → `flash_repay`

use anyhow::{Context, Result};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    instruction::Instruction,
    pubkey::Pubkey,
    signature::{Keypair, Signer, read_keypair_file},
};

use crate::config::AppConfig;
use crate::db::{
    self, LiquidationStatus, NewLiquidationRecord, SupabaseClient, UpdateLiquidationResult,
};
use crate::flash_loan::{self, FlashLoanProvider};
use crate::protocols::{self, LiquidationParams, Registry};

pub use crate::protocols::LiquidationParams as LiquidationParamsAlias;

/// Associated-token-account (ATA) program.
pub const ATA_PROGRAM: Pubkey = solana_sdk::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/// SPL Token program.
pub const SPL_TOKEN_PROGRAM: Pubkey =
    solana_sdk::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

/// Derive the associated-token-account address for `(wallet, mint)`.
pub fn derive_ata(wallet: &Pubkey, mint: &Pubkey) -> Pubkey {
    let (ata, _) = Pubkey::find_program_address(
        &[wallet.as_ref(), SPL_TOKEN_PROGRAM.as_ref(), mint.as_ref()],
        &ATA_PROGRAM,
    );
    ata
}

/// Build the full liquidation transaction for `params` without submitting it.
///
/// Returns the ordered instructions and the liquidator keypair. Used both by
/// [`execute_liquidation`] (which signs and submits) and by tests that want to
/// inspect the built transaction.
pub async fn build_liquidation_tx(
    config: &AppConfig,
    params: &LiquidationParams,
    flash_providers: &[Box<dyn FlashLoanProvider>],
) -> Result<(Vec<Instruction>, Keypair)> {
    let registry = Registry::new();
    let handler = registry.get(params.protocol);

    let rpc =
        RpcClient::new_with_commitment(config.rpc_url.clone(), CommitmentConfig::confirmed());

    let liquidator_keypair = read_keypair_file(&config.liquidator_keypair_path)
        .map_err(|e| anyhow::anyhow!("failed to read keypair: {e}"))?;
    let liquidator_pubkey = liquidator_keypair.pubkey();

    if params.positions.borrows.is_empty() || params.positions.deposits.is_empty() {
        anyhow::bail!("position has no active borrows or deposits");
    }

    let repay_pos = params
        .positions
        .borrows
        .iter()
        .max_by(|a, b| a.market_value_usd.total_cmp(&b.market_value_usd))
        .context("no borrow position found")?;
    let withdraw_pos = params
        .positions
        .deposits
        .iter()
        .max_by(|a, b| a.market_value_usd.total_cmp(&b.market_value_usd))
        .context("no deposit position found")?;

    // Debt mint drives flash-loan selection. Zero-pubkey is a real bug — the
    // protocol's parse_positions left mint unset for a borrow we want to repay.
    let debt_mint = repay_pos
        .mint
        .ok_or_else(|| anyhow::anyhow!("debt mint unknown for {}", params.protocol))?;

    let flash_provider = flash_loan::select_provider(flash_providers, &debt_mint);
    let flash_provider_name = flash_provider
        .map(|p| p.kind().to_string())
        .unwrap_or_else(|| "none".into());

    tracing::info!(
        protocol = %params.protocol,
        position = %params.position_pubkey,
        debt_mint = %debt_mint,
        flash_provider = %flash_provider_name,
        "preparing liquidation"
    );

    let liquidate_ix = handler.build_liquidate_ix(&rpc, config, params, &liquidator_pubkey)?;
    let setup_ixs = build_setup_ixs(&rpc, &liquidator_pubkey, &debt_mint, withdraw_pos)?;
    let repay_amount = handler.flash_loan_amount(repay_pos);

    let liquidator_token_account = derive_ata(&liquidator_pubkey, &debt_mint);

    let all_ixs = if let Some(provider) = flash_provider {
        let borrow_ix_index = setup_ixs.len() as u8;
        let flash_ixs = provider.build_instructions(
            &liquidator_pubkey,
            &liquidator_token_account,
            &debt_mint,
            repay_amount,
            borrow_ix_index,
        )?;

        tracing::info!(
            provider = %flash_ixs.provider,
            amount = repay_amount,
            fee_rate = %provider.fee_rate(),
            "using flash loan"
        );

        flash_loan::build_flash_loan_tx(setup_ixs, flash_ixs, liquidate_ix, None)
    } else {
        tracing::warn!(
            debt_mint = %debt_mint,
            "no flash-loan provider for this mint — submitting without flash loan"
        );
        let mut ixs = setup_ixs;
        ixs.push(liquidate_ix);
        ixs
    };

    Ok((all_ixs, liquidator_keypair))
}

/// Execute a liquidation end-to-end: build the tx, write an audit record, and
/// submit via Jito (or fall back to standard RPC when Jito is disabled).
pub async fn execute_liquidation(
    config: &AppConfig,
    params: &LiquidationParams,
    flash_providers: &[Box<dyn FlashLoanProvider>],
    supabase: Option<&SupabaseClient>,
) -> Result<()> {
    let rpc =
        RpcClient::new_with_commitment(config.rpc_url.clone(), CommitmentConfig::confirmed());

    let (all_ixs, liquidator_keypair) =
        build_liquidation_tx(config, params, flash_providers).await?;

    let repay_pos = params
        .positions
        .borrows
        .iter()
        .max_by(|a, b| a.market_value_usd.total_cmp(&b.market_value_usd))
        .context("no borrow position")?;
    let withdraw_pos = params
        .positions
        .deposits
        .iter()
        .max_by(|a, b| a.market_value_usd.total_cmp(&b.market_value_usd))
        .context("no deposit position")?;
    let debt_mint = repay_pos.mint.unwrap_or_default();

    let registry = Registry::new();
    let repay_amount = registry.get(params.protocol).flash_loan_amount(repay_pos);

    let flash_provider = flash_loan::select_provider(flash_providers, &debt_mint);
    let flash_provider_name = flash_provider
        .map(|p| p.kind().to_string())
        .unwrap_or_else(|| "none".into());

    let record_id = db::new_record_id();
    let db_record = NewLiquidationRecord {
        id: record_id.clone(),
        obligation_pubkey: params.position_pubkey.to_string(),
        obligation_owner: params.positions.owner.to_string(),
        lending_market: params.positions.market.to_string(),
        repay_reserve: repay_pos.reserve.to_string(),
        repay_mint: debt_mint.to_string(),
        withdraw_reserve: withdraw_pos.reserve.to_string(),
        withdraw_mint: withdraw_pos
            .mint
            .map_or_else(|| "unknown".into(), |m| m.to_string()),
        ltv_at_detection: params.health.current_ltv,
        unhealthy_ltv: params.health.unhealthy_ltv,
        repay_amount: repay_amount as i64,
        liquidation_bonus_bps: 0,
        flash_loan_fee_fraction: flash_provider.map_or(0.0, |p| p.fee_rate()),
        estimated_gross_profit_usd: 0.0,
        estimated_net_profit_usd: 0.0,
        status: LiquidationStatus::Pending.to_string(),
        error_message: None,
    };

    if let Some(sb) = supabase
        && let Err(e) = sb.insert_liquidation(&db_record).await
    {
        tracing::warn!(error = %e, "failed to record liquidation attempt");
    }

    // Tip sizing: 5% of estimated bonus, clamped to [MIN_TIP, max_tip_per_tx].
    let repay_usd_est = repay_amount as f64 / 1e6;
    let raw_tip = ((repay_usd_est * config.risk.estimated_bonus_rate * 0.05) / 140.0 * 1e9) as u64;
    let tip_lamports = raw_tip
        .max(crate::jito::MIN_TIP_LAMPORTS)
        .min(config.risk.max_tip_per_tx_lamports);

    if let Some(sb) = supabase {
        let _ = sb
            .update_liquidation(
                &record_id,
                &UpdateLiquidationResult {
                    status: LiquidationStatus::Submitted.to_string(),
                    updated_at: db::now_iso(),
                    tx_signature: None,
                    error_message: None,
                    actual_profit_usd: None,
                    sol_fee_paid: None,
                    slot_submitted: None,
                    slot_confirmed: None,
                },
            )
            .await;
    }

    match crate::jito::submit_liquidation(
        &config.jito,
        &rpc,
        all_ixs,
        tip_lamports,
        &liquidator_keypair,
    )
    .await
    {
        Ok(result_id) => {
            tracing::info!(
                protocol = %params.protocol,
                position = %params.position_pubkey,
                result = %result_id,
                flash_provider = %flash_provider_name,
                tip_lamports,
                "liquidation submitted"
            );
            if let Some(sb) = supabase {
                let _ = sb
                    .update_liquidation(
                        &record_id,
                        &UpdateLiquidationResult {
                            status: LiquidationStatus::Confirmed.to_string(),
                            updated_at: db::now_iso(),
                            tx_signature: Some(result_id),
                            error_message: None,
                            actual_profit_usd: None,
                            sol_fee_paid: Some(tip_lamports as i64),
                            slot_submitted: None,
                            slot_confirmed: None,
                        },
                    )
                    .await;
            }
            Ok(())
        }
        Err(e) => {
            tracing::error!(
                protocol = %params.protocol,
                position = %params.position_pubkey,
                error = %e,
                "liquidation submission failed"
            );
            if let Some(sb) = supabase {
                let _ = sb
                    .update_liquidation(
                        &record_id,
                        &UpdateLiquidationResult {
                            status: LiquidationStatus::Failed.to_string(),
                            updated_at: db::now_iso(),
                            tx_signature: None,
                            error_message: Some(e.to_string()),
                            actual_profit_usd: None,
                            sol_fee_paid: None,
                            slot_submitted: None,
                            slot_confirmed: None,
                        },
                    )
                    .await;
            }
            Err(e)
        }
    }
}

fn build_setup_ixs(
    rpc: &RpcClient,
    liquidator: &Pubkey,
    debt_mint: &Pubkey,
    withdraw_pos: &protocols::DepositPosition,
) -> Result<Vec<Instruction>> {
    let mut ixs = Vec::new();

    let debt_ata = derive_ata(liquidator, debt_mint);
    if rpc.get_account(&debt_ata).is_err() {
        ixs.push(
            spl_associated_token_account::instruction::create_associated_token_account(
                liquidator,
                liquidator,
                debt_mint,
                &spl_token::id(),
            ),
        );
    }

    if let Some(col_mint) = withdraw_pos.mint {
        let col_ata = derive_ata(liquidator, &col_mint);
        if rpc.get_account(&col_ata).is_err() {
            ixs.push(
                spl_associated_token_account::instruction::create_associated_token_account(
                    liquidator,
                    liquidator,
                    &col_mint,
                    &spl_token::id(),
                ),
            );
        }
    }

    Ok(ixs)
}
