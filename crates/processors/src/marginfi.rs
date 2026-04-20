//! MarginFi v2 processor.
//!
//! Scans for `lendingAccountLiquidate` in both top-level and inner
//! instructions. MarginFi's liquidation arg is `asset_amount` (collateral),
//! not debt. Repay amount must be derived from balance deltas.

use anyhow::Result;

use indexer_core::enrichment;
use indexer_core::events::{FailedLiquidationEvent, LiquidationEvent, ProcessedTransaction};
use marginfi_v2_decoder::instructions as mfi_ix;

use crate::common::{block_time_to_dt, collect_program_instructions, get_enrichment};
use crate::TxContext;

pub fn process(ctx: &TxContext, result: &mut ProcessedTransaction) -> Result<()> {
    let marginfi_program = *marginfi_v2_decoder::PROGRAM_ID;
    let enrichment = get_enrichment(ctx);

    let resolved = collect_program_instructions(ctx, &marginfi_program);

    for rix in &resolved {
        let decoded = match mfi_ix::decode(&rix.data, &rix.accounts) {
            Ok(Some(ix)) => ix,
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!(
                    tx = %ctx.tx_signature,
                    ix_index = rix.top_level_index,
                    inner = ?rix.inner_index,
                    error = %e,
                    "failed to decode marginfi instruction"
                );
                continue;
            }
        };

        if !decoded.is_liquidation() {
            continue;
        }

        let block_time = block_time_to_dt(ctx.block_time_unix);

        let event = match &decoded {
            mfi_ix::MarginfiInstruction::Liquidate { args, accounts: accts } => {
                LiquidationEvent {
                    venue: "marginfi".to_string(),
                    program_id: marginfi_program.to_string(),
                    slot: ctx.slot,
                    block_time,
                    tx_signature: ctx.tx_signature.clone(),
                    ix_index: rix.top_level_index,
                    inner_ix_index: rix.inner_index,

                    liquidator: accts.authority.to_string(),
                    liquidatee: None,
                    obligation: accts.liquidatee_marginfi_account.to_string(),
                    market: accts.marginfi_group.to_string(),

                    collateral_reserve: accts.asset_bank.to_string(),
                    debt_reserve: accts.liab_bank.to_string(),
                    collateral_mint: String::new(),
                    debt_mint: String::new(),
                    repay_amount: 0,
                    withdraw_amount: args.asset_amount as u128,

                    repay_amount_usd: None,
                    collateral_seized_usd: None,
                    liquidator_profit_usd: None,
                    collateral_price: None,
                    debt_price: None,
                    obligation_deposited_usd: None,
                    obligation_borrowed_usd: None,
                    liquidation_bonus_bps: Some(500),
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
                    absorbed_bad_debt: None,

                    raw_ix_data: rix.data.iter().map(|b| format!("{:02x}", b)).collect(),
                }
            }
            _ => continue,
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
