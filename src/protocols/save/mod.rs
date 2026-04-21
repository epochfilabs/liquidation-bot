//! Save (formerly Solend) lending protocol.
//!
//! Program ID: `So1endDq2YkqhipRh3WViPa8hFvz0XP1PV7qidbGAiN`
//!
//! Save uses the SPL token-lending layout (no Anchor discriminator). Obligations
//! store `deposited_value`, `borrowed_value`, and `unhealthy_borrow_value` as
//! WAD-scaled (10^18) u128 values.

pub mod instructions;

use anyhow::{Result, bail};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey};

use super::{
    BorrowPosition, DepositPosition, HealthResult, LendingProtocol, LiquidationParams, Positions,
    ProtocolKind,
};
use crate::config::AppConfig;

pub const PROGRAM_ID: Pubkey = solana_sdk::pubkey!("SLendK7ySfcEzyaFqy93gDnD3RtrpXJcnRwb6zFHJSh");

/// WAD scale factor used by Save (10^18).
const WAD: u128 = 1_000_000_000_000_000_000;

/// Save uses a 50% close factor for liquidations.
const CLOSE_FACTOR_DIVISOR: u64 = 2;

/// Save Obligation layout (no 8-byte Anchor discriminator).
///
/// Offsets:
///   +0:    version (u8)
///   +1:    last_update.slot (u64)
///   +9:    last_update.stale (u8)
///   +10:   lending_market (Pubkey, 32)
///   +42:   owner (Pubkey, 32)
///   +74:   deposited_value (u128 WAD, 16)
///   +90:   borrowed_value (u128 WAD, 16)
///   +106:  allowed_borrow_value (u128 WAD, 16)
///   +122:  unhealthy_borrow_value (u128 WAD, 16)
///   +138:  super_unhealthy_borrow_value (u128 WAD, 16) — Solend addition
///   +154:  borrowing_isolated_asset (u8)
///   +155:  deposits_len (u8)
///   +156:  borrows_len (u8)
///   +157:  data_flat start (deposits then borrows)
///
/// ObligationCollateral (56 bytes): reserve(32) + amount(u64) + market_value_wad(u128)
/// ObligationLiquidity  (80 bytes): reserve(32) + cumulative_rate(u128) +
///                                  borrowed_amount_wads(u128) + market_value_wad(u128)
mod offsets {
    pub const LENDING_MARKET: usize = 10;
    pub const OWNER: usize = 42;
    pub const DEPOSITED_VALUE: usize = 74;
    pub const BORROWED_VALUE: usize = 90;
    pub const UNHEALTHY_BORROW_VALUE: usize = 122;
    pub const DEPOSITS_LEN: usize = 155;
    pub const BORROWS_LEN: usize = 156;
    pub const DATA_FLAT: usize = 157;
}

const COLLATERAL_SIZE: usize = 56;
const LIQUIDITY_SIZE: usize = 80;

#[derive(Debug, Default)]
pub struct SaveProtocol;

impl SaveProtocol {
    pub fn new() -> Self {
        Self
    }
}

impl LendingProtocol for SaveProtocol {
    fn kind(&self) -> ProtocolKind {
        ProtocolKind::Save
    }

    fn program_id(&self) -> Pubkey {
        PROGRAM_ID
    }

    fn is_position_account(&self, data: &[u8]) -> bool {
        if data.len() < offsets::DATA_FLAT {
            return false;
        }
        let version = data[0];
        let deposits_len = data[offsets::DEPOSITS_LEN] as usize;
        let borrows_len = data[offsets::BORROWS_LEN] as usize;
        (1..=3).contains(&version)
            && deposits_len <= 10
            && borrows_len <= 10
            && data.len()
                >= offsets::DATA_FLAT
                    + deposits_len * COLLATERAL_SIZE
                    + borrows_len * LIQUIDITY_SIZE
    }

    fn evaluate_health(&self, data: &[u8]) -> Result<HealthResult> {
        if data.len() < offsets::UNHEALTHY_BORROW_VALUE + 16 {
            bail!("save obligation data too small: {} bytes", data.len());
        }

        let deposited_value = read_u128(data, offsets::DEPOSITED_VALUE);
        let borrowed_value = read_u128(data, offsets::BORROWED_VALUE);
        let unhealthy_borrow_value = read_u128(data, offsets::UNHEALTHY_BORROW_VALUE);

        let dep_usd = deposited_value as f64 / WAD as f64;
        let bor_usd = borrowed_value as f64 / WAD as f64;
        let unhealthy_usd = unhealthy_borrow_value as f64 / WAD as f64;

        let (current_ltv, unhealthy_ltv, is_liquidatable) = if dep_usd == 0.0 {
            if bor_usd > 0.0 {
                (f64::INFINITY, 0.0, true)
            } else {
                (0.0, 0.0, false)
            }
        } else {
            let ltv = bor_usd / dep_usd;
            let u_ltv = unhealthy_usd / dep_usd;
            let liq = borrowed_value > unhealthy_borrow_value && borrowed_value > 0;
            (ltv, u_ltv, liq)
        };

        Ok(HealthResult {
            current_ltv,
            unhealthy_ltv,
            is_liquidatable,
            deposited_value_usd: dep_usd,
            borrowed_value_usd: bor_usd,
        })
    }

