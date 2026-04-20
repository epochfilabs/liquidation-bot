//! Per-venue processors.
//!
//! Each processor takes a decoded instruction + transaction context and
//! produces canonical events for the ClickHouse tables.

pub mod common;
pub mod kamino;
pub mod jupiter_lend;
pub mod marginfi;
pub mod save;

use anyhow::Result;
use indexer_core::events::ProcessedTransaction;

/// Context for processing a single transaction.
#[derive(Debug, Clone)]
pub struct TxContext {
    pub slot: u64,
    pub block_time_unix: i64,
    pub tx_signature: String,
    pub succeeded: bool,
    pub fee_lamports: u64,
    pub compute_units_consumed: u32,
    pub log_messages: Vec<String>,
    /// All account keys (static + loaded from ALTs).
    pub account_keys: Vec<solana_sdk::pubkey::Pubkey>,
    /// Top-level instructions: (program_id_index, data, account_indices).
    pub instructions: Vec<RawInstruction>,
    /// Inner instructions grouped by top-level instruction index.
    pub inner_instructions: Vec<(u16, Vec<RawInstruction>)>,
    /// Pre/post token balances for amount change detection.
    pub pre_token_balances: Vec<TokenBalance>,
    pub post_token_balances: Vec<TokenBalance>,
}

/// A raw instruction before decoding.
#[derive(Debug, Clone)]
pub struct RawInstruction {
    pub program_id_index: u16,
    pub data: Vec<u8>,
    pub account_indices: Vec<u16>,
}

/// Token balance entry from transaction metadata.
#[derive(Debug, Clone)]
pub struct TokenBalance {
    pub account_index: u16,
    pub mint: String,
    pub owner: String,
    pub amount: u64,
    pub decimals: u8,
}

/// Process a transaction across all venue processors.
/// Returns a ProcessedTransaction with all events produced.
pub fn process_transaction(ctx: &TxContext) -> Result<ProcessedTransaction> {
    let mut result = common::build_base_result(ctx)?;

    // Try each venue's processor. Each one scans for its own program's instructions.
    kamino::process(ctx, &mut result)?;
    jupiter_lend::process(ctx, &mut result)?;
    marginfi::process(ctx, &mut result)?;
    save::process(ctx, &mut result)?;

    Ok(result)
}
