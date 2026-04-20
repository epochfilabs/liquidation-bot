//! Save (Solend) account decoding.
//!
//! Decodes Obligation and Reserve accounts from raw on-chain data.
//! These use SPL token-lending layout (no Anchor discriminator).
//!
//! Offsets are for the mainnet-deployed version which uses padding bytes
//! for additional fields (super_unhealthy_borrow_value, borrowing_isolated_asset).

use solana_sdk::pubkey::Pubkey;
use thiserror::Error;

/// WAD scale factor used by Save (10^18).
pub const WAD: u128 = 1_000_000_000_000_000_000;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum AccountDecodeError {
    #[error("account data too small: got {got} bytes, need >= {need}")]
    DataTooSmall { got: usize, need: usize },
    #[error("invalid version byte: {0}")]
    InvalidVersion(u8),
}

// ---------------------------------------------------------------------------
// Obligation
// ---------------------------------------------------------------------------

/// Mainnet obligation layout offsets.
/// The mainnet program uses the SDK's 64-byte padding region for extra fields,
/// shifting deposits_len/borrows_len/data_flat relative to the published SDK.
mod obligation_offsets {
    pub const VERSION: usize = 0;
    pub const LAST_UPDATE_SLOT: usize = 1;
    pub const LENDING_MARKET: usize = 10;
    pub const OWNER: usize = 42;
    pub const DEPOSITED_VALUE: usize = 74;
    pub const BORROWED_VALUE: usize = 90;
    pub const ALLOWED_BORROW_VALUE: usize = 106;
    pub const UNHEALTHY_BORROW_VALUE: usize = 122;
    pub const SUPER_UNHEALTHY_BORROW_VALUE: usize = 138; // mainnet extension
    pub const BORROWING_ISOLATED_ASSET: usize = 154; // mainnet extension
    pub const DEPOSITS_LEN: usize = 155; // mainnet offset
    pub const BORROWS_LEN: usize = 156; // mainnet offset
    pub const DATA_FLAT: usize = 157; // mainnet offset
}

/// ObligationCollateral entry size.
/// Mainnet uses 56 bytes per entry (no trailing padding).
/// SDK publishes 88 bytes (with 32-byte padding). Verify against on-chain data.
pub const OBLIGATION_COLLATERAL_SIZE: usize = 56;

/// ObligationLiquidity entry size.
/// Mainnet uses 80 bytes per entry (no trailing padding).
/// SDK publishes 112 bytes (with 32-byte padding). Verify against on-chain data.
pub const OBLIGATION_LIQUIDITY_SIZE: usize = 80;

/// Minimum obligation data size (header only, no positions).
pub const OBLIGATION_MIN_SIZE: usize = obligation_offsets::DATA_FLAT;

/// Decoded obligation header.
#[derive(Debug, Clone)]
pub struct ObligationHeader {
    pub version: u8,
    pub last_update_slot: u64,
    pub lending_market: Pubkey,
    pub owner: Pubkey,
    /// Total deposited value in USD (WAD-scaled u128).
    pub deposited_value: u128,
    /// Total borrowed value in USD (WAD-scaled u128).
    pub borrowed_value: u128,
    /// Allowed borrow value (WAD-scaled).
    pub allowed_borrow_value: u128,
    /// Unhealthy borrow value threshold (WAD-scaled).
    pub unhealthy_borrow_value: u128,
    /// Super-unhealthy threshold (mainnet extension).
    pub super_unhealthy_borrow_value: u128,
    pub deposits_len: u8,
    pub borrows_len: u8,
}

/// A single deposit position within an obligation.
#[derive(Debug, Clone)]
pub struct ObligationCollateral {
    pub deposit_reserve: Pubkey,
    pub deposited_amount: u64,
    /// Market value in USD (WAD-scaled).
    pub market_value: u128,
}

/// A single borrow position within an obligation.
#[derive(Debug, Clone)]
pub struct ObligationLiquidity {
    pub borrow_reserve: Pubkey,
    /// Cumulative borrow rate (WAD-scaled).
    pub cumulative_borrow_rate_wads: u128,
    /// Outstanding borrow amount (WAD-scaled).
    pub borrowed_amount_wads: u128,
    /// Market value in USD (WAD-scaled).
    pub market_value: u128,
}

