// CLI tool: using `println!`/`eprintln!` is intentional for user-facing
// progress output. `tracing` is still used for structured events.
#![allow(clippy::print_stdout, clippy::print_stderr)]

//! Backfill binary: fetches historical liquidation transactions and writes
//! decoded events to ClickHouse.
//!
//! Two modes:
//!
//! 1. **Signature file mode** (recommended, cheap):
//!    Export liquidation signatures from Dune Analytics, then fetch only those txs.
//!    ~14,355 getTransaction calls for one month of Kamino ≈ ~$7 in RPC credits.
//!
//!    BACKFILL_SIGNATURES_FILE=sigs.txt cargo run -p backfill
//!
//! 2. **Slot range mode** (expensive, scans every block):
//!    Iterates through every slot in a range. Most blocks have zero liquidations.
//!    Use only for small ranges or when you need complete coverage.
//!
//!    BACKFILL_START_SLOT=414544140 BACKFILL_END_SLOT=414544150 cargo run -p backfill

mod block_fetcher;
mod config;
mod sig_fetcher;
mod tx_parser;

use anyhow::Result;
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

use indexer_core::events::ProcessedTransaction;
use indexer_core::progress::ProgressTracker;
use indexer_core::writer;

use config::BackfillMode;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cfg = config::BackfillConfig::from_env()?;

    match &cfg.mode {
        BackfillMode::SignatureFile { path } => {
            tracing::info!(file = %path, "signature file mode");
        }
        BackfillMode::SlotRange { start_slot, end_slot } => {
            tracing::info!(start = start_slot, end = ?end_slot, "slot range mode");
        }
    }

    // Channel: fetcher → processor → writer
    let (tx_sender, tx_receiver) = mpsc::channel::<ProcessedTransaction>(1024);

    // Spawn the ClickHouse writer actor
    let writer_config = writer::WriterConfig {
        url: cfg.clickhouse_url.clone(),
        database: cfg.clickhouse_database.clone(),
        user: cfg.clickhouse_user.clone(),
        password: cfg.clickhouse_password.clone(),
        batch_size: 10_000,
        flush_interval_secs: 1,
    };

    let writer_handle = tokio::spawn(async move {
        if let Err(e) = writer::writer_actor(writer_config, tx_receiver).await {
            tracing::error!(error = %e, "writer actor failed");
        }
    });

    // Run the appropriate backfill mode
    let mut progress = ProgressTracker::new();

    let result = match &cfg.mode {
        BackfillMode::SignatureFile { path } => {
            sig_fetcher::run_signature_backfill(&cfg, path, tx_sender, &mut progress).await
        }
        BackfillMode::SlotRange { .. } => {
            block_fetcher::run_backfill(&cfg, tx_sender, &mut progress).await
        }
    };

    match &result {
        Ok(()) => {
            tracing::info!("backfill completed successfully");
        }
        Err(e) => {
            tracing::error!(error = %e, "backfill failed");
        }
    }

    // Wait for writer to finish
    writer_handle.await?;

    result
}
