//! Save (formerly Solend) protocol implementation.
//!
//! Program ID: So1endDq2YkqhipRh3WViPa8hFvz0XP1PV7qidbGAiN
//!
//! Save uses a similar architecture to Kamino (both derived from Solana token-lending).
//! Obligation accounts store deposited_value, borrowed_value, and unhealthy_borrow_value
//! as Decimal (u128 scaled by WAD = 10^18).

use anyhow::{bail, Result};
use solana_sdk::pubkey::Pubkey;

use super::{
    BorrowPosition, DepositPosition, HealthResult, LendingProtocol, Positions, ProtocolKind,
};

pub const PROGRAM_ID: &str = "SLendK7ySfcEzyaFqy93gDnD3RtrpXJcnRwb6zFHJSh";

/// WAD scale factor used by Save (10^18).
const WAD: u128 = 1_000_000_000_000_000_000;

/// Save Obligation account layout (no Anchor discriminator — uses Solana Program Library layout).
///
/// The Obligation struct from solend-token-lending:
///   version:                  u8    (1)
///   last_update:              LastUpdate (9 bytes: slot u64 + stale u8)
///   lending_market:           Pubkey (32)
///   owner:                    Pubkey (32)
///   deposited_value:          Decimal (u128 = 16, WAD-scaled USD value)
///   borrowed_value:           Decimal (u128 = 16, WAD-scaled USD value)
///   allowed_borrow_value:     Decimal (u128 = 16)
///   unhealthy_borrow_value:   Decimal (u128 = 16)
///   deposits_len:             u8
///   borrows_len:              u8
///   data_flat:                variable (deposits then borrows)
///
/// Offsets (no 8-byte discriminator — SPL programs don't use Anchor):
///   0:    version (1)
///   1:    last_update.slot (8)
///   9:    last_update.stale (1)
///   10:   lending_market (32)
///   42:   owner (32)
///   74:   deposited_value (16)
///   90:   borrowed_value (16)
///   106:  allowed_borrow_value (16)
///   122:  unhealthy_borrow_value (16)
///   138:  super_unhealthy_borrow_value (16)  -- added by Solend
///   154:  borrowing_isolated_asset (1)
///   155:  deposits_len (1)
///   156:  borrows_len (1)
///   157:  data_flat start
///
/// ObligationCollateral (56 bytes each):
///   0:  deposit_reserve (32)
///   32: deposited_amount (8)
///   40: market_value (16, Decimal)
///
/// ObligationLiquidity (80 bytes each):
///   0:  borrow_reserve (32)
///   32: cumulative_borrow_rate_wads (16, Decimal)
///   48: borrowed_amount_wads (16, Decimal)
///   64: market_value (16, Decimal)
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

pub struct SaveProtocol {
    pub program_id: Pubkey,
}

impl SaveProtocol {
    pub fn new() -> Self {
        Self {
            program_id: PROGRAM_ID.parse().unwrap(),
        }
    }
}

impl LendingProtocol for SaveProtocol {
    fn kind(&self) -> ProtocolKind {
        ProtocolKind::Save
    }

    fn program_id(&self) -> Pubkey {
        self.program_id
    }

    fn is_position_account(&self, data: &[u8]) -> bool {
        // Save obligations: version byte = 1 (for obligation), minimum size check
        if data.len() < offsets::DATA_FLAT {
            return false;
        }
        // version should be a small number (1-3), and deposits_len + borrows_len should be reasonable
        let version = data[0];
        let deposits_len = data[offsets::DEPOSITS_LEN] as usize;
        let borrows_len = data[offsets::BORROWS_LEN] as usize;
        version > 0
            && version <= 3
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
            // Save triggers liquidation when borrowed_value > unhealthy_borrow_value
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

        let mut deposits = Vec::new();
        for i in 0..deposits_len {
            let base = offsets::DATA_FLAT + i * COLLATERAL_SIZE;
            if base + COLLATERAL_SIZE > data.len() {
                break;
            }
            let reserve = read_pubkey(data, base);
            if reserve == Pubkey::default() {
                continue;
            }
            let amount = read_u64(data, base + 32);
            let market_value = read_u128(data, base + 40);
            if amount > 0 {
                deposits.push(DepositPosition {
                    reserve,
                    mint: None,
                    amount,
                    market_value_usd: market_value as f64 / WAD as f64,
                });
            }
        }

        let borrows_start = offsets::DATA_FLAT + deposits_len * COLLATERAL_SIZE;
        let mut borrows = Vec::new();
        for i in 0..borrows_len {
            let base = borrows_start + i * LIQUIDITY_SIZE;
            if base + LIQUIDITY_SIZE > data.len() {
                break;
            }
            let reserve = read_pubkey(data, base);
            if reserve == Pubkey::default() {
                continue;
            }
            let borrowed_amount_wads = read_u128(data, base + 48);
            let market_value = read_u128(data, base + 64);
            if borrowed_amount_wads > 0 {
                borrows.push(BorrowPosition {
                    reserve,
                    mint: None,
                    amount_sf: borrowed_amount_wads,
                    market_value_usd: market_value as f64 / WAD as f64,
                });
            }
        }

        Ok(Positions {
            deposits,
            borrows,
            market: lending_market,
            owner,
        })
    }
}

fn read_pubkey(data: &[u8], offset: usize) -> Pubkey {
    Pubkey::new_from_array(data[offset..offset + 32].try_into().unwrap())
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

fn read_u128(data: &[u8], offset: usize) -> u128 {
    u128::from_le_bytes(data[offset..offset + 16].try_into().unwrap())
}
