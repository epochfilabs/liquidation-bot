//! MarginFi v2 lending protocol.
//!
//! Program ID: `MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA`
//!
//! Each user has a `MarginfiAccount` containing up to 16 `Balance` entries.
//! Health = `sum(asset_value * weight) - sum(liability_value * weight)`, using
//! maintenance weights. Liquidation triggers when health drops below zero.
//!
//! Key difference from Kamino/Save: no pre-computed `deposited_value` /
//! `borrowed_value` fields — health must be computed by iterating balances and
//! looking up `Bank` accounts for weights and prices. For gRPC-based detection,
//! we use a simplified share-based check.

pub mod bank;
pub mod instructions;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use std::sync::LazyLock;

use super::{
    BorrowPosition, DepositPosition, HealthResult, LendingProtocol, LiquidationParams, Positions,
    ProtocolKind,
};
use crate::config::AppConfig;

pub const PROGRAM_ID: Pubkey = solana_sdk::pubkey!("MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA");

/// Anchor discriminator for `MarginfiAccount`.
static MARGINFI_ACCOUNT_DISCRIMINATOR: LazyLock<[u8; 8]> = LazyLock::new(|| {
    let hash = Sha256::digest(b"account:MarginfiAccount");
    let mut disc = [0u8; 8];
    disc.copy_from_slice(&hash[..8]);
    disc
});

/// `MarginfiAccount` layout (Anchor, zero_copy):
///   +0:   discriminator (8)
///   +8:   group (Pubkey, 32)
///   +40:  authority (Pubkey, 32)
///   +72:  lending_account.balances ([Balance; 16])
///
/// `Balance` (136 bytes):
///   +0:   active (u8)
///   +1:   bank_pk (Pubkey, 32)
///   +33:  pad0 (7)
///   +40:  asset_shares (WrappedI80F48 = i128, 16)
///   +56:  liability_shares (WrappedI80F48 = i128, 16)
///   +72:  emissions_outstanding (16)
///   +88:  last_update (u64, 8)
///   +96:  padding (40)
const GROUP_OFFSET: usize = 8;
const AUTHORITY_OFFSET: usize = 40;
const BALANCES_OFFSET: usize = 72;
const BALANCE_COUNT: usize = 16;
const BALANCE_SIZE: usize = 136;

const BALANCE_ACTIVE: usize = 0;
const BALANCE_BANK_PK: usize = 1;
const BALANCE_ASSET_SHARES: usize = 40;
const BALANCE_LIABILITY_SHARES: usize = 56;

const MIN_ACCOUNT_SIZE: usize = BALANCES_OFFSET + BALANCE_COUNT * BALANCE_SIZE;

#[derive(Debug, Default)]
pub struct MarginFiProtocol;

impl MarginFiProtocol {
    pub fn new() -> Self {
        Self
    }
}

impl LendingProtocol for MarginFiProtocol {
    fn kind(&self) -> ProtocolKind {
        ProtocolKind::MarginFi
    }

    fn program_id(&self) -> Pubkey {
        PROGRAM_ID
    }

    fn is_position_account(&self, data: &[u8]) -> bool {
        data.len() >= MIN_ACCOUNT_SIZE && data[..8] == *MARGINFI_ACCOUNT_DISCRIMINATOR
    }

    fn evaluate_health(&self, data: &[u8]) -> Result<HealthResult> {
        // Real health needs Bank lookups. Without them, we return a share-based
        // proxy and never auto-trigger — the caller is expected to do a detailed
        // on-chain check before acting.
        if data.len() < MIN_ACCOUNT_SIZE {
            bail!("marginfi account too small: {} bytes", data.len());
        }

        let (total_asset_shares, total_liability_shares, has_assets, has_liabilities) =
            balance_sums(data);

        let (current_ltv, is_liquidatable) = if has_assets && has_liabilities {
            let ratio = total_liability_shares / total_asset_shares.max(1e-18);
            (ratio, false)
        } else {
            (0.0, false)
        };

        Ok(HealthResult {
            current_ltv,
            unhealthy_ltv: 1.0,
            is_liquidatable,
            deposited_value_usd: total_asset_shares,
            borrowed_value_usd: total_liability_shares,
        })
    }

