//! Risk management: EV filter and daily loss cap.
//!
//! The EV filter rejects liquidation candidates that are unlikely to be
//! profitable after accounting for flash loan fees, Jito tips, and tx fees.
//!
//! The daily loss cap tracks cumulative tip spend on failed attempts and
//! pauses the bot when the cap is reached.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Risk configuration.
#[derive(Debug, Clone)]
pub struct RiskConfig {
    /// Minimum repay amount (in token units) to consider a liquidation.
    /// Based on analysis: sub-$1K events are net negative after tips.
    /// Default: 5,000,000 (= $5,000 for 6-decimal stablecoins like USDC)
    pub min_repay_amount: u64,

    /// Minimum estimated bonus in USD to submit a transaction.
    /// Must exceed expected tip + fee costs.
    /// Default: $10 (conservative — evoxx profits at $3.30/tx avg)
    pub min_estimated_bonus_usd: f64,

    /// Maximum daily tip spend in lamports before pausing.
    /// Default: 357,142,857 lamports = ~$50 at $140/SOL
    pub daily_tip_cap_lamports: u64,

    /// Maximum Jito tip per transaction in lamports.
    /// Default: 10,000,000 = ~$1.40 at $140/SOL
    /// evoxx pays ~$0.03 avg. 4NUiC pays $122 avg (and loses money).
    pub max_tip_per_tx_lamports: u64,

    /// Bonus rate estimate used for EV calculation.
    /// Kamino Jan 2026 average: 0.011 (1.1%)
    pub estimated_bonus_rate: f64,
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            min_repay_amount: 5_000_000,       // $5,000 for 6-decimal tokens
            min_estimated_bonus_usd: 10.0,     // $10 minimum estimated profit
            daily_tip_cap_lamports: 357_142_857, // ~$50/day at $140/SOL
            max_tip_per_tx_lamports: 10_000_000, // ~$1.40/tx
            estimated_bonus_rate: 0.011,        // 1.1% Kamino average
        }
    }
}

impl RiskConfig {
    /// Load from environment variables with defaults.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(v) = std::env::var("MIN_REPAY_AMOUNT") {
            if let Ok(n) = v.parse() { config.min_repay_amount = n; }
        }
        if let Ok(v) = std::env::var("MIN_ESTIMATED_BONUS_USD") {
            if let Ok(n) = v.parse() { config.min_estimated_bonus_usd = n; }
        }
        if let Ok(v) = std::env::var("DAILY_TIP_CAP_LAMPORTS") {
            if let Ok(n) = v.parse() { config.daily_tip_cap_lamports = n; }
        }
        if let Ok(v) = std::env::var("MAX_TIP_PER_TX_LAMPORTS") {
            if let Ok(n) = v.parse() { config.max_tip_per_tx_lamports = n; }
        }
        if let Ok(v) = std::env::var("ESTIMATED_BONUS_RATE") {
            if let Ok(n) = v.parse() { config.estimated_bonus_rate = n; }
        }

        config
    }
}

/// EV filter result.
#[derive(Debug)]
pub enum EvDecision {
    /// Submit this liquidation.
    Submit {
        estimated_bonus_usd: f64,
        recommended_tip_lamports: u64,
    },
    /// Skip — too small.
    SkipTooSmall {
        repay_amount: u64,
        min_required: u64,
    },
    /// Skip — estimated bonus below threshold.
    SkipLowEv {
        estimated_bonus_usd: f64,
        min_required: f64,
    },
    /// Skip — daily loss cap reached.
    SkipDailyCapReached {
        spent_today: u64,
        cap: u64,
    },
}

/// Evaluate whether a liquidation opportunity is worth submitting.
pub fn evaluate_opportunity(
    config: &RiskConfig,
    repay_amount: u64,
    token_decimals: u8,
    token_price_usd: f64,
    daily_tracker: &DailyTracker,
) -> EvDecision {
    // Check minimum size
    if repay_amount < config.min_repay_amount {
        return EvDecision::SkipTooSmall {
            repay_amount,
            min_required: config.min_repay_amount,
        };
    }

    // Check daily cap
    let spent = daily_tracker.spent_today();
    if spent >= config.daily_tip_cap_lamports {
        return EvDecision::SkipDailyCapReached {
            spent_today: spent,
            cap: config.daily_tip_cap_lamports,
        };
    }

    // Estimate profit
    let repay_usd = repay_amount as f64 / 10f64.powi(token_decimals as i32) * token_price_usd;
    let estimated_bonus_usd = repay_usd * config.estimated_bonus_rate;

    if estimated_bonus_usd < config.min_estimated_bonus_usd {
        return EvDecision::SkipLowEv {
            estimated_bonus_usd,
            min_required: config.min_estimated_bonus_usd,
        };
    }

    // Recommend a tip proportional to expected profit, capped
    // Use 5% of estimated bonus as tip (evoxx-style: keep tips minimal)
    let tip_fraction = 0.05;
    let recommended_tip_usd = estimated_bonus_usd * tip_fraction;
    let sol_price_usd = 140.0; // TODO: fetch live price
    let recommended_tip_lamports = (recommended_tip_usd / sol_price_usd * 1e9) as u64;
    let recommended_tip_lamports = recommended_tip_lamports.min(config.max_tip_per_tx_lamports);

    EvDecision::Submit {
        estimated_bonus_usd,
        recommended_tip_lamports,
    }
}

