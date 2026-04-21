//! Profitability estimation for liquidations.
//!
//! Calculates whether a liquidation is worth executing by estimating:
//!   profit = (collateral_value × (1 + liquidation_bonus)) - repay_amount - flash_loan_fee - tx_cost

use crate::protocols::kamino::reserve::ReserveData;

/// Scale factor for klend's u128 scaled fractions (2^60).
const SF_SHIFT: u128 = 1u128 << 60;

/// Estimated transaction cost in USD (priority fees + base fee).
/// Conservative estimate — can be tuned.
const ESTIMATED_TX_COST_USD: f64 = 0.01;

/// Result of a profitability check.
#[derive(Debug, Clone)]
pub struct ProfitEstimate {
    /// Estimated gross profit in USD (before tx cost).
    pub gross_profit_usd: f64,

    /// Estimated net profit in USD (after tx cost).
    pub net_profit_usd: f64,

    /// The liquidation bonus percentage applied.
    pub liquidation_bonus_bps: u16,

    /// The flash loan fee as a fraction.
    pub flash_loan_fee_fraction: f64,

    /// Whether this liquidation is estimated to be profitable.
    pub is_profitable: bool,

    /// The repay amount in token units.
    pub repay_amount: u64,
}

/// Estimate profit for a potential liquidation.
///
/// The liquidator repays `repay_amount` of the debt token and receives
/// collateral worth `repay_amount * (1 + liquidation_bonus)` in the
/// withdrawal token. The flash loan charges a fee on the borrowed amount.
///
/// Profit = collateral_received_value - repay_amount_value - flash_loan_fee_value
pub fn estimate_profit(
    repay_amount: u64,
    repay_reserve: &ReserveData,
    withdraw_reserve: &ReserveData,
    min_profit_lamports: u64,
) -> ProfitEstimate {
    // Get prices in USD (scaled fraction → f64)
    let repay_price = repay_reserve.market_price_sf as f64 / SF_SHIFT as f64;
    let _withdraw_price = withdraw_reserve.market_price_sf as f64 / SF_SHIFT as f64;

    // Repay value in USD
    let repay_value_usd = repay_amount as f64 * repay_price;

    // Liquidation bonus: use midpoint between min and max as estimate.
    // The actual bonus is calculated on-chain based on how underwater the position is.
    let bonus_bps = (repay_reserve.min_liquidation_bonus_bps
        + withdraw_reserve.min_liquidation_bonus_bps)
        / 2;
    let bonus_bps = bonus_bps.max(withdraw_reserve.min_liquidation_bonus_bps);
    let bonus_fraction = bonus_bps as f64 / 10_000.0;

    // Flash loan fee (stored as a scaled fraction u64)
    let flash_fee_fraction = if repay_reserve.flash_loan_fee_sf > 0 {
        repay_reserve.flash_loan_fee_sf as f64 / SF_SHIFT as f64
    } else {
        0.003 // default 0.3% if not set
    };

    // Protocol liquidation fee (taken from the bonus)
    let protocol_fee_fraction = repay_reserve.protocol_liquidation_fee_pct as f64 / 100.0;

    // Gross profit = bonus - flash_loan_fee - protocol_fee (all as fractions of repay_value)
    let gross_profit_fraction = bonus_fraction - flash_fee_fraction - (bonus_fraction * protocol_fee_fraction);
    let gross_profit_usd = repay_value_usd * gross_profit_fraction;
    let net_profit_usd = gross_profit_usd - ESTIMATED_TX_COST_USD;

    // Convert min_profit_lamports to a rough USD threshold
    // (assuming ~$150/SOL for a conservative check)
    let min_profit_usd = min_profit_lamports as f64 * 150.0 / 1_000_000_000.0;

    let is_profitable = net_profit_usd > min_profit_usd && net_profit_usd > 0.0;

    ProfitEstimate {
        gross_profit_usd,
        net_profit_usd,
        liquidation_bonus_bps: bonus_bps,
        flash_loan_fee_fraction: flash_fee_fraction,
        is_profitable,
        repay_amount,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocols::kamino::instructions::ReserveAccounts;
    use solana_sdk::pubkey::Pubkey;

    fn make_reserve(
        price_usd: f64,
        min_bonus_bps: u16,
        max_bonus_bps: u16,
        flash_fee_fraction: f64,
        protocol_liq_fee_pct: u8,
    ) -> ReserveData {
        ReserveData {
            accounts: ReserveAccounts {
                reserve: Pubkey::new_unique(),
                liquidity_mint: Pubkey::new_unique(),
                liquidity_supply_vault: Pubkey::new_unique(),
                liquidity_fee_vault: Pubkey::new_unique(),
                collateral_mint: Pubkey::new_unique(),
                collateral_supply_vault: Pubkey::new_unique(),
                token_program: Pubkey::new_unique(),
            },
            available_liquidity: u64::MAX,
            borrowed_amount_sf: 0,
            market_price_sf: (price_usd * SF_SHIFT as f64) as u128,
            liquidation_threshold_pct: 80,
            min_liquidation_bonus_bps: min_bonus_bps,
            max_liquidation_bonus_bps: max_bonus_bps,
            protocol_liquidation_fee_pct: protocol_liq_fee_pct,
            flash_loan_fee_sf: (flash_fee_fraction * SF_SHIFT as f64) as u64,
        }
    }

    #[test]
    fn profitable_liquidation() {
        // Repay 1000 USDC (6 decimals, so 1_000_000_000 smallest units)
        // Price per smallest unit = $1 / 1_000_000 = 0.000001
        let repay_reserve = make_reserve(0.000001, 500, 500, 0.003, 0);
        let withdraw_reserve = make_reserve(0.000000001, 500, 500, 0.0, 0);

        let estimate = estimate_profit(1_000_000_000, &repay_reserve, &withdraw_reserve, 10_000);

        // repay_value = 1B * 0.000001 = $1000
        // Profit ≈ 1000 * (0.05 - 0.003) = $47
        assert!(estimate.is_profitable);
        assert!(estimate.net_profit_usd > 40.0);
    }

    #[test]
    fn unprofitable_tiny_liquidation() {
        // Repay 0.01 USDC = 10_000 smallest units at $0.000001 each = $0.01
        let repay_reserve = make_reserve(0.000001, 500, 500, 0.003, 0);
        let withdraw_reserve = make_reserve(0.000000001, 500, 500, 0.0, 0);

        let estimate = estimate_profit(10_000, &repay_reserve, &withdraw_reserve, 10_000);

        // repay_value = $0.01, profit ≈ $0.01 * 0.047 = $0.00047 < tx cost $0.01
        assert!(!estimate.is_profitable);
    }

    #[test]
    fn high_flash_fee_kills_profit() {
        // 5% bonus but 6% flash loan fee → unprofitable
        let repay_reserve = make_reserve(0.000001, 500, 500, 0.06, 0);
        let withdraw_reserve = make_reserve(0.000000001, 500, 500, 0.0, 0);

        let estimate = estimate_profit(1_000_000_000, &repay_reserve, &withdraw_reserve, 0);

        assert!(!estimate.is_profitable);
        assert!(estimate.net_profit_usd < 0.0);
    }
}
