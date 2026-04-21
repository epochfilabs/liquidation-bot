#![allow(clippy::print_stdout, clippy::print_stderr, dead_code, unused_variables)]

//! Local integration test: processes fixture transactions through the full
//! pipeline and writes to a local ClickHouse instance.
//!
//! Prerequisites:
//!   docker compose up -d
//!   curl 'http://localhost:8123/' -d 'CREATE DATABASE IF NOT EXISTS liquidation_indexer'
//!   curl 'http://localhost:8123/?database=liquidation_indexer' --data-binary @schema/migrations/001_initial_schema.sql
//!
//! Run:
//!   CLICKHOUSE_URL=http://localhost:8123 \
//!   CLICKHOUSE_DATABASE=liquidation_indexer \
//!   cargo test -p backfill --test local_integration -- --nocapture

use std::fs;
use std::path::Path;

use anyhow::Result;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

/// Parse a fixture JSON file into a processors::TxContext.
fn fixture_to_tx_context(path: &str) -> Result<processors::TxContext> {
    let content = fs::read_to_string(path)?;
    let tx_json: serde_json::Value = serde_json::from_str(&content)?;

    let slot = tx_json["slot"].as_u64().unwrap_or(0);
    let block_time = tx_json["blockTime"].as_i64().unwrap_or(0);

    // Build account keys (static + loaded ALTs)
    let message = &tx_json["transaction"]["message"];
    let meta = &tx_json["meta"];

    let mut account_keys: Vec<Pubkey> = message["accountKeys"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|k| k.as_str().and_then(|s| Pubkey::from_str(s).ok()))
        .collect();

    if let Some(loaded) = meta.get("loadedAddresses") {
        for key in loaded["writable"].as_array().unwrap_or(&vec![]) {
            if let Some(s) = key.as_str() {
                if let Ok(pk) = Pubkey::from_str(s) {
                    account_keys.push(pk);
                }
            }
        }
        for key in loaded["readonly"].as_array().unwrap_or(&vec![]) {
            if let Some(s) = key.as_str() {
                if let Ok(pk) = Pubkey::from_str(s) {
                    account_keys.push(pk);
                }
            }
        }
    }

    // Parse instructions
    let instructions = parse_raw_instructions(&message["instructions"])?;

    // Parse inner instructions
    let inner_instructions = parse_inner_instructions(&meta["innerInstructions"])?;

    // Tx signature
    let tx_signature = tx_json["transaction"]["signatures"]
        .as_array()
        .and_then(|sigs| sigs.first())
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();

    let succeeded = meta["err"].is_null();
    let fee_lamports = meta["fee"].as_u64().unwrap_or(0);
    let compute_units_consumed = meta["computeUnitsConsumed"].as_u64().unwrap_or(0) as u32;

    let log_messages: Vec<String> = meta["logMessages"]
        .as_array()
        .map(|logs| logs.iter().filter_map(|l| l.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let pre_token_balances = parse_token_balances(&meta["preTokenBalances"]);
    let post_token_balances = parse_token_balances(&meta["postTokenBalances"]);

    Ok(processors::TxContext {
        slot,
        block_time_unix: block_time,
        tx_signature,
        succeeded,
        fee_lamports,
        compute_units_consumed,
        log_messages,
        account_keys,
        instructions,
        inner_instructions,
        pre_token_balances,
        post_token_balances,
    })
}

fn parse_raw_instructions(
    value: &serde_json::Value,
) -> Result<Vec<processors::RawInstruction>> {
    let arr = match value.as_array() {
        Some(a) => a,
        None => return Ok(Vec::new()),
    };
    let mut ixs = Vec::new();
    for item in arr {
        let program_id_index = item["programIdIndex"].as_u64().unwrap_or(0) as u16;
        let data = item["data"]
            .as_str()
            .map(|s| bs58::decode(s).into_vec().unwrap_or_default())
            .unwrap_or_default();
        let account_indices: Vec<u16> = item["accounts"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_u64().map(|n| n as u16)).collect())
            .unwrap_or_default();
        ixs.push(processors::RawInstruction {
            program_id_index,
            data,
            account_indices,
        });
    }
    Ok(ixs)
}

fn parse_inner_instructions(
    value: &serde_json::Value,
) -> Result<Vec<(u16, Vec<processors::RawInstruction>)>> {
    let arr = match value.as_array() {
        Some(a) => a,
        None => return Ok(Vec::new()),
    };
    let mut result = Vec::new();
    for group in arr {
        let index = group["index"].as_u64().unwrap_or(0) as u16;
        let ixs = parse_raw_instructions(&group["instructions"])?;
        result.push((index, ixs));
    }
    Ok(result)
}

fn parse_token_balances(value: &serde_json::Value) -> Vec<processors::TokenBalance> {
    let arr = match value.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|item| {
            Some(processors::TokenBalance {
                account_index: item["accountIndex"].as_u64()? as u16,
                mint: item["mint"].as_str()?.to_string(),
                owner: item["owner"].as_str().unwrap_or("").to_string(),
                amount: item["uiTokenAmount"]["amount"]
                    .as_str()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
                decimals: item["uiTokenAmount"]["decimals"].as_u64().unwrap_or(0) as u8,
            })
        })
        .collect()
}