/// Tracks daily tip spend for loss cap enforcement.
///
/// Resets at midnight UTC. Thread-safe via atomics.
pub struct DailyTracker {
    spent_lamports: AtomicU64,
    current_day: AtomicU64, // unix day number
    successes: AtomicU64,
    failures: AtomicU64,
    skips: AtomicU64,
}

impl DailyTracker {
    pub fn new() -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let day = now / 86400;

        Self {
            spent_lamports: AtomicU64::new(0),
            current_day: AtomicU64::new(day),
            successes: AtomicU64::new(0),
            failures: AtomicU64::new(0),
            skips: AtomicU64::new(0),
        }
    }

    /// Record a tip spend (success or failure).
    pub fn record_tip(&self, lamports: u64) {
        self.maybe_reset();
        self.spent_lamports.fetch_add(lamports, Ordering::Relaxed);
    }

    /// Record a successful liquidation.
    pub fn record_success(&self) {
        self.successes.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a failed attempt.
    pub fn record_failure(&self) {
        self.failures.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a skipped opportunity.
    pub fn record_skip(&self) {
        self.skips.fetch_add(1, Ordering::Relaxed);
    }

    /// Total tip spend today in lamports.
    pub fn spent_today(&self) -> u64 {
        self.maybe_reset();
        self.spent_lamports.load(Ordering::Relaxed)
    }

    /// Daily stats.
    pub fn stats(&self) -> (u64, u64, u64, u64) {
        (
            self.spent_lamports.load(Ordering::Relaxed),
            self.successes.load(Ordering::Relaxed),
            self.failures.load(Ordering::Relaxed),
            self.skips.load(Ordering::Relaxed),
        )
    }

    /// Reset if a new day has started.
    fn maybe_reset(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let today = now / 86400;
        let stored = self.current_day.load(Ordering::Relaxed);

        if today > stored {
            // New day — reset counters
            if self.current_day.compare_exchange(stored, today, Ordering::SeqCst, Ordering::Relaxed).is_ok() {
                let spent = self.spent_lamports.swap(0, Ordering::Relaxed);
                let s = self.successes.swap(0, Ordering::Relaxed);
                let f = self.failures.swap(0, Ordering::Relaxed);
                let sk = self.skips.swap(0, Ordering::Relaxed);
                tracing::info!(
                    spent_lamports = spent,
                    successes = s,
                    failures = f,
                    skips = sk,
                    "daily tracker reset — new day"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_too_small() {
        let config = RiskConfig::default();
        let tracker = DailyTracker::new();
        let decision = evaluate_opportunity(&config, 1000, 6, 1.0, &tracker); // $0.001
        assert!(matches!(decision, EvDecision::SkipTooSmall { .. }));
    }

    #[test]
    fn skip_low_ev() {
        let config = RiskConfig {
            min_repay_amount: 0,
            min_estimated_bonus_usd: 100.0,
            ..Default::default()
        };
        let tracker = DailyTracker::new();
        // $100 repay × 1.1% bonus = $1.10 < $100 threshold
        let decision = evaluate_opportunity(&config, 100_000_000, 6, 1.0, &tracker);
        assert!(matches!(decision, EvDecision::SkipLowEv { .. }));
    }

    #[test]
    fn submit_profitable() {
        let config = RiskConfig::default();
        let tracker = DailyTracker::new();
        // $50,000 repay × 1.1% bonus = $550 — should submit
        let decision = evaluate_opportunity(&config, 50_000_000_000, 6, 1.0, &tracker);
        match decision {
            EvDecision::Submit { estimated_bonus_usd, recommended_tip_lamports } => {
                assert!(estimated_bonus_usd > 500.0);
                assert!(recommended_tip_lamports > 0);
                assert!(recommended_tip_lamports <= config.max_tip_per_tx_lamports);
            }
            _ => panic!("expected Submit, got {:?}", decision),
        }
    }

    #[test]
    fn daily_cap_enforced() {
        let config = RiskConfig {
            daily_tip_cap_lamports: 1000,
            min_repay_amount: 0,
            min_estimated_bonus_usd: 0.0,
            ..Default::default()
        };
        let tracker = DailyTracker::new();
        tracker.record_tip(1001); // exceed cap
        let decision = evaluate_opportunity(&config, 50_000_000_000, 6, 1.0, &tracker);
        assert!(matches!(decision, EvDecision::SkipDailyCapReached { .. }));
    }

    #[test]
    fn tracker_resets_daily() {
        let tracker = DailyTracker::new();
        tracker.record_tip(5000);
        assert_eq!(tracker.spent_today(), 5000);
        // Can't easily test the reset without mocking time, but verify it doesn't panic
    }
}