    fn parse_positions(&self, data: &[u8]) -> Result<Positions> {
        if data.len() < offsets::DATA_FLAT {
            bail!("save obligation data too small");
        }

        let lending_market = read_pubkey(data, offsets::LENDING_MARKET);
        let owner = read_pubkey(data, offsets::OWNER);
        let deposits_len = data[offsets::DEPOSITS_LEN] as usize;
        let borrows_len = data[offsets::BORROWS_LEN] as usize;

        let deposits = (0..deposits_len)
            .filter_map(|i| {
                let base = offsets::DATA_FLAT + i * COLLATERAL_SIZE;
                if base + COLLATERAL_SIZE > data.len() {
                    return None;
                }
                let reserve = read_pubkey(data, base);
                let amount = read_u64(data, base + 32);
                if reserve == Pubkey::default() || amount == 0 {
                    return None;
                }
                Some(DepositPosition {
                    reserve,
                    mint: None,
                    amount,
                    market_value_usd: read_u128(data, base + 40) as f64 / WAD as f64,
                })
            })
            .collect();

        let borrows_start = offsets::DATA_FLAT + deposits_len * COLLATERAL_SIZE;
        let borrows = (0..borrows_len)
            .filter_map(|i| {
                let base = borrows_start + i * LIQUIDITY_SIZE;
                if base + LIQUIDITY_SIZE > data.len() {
                    return None;
                }
                let reserve = read_pubkey(data, base);
                let borrowed_amount_wads = read_u128(data, base + 48);
                if reserve == Pubkey::default() || borrowed_amount_wads == 0 {
                    return None;
                }
                Some(BorrowPosition {
                    reserve,
                    mint: None,
                    amount_sf: borrowed_amount_wads,
                    market_value_usd: read_u128(data, base + 64) as f64 / WAD as f64,
                })
            })
            .collect();

        Ok(Positions {
            deposits,
            borrows,
            market: lending_market,
            owner,
        })
    }

    fn flash_loan_amount(&self, borrow: &BorrowPosition) -> u64 {
        (borrow.amount_sf / WAD) as u64
    }

    fn build_liquidate_ix(
        &self,
        rpc: &RpcClient,
        _cfg: &AppConfig,
        params: &LiquidationParams,
        liquidator: &Pubkey,
    ) -> Result<Instruction> {
        use anyhow::Context;

        let repay_pos = params
            .positions
            .borrows
            .iter()
            .max_by(|a, b| a.market_value_usd.total_cmp(&b.market_value_usd))
            .ok_or_else(|| anyhow::anyhow!("no borrows for save liquidation"))?;
        let withdraw_pos = params
            .positions
            .deposits
            .iter()
            .max_by(|a, b| a.market_value_usd.total_cmp(&b.market_value_usd))
            .ok_or_else(|| anyhow::anyhow!("no deposits for save liquidation"))?;

        let repay_reserve_data = rpc
            .get_account(&repay_pos.reserve)
            .context("failed to fetch save repay reserve")?;
        let withdraw_reserve_data = rpc
            .get_account(&withdraw_pos.reserve)
            .context("failed to fetch save withdraw reserve")?;

        let repay_reserve = parse_reserve_accounts(&repay_pos.reserve, &repay_reserve_data.data)?;
        let withdraw_reserve =
            parse_reserve_accounts(&withdraw_pos.reserve, &withdraw_reserve_data.data)?;

        let (lending_market_authority, _) =
            instructions::derive_lending_market_authority(&params.positions.market);

        let repay_amount = self.flash_loan_amount(repay_pos) / CLOSE_FACTOR_DIVISOR;

        let liquidator_repay_ata =
            crate::liquidator::executor::derive_ata(liquidator, &repay_reserve.liquidity_mint);
        let liquidator_collateral_ata = crate::liquidator::executor::derive_ata(
            liquidator,
            &withdraw_reserve.collateral_mint,
        );
        let liquidator_withdraw_ata = crate::liquidator::executor::derive_ata(
            liquidator,
            &withdraw_reserve.liquidity_mint,
        );

        Ok(instructions::liquidate_obligation_and_redeem(
            repay_amount,
            &liquidator_repay_ata,
            &liquidator_collateral_ata,
            &liquidator_withdraw_ata,
            &repay_reserve,
            &withdraw_reserve,
            &params.position_pubkey,
            &params.positions.market,
            &lending_market_authority,
            liquidator,
        ))
    }
}

fn parse_reserve_accounts(
    reserve_pubkey: &Pubkey,
    data: &[u8],
) -> Result<instructions::SaveReserveAccounts> {
    if data.len() < 300 {
        bail!("save reserve too small: {} bytes", data.len());
    }
    Ok(instructions::SaveReserveAccounts {
        reserve: *reserve_pubkey,
        liquidity_mint: read_pubkey(data, 42),
        liquidity_supply: read_pubkey(data, 74),
        liquidity_fee_receiver: read_pubkey(data, 106),
        collateral_mint: read_pubkey(data, 226),
        collateral_supply: read_pubkey(data, 258),
        token_program: instructions::SPL_TOKEN_PROGRAM,
    })
}

fn read_pubkey(data: &[u8], offset: usize) -> Pubkey {
    Pubkey::new_from_array(data[offset..offset + 32].try_into().expect("pubkey slice"))
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().expect("u64 slice"))
}

fn read_u128(data: &[u8], offset: usize) -> u128 {
    u128::from_le_bytes(data[offset..offset + 16].try_into().expect("u128 slice"))
}