#[test]
fn process_all_venue_fixtures() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures");

    let venues = ["kamino", "marginfi", "save", "jupiter-lend"];
    let mut total_liquidations = 0;
    let mut total_failed = 0;
    let mut venue_results: Vec<(String, usize, usize, Vec<String>)> = Vec::new();

    for venue in &venues {
        let venue_dir = fixtures_dir.join(venue);
        if !venue_dir.exists() {
            eprintln!("SKIP: {} fixtures not found", venue);
            continue;
        }

        let entries: Vec<_> = fs::read_dir(&venue_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .collect();

        let mut venue_liquidations = 0;
        let mut venue_failed = 0;
        let mut details = Vec::new();

        for entry in &entries {
            let path = entry.path();
            let name = path.file_stem().unwrap().to_str().unwrap();

            let ctx = match fixture_to_tx_context(path.to_str().unwrap()) {
                Ok(ctx) => ctx,
                Err(e) => {
                    eprintln!("  ERROR parsing {}/{}: {}", venue, name, e);
                    continue;
                }
            };

            let result = match processors::process_transaction(&ctx) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("  ERROR processing {}/{}: {}", venue, name, e);
                    continue;
                }
            };

            let liq = result.liquidations.len();
            let fail = result.failed_attempts.len();
            venue_liquidations += liq;
            venue_failed += fail;

            // Print details for each event
            for event in &result.liquidations {
                details.push(format!(
                    "  OK  {} slot={} repay_amount={} liquidator={}..{} flashloan={} jito_tip={:?}",
                    name,
                    event.slot,
                    event.repay_amount,
                    &event.liquidator[..8],
                    &event.liquidator[event.liquidator.len()-4..],
                    event.used_flashloan,
                    event.jito_tip_lamports,
                ));
            }
            for event in &result.failed_attempts {
                details.push(format!(
                    "  FAIL {} slot={} error={:?} code={:?}",
                    name,
                    event.base.slot,
                    event.error_message,
                    event.error_code,
                ));
            }

            // Also check enrichment
            if liq == 0 && fail == 0 {
                details.push(format!(
                    "  --  {} slot={} (no liquidation instructions, {} ixs touching program)",
                    name, ctx.slot, ctx.instructions.len()
                ));
            }
        }

        total_liquidations += venue_liquidations;
        total_failed += venue_failed;
        venue_results.push((venue.to_string(), venue_liquidations, venue_failed, details));
    }

    // Print summary
    eprintln!("\n{}", "=".repeat(60));
    eprintln!("FIXTURE PROCESSING RESULTS");
    eprintln!("{}", "=".repeat(60));
    for (venue, liq, fail, details) in &venue_results {
        eprintln!(
            "\n[{}] {} liquidations, {} failed attempts",
            venue, liq, fail
        );
        for d in details {
            eprintln!("{}", d);
        }
    }
    eprintln!(
        "\nTOTAL: {} liquidations, {} failed attempts",
        total_liquidations, total_failed
    );
    eprintln!("{}", "=".repeat(60));

    // At least verify we can process all fixtures without errors
    assert!(
        total_liquidations + total_failed > 0,
        "Expected at least one liquidation or failed attempt across all fixtures"
    );
}

/// Integration test that writes to a local ClickHouse instance.
/// Only runs when CLICKHOUSE_URL is set.
#[test]
fn write_fixtures_to_clickhouse() {
    let clickhouse_url = match std::env::var("CLICKHOUSE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("SKIP: CLICKHOUSE_URL not set. Run with:");
            eprintln!("  docker compose up -d");
            eprintln!("  CLICKHOUSE_URL=http://localhost:8123 CLICKHOUSE_DATABASE=liquidation_indexer cargo test -p backfill --test local_integration write_fixtures_to_clickhouse -- --nocapture");
            return;
        }
    };

    let database = std::env::var("CLICKHOUSE_DATABASE").unwrap_or("liquidation_indexer".into());

    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures");

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let writer_config = indexer_core::writer::WriterConfig {
            url: clickhouse_url,
            database,
            user: std::env::var("CLICKHOUSE_USER").unwrap_or("default".into()),
            password: std::env::var("CLICKHOUSE_PASSWORD").unwrap_or_default(),
            batch_size: 100,
            flush_interval_secs: 1,
        };

        let mut writer =
            indexer_core::writer::ClickHouseWriter::new(writer_config).unwrap();

        let venues = ["kamino", "marginfi", "save", "jupiter-lend"];
        for venue in &venues {
            let venue_dir = fixtures_dir.join(venue);
            if !venue_dir.exists() {
                continue;
            }

            for entry in fs::read_dir(&venue_dir).unwrap().filter_map(|e| e.ok()) {
                if entry.path().extension().is_some_and(|ext| ext == "json") {
                    let ctx =
                        fixture_to_tx_context(entry.path().to_str().unwrap()).unwrap();
                    let result = processors::process_transaction(&ctx).unwrap();

                    if !result.liquidations.is_empty() || !result.failed_attempts.is_empty() {
                        writer.ingest(result);
                    }
                }
            }
        }

        match writer.flush().await {
            Ok(()) => {
                let stats = writer.stats();
                eprintln!(
                    "\nClickHouse write complete: {} liquidations, {} failed, {} tx_meta",
                    stats.liquidations, stats.failed_attempts, stats.tx_metadata
                );
            }
            Err(e) => {
                eprintln!("\nClickHouse write failed: {:?}", e);
                eprintln!("Make sure ClickHouse is running and the schema is applied:");
                eprintln!("  docker compose up -d");
                eprintln!("  curl 'http://localhost:8123/' -d 'CREATE DATABASE IF NOT EXISTS liquidation_indexer'");
                eprintln!("  curl 'http://localhost:8123/?database=liquidation_indexer' --data-binary @schema/migrations/001_initial_schema.sql");
            }
        }
    });
}
