use anyhow::{bail, Result};

use crate::config::AppConfig;

/// Scaled fraction shift used by klend (60-bit fixed point).
/// All `_sf` fields in the on-chain Obligation struct are u128 values
/// representing `value * 2^60`.
const SCALE_FACTOR: u128 = 1u128 << 60;

/// Result of evaluating an obligation's health.
#[derive(Debug, Clone)]
pub struct HealthResult {
    /// Current loan-to-value ratio (borrow-factor adjusted) as a float.
    pub current_ltv: f64,

    /// The LTV threshold at which the obligation becomes liquidatable.
    pub unhealthy_ltv: f64,

    /// Whether the obligation is currently liquidatable.
    pub is_liquidatable: bool,

    /// Raw deposited value (scaled fraction).
    pub deposited_value_sf: u128,

    /// Raw borrow-factor-adjusted debt value (scaled fraction).
    pub borrow_factor_adjusted_debt_value_sf: u128,

    /// Raw unhealthy borrow value (scaled fraction).
    pub unhealthy_borrow_value_sf: u128,

    /// Raw borrowed assets market value without borrow factor (scaled fraction).
    pub borrowed_assets_market_value_sf: u128,
}

/// Offsets into the Obligation account data for the key fields.
/// Validated against live mainnet data (2026-04-09).
///
/// Obligation struct layout (all offsets include 8-byte Anchor discriminator):
///   +0:    discriminator (8 bytes)
///   +8:    tag (u64 = 8 bytes)
///   +16:   last_update (LastUpdate = 16 bytes)
///   +32:   lending_market (Pubkey = 32 bytes)
///   +64:   owner (Pubkey = 32 bytes)
///   +96:   deposits (ObligationCollateral[8], each 136 bytes = 1088 bytes)
///   +1184: lowest_reserve_deposit_liquidation_ltv (u64 = 8 bytes)
///   +1192: deposited_value_sf (u128 = 16 bytes)
///   +1208: borrows (ObligationLiquidity[5], each 200 bytes = 1000 bytes)
///   +2208: borrow_factor_adjusted_debt_value_sf (u128 = 16 bytes)
///   +2224: borrowed_assets_market_value_sf (u128 = 16 bytes)
///   +2240: allowed_borrow_value_sf (u128 = 16 bytes)
///   +2256: unhealthy_borrow_value_sf (u128 = 16 bytes)
///   ...
///
/// Total account size: 3344 bytes (3336 struct + 8 discriminator).
mod offsets {
    pub const DEPOSITED_VALUE_SF: usize = 1192;
    pub const BORROW_FACTOR_ADJUSTED_DEBT_VALUE_SF: usize = 2208;
    pub const BORROWED_ASSETS_MARKET_VALUE_SF: usize = 2224;
    pub const UNHEALTHY_BORROW_VALUE_SF: usize = 2256;
}

/// Expected obligation account data size.
pub const OBLIGATION_ACCOUNT_SIZE: usize = 3344;

/// Evaluate the health of an obligation from its raw account data.
///
/// Replicates the on-chain logic:
///   ltv = borrow_factor_adjusted_debt_value_sf / deposited_value_sf
///   unhealthy_ltv = unhealthy_borrow_value_sf / deposited_value_sf
///   liquidatable = ltv >= unhealthy_ltv
pub fn evaluate(data: &[u8], _config: &AppConfig) -> Result<HealthResult> {
    let min_size = offsets::UNHEALTHY_BORROW_VALUE_SF + 16;
    if data.len() < min_size {
        bail!(
            "obligation account data too small: {} bytes (expected >= {})",
            data.len(),
            min_size
        );
    }

    let deposited_value_sf = read_u128(data, offsets::DEPOSITED_VALUE_SF);
    let borrow_factor_adjusted_debt_value_sf =
        read_u128(data, offsets::BORROW_FACTOR_ADJUSTED_DEBT_VALUE_SF);
    let unhealthy_borrow_value_sf = read_u128(data, offsets::UNHEALTHY_BORROW_VALUE_SF);
    let borrowed_assets_market_value_sf =
        read_u128(data, offsets::BORROWED_ASSETS_MARKET_VALUE_SF);

    let (current_ltv, unhealthy_ltv, is_liquidatable) = if deposited_value_sf == 0 {
        if borrow_factor_adjusted_debt_value_sf > 0 {
            (f64::INFINITY, 0.0, true)
        } else {
            (0.0, 0.0, false)
        }
    } else {
        let ltv = sf_to_f64(borrow_factor_adjusted_debt_value_sf)
            / sf_to_f64(deposited_value_sf);
        let u_ltv = sf_to_f64(unhealthy_borrow_value_sf) / sf_to_f64(deposited_value_sf);

        // On-chain uses integer comparison: debt_sf >= unhealthy_sf
        let liquidatable =
            borrow_factor_adjusted_debt_value_sf >= unhealthy_borrow_value_sf
                && borrow_factor_adjusted_debt_value_sf > 0;

        (ltv, u_ltv, liquidatable)
    };

    Ok(HealthResult {
        current_ltv,
        unhealthy_ltv,
        is_liquidatable,
        deposited_value_sf,
        borrow_factor_adjusted_debt_value_sf,
        unhealthy_borrow_value_sf,
        borrowed_assets_market_value_sf,
    })
}

