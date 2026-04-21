//! Backfill configuration from environment variables.

use anyhow::{Context, Result};

/// Backfill mode: either scan a slot range or fetch specific signatures.
#[derive(Debug, Clone)]
pub enum BackfillMode {
    /// Scan blocks slot-by-slot (expensive: touches every block in range).
    SlotRange {
        start_slot: u64,
        end_slot: Option<u64>,
    },
    /// Fetch specific transactions by signature from a file (cheap: only the txs you want).
    /// File format: one base58 signature per line. Lines starting with # are comments.
    /// Export from Dune, then fetch raw txs via getTransaction (~$7 per 14K sigs).
    SignatureFile {
        path: String,
    },
}

#[derive(Debug, Clone)]
pub struct BackfillConfig {
    /// Solana RPC URL (Triton).
    pub rpc_url: String,

    /// Backfill mode.
    pub mode: BackfillMode,

    /// How many concurrent RPC requests (for signature mode).
    pub concurrency: usize,

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

        // Determine mode: signature file takes priority over slot range
        let mode = if let Ok(path) = std::env::var("BACKFILL_SIGNATURES_FILE") {
            BackfillMode::SignatureFile { path }
        } else {
            let start_slot = std::env::var("BACKFILL_START_SLOT")
                .unwrap_or_else(|_| "0".to_string())
                .parse::<u64>()
                .context("invalid BACKFILL_START_SLOT")?;
            let end_slot = std::env::var("BACKFILL_END_SLOT")
                .ok()
                .and_then(|s| s.parse::<u64>().ok());
            BackfillMode::SlotRange { start_slot, end_slot }
        };

        let concurrency = std::env::var("BACKFILL_CONCURRENCY")
            .unwrap_or_else(|_| "10".to_string())
            .parse()
            .unwrap_or(10);

        Ok(Self {
            rpc_url,
            mode,
            concurrency,
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
