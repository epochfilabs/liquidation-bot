//! Jito bundle submission.
//!
//! Sends transactions as atomic Jito bundles for MEV protection via the Jito
//! Block Engine JSON-RPC API (no extra crate needed). A bundle is up to 5
//! transactions that execute atomically and in order; for liquidation we
//! typically send a single transaction containing
//! `flash_borrow → liquidate → [swap] → flash_repay` with a tip instruction
//! appended as the last account transfer.

use anyhow::{Context, Result};
use base64::Engine;
use serde::Deserialize;
use solana_sdk::{
    hash::Hash,
    instruction::Instruction,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};

// `system_instruction` was renamed to `solana_system_interface::instruction` in
// newer solana-sdk versions — use the same-named function through the still-
// exported module for now. When the workspace upgrades solana-sdk, swap this.
#[allow(deprecated)]
use solana_sdk::system_instruction;

/// Jito Block Engine endpoints.
pub const JITO_MAINNET_ENDPOINT: &str = "https://mainnet.block-engine.jito.wtf/api/v1/bundles";
pub const JITO_FRANKFURT_ENDPOINT: &str =
    "https://frankfurt.mainnet.block-engine.jito.wtf/api/v1/bundles";
pub const JITO_NY_ENDPOINT: &str = "https://ny.mainnet.block-engine.jito.wtf/api/v1/bundles";
pub const JITO_TOKYO_ENDPOINT: &str = "https://tokyo.mainnet.block-engine.jito.wtf/api/v1/bundles";
pub const JITO_AMSTERDAM_ENDPOINT: &str =
    "https://amsterdam.mainnet.block-engine.jito.wtf/api/v1/bundles";

/// Minimum Jito tip: 1,000 lamports.
pub const MIN_TIP_LAMPORTS: u64 = 1_000;

/// The 8 mainnet Jito tip accounts (public, published by Jito).
const TIP_ACCOUNTS: [Pubkey; 8] = [
    solana_sdk::pubkey!("96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5"),
    solana_sdk::pubkey!("HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe"),
    solana_sdk::pubkey!("Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY"),
    solana_sdk::pubkey!("ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49"),
    solana_sdk::pubkey!("DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh"),
    solana_sdk::pubkey!("ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt"),
    solana_sdk::pubkey!("DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL"),
    solana_sdk::pubkey!("3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT"),
];

/// Jito bundle submission configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct JitoConfig {
    /// Block-engine endpoint URL.
    pub endpoint: String,
    /// Whether to use Jito bundles. When `false`, fall back to standard `sendTransaction`.
    pub enabled: bool,
}

impl Default for JitoConfig {
    fn default() -> Self {
        Self {
            endpoint: JITO_MAINNET_ENDPOINT.to_string(),
            enabled: true,
        }
    }
}

/// Return a pseudo-random Jito tip account. The nanosecond-based nonce is not
/// cryptographic; it's sufficient to spread traffic across the 8 tip accounts.
pub fn random_tip_account() -> Pubkey {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos() as usize);
    TIP_ACCOUNTS[nonce % TIP_ACCOUNTS.len()]
}

/// Create a tip instruction (`SystemProgram::Transfer` to a Jito tip account).
pub fn tip_instruction(payer: &Pubkey, lamports: u64) -> Instruction {
    let tip_account = random_tip_account();
    system_instruction::transfer(payer, &tip_account, lamports.max(MIN_TIP_LAMPORTS))
}

/// Build a signed transaction with a Jito tip appended.
pub fn build_tipped_transaction(
    instructions: Vec<Instruction>,
    tip_lamports: u64,
    payer: &Keypair,
    recent_blockhash: Hash,
) -> Transaction {
    let mut all_ixs = instructions;
    all_ixs.push(tip_instruction(&payer.pubkey(), tip_lamports));

    Transaction::new_signed_with_payer(&all_ixs, Some(&payer.pubkey()), &[payer], recent_blockhash)
}

/// Send a bundle (1-5 base64-encoded signed transactions) to the Jito Block Engine.
pub async fn send_bundle(config: &JitoConfig, transactions: &[Transaction]) -> Result<String> {
    let encoded_txs: Vec<String> = transactions
        .iter()
        .map(|tx| {
            bincode::serialize(tx)
                .context("failed to serialize transaction for Jito bundle")
                .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
        })
        .collect::<Result<_>>()?;

    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "sendBundle",
        "params": [encoded_txs],
    });

    let client = reqwest::Client::new();
    let response = client
        .post(&config.endpoint)
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .context("failed to send bundle to Jito")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Jito sendBundle failed ({status}): {body}");
    }

    let result: serde_json::Value =
        response.json().await.context("failed to parse Jito response")?;

    if let Some(error) = result.get("error") {
        anyhow::bail!("Jito bundle error: {error}");
    }

    let bundle_id = result["result"].as_str().unwrap_or("unknown").to_string();
    tracing::info!(bundle_id = %bundle_id, txs = transactions.len(), "bundle submitted to Jito");
    Ok(bundle_id)
}

/// Send a single liquidation transaction as a Jito bundle (or fall back to
/// standard RPC when Jito is disabled).
pub async fn submit_liquidation(
    jito_config: &JitoConfig,
    rpc: &solana_client::rpc_client::RpcClient,
    instructions: Vec<Instruction>,
    tip_lamports: u64,
    payer: &Keypair,
) -> Result<String> {
    let recent_blockhash = rpc.get_latest_blockhash().context("failed to get blockhash")?;

    if jito_config.enabled {
        let tx = build_tipped_transaction(instructions, tip_lamports, payer, recent_blockhash);
        let bundle_id = send_bundle(jito_config, &[tx]).await?;
        Ok(bundle_id)
    } else {
        tracing::warn!("Jito disabled — submitting via standard RPC (no MEV protection)");
        let tx = Transaction::new_signed_with_payer(
            &instructions,
            Some(&payer.pubkey()),
            &[payer],
            recent_blockhash,
        );
        let sig = rpc
            .send_and_confirm_transaction(&tx)
            .context("standard RPC submission failed")?;
        Ok(sig.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tip_accounts_valid() {
        assert_eq!(TIP_ACCOUNTS.len(), 8);
        for account in TIP_ACCOUNTS.iter() {
            assert_ne!(*account, Pubkey::default());
        }
    }

    #[test]
    fn random_tip_is_from_set() {
        let tip = random_tip_account();
        assert!(TIP_ACCOUNTS.contains(&tip));
    }

    #[test]
    fn tip_instruction_valid() {
        let payer = Pubkey::new_unique();
        let ix = tip_instruction(&payer, 5000);
        assert_eq!(ix.program_id, solana_sdk::system_program::ID);
        assert_eq!(ix.accounts.len(), 2);
        assert_eq!(ix.accounts[0].pubkey, payer);
        assert!(TIP_ACCOUNTS.contains(&ix.accounts[1].pubkey));
    }

    #[test]
    fn min_tip_enforced() {
        let payer = Pubkey::new_unique();
        let ix = tip_instruction(&payer, 0);
        let amount = u64::from_le_bytes(ix.data[4..12].try_into().unwrap());
        assert_eq!(amount, MIN_TIP_LAMPORTS);
    }

    #[test]
    fn jito_config_default() {
        let config = JitoConfig::default();
        assert!(config.enabled);
        assert!(config.endpoint.contains("jito.wtf"));
    }
}
