//! Kamino Lend (klend) protocol.
//!
//! Implements [`LendingProtocol`] over raw `Obligation` account bytes — the
//! submodules cover discriminator detection, health evaluation, position
//! parsing, reserve/market parsing, and on-chain instruction encoding.

pub mod decoder;
pub mod health;
pub mod instructions;
pub mod positions;
pub mod reserve;

use anyhow::{Context, Result};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey};

use super::{
    BorrowPosition, DepositPosition, HealthResult, LendingProtocol, LiquidationParams, Positions,
    ProtocolKind,
};
use crate::config::AppConfig;

/// Kamino Lend program ID (mainnet production).
pub const PROGRAM_ID: Pubkey = instructions::KLEND_PROGRAM_ID;

/// Scaled-fraction shift (2^60) — all `_sf` fields are `value * 2^60`.
const SF_SHIFT: u128 = 1u128 << 60;
const SF_SHIFT_F64: f64 = SF_SHIFT as f64;

#[derive(Debug, Default)]
pub struct KaminoProtocol;

impl KaminoProtocol {
    pub fn new() -> Self {
        Self
    }
}

impl LendingProtocol for KaminoProtocol {
    fn kind(&self) -> ProtocolKind {
        ProtocolKind::Kamino
    }

    fn program_id(&self) -> Pubkey {
        PROGRAM_ID
    }

    fn is_position_account(&self, data: &[u8]) -> bool {
        data.len() == health::OBLIGATION_ACCOUNT_SIZE && decoder::is_obligation_account(data)
    }

    fn evaluate_health(&self, data: &[u8]) -> Result<HealthResult> {
        let h = health::evaluate(data)?;
        Ok(HealthResult {
            current_ltv: h.current_ltv,
            unhealthy_ltv: h.unhealthy_ltv,
            is_liquidatable: h.is_liquidatable,
            deposited_value_usd: h.deposited_value_sf as f64 / SF_SHIFT_F64,
            borrowed_value_usd: h.borrow_factor_adjusted_debt_value_sf as f64 / SF_SHIFT_F64,
        })
    }

    fn parse_positions(&self, data: &[u8]) -> Result<Positions> {
        let p = positions::parse_positions(data)?;
        Ok(Positions {
            deposits: p
                .deposits
                .into_iter()
                .map(|d| DepositPosition {
                    reserve: d.reserve,
                    mint: None,
                    amount: d.deposited_amount,
                    market_value_usd: d.market_value_sf as f64 / SF_SHIFT_F64,
                })
                .collect(),
            borrows: p
                .borrows
                .into_iter()
                .map(|b| BorrowPosition {
                    reserve: b.reserve,
                    mint: None,
                    amount_sf: b.borrowed_amount_sf,
                    market_value_usd: b.market_value_sf as f64 / SF_SHIFT_F64,
                })
                .collect(),
            market: p.lending_market,
            owner: p.owner,
        })
    }

    fn flash_loan_amount(&self, borrow: &BorrowPosition) -> u64 {
        (borrow.amount_sf / SF_SHIFT) as u64
    }

    fn build_liquidate_ix(
        &self,
        rpc: &RpcClient,
        cfg: &AppConfig,
        params: &LiquidationParams,
        liquidator: &Pubkey,
    ) -> Result<Instruction> {
        let repay_pos = params
            .positions
            .borrows
            .iter()
            .max_by(|a, b| a.market_value_usd.total_cmp(&b.market_value_usd))
            .ok_or_else(|| anyhow::anyhow!("no borrows for kamino liquidation"))?;
        let withdraw_pos = params
            .positions
            .deposits
            .iter()
            .max_by(|a, b| a.market_value_usd.total_cmp(&b.market_value_usd))
            .ok_or_else(|| anyhow::anyhow!("no deposits for kamino liquidation"))?;

        let repay_reserve_acct = rpc
            .get_account(&repay_pos.reserve)
            .context("failed to fetch repay reserve")?;
        let repay_reserve =
            reserve::parse_reserve(&repay_pos.reserve, &repay_reserve_acct.data)?;

        let withdraw_reserve_acct = rpc
            .get_account(&withdraw_pos.reserve)
            .context("failed to fetch withdraw reserve")?;
        let withdraw_reserve =
            reserve::parse_reserve(&withdraw_pos.reserve, &withdraw_reserve_acct.data)?;

        let market_acct = rpc
            .get_account(&params.positions.market)
            .context("failed to fetch lending market")?;
        let market = reserve::parse_lending_market(&market_acct.data)?;

        let program_id = cfg.klend_program_id;
        let (lending_market_authority, _) = instructions::derive_lending_market_authority(
            &params.positions.market,
            &program_id,
        );

        let borrowed_amount = self.flash_loan_amount(repay_pos);
        let close_factor = u64::from(market.liquidation_max_debt_close_factor_pct);
        let repay_amount =
            (borrowed_amount * close_factor / 100).min(repay_reserve.available_liquidity);

        let liquidator_repay_ata = crate::liquidator::executor::derive_ata(
            liquidator,
            &repay_reserve.accounts.liquidity_mint,
        );
        let liquidator_collateral_ata = crate::liquidator::executor::derive_ata(
            liquidator,
            &withdraw_reserve.accounts.collateral_mint,
        );
        let liquidator_withdraw_ata = crate::liquidator::executor::derive_ata(
            liquidator,
            &withdraw_reserve.accounts.liquidity_mint,
        );

        Ok(instructions::liquidate_obligation_and_redeem_reserve_collateral(
            &program_id,
            &instructions::LiquidateParams {
                liquidity_amount: repay_amount,
                min_acceptable_received_liquidity_amount: 0,
            },
            liquidator,
            &params.position_pubkey,
            &params.positions.market,
            &lending_market_authority,
            &repay_reserve.accounts,
            &withdraw_reserve.accounts,
            &liquidator_repay_ata,
            &liquidator_collateral_ata,
            &liquidator_withdraw_ata,
        ))
    }
}
