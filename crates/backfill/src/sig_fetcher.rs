//! Signature-based fetcher with concurrent RPC calls.
//!
//! Reads transaction signatures from a file, fetches them in parallel via
//! getTransaction, processes through venue processors, and sends results
//! to the ClickHouse writer.
//!
//! With BACKFILL_CONCURRENCY=10 (default), processes ~20 tx/sec instead of ~2.
//! 42,768 signatures takes ~35 minutes instead of 6 hours.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use solana_client::rpc_client::RpcClient;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_transaction_status::UiTransactionEncoding;
use tokio::sync::{mpsc, Semaphore};

use indexer_core::events::ProcessedTransaction;
use indexer_core::progress::ProgressTracker;

use crate::config::BackfillConfig;
use crate::tx_parser;

/// Run the signature-based backfill with concurrent fetching.
pub async fn run_signature_backfill(
    config: &BackfillConfig,
    sig_file_path: &str,
    tx_sender: mpsc::Sender<ProcessedTransaction>,
    _progress: &mut ProgressTracker,
) -> Result<()> {
    let signatures = read_signatures(sig_file_path)?;
    let total = signatures.len();
    tracing::info!(file = sig_file_path, signatures = total, concurrency = config.concurrency, "loaded signatures");

    if total == 0 {
        tracing::warn!("no signatures found in file");
        return Ok(());
    }

    let rpc = Arc::new(RpcClient::new_with_commitment(
        config.rpc_url.clone(),
        CommitmentConfig::confirmed(),
    ));
    let semaphore = Arc::new(Semaphore::new(config.concurrency));
    let tx_sender = Arc::new(tx_sender);

    // Counters
    let processed = Arc::new(AtomicU64::new(0));
    let liquidations = Arc::new(AtomicU64::new(0));
    let failed_attempts = Arc::new(AtomicU64::new(0));
    let fetch_errors = Arc::new(AtomicU64::new(0));

    // Process signatures concurrently
    stream::iter(signatures.into_iter().enumerate())
        .for_each_concurrent(config.concurrency, |(i, sig)| {
            let rpc = Arc::clone(&rpc);
            let sem = Arc::clone(&semaphore);
            let sender = Arc::clone(&tx_sender);
            let processed = Arc::clone(&processed);
            let liq_count = Arc::clone(&liquidations);
            let fail_count = Arc::clone(&failed_attempts);
            let err_count = Arc::clone(&fetch_errors);

            async move {
                let _permit = sem.acquire().await.unwrap();

                // Fetch transaction (blocking RPC call in spawn_blocking)
                let sig_clone = sig.clone();
                let rpc_clone = Arc::clone(&rpc);
                let tx_json = match tokio::task::spawn_blocking(move || {
                    fetch_transaction_json(&rpc_clone, &sig_clone)
                }).await {
                    Ok(Ok(Some(json))) => json,
                    Ok(Ok(None)) => {
                        err_count.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    Ok(Err(e)) => {
                        tracing::debug!(sig = %sig, error = %e, "fetch failed");
                        err_count.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    Err(e) => {
                        tracing::debug!(sig = %sig, error = %e, "spawn_blocking failed");
                        err_count.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                };

                let slot = tx_json["slot"].as_u64().unwrap_or(0);
                let block_time = tx_json["blockTime"].as_i64().unwrap_or(0);

                // Parse into TxContext
                let ctx = match tx_parser::parse_transaction(&tx_json, slot, block_time) {
                    Ok(Some(ctx)) => ctx,
                    Ok(None) => return,
                    Err(e) => {
                        tracing::debug!(sig = %sig, error = %e, "parse failed");
                        return;
                    }
                };

                // Process through venue processors
                let result = match processors::process_transaction(&ctx) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::debug!(sig = %sig, error = %e, "process failed");
                        return;
                    }
                };

                let liq = result.liquidations.len() as u64;
                let fail = result.failed_attempts.len() as u64;

                if liq > 0 || fail > 0 {
                    liq_count.fetch_add(liq, Ordering::Relaxed);
                    fail_count.fetch_add(fail, Ordering::Relaxed);

                    let _ = sender.send(result).await;
                }

                let done = processed.fetch_add(1, Ordering::Relaxed) + 1;

                // Progress logging every 500
                if done % 500 == 0 {
                    let pct = (done as f64 / total as f64) * 100.0;
                    tracing::info!(
                        progress = format!("{}/{} ({:.1}%)", done, total, pct),
                        liquidations = liq_count.load(Ordering::Relaxed),
                        failed = fail_count.load(Ordering::Relaxed),
                        errors = err_count.load(Ordering::Relaxed),
                        "backfill progress"
                    );
                }
            }
        })
        .await;

    let final_liq = liquidations.load(Ordering::Relaxed);
    let final_fail = failed_attempts.load(Ordering::Relaxed);
    let final_proc = processed.load(Ordering::Relaxed);
    let final_err = fetch_errors.load(Ordering::Relaxed);

    tracing::info!(
        total_signatures = total,
        fetched = final_proc,
        fetch_errors = final_err,
        liquidations = final_liq,
        failed_attempts = final_fail,
        "signature backfill complete"
    );

    Ok(())
}

/// Read transaction signatures from a file.
fn read_signatures(path: &str) -> Result<Vec<String>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read signatures file: {}", path))?;

    let mut sigs = Vec::new();
    let mut is_first_line = true;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let sig = if line.contains(',') {
            line.split(',').next().unwrap_or("").trim().trim_matches('"')
        } else {
            line
        };

        if is_first_line {
            is_first_line = false;
            if sig.contains("signature") || sig.contains("tx_") || sig.len() < 32 {
                continue;
            }
        }

        if sig.len() >= 80 && sig.len() <= 90 && sig.chars().all(|c| c.is_alphanumeric()) {
            sigs.push(sig.to_string());
        } else if sig.len() >= 32 {
            sigs.push(sig.to_string());
        }
    }

    Ok(sigs)
}

