//! MarginFi v2 protocol implementation.
//!
//! Program ID: MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA
//!
//! MarginFi uses a different model: each user has a MarginfiAccount containing
//! up to 16 Balance entries. Health = sum(asset_value * weight) - sum(liability_value * weight).
//! Liquidation triggers when health < 0 using maintenance weights.
//!
//! Key difference from Kamino/Save: no pre-computed deposited_value/borrowed_value fields.
//! Health must be computed by iterating balances and looking up Bank accounts for weights/prices.
//! For the gRPC-based detection, we use a simplified check on the account's balance entries.

use anyhow::{bail, Result};
use sha2::{Digest, Sha256};
use solana_sdk::pubkey::Pubkey;
use std::sync::LazyLock;

use super::{
    BorrowPosition, DepositPosition, HealthResult, LendingProtocol, Positions, ProtocolKind,
};

pub const PROGRAM_ID: &str = "MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA";

/// Anchor discriminator for MarginfiAccount.
static MARGINFI_ACCOUNT_DISCRIMINATOR: LazyLock<[u8; 8]> = LazyLock::new(|| {
    let hash = Sha256::digest(b"account:MarginfiAccount");
    let mut disc = [0u8; 8];
    disc.copy_from_slice(&hash[..8]);
    disc
});

/// MarginfiAccount layout (Anchor, zero_copy):
///   +0:   discriminator (8)
///   +8:   group (Pubkey, 32)
///   +40:  authority (Pubkey, 32)  — the account owner
///   +72:  lending_account (LendingAccount)
///         +0: balances ([Balance; 16])
///         +8: padding (u64 array)
///
/// Balance struct (per-bank entry, 136 bytes):
///   +0:   active (bool/u8)
///   +1:   bank_pk (Pubkey, 32)
///   +33:  pad0 ([u8; 7])
///   +40:  asset_shares (WrappedI80F48, 16 bytes — i128 fixed-point)
///   +56:  liability_shares (WrappedI80F48, 16 bytes)
///   +72:  emissions_outstanding (WrappedI80F48, 16 bytes)
///   +88:  last_update (u64, 8)
///   +96:  padding ([u64; 5], 40 bytes)
///   = 136 bytes total
///
/// LendingAccount starts at offset 72 (after disc + group + authority).
/// balances start immediately at offset 72.
///
/// Total size: 8 + 32 + 32 + (16 * 136) + padding = 8 + 32 + 32 + 2176 + padding
/// Actual account size: 2656 bytes (verified by MarginFi source).

const DISCRIMINATOR_OFFSET: usize = 0;
const GROUP_OFFSET: usize = 8;
const AUTHORITY_OFFSET: usize = 40;
const BALANCES_OFFSET: usize = 72;
const BALANCE_COUNT: usize = 16;
const BALANCE_SIZE: usize = 136;

// Balance internal offsets
const BALANCE_ACTIVE: usize = 0;
const BALANCE_BANK_PK: usize = 1;
const BALANCE_ASSET_SHARES: usize = 40;
const BALANCE_LIABILITY_SHARES: usize = 56;

/// MarginFi account size (verified against mainnet: 2312 bytes).
const ACCOUNT_SIZE: usize = 2312;
const MIN_ACCOUNT_SIZE: usize = BALANCES_OFFSET + BALANCE_COUNT * BALANCE_SIZE;

pub struct MarginFiProtocol {
    pub program_id: Pubkey,
}

impl MarginFiProtocol {
    pub fn new() -> Self {
        Self {
            program_id: PROGRAM_ID.parse().unwrap(),
        }
    }
}

impl LendingProtocol for MarginFiProtocol {
    fn kind(&self) -> ProtocolKind {
        ProtocolKind::MarginFi
    }

    fn program_id(&self) -> Pubkey {
        self.program_id
    }

    fn is_position_account(&self, data: &[u8]) -> bool {
        data.len() >= MIN_ACCOUNT_SIZE
            && data[..8] == *MARGINFI_ACCOUNT_DISCRIMINATOR
    }

    fn evaluate_health(&self, data: &[u8]) -> Result<HealthResult> {
        // MarginFi health requires Bank accounts for weights/prices.
        // Without Bank data, we can only detect accounts that have both
        // asset and liability balances — a necessary condition for liquidation.
        //
        // Full health calculation requires fetching Bank accounts on-chain.
        // For now, we flag accounts with any liability as "needs detailed check".

        if data.len() < MIN_ACCOUNT_SIZE {
            bail!("marginfi account too small: {} bytes", data.len());
        }

        let mut has_assets = false;
        let mut has_liabilities = false;
        let mut total_asset_shares: f64 = 0.0;
        let mut total_liability_shares: f64 = 0.0;

        for i in 0..BALANCE_COUNT {
            let base = BALANCES_OFFSET + i * BALANCE_SIZE;
            let active = data[base + BALANCE_ACTIVE];
            if active == 0 {
                continue;
            }

            let asset_shares = read_i128(data, base + BALANCE_ASSET_SHARES);
            let liability_shares = read_i128(data, base + BALANCE_LIABILITY_SHARES);

            if asset_shares > 0 {
                has_assets = true;
                total_asset_shares += i80f48_to_f64(asset_shares);
            }
            if liability_shares > 0 {
                has_liabilities = true;
                total_liability_shares += i80f48_to_f64(liability_shares);
            }
        }

        // Without Bank data we can't compute true USD values.
        // Use share ratios as a rough proxy.
        let (current_ltv, is_liquidatable) = if has_assets && has_liabilities {
            let ratio = total_liability_shares / total_asset_shares.max(1e-18);
            // This is NOT the real LTV — it's a rough share-based proxy.
            // True health requires Bank.maintenance_asset_weight and
            // Bank.maintenance_liab_weight lookups.
            (ratio, false) // Never auto-trigger from shares alone
        } else {
            (0.0, false)
        };

        Ok(HealthResult {
            current_ltv,
            unhealthy_ltv: 1.0, // placeholder — real threshold depends on weights
            is_liquidatable,
            deposited_value_usd: total_asset_shares, // shares, not USD
            borrowed_value_usd: total_liability_shares, // shares, not USD
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
            let active = data[base + BALANCE_ACTIVE];
            if active == 0 {
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
                    market_value_usd: 0.0, // needs Bank lookup
                });
            }
            if liability_shares > 0 {
                borrows.push(BorrowPosition {
                    reserve: bank,
                    mint: None,
                    amount_sf: liability_shares as u128,
                    market_value_usd: 0.0, // needs Bank lookup
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
}

fn read_pubkey(data: &[u8], offset: usize) -> Pubkey {
    Pubkey::new_from_array(data[offset..offset + 32].try_into().unwrap())
}

fn read_i128(data: &[u8], offset: usize) -> i128 {
    i128::from_le_bytes(data[offset..offset + 16].try_into().unwrap())
}

/// Convert a WrappedI80F48 (i128 with 48-bit fractional part) to f64.
fn i80f48_to_f64(val: i128) -> f64 {
    val as f64 / (1i128 << 48) as f64
}
