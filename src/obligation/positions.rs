//! Extract deposit and borrow positions from raw Obligation account data.
//!
//! Each obligation has up to 8 deposits and 5 borrows. We need these to
//! determine which reserves to interact with during liquidation.

use anyhow::{bail, Result};
use solana_sdk::pubkey::Pubkey;

/// Obligation layout offsets (including 8-byte Anchor discriminator).
/// Validated against live mainnet data (2026-04-09).
///
/// deposits:  offset 96, 8 entries × 136 bytes each = 1088 bytes
/// borrows:   offset 1208, 5 entries × 200 bytes each = 1000 bytes
const DEPOSITS_OFFSET: usize = 96;
const DEPOSITS_COUNT: usize = 8;
const DEPOSIT_SIZE: usize = 136;

const BORROWS_OFFSET: usize = 1208;
const BORROWS_COUNT: usize = 5;
const BORROW_SIZE: usize = 200;

/// ObligationCollateral layout (136 bytes):
///   +0:  deposit_reserve (Pubkey, 32)
///   +32: deposited_amount (u64, 8)
///   +40: market_value_sf (u128, 16)
///   +56: borrowed_amount_against_this_collateral_in_elevation_group (u64, 8)
///   +64: padding ([u64; 9] = 72 bytes)
const DEPOSIT_RESERVE_OFFSET: usize = 0;
const DEPOSIT_AMOUNT_OFFSET: usize = 32;
const DEPOSIT_MARKET_VALUE_SF_OFFSET: usize = 40;

/// ObligationLiquidity layout (200 bytes):
///   +0:   borrow_reserve (Pubkey, 32)
///   +32:  cumulative_borrow_rate_bsf (BigFractionBytes, 48)
///   +80:  last_borrowed_at_timestamp (u64, 8)
///   +88:  borrowed_amount_sf (u128, 16)
///   +104: market_value_sf (u128, 16)
///   +120: borrow_factor_adjusted_market_value_sf (u128, 16)
///   +136: borrowed_amount_outside_elevation_groups (u64, 8)
///   +144: fixed_term_borrow_rollover_config (16 bytes)
///   +160: borrowed_amount_at_expiration (u64, 8)
///   +168: padding2 ([u64; 4] = 32 bytes)
const BORROW_RESERVE_OFFSET: usize = 0;
const BORROW_AMOUNT_SF_OFFSET: usize = 88;
const BORROW_MARKET_VALUE_SF_OFFSET: usize = 104;

/// lending_market is at struct offset 24 + 8 disc = 32
const LENDING_MARKET_OFFSET: usize = 32;
/// owner is at struct offset 56 + 8 disc = 64
const OWNER_OFFSET: usize = 64;

#[derive(Debug, Clone)]
pub struct DepositPosition {
    pub reserve: Pubkey,
    pub deposited_amount: u64,
    pub market_value_sf: u128,
}

#[derive(Debug, Clone)]
pub struct BorrowPosition {
    pub reserve: Pubkey,
    pub borrowed_amount_sf: u128,
    pub market_value_sf: u128,
}

/// All active positions on an obligation.
#[derive(Debug, Clone)]
pub struct ObligationPositions {
    pub deposits: Vec<DepositPosition>,
    pub borrows: Vec<BorrowPosition>,
    pub lending_market: Pubkey,
    pub owner: Pubkey,
}