/// Fetch a single transaction by signature via RPC.
fn fetch_transaction_json(
    rpc: &RpcClient,
    signature: &str,
) -> Result<Option<serde_json::Value>> {
    use solana_sdk::signature::Signature;
    use std::str::FromStr;
    use solana_client::rpc_config::RpcTransactionConfig;

    let sig = Signature::from_str(signature)
        .with_context(|| format!("invalid signature: {}", signature))?;

    let config = RpcTransactionConfig {
        encoding: Some(UiTransactionEncoding::Json),
        commitment: Some(CommitmentConfig::confirmed()),
        max_supported_transaction_version: Some(0),
    };

    match rpc.get_transaction_with_config(&sig, config) {
        Ok(tx) => {
            let json = serde_json::to_value(&tx)?;
            Ok(Some(json))
        }
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("not found") || err_str.contains("not confirmed") {
                Ok(None)
            } else {
                Err(e.into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn read_plain_signatures() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, "# Kamino liquidation signatures - January 2026").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "5yH2TMYXno2as51NRCwMF9Lq1wpZfxSzwQiVge2absKz4TxRUYMEqZKarGEvL55555555555555555555").unwrap();
        writeln!(f, "4UEZWXXQKHnmnC6CmALsGewHJzDq8HpKLmTSu7777777777777777777777777777777777777777777777").unwrap();
        writeln!(f, "# comment line").unwrap();

        let sigs = read_signatures(f.path().to_str().unwrap()).unwrap();
        assert_eq!(sigs.len(), 2);
    }

    #[test]
    fn read_csv_signatures() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, "tx_id").unwrap();
        writeln!(f, "5yH2TMYXno2as51NRCwMF9Lq1wpZfxSzwQiVge2absKz4TxRUYMEqZKarGEvL55555555555555555555").unwrap();
        writeln!(f, "4UEZWXXQKHnmnC6CmALsGewHJzDq8HpKLmTSu7777777777777777777777777777777777777777777777").unwrap();

        let sigs = read_signatures(f.path().to_str().unwrap()).unwrap();
        assert_eq!(sigs.len(), 2);
    }

    #[test]
    fn skip_empty_and_comments() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, "# header comment").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "   ").unwrap();

        let sigs = read_signatures(f.path().to_str().unwrap()).unwrap();
        assert_eq!(sigs.len(), 0);
    }
}
