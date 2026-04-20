//! Canonical event types matching the ClickHouse schema.
//!
//! These structs are the contract between processors (which produce events)
//! and the writer (which inserts them into ClickHouse). They mirror the
//! columns in `schema/migrations/001_initial_schema.sql`.

use chrono::{DateTime, Utc};

/// Serialize DateTime<Utc> as "YYYY-MM-DD HH:MM:SS.sss" for ClickHouse JSONEachRow.
mod ch_datetime {
    use chrono::{DateTime, Utc};
    use serde::Serializer;
    pub fn serialize<S: Serializer>(dt: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&dt.format("%Y-%m-%d %H:%M:%S%.3f").to_string())
    }
}
use serde::Serialize;

/// Venue identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Venue {
    Kamino,
    JupiterLend,
    Marginfi,
    Save,
}

impl Venue {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Kamino => "kamino",
            Self::JupiterLend => "jupiter_lend",
            Self::Marginfi => "marginfi",
            Self::Save => "save",
        }
    }
}

impl std::fmt::Display for Venue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A successful liquidation event — one row in `liquidations`.
#[derive(Debug, Clone, Serialize)]
pub struct LiquidationEvent {
    // Identity
    pub venue: String,
    pub program_id: String,
    pub slot: u64,
    #[serde(serialize_with = "ch_datetime::serialize")]
    pub block_time: DateTime<Utc>,
    pub tx_signature: String,
    pub ix_index: u16,
    pub inner_ix_index: Option<u16>,

    // Participants
    pub liquidator: String,
    pub liquidatee: Option<String>,
    pub obligation: String,
    pub market: String,

    // Collateral & Debt
    pub collateral_reserve: String,
    pub debt_reserve: String,
    pub collateral_mint: String,
    pub debt_mint: String,
    pub repay_amount: u128,
    pub withdraw_amount: u128,

    // USD values (enriched)
    pub repay_amount_usd: Option<f64>,
    pub collateral_seized_usd: Option<f64>,
    pub liquidator_profit_usd: Option<f64>,
    pub collateral_price: Option<f64>,
    pub debt_price: Option<f64>,

    // Denormalized obligation size
    pub obligation_deposited_usd: Option<f64>,
    pub obligation_borrowed_usd: Option<f64>,

    // Bonus & Fees
    pub liquidation_bonus_bps: Option<u32>,
    pub close_factor_pct: Option<u16>,
    pub protocol_fee_amount: Option<u128>,
    pub insurance_fee_amount: Option<u128>,

    // Transaction metadata
    pub tx_fee_lamports: u64,
    pub priority_fee_lamports: u64,
    pub jito_tip_lamports: Option<u64>,
    pub compute_units_consumed: u32,

    // Bundling
    pub used_flashloan: bool,
    pub flashloan_source: Option<String>,
    pub used_jupiter_swap: bool,

    // Venue-specific
    pub liquidation_reason: Option<String>,
    pub tick_start: Option<i32>,
    pub tick_end: Option<i32>,
    pub absorbed_bad_debt: Option<bool>,

    // Raw
    pub raw_ix_data: String,
}

/// A failed liquidation attempt — one row in `failed_liquidation_attempts`.
#[derive(Debug, Clone, Serialize)]
pub struct FailedLiquidationEvent {
    /// All fields from LiquidationEvent, flattened into this struct for ClickHouse.
    #[serde(flatten)]
    pub base: LiquidationEvent,
    pub error_code: Option<u32>,
    pub error_message: Option<String>,
}

/// Obligation snapshot at liquidation time — one row in `obligations_snapshots`.
#[derive(Debug, Clone, Serialize)]
pub struct ObligationSnapshot {
    pub venue: String,
    pub slot: u64,
    #[serde(serialize_with = "ch_datetime::serialize")]
    pub block_time: DateTime<Utc>,
    pub tx_signature: String,
    pub ix_index: u16,
    pub obligation: String,
    pub owner: Option<String>,
    pub market: String,

    // Health
    pub deposited_value_usd: Option<f64>,
    pub borrowed_value_usd: Option<f64>,
    pub ltv: Option<f64>,
    pub unhealthy_ltv: Option<f64>,
    pub health_factor: Option<f64>,

    // Positions (JSON arrays)
    pub deposits: String,
    pub borrows: String,

    // Raw
    pub obligation_data_b64: Option<String>,
}

/// Reserve snapshot at liquidation time — one row in `reserves_snapshots`.
#[derive(Debug, Clone, Serialize)]
pub struct ReserveSnapshot {
    pub venue: String,
    pub slot: u64,
    #[serde(serialize_with = "ch_datetime::serialize")]
    pub block_time: DateTime<Utc>,
    pub reserve: String,
    pub market: String,
    pub mint: String,
    pub role: String, // "repay" or "withdraw"

    pub available_liquidity: Option<u128>,
    pub total_borrows: Option<u128>,
    pub utilization_pct: Option<f64>,

    pub liquidation_threshold_bps: Option<u16>,
    pub liquidation_bonus_bps: Option<u32>,
    pub max_liquidation_bonus_bps: Option<u32>,
    pub protocol_liquidation_fee_bps: Option<u16>,
    pub flash_loan_fee_bps: Option<u32>,

    pub oracle_price: Option<f64>,
    pub oracle_source: Option<String>,
    pub venue_ext: Option<String>, // JSON for venue-specific fields

    pub reserve_data_b64: Option<String>,
}

/// Transaction metadata — one row in `tx_metadata`.
#[derive(Debug, Clone, Serialize)]
pub struct TxMetadata {
    pub tx_signature: String,
    pub slot: u64,
    #[serde(serialize_with = "ch_datetime::serialize")]
    pub block_time: DateTime<Utc>,
    pub succeeded: bool,
    pub fee_lamports: u64,
    pub priority_fee_lamports: u64,
    pub jito_tip_lamports: Option<u64>,
    pub compute_units_consumed: u32,
    pub compute_units_requested: Option<u32>,
    pub num_instructions: u16,
    pub num_inner_instructions: u16,
    pub signers: Vec<String>,
    pub fee_payer: String,
    pub uses_address_lookup_table: bool,
}

/// Complete output from processing one transaction.
#[derive(Debug, Clone)]
pub struct ProcessedTransaction {
    pub tx_meta: TxMetadata,
    pub liquidations: Vec<LiquidationEvent>,
    pub failed_attempts: Vec<FailedLiquidationEvent>,
    pub obligation_snapshots: Vec<ObligationSnapshot>,
    pub reserve_snapshots: Vec<ReserveSnapshot>,
}
