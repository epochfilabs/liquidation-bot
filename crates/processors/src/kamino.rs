//! Kamino Lend processor.
//!
//! Scans for klend liquidation instructions (v1 and v2) in both top-level
//! and inner instructions (CPI). Real-world liquidators often invoke klend
//! through wrapper programs.

use anyhow::Result;

use indexer_core::enrichment;
use indexer_core::events::{FailedLiquidationEvent, LiquidationEvent, ProcessedTransaction};
use klend_decoder::instructions as klend_ix;

use crate::common::{self, block_time_to_dt, collect_program_instructions, get_enrichment};
use crate::TxContext;

pub fn process(ctx: &TxContext, result: &mut ProcessedTransaction) -> Result<()> {
    let klend_program = *klend_decoder::PROGRAM_ID;
    let enrichment = get_enrichment(ctx);

    let resolved = collect_program_instructions(ctx, &klend_program);

    for rix in &resolved {
        let decoded = match klend_ix::decode(&rix.data, &rix.accounts) {
            Ok(Some(ix)) => ix,
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!(
                    tx = %ctx.tx_signature,
                    ix_index = rix.top_level_index,
                    inner = ?rix.inner_index,
                    error = %e,
                    "failed to decode klend instruction"
                );
                continue;
            }
        };

        if !decoded.is_liquidation() {
            continue;
        }

        let block_time = block_time_to_dt(ctx.block_time_unix);
        let raw_ix_data = rix.data.iter().map(|b| format!("{:02x}", b)).collect::<String>();

        let event = LiquidationEvent {
            venue: "kamino".to_string(),
            program_id: klend_program.to_string(),
            slot: ctx.slot,
            block_time,
            tx_signature: ctx.tx_signature.clone(),
            ix_index: rix.top_level_index,
            inner_ix_index: rix.inner_index,

            liquidator: decoded.liquidator().map(|p| p.to_string()).unwrap_or_default(),
            liquidatee: None,
            obligation: decoded.obligation().map(|p| p.to_string()).unwrap_or_default(),
            market: decoded.lending_market().map(|p| p.to_string()).unwrap_or_default(),

            collateral_reserve: match &decoded {
                klend_ix::KlendInstruction::LiquidateV1 { accounts, .. }
                | klend_ix::KlendInstruction::LiquidateV2 { accounts, .. } => {
                    accounts.withdraw_reserve.to_string()
                }
                _ => String::new(),
            },
            debt_reserve: match &decoded {
                klend_ix::KlendInstruction::LiquidateV1 { accounts, .. }
                | klend_ix::KlendInstruction::LiquidateV2 { accounts, .. } => {
                    accounts.repay_reserve.to_string()
                }
                _ => String::new(),
            },
            collateral_mint: match &decoded {
                klend_ix::KlendInstruction::LiquidateV1 { accounts, .. }
                | klend_ix::KlendInstruction::LiquidateV2 { accounts, .. } => {
                    accounts.withdraw_reserve_liquidity_mint.to_string()
                }
                _ => String::new(),
            },
            debt_mint: match &decoded {
                klend_ix::KlendInstruction::LiquidateV1 { accounts, .. }
                | klend_ix::KlendInstruction::LiquidateV2 { accounts, .. } => {
                    accounts.repay_reserve_liquidity_mint.to_string()
                }
                _ => String::new(),
            },
            repay_amount: decoded.liquidity_amount() as u128,
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

            liquidation_reason: Some("ltv_exceeded".to_string()),
            tick_start: None,
            tick_end: None,
            absorbed_bad_debt: None,

            raw_ix_data,
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