/// Fully decoded obligation.
#[derive(Debug, Clone)]
pub struct Obligation {
    pub header: ObligationHeader,
    pub deposits: Vec<ObligationCollateral>,
    pub borrows: Vec<ObligationLiquidity>,
}

impl Obligation {
    /// Deposited value in USD as f64.
    pub fn deposited_value_usd(&self) -> f64 {
        self.header.deposited_value as f64 / WAD as f64
    }

    /// Borrowed value in USD as f64.
    pub fn borrowed_value_usd(&self) -> f64 {
        self.header.borrowed_value as f64 / WAD as f64
    }

    /// Current LTV ratio.
    pub fn ltv(&self) -> f64 {
        let dep = self.deposited_value_usd();
        if dep == 0.0 {
            if self.borrowed_value_usd() > 0.0 {
                f64::INFINITY
            } else {
                0.0
            }
        } else {
            self.borrowed_value_usd() / dep
        }
    }

    /// Whether the obligation is liquidatable.
    pub fn is_liquidatable(&self) -> bool {
        self.header.borrowed_value > self.header.unhealthy_borrow_value
            && self.header.borrowed_value > 0
    }
}

/// Decode an obligation from raw account data.
pub fn decode_obligation(data: &[u8]) -> Result<Obligation, AccountDecodeError> {
    if data.len() < OBLIGATION_MIN_SIZE {
        return Err(AccountDecodeError::DataTooSmall {
            got: data.len(),
            need: OBLIGATION_MIN_SIZE,
        });
    }

    let version = data[obligation_offsets::VERSION];
    if version == 0 || version > 3 {
        return Err(AccountDecodeError::InvalidVersion(version));
    }

    let deposits_len = data[obligation_offsets::DEPOSITS_LEN] as usize;
    let borrows_len = data[obligation_offsets::BORROWS_LEN] as usize;

    let expected_size = obligation_offsets::DATA_FLAT
        + deposits_len * OBLIGATION_COLLATERAL_SIZE
        + borrows_len * OBLIGATION_LIQUIDITY_SIZE;

    if data.len() < expected_size {
        return Err(AccountDecodeError::DataTooSmall {
            got: data.len(),
            need: expected_size,
        });
    }

    let header = ObligationHeader {
        version,
        last_update_slot: read_u64(data, obligation_offsets::LAST_UPDATE_SLOT),
        lending_market: read_pubkey(data, obligation_offsets::LENDING_MARKET),
        owner: read_pubkey(data, obligation_offsets::OWNER),
        deposited_value: read_u128(data, obligation_offsets::DEPOSITED_VALUE),
        borrowed_value: read_u128(data, obligation_offsets::BORROWED_VALUE),
        allowed_borrow_value: read_u128(data, obligation_offsets::ALLOWED_BORROW_VALUE),
        unhealthy_borrow_value: read_u128(data, obligation_offsets::UNHEALTHY_BORROW_VALUE),
        super_unhealthy_borrow_value: read_u128(
            data,
            obligation_offsets::SUPER_UNHEALTHY_BORROW_VALUE,
        ),
        deposits_len: deposits_len as u8,
        borrows_len: borrows_len as u8,
    };

    let mut deposits = Vec::with_capacity(deposits_len);
    for i in 0..deposits_len {
        let base = obligation_offsets::DATA_FLAT + i * OBLIGATION_COLLATERAL_SIZE;
        let reserve = read_pubkey(data, base);
        if reserve == Pubkey::default() {
            continue;
        }
        deposits.push(ObligationCollateral {
            deposit_reserve: reserve,
            deposited_amount: read_u64(data, base + 32),
            market_value: read_u128(data, base + 40),
        });
    }

    let borrows_start =
        obligation_offsets::DATA_FLAT + deposits_len * OBLIGATION_COLLATERAL_SIZE;
    let mut borrows = Vec::with_capacity(borrows_len);
    for i in 0..borrows_len {
        let base = borrows_start + i * OBLIGATION_LIQUIDITY_SIZE;
        let reserve = read_pubkey(data, base);
        if reserve == Pubkey::default() {
            continue;
        }
        borrows.push(ObligationLiquidity {
            borrow_reserve: reserve,
            cumulative_borrow_rate_wads: read_u128(data, base + 32),
            borrowed_amount_wads: read_u128(data, base + 48),
            market_value: read_u128(data, base + 64),
        });
    }

    Ok(Obligation {
        header,
        deposits,
        borrows,
    })
}

