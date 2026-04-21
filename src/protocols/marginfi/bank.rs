//! MarginFi `Bank` account deserialization and health calculation.
//!
//! Bank accounts store per-asset configuration: share values (to convert
//! shares → underlying amounts), oracle prices, and maintenance weights.
//!
//! Bank account size: 1864 bytes (1856 struct + 8 discriminator).
//! Discriminator: `[0x8e, 0x31, 0xa6, 0xf2, 0x32, 0x42, 0x61, 0xbc]`

use anyhow::{Result, bail};
use solana_sdk::pubkey::Pubkey;

pub const BANK_DISCRIMINATOR: [u8; 8] = [0x8e, 0x31, 0xa6, 0xf2, 0x32, 0x42, 0x61, 0xbc];
pub const BANK_ACCOUNT_SIZE: usize = 1864;

/// Key offsets within Bank account data (all include the 8-byte discriminator).
mod offsets {
    pub const MINT: usize = 8;
    pub const MINT_DECIMALS: usize = 40;
    pub const GROUP: usize = 41;
    pub const ASSET_SHARE_VALUE: usize = 80;
    pub const LIABILITY_SHARE_VALUE: usize = 96;
    pub const LIQUIDITY_VAULT: usize = 112;
    pub const INSURANCE_VAULT: usize = 146;
    // BankConfig starts at struct offset 288 + 8 disc = 296
    pub const MAINT_ASSET_WEIGHT: usize = 312; // config+16
    pub const MAINT_LIABILITY_WEIGHT: usize = 344; // config+48
}

/// Parsed Bank data needed for health calculation and liquidation.
#[derive(Debug, Clone)]
pub struct BankData {
    pub mint: Pubkey,
    pub mint_decimals: u8,
    pub group: Pubkey,
    pub asset_share_value: f64,
    pub liability_share_value: f64,
    pub liquidity_vault: Pubkey,
    pub insurance_vault: Pubkey,
    pub maint_asset_weight: f64,
    pub maint_liability_weight: f64,
}

pub fn parse_bank(data: &[u8]) -> Result<BankData> {
    if data.len() < BANK_ACCOUNT_SIZE {
        bail!(
            "marginfi bank too small: {} bytes (expected {BANK_ACCOUNT_SIZE})",
            data.len(),
        );
    }
    if data[..8] != BANK_DISCRIMINATOR {
        bail!("not a marginfi bank account");
    }

    Ok(BankData {
        mint: read_pubkey(data, offsets::MINT),
        mint_decimals: data[offsets::MINT_DECIMALS],
        group: read_pubkey(data, offsets::GROUP),
        asset_share_value: i80f48_to_f64(read_i128(data, offsets::ASSET_SHARE_VALUE)),
        liability_share_value: i80f48_to_f64(read_i128(data, offsets::LIABILITY_SHARE_VALUE)),
        liquidity_vault: read_pubkey(data, offsets::LIQUIDITY_VAULT),
        insurance_vault: read_pubkey(data, offsets::INSURANCE_VAULT),
        maint_asset_weight: i80f48_to_f64(read_i128(data, offsets::MAINT_ASSET_WEIGHT)),
        maint_liability_weight: i80f48_to_f64(read_i128(data, offsets::MAINT_LIABILITY_WEIGHT)),
    })
}

/// Calculate a MarginFi account's health given its balances and their Bank data.
///
/// `health = sum(asset_value * maint_asset_weight) - sum(liability_value * maint_liability_weight)`
///
/// An account is liquidatable when `health < 0`. Returns `(ltv, total_assets,
/// total_liabilities, is_liquidatable)`.
pub fn calculate_health(balances: &[(f64, f64, &BankData)]) -> (f64, f64, f64, bool) {
    let mut weighted_assets = 0.0_f64;
    let mut weighted_liabilities = 0.0_f64;
    let mut total_assets = 0.0_f64;
    let mut total_liabilities = 0.0_f64;

    for (asset_shares, liability_shares, bank) in balances {
        let asset_amount = asset_shares * bank.asset_share_value;
        let liability_amount = liability_shares * bank.liability_share_value;

        weighted_assets += asset_amount * bank.maint_asset_weight;
        weighted_liabilities += liability_amount * bank.maint_liability_weight;

        total_assets += asset_amount;
        total_liabilities += liability_amount;
    }

    let health = weighted_assets - weighted_liabilities;
    let is_liquidatable = health < 0.0 && total_liabilities > 0.0;

    let ltv = if total_assets > 0.0 {
        total_liabilities / total_assets
    } else if total_liabilities > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };

    (ltv, total_assets, total_liabilities, is_liquidatable)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bank(asset_w: f64, liab_w: f64, asset_sv: f64, liab_sv: f64) -> BankData {
        BankData {
            mint: Pubkey::new_unique(),
            mint_decimals: 6,
            group: Pubkey::new_unique(),
            asset_share_value: asset_sv,
            liability_share_value: liab_sv,
            liquidity_vault: Pubkey::new_unique(),
            insurance_vault: Pubkey::new_unique(),
            maint_asset_weight: asset_w,
            maint_liability_weight: liab_w,
        }
    }

    #[test]
    fn health_calculation_healthy() {
        let bank = make_bank(0.8, 1.2, 1.05, 1.02);
        let balances = vec![(1000.0, 500.0, &bank)];
        let (ltv, _assets, _liabs, is_liquidatable) = calculate_health(&balances);
        assert!(!is_liquidatable);
        assert!(ltv < 1.0);
    }

    #[test]
    fn health_calculation_liquidatable() {
        let bank = make_bank(0.8, 1.2, 1.0, 1.0);
        let balances = vec![(100.0, 80.0, &bank)];
        let (_ltv, _assets, _liabs, is_liquidatable) = calculate_health(&balances);
        assert!(is_liquidatable);
    }
}
