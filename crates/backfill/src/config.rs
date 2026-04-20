//! Backfill configuration from environment variables.

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct BackfillConfig {
    /// Solana RPC URL (Old Faithful local or Triton remote).
    pub rpc_url: String,

    /// Start slot for backfill.
    pub start_slot: u64,

    /// End slot for backfill (None = run until latest).
    pub end_slot: Option<u64>,

    /// How many blocks to fetch per batch.
    pub batch_size: u64,

    /// ClickHouse connection.
    pub clickhouse_url: String,
    pub clickhouse_database: String,
    pub clickhouse_user: String,
    pub clickhouse_password: String,

    /// Whether to include failed transactions.
    pub include_failed: bool,
}

impl BackfillConfig {
    pub fn from_env() -> Result<Self> {
        let rpc_url = std::env::var("SOLANA_RPC_URL")
            .context("SOLANA_RPC_URL not set")?;

        let start_slot = std::env::var("BACKFILL_START_SLOT")
            .unwrap_or_else(|_| "0".to_string())
            .parse::<u64>()
            .context("invalid BACKFILL_START_SLOT")?;

        let end_slot = std::env::var("BACKFILL_END_SLOT")
            .ok()
            .and_then(|s| s.parse::<u64>().ok());

        let batch_size = std::env::var("BACKFILL_BATCH_SIZE")
            .unwrap_or_else(|_| "100".to_string())
            .parse::<u64>()
            .unwrap_or(100);

        Ok(Self {
            rpc_url,
            start_slot,
            end_slot,
            batch_size,
            clickhouse_url: std::env::var("CLICKHOUSE_URL")
                .unwrap_or_else(|_| "http://localhost:8123".to_string()),
            clickhouse_database: std::env::var("CLICKHOUSE_DATABASE")
                .unwrap_or_else(|_| "default".to_string()),
            clickhouse_user: std::env::var("CLICKHOUSE_USER")
                .unwrap_or_else(|_| "default".to_string()),
            clickhouse_password: std::env::var("CLICKHOUSE_PASSWORD")
                .unwrap_or_default(),
            include_failed: std::env::var("BACKFILL_INCLUDE_FAILED")
                .unwrap_or_else(|_| "true".to_string())
                .parse()
                .unwrap_or(true),
        })
    }
}
