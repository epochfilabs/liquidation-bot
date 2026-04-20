//! ClickHouse batch writer.
//!
//! Accepts events via channels and flushes to ClickHouse in batches
//! (>=10k rows or 1s flush interval, whichever comes first).
//!
//! Uses the `clickhouse` crate's `inserter` feature for efficient
//! batch writes. Events are serialized to ClickHouse's row format
//! via serde.

use anyhow::{Context, Result};
use clickhouse::Client;
use tokio::sync::mpsc;

use crate::events::{
    FailedLiquidationEvent, LiquidationEvent, ObligationSnapshot, ProcessedTransaction,
    ReserveSnapshot, TxMetadata,
};

/// Configuration for the ClickHouse writer.
#[derive(Debug, Clone)]
pub struct WriterConfig {
    pub url: String,
    pub database: String,
    pub user: String,
    pub password: String,
    pub batch_size: usize,
    pub flush_interval_secs: u64,
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:8123".to_string(),
            database: "default".to_string(),
            user: "default".to_string(),
            password: String::new(),
            batch_size: 10_000,
            flush_interval_secs: 1,
        }
    }
}

/// Batched ClickHouse writer.
pub struct ClickHouseWriter {
    client: Client,
    config: WriterConfig,
    liquidations: Vec<LiquidationEvent>,
    failed_attempts: Vec<FailedLiquidationEvent>,
    obligation_snapshots: Vec<ObligationSnapshot>,
    reserve_snapshots: Vec<ReserveSnapshot>,
    tx_metadata: Vec<TxMetadata>,
    total_written: WriterStats,
}

#[derive(Debug, Clone, Default)]
pub struct WriterStats {
    pub liquidations: u64,
    pub failed_attempts: u64,
    pub obligation_snapshots: u64,
    pub reserve_snapshots: u64,
    pub tx_metadata: u64,
    pub flush_count: u64,
}

impl ClickHouseWriter {
    pub fn new(config: WriterConfig) -> Result<Self> {
        let client = Client::default()
            .with_url(&config.url)
            .with_database(&config.database)
            .with_user(&config.user)
            .with_password(&config.password);

        Ok(Self {
            client,
            config,
            liquidations: Vec::with_capacity(10_000),
            failed_attempts: Vec::with_capacity(1_000),
            obligation_snapshots: Vec::with_capacity(10_000),
            reserve_snapshots: Vec::with_capacity(20_000),
            tx_metadata: Vec::with_capacity(10_000),
            total_written: WriterStats::default(),
        })
    }

    pub fn ingest(&mut self, tx: ProcessedTransaction) {
        self.tx_metadata.push(tx.tx_meta);
        self.liquidations.extend(tx.liquidations);
        self.failed_attempts.extend(tx.failed_attempts);
        self.obligation_snapshots.extend(tx.obligation_snapshots);
        self.reserve_snapshots.extend(tx.reserve_snapshots);
    }

    pub fn buffer_size(&self) -> usize {
        self.liquidations.len()
            + self.failed_attempts.len()
            + self.obligation_snapshots.len()
            + self.reserve_snapshots.len()
            + self.tx_metadata.len()
    }

    pub fn should_flush(&self) -> bool {
        self.buffer_size() >= self.config.batch_size
    }

