//! Common processing logic shared across all venues.

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use solana_sdk::pubkey::Pubkey;

use indexer_core::enrichment::{self, InstructionInfo, TxEnrichment};
use indexer_core::events::{ProcessedTransaction, TxMetadata};

use crate::{TxContext, RawInstruction};

/// Build the base ProcessedTransaction with tx_metadata and enrichment.
pub fn build_base_result(ctx: &TxContext) -> Result<ProcessedTransaction> {
    let block_time = Utc
        .timestamp_opt(ctx.block_time_unix, 0)
        .single()
        .unwrap_or_else(Utc::now);

    // Collect all instructions for enrichment
    let top_level = ctx.instructions.iter()
        .map(|ix| to_instruction_info(ix, &ctx.account_keys))
        .collect::<Vec<_>>();

    let mut inner = Vec::new();
    for (_, ixs) in &ctx.inner_instructions {
        for ix in ixs {
            inner.push(to_instruction_info(ix, &ctx.account_keys));
        }
    }

    let enrichment = enrichment::enrich_transaction(&top_level, &inner, ctx.fee_lamports);

    // Build signers list (first N accounts that are signers in the message)
    // For simplicity, fee_payer is always account_keys[0]
    let fee_payer = if ctx.account_keys.is_empty() {
        Pubkey::default().to_string()
    } else {
        ctx.account_keys[0].to_string()
    };

    let num_inner: u16 = ctx.inner_instructions.iter()
        .map(|(_, ixs)| ixs.len() as u16)
        .sum();

    let tx_meta = TxMetadata {
        tx_signature: ctx.tx_signature.clone(),
        slot: ctx.slot,
        block_time,
        succeeded: ctx.succeeded,
        fee_lamports: ctx.fee_lamports,
        priority_fee_lamports: enrichment.priority_fee_lamports,
        jito_tip_lamports: enrichment.jito_tip_lamports,
        compute_units_consumed: ctx.compute_units_consumed,
        compute_units_requested: enrichment.compute_units_requested,
        num_instructions: ctx.instructions.len() as u16,
        num_inner_instructions: num_inner,
        signers: vec![fee_payer.clone()], // Simplified; full signer list needs message header
        fee_payer,
        uses_address_lookup_table: ctx.account_keys.len() > 64, // Heuristic: ALTs used if >64 accounts
    };

    Ok(ProcessedTransaction {
        tx_meta,
        liquidations: Vec::new(),
        failed_attempts: Vec::new(),
        obligation_snapshots: Vec::new(),
        reserve_snapshots: Vec::new(),
    })
}

/// Convert a RawInstruction to an InstructionInfo for enrichment.
pub fn to_instruction_info(ix: &RawInstruction, account_keys: &[Pubkey]) -> InstructionInfo {
    InstructionInfo {
        program_id: account_keys
            .get(ix.program_id_index as usize)
            .copied()
            .unwrap_or_default(),
        data: ix.data.clone(),
        accounts: ix.account_indices.iter()
            .filter_map(|&idx| account_keys.get(idx as usize).copied())
            .collect(),
    }
}

/// Get the enrichment from a TxContext.
pub fn get_enrichment(ctx: &TxContext) -> TxEnrichment {
    let top_level: Vec<InstructionInfo> = ctx.instructions.iter()
        .map(|ix| to_instruction_info(ix, &ctx.account_keys))
        .collect();

    let mut inner = Vec::new();
    for (_, ixs) in &ctx.inner_instructions {
        for ix in ixs {
            inner.push(to_instruction_info(ix, &ctx.account_keys));
        }
    }

    enrichment::enrich_transaction(&top_level, &inner, ctx.fee_lamports)
}

/// Convert a block timestamp to DateTime<Utc>.
pub fn block_time_to_dt(unix_ts: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(unix_ts, 0).single().unwrap_or_else(Utc::now)
}

/// Resolve an account pubkey from the account_keys array.
pub fn resolve_account(account_keys: &[Pubkey], index: u16) -> Pubkey {
    account_keys.get(index as usize).copied().unwrap_or_default()
}

/// A resolved instruction ready for venue-specific decoding.
/// Includes both top-level and inner instructions.
#[derive(Debug, Clone)]
pub struct ResolvedInstruction {
    pub program_id: Pubkey,
    pub data: Vec<u8>,
    pub accounts: Vec<Pubkey>,
    /// Index of the top-level instruction (for ix_index in the event).
    pub top_level_index: u16,
    /// If this is an inner instruction, its index within the group.
    pub inner_index: Option<u16>,
}

/// Collect all instructions (top-level + inner) that invoke a specific program.
/// This is critical because real-world liquidators often invoke lending programs
/// via CPI through wrapper/router programs.
pub fn collect_program_instructions(
    ctx: &crate::TxContext,
    program_id: &Pubkey,
) -> Vec<ResolvedInstruction> {
    let mut result = Vec::new();

    // Top-level instructions
    for (ix_idx, ix) in ctx.instructions.iter().enumerate() {
        let pid = ctx.account_keys
            .get(ix.program_id_index as usize)
            .copied()
            .unwrap_or_default();
        if pid == *program_id {
            result.push(ResolvedInstruction {
                program_id: pid,
                data: ix.data.clone(),
                accounts: ix.account_indices.iter()
                    .filter_map(|&idx| ctx.account_keys.get(idx as usize).copied())
                    .collect(),
                top_level_index: ix_idx as u16,
                inner_index: None,
            });
        }
    }

    // Inner instructions (CPI)
    for (parent_idx, inner_ixs) in &ctx.inner_instructions {
        for (inner_idx, ix) in inner_ixs.iter().enumerate() {
            let pid = ctx.account_keys
                .get(ix.program_id_index as usize)
                .copied()
                .unwrap_or_default();
            if pid == *program_id {
                result.push(ResolvedInstruction {
                    program_id: pid,
                    data: ix.data.clone(),
                    accounts: ix.account_indices.iter()
                        .filter_map(|&idx| ctx.account_keys.get(idx as usize).copied())
                        .collect(),
                    top_level_index: *parent_idx,
                    inner_index: Some(inner_idx as u16),
                });
            }
        }
    }

    result
}
