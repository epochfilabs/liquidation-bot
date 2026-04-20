//! Transaction parser: converts RPC transaction responses into the TxContext
//! format expected by the processors.

use anyhow::Result;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

use processors::{RawInstruction, TokenBalance, TxContext};

/// Parse a transaction from a JSON `getBlock` response entry.
///
/// The `tx_json` is a single transaction object from the block's `transactions` array.
/// Format: { "transaction": { "message": {...} }, "meta": {...} }
pub fn parse_transaction(
    tx_json: &serde_json::Value,
    slot: u64,
    block_time: i64,
) -> Result<Option<TxContext>> {
    let tx = &tx_json["transaction"];
    let meta = &tx_json["meta"];

    // Skip if meta is null (shouldn't happen with maxSupportedTransactionVersion)
    if meta.is_null() {
        return Ok(None);
    }

    // Check success/failure
    let err = &meta["err"];
    let succeeded = err.is_null();

    // Build account keys: static + loaded (ALTs)
    let message = &tx["message"];
    let mut account_keys = parse_pubkey_array(&message["accountKeys"])?;

    // Merge loaded addresses from ALTs (v0 transactions)
    if let Some(loaded) = meta.get("loadedAddresses") {
        if let Some(writable) = loaded.get("writable").and_then(|v| v.as_array()) {
            for key in writable {
                if let Some(s) = key.as_str() {
                    account_keys.push(Pubkey::from_str(s)?);
                }
            }
        }
        if let Some(readonly) = loaded.get("readonly").and_then(|v| v.as_array()) {
            for key in readonly {
                if let Some(s) = key.as_str() {
                    account_keys.push(Pubkey::from_str(s)?);
                }
            }
        }
    }

    // Quick filter: does this transaction touch any of our program IDs?
    let dominated_programs = [
        *klend_decoder::PROGRAM_ID,
        *jupiter_lend_vaults_decoder::PROGRAM_ID,
        *marginfi_v2_decoder::PROGRAM_ID,
        *save_decoder::PROGRAM_ID,
    ];

    let touches_our_program = account_keys.iter().any(|k| dominated_programs.contains(k));
    if !touches_our_program {
        return Ok(None);
    }

    // Parse top-level instructions
    let instructions = parse_instructions(&message["instructions"])?;

    // Parse inner instructions
    let inner_instructions = parse_inner_instructions(&meta["innerInstructions"])?;

    // Parse tx signature
    let tx_signature = tx["signatures"]
        .as_array()
        .and_then(|sigs| sigs.first())
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();

    // Parse fee and compute units
    let fee_lamports = meta["fee"].as_u64().unwrap_or(0);
    let compute_units_consumed = meta["computeUnitsConsumed"].as_u64().unwrap_or(0) as u32;

    // Parse log messages
    let log_messages = meta["logMessages"]
        .as_array()
        .map(|logs| {
            logs.iter()
                .filter_map(|l| l.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // Parse token balances
    let pre_token_balances = parse_token_balances(&meta["preTokenBalances"]);
    let post_token_balances = parse_token_balances(&meta["postTokenBalances"]);

    Ok(Some(TxContext {
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
    }))
}

fn parse_pubkey_array(value: &serde_json::Value) -> Result<Vec<Pubkey>> {
    let arr = match value.as_array() { Some(a) => a, None => return Ok(Vec::new()) };
    let mut keys = Vec::with_capacity(arr.len());
    for item in arr {
        if let Some(s) = item.as_str() {
            keys.push(Pubkey::from_str(s)?);
        }
    }
    Ok(keys)
}

fn parse_instructions(value: &serde_json::Value) -> Result<Vec<RawInstruction>> {
    let arr = match value.as_array() { Some(a) => a, None => return Ok(Vec::new()) };
    let mut ixs = Vec::with_capacity(arr.len());
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

        ixs.push(RawInstruction {
            program_id_index,
            data,
            account_indices,
        });
    }
    Ok(ixs)
}

fn parse_inner_instructions(
    value: &serde_json::Value,
) -> Result<Vec<(u16, Vec<RawInstruction>)>> {
    let arr = match value.as_array() { Some(a) => a, None => return Ok(Vec::new()) };
    let mut result = Vec::with_capacity(arr.len());
    for group in arr {
        let index = group["index"].as_u64().unwrap_or(0) as u16;
        let ixs = parse_instructions(&group["instructions"])?;
        result.push((index, ixs));
    }
    Ok(result)
}

fn parse_token_balances(value: &serde_json::Value) -> Vec<TokenBalance> {
    let arr = match value.as_array() { Some(a) => a, None => return Vec::new() };
    arr.iter()
        .filter_map(|item| {
            Some(TokenBalance {
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