    /// Flush all buffered events to ClickHouse via JSON insert.
    pub async fn flush(&mut self) -> Result<()> {
        if self.buffer_size() == 0 {
            return Ok(());
        }

        let liq_count = self.liquidations.len();
        let fail_count = self.failed_attempts.len();
        let obl_count = self.obligation_snapshots.len();
        let res_count = self.reserve_snapshots.len();
        let tx_count = self.tx_metadata.len();

        tracing::info!(
            liquidations = liq_count,
            failed = fail_count,
            obligations = obl_count,
            reserves = res_count,
            tx_meta = tx_count,
            "flushing batch to ClickHouse"
        );

        // Flush each table via JSON insert
        if !self.liquidations.is_empty() {
            let rows = std::mem::take(&mut self.liquidations);
            self.insert_json("liquidations", &rows).await
                .context("failed to flush liquidations")?;
        }
        if !self.failed_attempts.is_empty() {
            let rows = std::mem::take(&mut self.failed_attempts);
            self.insert_json("failed_liquidation_attempts", &rows).await
                .context("failed to flush failed_liquidation_attempts")?;
        }
        if !self.obligation_snapshots.is_empty() {
            let rows = std::mem::take(&mut self.obligation_snapshots);
            self.insert_json("obligations_snapshots", &rows).await
                .context("failed to flush obligations_snapshots")?;
        }
        if !self.reserve_snapshots.is_empty() {
            let rows = std::mem::take(&mut self.reserve_snapshots);
            self.insert_json("reserves_snapshots", &rows).await
                .context("failed to flush reserves_snapshots")?;
        }
        if !self.tx_metadata.is_empty() {
            let rows = std::mem::take(&mut self.tx_metadata);
            self.insert_json("tx_metadata", &rows).await
                .context("failed to flush tx_metadata")?;
        }

        self.total_written.liquidations += liq_count as u64;
        self.total_written.failed_attempts += fail_count as u64;
        self.total_written.obligation_snapshots += obl_count as u64;
        self.total_written.reserve_snapshots += res_count as u64;
        self.total_written.tx_metadata += tx_count as u64;
        self.total_written.flush_count += 1;

        Ok(())
    }

    pub fn stats(&self) -> &WriterStats {
        &self.total_written
    }

    /// Insert rows as JSON via ClickHouse HTTP interface.
    async fn insert_json<T: serde::Serialize>(
        &self,
        table: &str,
        rows: &[T],
    ) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }

        // Build NDJSON (one JSON object per line)
        let mut body = String::with_capacity(rows.len() * 512);
        for row in rows {
            let json = serde_json::to_string(row)
                .context("failed to serialize row")?;
            body.push_str(&json);
            body.push('\n');
        }

        // POST to ClickHouse: INSERT INTO table FORMAT JSONEachRow
        let url = format!(
            "{}/?database={}&user={}&password={}&query=INSERT%20INTO%20{}%20FORMAT%20JSONEachRow",
            self.config.url, self.config.database,
            self.config.user, self.config.password,
            table
        );

        let response = reqwest::Client::new()
            .post(&url)
            .header("Content-Type", "application/x-ndjson")
            .body(body)
            .send()
            .await
            .context("ClickHouse HTTP request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("ClickHouse insert failed ({}): {}", status, body);
        }

        tracing::debug!(table = table, rows = rows.len(), "inserted rows");
        Ok(())
    }
}

/// Run a writer actor that receives processed transactions from a channel.
pub async fn writer_actor(
    config: WriterConfig,
    mut rx: mpsc::Receiver<ProcessedTransaction>,
) -> Result<()> {
    let mut writer = ClickHouseWriter::new(config.clone())?;
    let flush_interval = tokio::time::Duration::from_secs(config.flush_interval_secs);
    let mut flush_timer = tokio::time::interval(flush_interval);
    flush_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            Some(tx) = rx.recv() => {
                writer.ingest(tx);
                if writer.should_flush() {
                    if let Err(e) = writer.flush().await {
                        tracing::error!(error = %e, "ClickHouse flush failed");
                    }
                }
            }
            _ = flush_timer.tick() => {
                if writer.buffer_size() > 0 {
                    if let Err(e) = writer.flush().await {
                        tracing::error!(error = %e, "ClickHouse periodic flush failed");
                    }
                }
            }
            else => {
                tracing::info!("writer channel closed, performing final flush");
                writer.flush().await?;
                let stats = writer.stats();
                tracing::info!(
                    liquidations = stats.liquidations,
                    failed = stats.failed_attempts,
                    obligations = stats.obligation_snapshots,
                    reserves = stats.reserve_snapshots,
                    tx_meta = stats.tx_metadata,
                    flushes = stats.flush_count,
                    "writer actor finished"
                );
                return Ok(());
            }
        }
    }
}