    fn parse_positions(&self, data: &[u8]) -> Result<Positions> {
        if data.len() < MIN_ACCOUNT_SIZE {
            bail!("marginfi account too small");
        }

        let group = read_pubkey(data, GROUP_OFFSET);
        let authority = read_pubkey(data, AUTHORITY_OFFSET);

        let mut deposits = Vec::new();
        let mut borrows = Vec::new();

        for i in 0..BALANCE_COUNT {
            let base = BALANCES_OFFSET + i * BALANCE_SIZE;
            if data[base + BALANCE_ACTIVE] == 0 {
                continue;
            }
            let bank = read_pubkey(data, base + BALANCE_BANK_PK);
            let asset_shares = read_i128(data, base + BALANCE_ASSET_SHARES);
            let liability_shares = read_i128(data, base + BALANCE_LIABILITY_SHARES);

            if asset_shares > 0 {
                deposits.push(DepositPosition {
                    reserve: bank,
                    mint: None,
                    amount: i80f48_to_f64(asset_shares) as u64,
                    market_value_usd: 0.0,
                });
            }
            if liability_shares > 0 {
                borrows.push(BorrowPosition {
                    reserve: bank,
                    mint: None,
                    amount_sf: liability_shares as u128,
                    market_value_usd: 0.0,
                });
            }
        }

        Ok(Positions {
            deposits,
            borrows,
            market: group,
            owner: authority,
        })
    }

    fn flash_loan_amount(&self, borrow: &BorrowPosition) -> u64 {
        // MarginFi stores liability shares — not directly a repay amount.
        // The real repay amount comes from `shares × liability_share_value`, which
        // requires the Bank account. The caller passes this through unchanged for
        // now; the executor's flash-loan selection operates on this value.
        borrow.amount_sf as u64
    }

    fn build_liquidate_ix(
        &self,
        rpc: &RpcClient,
        _cfg: &AppConfig,
        params: &LiquidationParams,
        liquidator: &Pubkey,
    ) -> Result<Instruction> {
        let asset_pos = params
            .positions
            .deposits
            .iter()
            .max_by(|a, b| a.market_value_usd.total_cmp(&b.market_value_usd))
            .ok_or_else(|| anyhow::anyhow!("no deposits for marginfi liquidation"))?;
        let liab_pos = params
            .positions
            .borrows
            .iter()
            .max_by(|a, b| a.market_value_usd.total_cmp(&b.market_value_usd))
            .ok_or_else(|| anyhow::anyhow!("no borrows for marginfi liquidation"))?;

        let liab_bank_account = rpc
            .get_account(&liab_pos.reserve)
            .context("failed to fetch marginfi liab bank")?;
        let liab_bank = bank::parse_bank(&liab_bank_account.data)?;

        // Ensure the asset bank exists (its fields aren't used here but absence
        // would make the liquidation ix invalid).
        let _asset_bank_account = rpc
            .get_account(&asset_pos.reserve)
            .context("failed to fetch marginfi asset bank")?;

        let (liab_vault_authority, _) =
            instructions::derive_liquidity_vault_authority(&liab_pos.reserve);

        let (liquidator_mfi_account, _) = Pubkey::find_program_address(
            &[
                b"marginfi_account",
                params.positions.market.as_ref(),
                liquidator.as_ref(),
            ],
            &PROGRAM_ID,
        );

        let remaining = vec![
            AccountMeta::new_readonly(asset_pos.reserve, false),
            AccountMeta::new_readonly(liab_pos.reserve, false),
            AccountMeta::new_readonly(asset_pos.reserve, false),
            AccountMeta::new_readonly(liab_pos.reserve, false),
        ];

        Ok(instructions::lending_account_liquidate(
            asset_pos.amount,
            &params.positions.market,
            &asset_pos.reserve,
            &liab_pos.reserve,
            &liquidator_mfi_account,
            liquidator,
            &params.position_pubkey,
            &liab_vault_authority,
            &liab_bank.liquidity_vault,
            &liab_bank.insurance_vault,
            &remaining,
        ))
    }
}

fn balance_sums(data: &[u8]) -> (f64, f64, bool, bool) {
    let mut total_assets = 0.0_f64;
    let mut total_liabilities = 0.0_f64;
    let mut has_assets = false;
    let mut has_liabilities = false;

    for i in 0..BALANCE_COUNT {
        let base = BALANCES_OFFSET + i * BALANCE_SIZE;
        if data[base + BALANCE_ACTIVE] == 0 {
            continue;
        }
        let asset_shares = read_i128(data, base + BALANCE_ASSET_SHARES);
        let liability_shares = read_i128(data, base + BALANCE_LIABILITY_SHARES);

        if asset_shares > 0 {
            has_assets = true;
            total_assets += i80f48_to_f64(asset_shares);
        }
        if liability_shares > 0 {
            has_liabilities = true;
            total_liabilities += i80f48_to_f64(liability_shares);
        }
    }

    (total_assets, total_liabilities, has_assets, has_liabilities)
}

fn read_pubkey(data: &[u8], offset: usize) -> Pubkey {
    Pubkey::new_from_array(data[offset..offset + 32].try_into().expect("pubkey slice"))
}

fn read_i128(data: &[u8], offset: usize) -> i128 {
    i128::from_le_bytes(data[offset..offset + 16].try_into().expect("i128 slice"))
}

fn i80f48_to_f64(val: i128) -> f64 {
    val as f64 / (1i128 << 48) as f64
}
