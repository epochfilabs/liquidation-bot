//! Save (Solend) processor.
//!
//! Scans for Save liquidation instructions (tag 12 and tag 17) in both
//! top-level and inner instructions. Real-world Save liquidators invoke
//! the program via CPI through wrapper programs.

use anyhow::Result;

use indexer_core::enrichment;
use indexer_core::events::{FailedLiquidationEvent, LiquidationEvent, ProcessedTransaction};
use save_decoder::instructions as save_ix;

use crate::common::{block_time_to_dt, collect_program_instructions, get_enrichment};
use crate::TxContext;

pub fn process(ctx: &TxContext, result: &mut ProcessedTransaction) -> Result<()> {
    let save_program = *save_decoder::PROGRAM_ID;
    let enrichment = get_enrichment(ctx);

    let resolved = collect_program_instructions(ctx, &save_program);

    for rix in &resolved {
        let decoded = match save_ix::decode(&rix.data, &rix.accounts) {
            Ok(Some(ix)) => ix,
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!(
                    tx = %ctx.tx_signature,
                    ix_index = rix.top_level_index,
                    inner = ?rix.inner_index,
                    error = %e,
                    "failed to decode save instruction"
                );
                continue;
            }
        };

        if !decoded.is_liquidation() {
            continue;
        }

        let block_time = block_time_to_dt(ctx.block_time_unix);

        let (liquidator, obligation, market, collateral_reserve, debt_reserve, repay_amount) =
            match &decoded {
                save_ix::SaveInstruction::LiquidateObligation(liq) => (
                    liq.accounts.user_transfer_authority.to_string(),
                    liq.accounts.obligation.to_string(),
                    liq.accounts.lending_market.to_string(),
                    liq.accounts.withdraw_reserve.to_string(),
                    liq.accounts.repay_reserve.to_string(),
                    liq.liquidity_amount as u128,
                ),
                save_ix::SaveInstruction::LiquidateObligationAndRedeem(liq) => (
                    liq.accounts.user_transfer_authority.to_string(),
                    liq.accounts.obligation.to_string(),
                    liq.accounts.lending_market.to_string(),
                    liq.accounts.withdraw_reserve.to_string(),
                    liq.accounts.repay_reserve.to_string(),
                    liq.liquidity_amount as u128,
                ),
                _ => continue,
            };

        let event = LiquidationEvent {
            venue: "save".to_string(),
            program_id: save_program.to_string(),
            slot: ctx.slot,
            block_time,
            tx_signature: ctx.tx_signature.clone(),
            ix_index: rix.top_level_index,
            inner_ix_index: rix.inner_index,

            liquidator,
            liquidatee: None,
            obligation,
            market,

            collateral_reserve,
            debt_reserve,
            collateral_mint: String::new(),
            debt_mint: String::new(),
            repay_amount,
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
            absorbed_bad_debt: None,

            raw_ix_data: rix.data.iter().map(|b| format!("{:02x}", b)).collect(),
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