/// Read a little-endian u128 from a byte slice at the given offset.
fn read_u128(data: &[u8], offset: usize) -> u128 {
    let bytes: [u8; 16] = data[offset..offset + 16]
        .try_into()
        .expect("slice length mismatch");
    u128::from_le_bytes(bytes)
}

/// Convert a scaled fraction (u128 with 2^60 scale) to f64.
fn sf_to_f64(sf: u128) -> f64 {
    sf as f64 / SCALE_FACTOR as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_obligation_data(
        deposited_value_sf: u128,
        borrow_factor_adjusted_debt_sf: u128,
        unhealthy_borrow_sf: u128,
        borrowed_market_value_sf: u128,
    ) -> Vec<u8> {
        let mut data = vec![0u8; OBLIGATION_ACCOUNT_SIZE];
        write_u128(&mut data, offsets::DEPOSITED_VALUE_SF, deposited_value_sf);
        write_u128(
            &mut data,
            offsets::BORROW_FACTOR_ADJUSTED_DEBT_VALUE_SF,
            borrow_factor_adjusted_debt_sf,
        );
        write_u128(&mut data, offsets::UNHEALTHY_BORROW_VALUE_SF, unhealthy_borrow_sf);
        write_u128(
            &mut data,
            offsets::BORROWED_ASSETS_MARKET_VALUE_SF,
            borrowed_market_value_sf,
        );
        data
    }

    fn write_u128(data: &mut [u8], offset: usize, value: u128) {
        data[offset..offset + 16].copy_from_slice(&value.to_le_bytes());
    }

    fn dummy_config() -> AppConfig {
        AppConfig {
            rpc_url: String::new(),
            grpc_url: String::new(),
            grpc_token: None,
            kamino_market: "11111111111111111111111111111111".to_string(),
            klend_program_id: "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD".to_string(),
            liquidator_keypair_path: String::new(),
            min_profit_lamports: 0,
            supabase_url: None,
            supabase_service_role_key: None,
        }
    }

    #[test]
    fn healthy_obligation() {
        let deposited = 100u128 * SCALE_FACTOR;
        let debt = 50u128 * SCALE_FACTOR;
        let unhealthy = 80u128 * SCALE_FACTOR;
        let market_value = 50u128 * SCALE_FACTOR;

        let data = make_obligation_data(deposited, debt, unhealthy, market_value);
        let result = evaluate(&data, &dummy_config()).unwrap();

        assert!(!result.is_liquidatable);
        assert!((result.current_ltv - 0.5).abs() < 1e-9);
        assert!((result.unhealthy_ltv - 0.8).abs() < 1e-9);
    }

    #[test]
    fn liquidatable_obligation() {
        let deposited = 100u128 * SCALE_FACTOR;
        let debt = 85u128 * SCALE_FACTOR;
        let unhealthy = 80u128 * SCALE_FACTOR;
        let market_value = 85u128 * SCALE_FACTOR;

        let data = make_obligation_data(deposited, debt, unhealthy, market_value);
        let result = evaluate(&data, &dummy_config()).unwrap();

        assert!(result.is_liquidatable);
        assert!((result.current_ltv - 0.85).abs() < 1e-9);
    }

    #[test]
    fn exactly_at_threshold() {
        let deposited = 100u128 * SCALE_FACTOR;
        let debt = 80u128 * SCALE_FACTOR;
        let unhealthy = 80u128 * SCALE_FACTOR;
        let market_value = 80u128 * SCALE_FACTOR;

        let data = make_obligation_data(deposited, debt, unhealthy, market_value);
        let result = evaluate(&data, &dummy_config()).unwrap();

        assert!(result.is_liquidatable);
    }

    #[test]
    fn zero_deposits_with_debt_is_liquidatable() {
        let data = make_obligation_data(0, 1u128 * SCALE_FACTOR, 0, 1u128 * SCALE_FACTOR);
        let result = evaluate(&data, &dummy_config()).unwrap();
        assert!(result.is_liquidatable);
    }

    #[test]
    fn zero_everything_is_healthy() {
        let data = make_obligation_data(0, 0, 0, 0);
        let result = evaluate(&data, &dummy_config()).unwrap();
        assert!(!result.is_liquidatable);
    }
}