// ---------------------------------------------------------------------------
// Reserve (partial — fields needed for liquidation indexing)
// ---------------------------------------------------------------------------

/// Save Reserve layout offsets (SPL token-lending, no Anchor discriminator).
mod reserve_offsets {
    pub const VERSION: usize = 0;
    pub const LENDING_MARKET: usize = 10;
    pub const LIQUIDITY_MINT: usize = 42;
    pub const LIQUIDITY_SUPPLY: usize = 74;
    pub const LIQUIDITY_FEE_RECEIVER: usize = 106;
    pub const PYTH_ORACLE: usize = 65; // within ReserveLiquidity sub-struct
    pub const SWITCHBOARD_ORACLE: usize = 97;
    pub const COLLATERAL_MINT: usize = 226;
    pub const COLLATERAL_SUPPLY: usize = 258;
    pub const LIQUIDATION_BONUS: usize = 260; // approximate — within ReserveConfig
}

/// Partial reserve data relevant for liquidation indexing.
#[derive(Debug, Clone)]
pub struct ReserveInfo {
    pub version: u8,
    pub lending_market: Pubkey,
    pub liquidity_mint: Pubkey,
    pub liquidity_supply: Pubkey,
    pub liquidity_fee_receiver: Pubkey,
    pub collateral_mint: Pubkey,
    pub collateral_supply: Pubkey,
    pub pyth_oracle: Pubkey,
    pub switchboard_oracle: Pubkey,
}

/// Decode partial reserve info from raw account data.
pub fn decode_reserve_info(data: &[u8]) -> Result<ReserveInfo, AccountDecodeError> {
    if data.len() < 300 {
        return Err(AccountDecodeError::DataTooSmall {
            got: data.len(),
            need: 300,
        });
    }

    Ok(ReserveInfo {
        version: data[reserve_offsets::VERSION],
        lending_market: read_pubkey(data, reserve_offsets::LENDING_MARKET),
        liquidity_mint: read_pubkey(data, reserve_offsets::LIQUIDITY_MINT),
        liquidity_supply: read_pubkey(data, reserve_offsets::LIQUIDITY_SUPPLY),
        liquidity_fee_receiver: read_pubkey(data, reserve_offsets::LIQUIDITY_FEE_RECEIVER),
        collateral_mint: read_pubkey(data, reserve_offsets::COLLATERAL_MINT),
        collateral_supply: read_pubkey(data, reserve_offsets::COLLATERAL_SUPPLY),
        pyth_oracle: read_pubkey(data, reserve_offsets::PYTH_ORACLE),
        switchboard_oracle: read_pubkey(data, reserve_offsets::SWITCHBOARD_ORACLE),
    })
}

// ---------------------------------------------------------------------------
// Byte reading helpers
// ---------------------------------------------------------------------------

