//! Jupiter Lend processor.
//!
//! Scans for Jupiter Lend Vaults `liquidate` instructions in both
//! top-level and inner instructions. `liquidatee` is always NULL
//! (tick-based liquidation).

use anyhow::Result;

use indexer_core::enrichment;
use indexer_core::events::{FailedLiquidationEvent, LiquidationEvent, ProcessedTransaction};
use jupiter_lend_vaults_decoder::instructions as vaults_ix;

use crate::common::{block_time_to_dt, collect_program_instructions, get_enrichment};
use crate::TxContext;

pub fn process(ctx: &TxContext, result: &mut ProcessedTransaction) -> Result<()> {
    let vaults_program = *jupiter_lend_vaults_decoder::PROGRAM_ID;
    let enrichment = get_enrichment(ctx);

    let resolved = collect_program_instructions(ctx, &vaults_program);

    for rix in &resolved {
        let decoded = match vaults_ix::decode(&rix.data, &rix.accounts) {
            Ok(Some(ix)) => ix,
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!(
                    tx = %ctx.tx_signature,
                    ix_index = rix.top_level_index,
                    inner = ?rix.inner_index,
                    error = %e,
                    "failed to decode jupiter lend vaults instruction"
                );
                continue;
            }
        };

        let block_time = block_time_to_dt(ctx.block_time_unix);

        let event = match &decoded {
            vaults_ix::VaultsInstruction::Liquidate { args, accounts: accts } => {
                LiquidationEvent {
                    venue: "jupiter_lend".to_string(),
                    program_id: vaults_program.to_string(),
                    slot: ctx.slot,
                    block_time,
                    tx_signature: ctx.tx_signature.clone(),
                    ix_index: rix.top_level_index,
                    inner_ix_index: rix.inner_index,

                    liquidator: accts.signer.to_string(),
                    liquidatee: None,
                    obligation: accts.vault_config.to_string(),
                    market: accts.vault_config.to_string(),

                    collateral_reserve: accts.vault_config.to_string(),
                    debt_reserve: accts.vault_config.to_string(),
                    collateral_mint: accts.supply_token.to_string(),
                    debt_mint: accts.borrow_token.to_string(),
                    repay_amount: args.debt_amt as u128,
                    withdraw_amount: 0,

                    repay_amount_usd: None,
                    collateral_seized_usd: None,
                    liquidator_profit_usd: None,
                    collateral_price: None,
                    debt_price: None,
                    obligation_deposited_usd: None,
                    obligation_borrowed_usd: None,
                    liquidation_bonus_bps: None,
                    close_factor_pct: None,
                    protocol_fee_amount: None,
                    insurance_fee_amount: None,

                    tx_fee_lamports: ctx.fee_lamports,
                    priority_fee_lamports: enrichment.priority_fee_lamports,
                    jito_tip_lamports: enrichment.jito_tip_lamports,
                    compute_units_consumed: ctx.compute_units_consumed,

                    used_flashloan: enrichment.used_flashloan,
                    flashloan_source: enrichment.flashloan_source.clone(),
                    used_jupiter_swap: enrichment.used_jupiter_swap,

                    liquidation_reason: None,
                    tick_start: None,
                    tick_end: None,
                    absorbed_bad_debt: Some(args.absorb),

                    raw_ix_data: rix.data.iter().map(|b| format!("{:02x}", b)).collect(),
                }
            }
        };

        if ctx.succeeded {
            result.liquidations.push(event);
        } else {
            let (error_code, error_message) =
                enrichment::parse_error_from_logs(&ctx.log_messages);
            result.failed_attempts.push(FailedLiquidationEvent {
                base: event,
                error_code,
                error_message,
            });
        }
    }

    Ok(())
}
