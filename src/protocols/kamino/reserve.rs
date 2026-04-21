//! Reserve and LendingMarket account deserialization from raw bytes.
//!
//! Extracts fields needed for flash-loan liquidation from the on-chain
//! `Reserve` and `LendingMarket` accounts using known byte offsets from
//! the klend program source.

use anyhow::{Result, bail};
use solana_sdk::pubkey::Pubkey;

use super::instructions::ReserveAccounts;

/// Minimum expected size of a Reserve account (discriminator + struct).
const RESERVE_MIN_SIZE: usize = 8 + 5784;

/// Offsets within Reserve account data (includes 8-byte Anchor discriminator).
mod offsets {
    // ReserveLiquidity fields (struct starts at byte 120, + 8 disc = 128)
    pub const LIQUIDITY_MINT: usize = 128; // Pubkey (32)
    pub const LIQUIDITY_SUPPLY_VAULT: usize = 160; // Pubkey (32)
    pub const LIQUIDITY_FEE_VAULT: usize = 192; // Pubkey (32)
    pub const LIQUIDITY_AVAILABLE: usize = 224; // u64 (8)
    pub const LIQUIDITY_BORROWED_SF: usize = 232; // u128 (16)
    pub const LIQUIDITY_MARKET_PRICE_SF: usize = 248; // u128 (16)
    pub const LIQUIDITY_TOKEN_PROGRAM: usize = 408; // Pubkey (32)

    // ReserveCollateral fields (struct starts at byte 2552, + 8 disc = 2560)
    pub const COLLATERAL_MINT: usize = 2560; // Pubkey (32)
    pub const COLLATERAL_SUPPLY_VAULT: usize = 2600; // Pubkey (32)

    // ReserveConfig fields (struct starts at byte 4848, + 8 disc = 4856)
    pub const CONFIG_PROTOCOL_LIQUIDATION_FEE_PCT: usize = 4871; // u8
    pub const CONFIG_LIQUIDATION_THRESHOLD_PCT: usize = 4873; // u8
    pub const CONFIG_MIN_LIQUIDATION_BONUS_BPS: usize = 4874; // u16 LE
    pub const CONFIG_MAX_LIQUIDATION_BONUS_BPS: usize = 4876; // u16 LE

    // ReserveFees (within config, struct offset 4888 + 8 disc = 4896)
    pub const FLASH_LOAN_FEE_SF: usize = 4904; // u64 (origination_fee is at 4896)
}

/// Parsed reserve data relevant to liquidation.
#[derive(Debug, Clone)]
pub struct ReserveData {
    pub accounts: ReserveAccounts,
    pub available_liquidity: u64,
    pub borrowed_amount_sf: u128,
    pub market_price_sf: u128,
    pub liquidation_threshold_pct: u8,
    pub min_liquidation_bonus_bps: u16,
    pub max_liquidation_bonus_bps: u16,
    pub protocol_liquidation_fee_pct: u8,
    pub flash_loan_fee_sf: u64,
}

/// Parse a Reserve account from raw account data.
pub fn parse_reserve(reserve_pubkey: &Pubkey, data: &[u8]) -> Result<ReserveData> {
    if data.len() < RESERVE_MIN_SIZE {
        bail!(
            "reserve account too small: {} bytes (expected >= {RESERVE_MIN_SIZE})",
            data.len(),
        );
    }

    Ok(ReserveData {
        accounts: ReserveAccounts {
            reserve: *reserve_pubkey,
            liquidity_mint: read_pubkey(data, offsets::LIQUIDITY_MINT),
            liquidity_supply_vault: read_pubkey(data, offsets::LIQUIDITY_SUPPLY_VAULT),
            liquidity_fee_vault: read_pubkey(data, offsets::LIQUIDITY_FEE_VAULT),
            collateral_mint: read_pubkey(data, offsets::COLLATERAL_MINT),
            collateral_supply_vault: read_pubkey(data, offsets::COLLATERAL_SUPPLY_VAULT),
            token_program: read_pubkey(data, offsets::LIQUIDITY_TOKEN_PROGRAM),
        },
        available_liquidity: read_u64(data, offsets::LIQUIDITY_AVAILABLE),
        borrowed_amount_sf: read_u128(data, offsets::LIQUIDITY_BORROWED_SF),
        market_price_sf: read_u128(data, offsets::LIQUIDITY_MARKET_PRICE_SF),
        liquidation_threshold_pct: data[offsets::CONFIG_LIQUIDATION_THRESHOLD_PCT],
        min_liquidation_bonus_bps: read_u16(data, offsets::CONFIG_MIN_LIQUIDATION_BONUS_BPS),
        max_liquidation_bonus_bps: read_u16(data, offsets::CONFIG_MAX_LIQUIDATION_BONUS_BPS),
        protocol_liquidation_fee_pct: data[offsets::CONFIG_PROTOCOL_LIQUIDATION_FEE_PCT],
        flash_loan_fee_sf: read_u64(data, offsets::FLASH_LOAN_FEE_SF),
    })
}

/// Parsed lending-market parameters relevant to liquidation.
#[derive(Debug, Clone)]
pub struct LendingMarketData {
    pub max_liquidatable_debt_market_value_at_once: u64,
    pub liquidation_max_debt_close_factor_pct: u8,
}

mod market_offsets {
    pub const LIQUIDATION_MAX_DEBT_CLOSE_FACTOR_PCT: usize = 126;
    pub const MAX_LIQUIDATABLE_DEBT_MARKET_VALUE_AT_ONCE: usize = 136;
}

pub fn parse_lending_market(data: &[u8]) -> Result<LendingMarketData> {
    if data.len() < 144 {
        bail!("lending market account too small: {} bytes", data.len());
    }

    Ok(LendingMarketData {
        liquidation_max_debt_close_factor_pct: data
            [market_offsets::LIQUIDATION_MAX_DEBT_CLOSE_FACTOR_PCT],
        max_liquidatable_debt_market_value_at_once: read_u64(
            data,
            market_offsets::MAX_LIQUIDATABLE_DEBT_MARKET_VALUE_AT_ONCE,
        ),
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

fn read_u16(data: &[u8], offset: usize) -> u16 {
    let bytes: [u8; 2] = data[offset..offset + 2].try_into().expect("u16 slice");
    u16::from_le_bytes(bytes)
}