fn read_pubkey(data: &[u8], offset: usize) -> Pubkey {
    Pubkey::new_from_array(data[offset..offset + 32].try_into().unwrap())
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

fn read_u128(data: &[u8], offset: usize) -> u128 {
    u128::from_le_bytes(data[offset..offset + 16].try_into().unwrap())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_obligation(
        deposits: u8,
        borrows: u8,
        deposited_value: u128,
        borrowed_value: u128,
        unhealthy_borrow: u128,
    ) -> Vec<u8> {
        let total = obligation_offsets::DATA_FLAT
            + deposits as usize * OBLIGATION_COLLATERAL_SIZE
            + borrows as usize * OBLIGATION_LIQUIDITY_SIZE;
        let mut data = vec![0u8; total];

        data[0] = 1; // version
        data[obligation_offsets::DEPOSITS_LEN] = deposits;
        data[obligation_offsets::BORROWS_LEN] = borrows;
        data[obligation_offsets::DEPOSITED_VALUE..obligation_offsets::DEPOSITED_VALUE + 16]
            .copy_from_slice(&deposited_value.to_le_bytes());
        data[obligation_offsets::BORROWED_VALUE..obligation_offsets::BORROWED_VALUE + 16]
            .copy_from_slice(&borrowed_value.to_le_bytes());
        data[obligation_offsets::UNHEALTHY_BORROW_VALUE
            ..obligation_offsets::UNHEALTHY_BORROW_VALUE + 16]
            .copy_from_slice(&unhealthy_borrow.to_le_bytes());
        data
    }

    #[test]
    fn decode_empty_obligation() {
        let data = make_obligation(0, 0, 0, 0, 0);
        let obl = decode_obligation(&data).unwrap();
        assert_eq!(obl.header.version, 1);
        assert!(obl.deposits.is_empty());
        assert!(obl.borrows.is_empty());
        assert!(!obl.is_liquidatable());
        assert_eq!(obl.ltv(), 0.0);
    }

    #[test]
    fn healthy_obligation() {
        let dep = 100 * WAD;
        let bor = 50 * WAD;
        let unhealthy = 80 * WAD;
        let data = make_obligation(0, 0, dep, bor, unhealthy);
        let obl = decode_obligation(&data).unwrap();

        assert!(!obl.is_liquidatable());
        assert!((obl.ltv() - 0.5).abs() < 1e-9);
        assert!((obl.deposited_value_usd() - 100.0).abs() < 1e-9);
        assert!((obl.borrowed_value_usd() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn liquidatable_obligation() {
        let dep = 100 * WAD;
        let bor = 85 * WAD;
        let unhealthy = 80 * WAD;
        let data = make_obligation(0, 0, dep, bor, unhealthy);
        let obl = decode_obligation(&data).unwrap();

        assert!(obl.is_liquidatable());
        assert!((obl.ltv() - 0.85).abs() < 1e-9);
    }

    #[test]
    fn obligation_with_deposit() {
        let mut data = make_obligation(1, 0, 100 * WAD, 0, 80 * WAD);
        let reserve = Pubkey::new_unique();
        let base = obligation_offsets::DATA_FLAT;
        data[base..base + 32].copy_from_slice(reserve.as_ref());
        data[base + 32..base + 40].copy_from_slice(&1000u64.to_le_bytes());

        let obl = decode_obligation(&data).unwrap();
        assert_eq!(obl.deposits.len(), 1);
        assert_eq!(obl.deposits[0].deposit_reserve, reserve);
        assert_eq!(obl.deposits[0].deposited_amount, 1000);
    }

    #[test]
    fn obligation_with_borrow() {
        let mut data = make_obligation(0, 1, 100 * WAD, 50 * WAD, 80 * WAD);
        let reserve = Pubkey::new_unique();
        let base = obligation_offsets::DATA_FLAT;
        data[base..base + 32].copy_from_slice(reserve.as_ref());
        let borrowed_wads = 500u128 * WAD;
        data[base + 48..base + 64].copy_from_slice(&borrowed_wads.to_le_bytes());

        let obl = decode_obligation(&data).unwrap();
        assert_eq!(obl.borrows.len(), 1);
        assert_eq!(obl.borrows[0].borrow_reserve, reserve);
        assert_eq!(obl.borrows[0].borrowed_amount_wads, borrowed_wads);
    }

    #[test]
    fn too_small_errors() {
        let data = vec![0u8; 10];
        assert!(decode_obligation(&data).is_err());
    }

    #[test]
    fn invalid_version_errors() {
        let mut data = make_obligation(0, 0, 0, 0, 0);
        data[0] = 0; // invalid version
        assert!(decode_obligation(&data).is_err());

        data[0] = 4; // also invalid
        assert!(decode_obligation(&data).is_err());
    }
}
