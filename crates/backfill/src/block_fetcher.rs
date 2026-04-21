//! Block fetcher: iterates through slot ranges, fetches blocks from RPC,
//! parses transactions, runs processors, and sends results to the writer.
//!
//! Supports both Old Faithful (local RPC from CAR files) and Triton RPC.

use anyhow::{Context, Result};
use solana_client::rpc_client::RpcClient;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_transaction_status::{
    TransactionDetails, UiTransactionEncoding,
};
use tokio::sync::mpsc;

use indexer_core::events::ProcessedTransaction;
use indexer_core::progress::ProgressTracker;

use crate::config::BackfillConfig;
use crate::tx_parser;

/// Run the backfill pipeline: fetch blocks, process transactions, send to writer.
pub async fn run_backfill(
    config: &BackfillConfig,
    tx_sender: mpsc::Sender<ProcessedTransaction>,
    progress: &mut ProgressTracker,
) -> Result<()> {
    let rpc = RpcClient::new_with_commitment(
        config.rpc_url.clone(),
        CommitmentConfig::confirmed(),
    );

    let (start_slot, end_slot_opt) = match &config.mode {
        crate::config::BackfillMode::SlotRange { start_slot, end_slot } => (*start_slot, *end_slot),
        _ => return Ok(()), // Not slot range mode
    };

    let end_slot = match end_slot_opt {
        Some(s) => s,
        None => {
            let slot = rpc.get_slot().context("failed to get current slot")?;
            tracing::info!(current_slot = slot, "no end_slot specified, using current");
            slot
        }
    };
    let total_slots = end_slot.saturating_sub(start_slot);
    tracing::info!(
        start = start_slot,
        end = end_slot,
        total_slots = total_slots,
        "backfill range"
    );

    let mut processed_slots = 0u64;
    let mut processed_txs = 0u64;
    let mut found_liquidations = 0u64;
    let mut found_failed = 0u64;
    let mut skipped_slots = 0u64;

    let mut current_slot = start_slot;

    while current_slot <= end_slot {
        // Fetch block
        let block = match fetch_block_json(&rpc, current_slot) {
            Ok(Some(block)) => block,
            Ok(None) => {
                // Slot was skipped (no block produced)
                skipped_slots += 1;
                current_slot += 1;
                continue;
            }
            Err(e) => {
                tracing::warn!(slot = current_slot, error = %e, "failed to fetch block, skipping");
                skipped_slots += 1;
                current_slot += 1;
                continue;
            }
        };

        let block_time = block["blockTime"].as_i64().unwrap_or(0);
        let transactions = block["transactions"].as_array();

        if let Some(txs) = transactions {
            for tx_json in txs {
                // Skip failed txs if not configured to include them
                if !config.include_failed {
                    let err = &tx_json["meta"]["err"];
                    if !err.is_null() {
                        continue;
                    }
                }

                // Parse transaction into TxContext
                let ctx = match tx_parser::parse_transaction(tx_json, current_slot, block_time) {
                    Ok(Some(ctx)) => ctx,
                    Ok(None) => continue, // Doesn't touch our programs
                    Err(e) => {
                        tracing::debug!(
                            slot = current_slot,
                            error = %e,
                            "failed to parse transaction"
                        );
                        continue;
                    }
                };

                // Process through all venue processors
                let result = match processors::process_transaction(&ctx) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(
                            slot = current_slot,
                            tx = %ctx.tx_signature,
                            error = %e,
                            "failed to process transaction"
                        );
                        continue;
                    }
                };

                let liq_count = result.liquidations.len() as u64;
                let fail_count = result.failed_attempts.len() as u64;
                found_liquidations += liq_count;
                found_failed += fail_count;

                if liq_count > 0 || fail_count > 0 {
                    tracing::info!(
                        slot = current_slot,
                        tx = %ctx.tx_signature,
                        liquidations = liq_count,
                        failed = fail_count,
                        "found liquidation event(s)"
                    );
                }

                // Send to writer if there's anything to write
                if liq_count > 0 || fail_count > 0 {
                    if tx_sender.send(result).await.is_err() {
                        tracing::error!("writer channel closed");
                        return Ok(());
                    }
                }

                processed_txs += 1;
            }
        }

        processed_slots += 1;
        current_slot += 1;

        // Log progress periodically
        if processed_slots % 1000 == 0 {
            let pct = if total_slots > 0 {
                (processed_slots as f64 / total_slots as f64) * 100.0
            } else {
                100.0
            };
            tracing::info!(
                slot = current_slot,
                processed_slots = processed_slots,
                skipped = skipped_slots,
                txs = processed_txs,
                liquidations = found_liquidations,
                failed = found_failed,
                progress_pct = format!("{:.1}%", pct),
                "backfill progress"
            );
        }

        // Update progress tracker
        for venue in ["kamino", "jupiter_lend", "marginfi", "save"] {
            let rec = progress.get_or_create(venue, "old_faithful");
            rec.advance(
                current_slot,
                "",
                0, 0, 0, // Per-venue counts not tracked at this level
            );
        }
    }

    tracing::info!(
        processed_slots = processed_slots,
        skipped = skipped_slots,
        txs_touching_programs = processed_txs,
        liquidations = found_liquidations,
        failed_attempts = found_failed,
        "backfill complete"
    );

    Ok(())
}

/// Fetch a block from RPC as JSON.
///
/// Uses `getBlock` with json encoding and full transaction details.
/// Returns None for skipped slots (no block produced).
fn fetch_block_json(
    rpc: &RpcClient,
    slot: u64,
) -> Result<Option<serde_json::Value>> {
    use solana_client::rpc_config::RpcBlockConfig;

    let config = RpcBlockConfig {
        encoding: Some(UiTransactionEncoding::Json),
        transaction_details: Some(TransactionDetails::Full),
        rewards: Some(false),
        commitment: Some(CommitmentConfig::confirmed()),
        max_supported_transaction_version: Some(0),
    };

    match rpc.get_block_with_config(slot, config) {
        Ok(block) => {
            // Serialize to JSON for our parser (the parser works on JSON values)
            let json = serde_json::to_value(&block)?;
            Ok(Some(json))
        }
        Err(e) => {
            let err_str = e.to_string();
            // Slot was skipped — not an error
            if err_str.contains("Slot was skipped")
                || err_str.contains("was cleaned up")
                || err_str.contains("not available")
            {
                Ok(None)
            } else {
                Err(e.into())
            }
        }
    }
}