/// Parse deposit and borrow positions from raw obligation account data.
pub fn parse_positions(data: &[u8]) -> Result<ObligationPositions> {
    let min_size = BORROWS_OFFSET + BORROWS_COUNT * BORROW_SIZE;
    if data.len() < min_size {
        bail!("obligation data too small for position parsing: {} < {}", data.len(), min_size);
    }

    let lending_market = read_pubkey(data, LENDING_MARKET_OFFSET);
    let owner = read_pubkey(data, OWNER_OFFSET);

    let mut deposits = Vec::new();
    for i in 0..DEPOSITS_COUNT {
        let base = DEPOSITS_OFFSET + i * DEPOSIT_SIZE;
        let reserve = read_pubkey(data, base + DEPOSIT_RESERVE_OFFSET);
        if reserve == Pubkey::default() {
            continue;
        }
        let deposited_amount = read_u64(data, base + DEPOSIT_AMOUNT_OFFSET);
        let market_value_sf = read_u128(data, base + DEPOSIT_MARKET_VALUE_SF_OFFSET);
        if deposited_amount > 0 {
            deposits.push(DepositPosition {
                reserve,
                deposited_amount,
                market_value_sf,
            });
        }
    }

    let mut borrows = Vec::new();
    for i in 0..BORROWS_COUNT {
        let base = BORROWS_OFFSET + i * BORROW_SIZE;
        let reserve = read_pubkey(data, base + BORROW_RESERVE_OFFSET);
        if reserve == Pubkey::default() {
            continue;
        }
        let borrowed_amount_sf = read_u128(data, base + BORROW_AMOUNT_SF_OFFSET);
        let market_value_sf = read_u128(data, base + BORROW_MARKET_VALUE_SF_OFFSET);
        if borrowed_amount_sf > 0 {
            borrows.push(BorrowPosition {
                reserve,
                borrowed_amount_sf,
                market_value_sf,
            });
        }
    }

    Ok(ObligationPositions {
        deposits,
        borrows,
        lending_market,
        owner,
    })
}

fn read_pubkey(data: &[u8], offset: usize) -> Pubkey {
    let bytes: [u8; 32] = data[offset..offset + 32]
        .try_into()
        .expect("pubkey slice");
    Pubkey::new_from_array(bytes)
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    let bytes: [u8; 8] = data[offset..offset + 8].try_into().expect("u64 slice");
    u64::from_le_bytes(bytes)
}

fn read_u128(data: &[u8], offset: usize) -> u128 {
    let bytes: [u8; 16] = data[offset..offset + 16].try_into().expect("u128 slice");
    u128::from_le_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_obligation_has_no_positions() {
        let data = vec![0u8; BORROWS_OFFSET + BORROWS_COUNT * BORROW_SIZE];
        let positions = parse_positions(&data).unwrap();
        assert!(positions.deposits.is_empty());
        assert!(positions.borrows.is_empty());
    }

    #[test]
    fn parses_single_deposit() {
        let mut data = vec![0u8; BORROWS_OFFSET + BORROWS_COUNT * BORROW_SIZE];

        let reserve = Pubkey::new_unique();
        data[DEPOSITS_OFFSET..DEPOSITS_OFFSET + 32]
            .copy_from_slice(reserve.as_ref());
        data[DEPOSITS_OFFSET + DEPOSIT_AMOUNT_OFFSET
            ..DEPOSITS_OFFSET + DEPOSIT_AMOUNT_OFFSET + 8]
            .copy_from_slice(&1000u64.to_le_bytes());

        let positions = parse_positions(&data).unwrap();
        assert_eq!(positions.deposits.len(), 1);
        assert_eq!(positions.deposits[0].reserve, reserve);
        assert_eq!(positions.deposits[0].deposited_amount, 1000);
    }

    #[test]
    fn parses_single_borrow() {
        let mut data = vec![0u8; BORROWS_OFFSET + BORROWS_COUNT * BORROW_SIZE];

        let reserve = Pubkey::new_unique();
        let base = BORROWS_OFFSET;
        data[base..base + 32].copy_from_slice(reserve.as_ref());
        let amount_sf = 500u128 << 60;
        data[base + BORROW_AMOUNT_SF_OFFSET..base + BORROW_AMOUNT_SF_OFFSET + 16]
            .copy_from_slice(&amount_sf.to_le_bytes());

        let positions = parse_positions(&data).unwrap();
        assert_eq!(positions.borrows.len(), 1);
        assert_eq!(positions.borrows[0].reserve, reserve);
        assert_eq!(positions.borrows[0].borrowed_amount_sf, amount_sf);
    }
}
