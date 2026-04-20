//! Backfill binary: fetches blocks from RPC (Old Faithful or Triton),
//! decodes liquidation events across all four venues, writes to ClickHouse.
//!
//! Architecture:
//!   1. Block fetcher (RPC) → channel → processor → channel → ClickHouse writer
//!   2. Per-venue filter at the instruction level (not block level)
//!   3. Idempotent: ReplacingMergeTree deduplicates on (tx_signature, ix_index)
//!   4. Resumable: _indexer_progress tracks last slot per venue
//!
//! Usage:
//!   SOLANA_RPC_URL=<old-faithful-or-triton> \
//!   CLICKHOUSE_URL=http://localhost:8123 \
//!   cargo run -p backfill -- --start-slot <N> --end-slot <M>
//!
//! For Old Faithful backfill:
//!   1. Download epoch CAR: curl -O https://files.old-faithful.net/<epoch>/epoch-<epoch>.car
//!   2. Run local RPC: faithful-cli rpc epoch-<epoch>.car --listen :8899
//!   3. Point SOLANA_RPC_URL=http://localhost:8899

mod block_fetcher;
mod config;
mod tx_parser;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

use indexer_core::events::ProcessedTransaction;
use indexer_core::progress::ProgressTracker;
use indexer_core::writer;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cfg = config::BackfillConfig::from_env()?;
    tracing::info!(
        rpc_url = %cfg.rpc_url,
        start_slot = cfg.start_slot,
        end_slot = ?cfg.end_slot,
        "starting backfill"
    );

    // Channel: block_fetcher → processor → writer
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

    // Run the block fetcher + processor pipeline
    let mut progress = ProgressTracker::new();
    let fetch_result = block_fetcher::run_backfill(&cfg, tx_sender, &mut progress).await;

    match &fetch_result {
        Ok(()) => {
            tracing::info!("backfill completed successfully");
            for rec in progress.all_records() {
                tracing::info!(
                    venue = %rec.venue,
                    last_slot = rec.last_slot,
                    liquidations = rec.rows_liquidations,
                    failed = rec.rows_failed,
                    total = rec.rows_total_processed,
                    "venue progress"
                );
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "backfill failed");
        }
    }

    // Wait for writer to finish
    writer_handle.await?;

    fetch_result
}
