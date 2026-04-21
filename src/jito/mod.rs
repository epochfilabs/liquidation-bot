//! Jito bundle submission.
//!
//! Sends transactions as atomic Jito bundles for MEV protection.
//! Uses the Jito Block Engine JSON-RPC API directly (no extra crate needed).
//!
//! A bundle is up to 5 transactions that execute atomically and in order.
//! For liquidation, we typically send a single transaction containing:
//!   flash_borrow → liquidate → [swap] → flash_repay + tip
//!
//! The tip is a SystemProgram::Transfer to a random Jito tip account,
//! added as the last instruction in the transaction.

use anyhow::{Context, Result};
use solana_sdk::{
    instruction::Instruction,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    system_instruction,
    transaction::Transaction,
};
use std::str::FromStr;
use std::sync::LazyLock;

/// Jito Block Engine endpoints.
pub const JITO_MAINNET_ENDPOINT: &str = "https://mainnet.block-engine.jito.wtf/api/v1/bundles";
pub const JITO_FRANKFURT_ENDPOINT: &str = "https://frankfurt.mainnet.block-engine.jito.wtf/api/v1/bundles";
pub const JITO_NY_ENDPOINT: &str = "https://ny.mainnet.block-engine.jito.wtf/api/v1/bundles";
pub const JITO_TOKYO_ENDPOINT: &str = "https://tokyo.mainnet.block-engine.jito.wtf/api/v1/bundles";
pub const JITO_AMSTERDAM_ENDPOINT: &str = "https://amsterdam.mainnet.block-engine.jito.wtf/api/v1/bundles";

/// Minimum Jito tip: 1,000 lamports.
pub const MIN_TIP_LAMPORTS: u64 = 1_000;

/// The 8 Jito tip accounts on mainnet.
static TIP_ACCOUNTS: LazyLock<Vec<Pubkey>> = LazyLock::new(|| {
    vec![
        Pubkey::from_str("96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5").unwrap(),
        Pubkey::from_str("HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe").unwrap(),
        Pubkey::from_str("Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY").unwrap(),
        Pubkey::from_str("ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49").unwrap(),
        Pubkey::from_str("DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh").unwrap(),
        Pubkey::from_str("ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt").unwrap(),
        Pubkey::from_str("DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL").unwrap(),
        Pubkey::from_str("3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT").unwrap(),
    ]
});

/// Jito bundle submission configuration.
#[derive(Debug, Clone)]
pub struct JitoConfig {
    /// Block engine endpoint URL.
    pub endpoint: String,
    /// Whether to use Jito bundles (false = use standard sendTransaction).
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

impl JitoConfig {
    pub fn from_env() -> Self {
        let endpoint = std::env::var("JITO_ENDPOINT")
            .unwrap_or_else(|_| JITO_MAINNET_ENDPOINT.to_string());
        let enabled = std::env::var("JITO_ENABLED")
            .unwrap_or_else(|_| "true".to_string())
            .parse()
            .unwrap_or(true);
        Self { endpoint, enabled }
    }
}

/// Get a random Jito tip account.
pub fn random_tip_account() -> Pubkey {
    use std::time::SystemTime;
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as usize;
    TIP_ACCOUNTS[nonce % TIP_ACCOUNTS.len()]
}

/// Create a tip instruction (SystemProgram::Transfer to a Jito tip account).
pub fn tip_instruction(payer: &Pubkey, lamports: u64) -> Instruction {
    let tip_account = random_tip_account();
    system_instruction::transfer(payer, &tip_account, lamports.max(MIN_TIP_LAMPORTS))
}

/// Build a signed transaction with a Jito tip appended.
///
/// Takes the liquidation instructions and appends a tip instruction as the last ix.
pub fn build_tipped_transaction(
    instructions: Vec<Instruction>,
    tip_lamports: u64,
    payer: &Keypair,
    recent_blockhash: solana_sdk::hash::Hash,
) -> Transaction {
    let mut all_ixs = instructions;
    all_ixs.push(tip_instruction(&payer.pubkey(), tip_lamports));

    Transaction::new_signed_with_payer(
        &all_ixs,
        Some(&payer.pubkey()),
        &[payer],
        recent_blockhash,
    )
}

/// Send a bundle to the Jito Block Engine.
///
/// A bundle contains 1-5 base64-encoded signed transactions.
/// Returns the bundle ID on success.
pub async fn send_bundle(
    config: &JitoConfig,
    transactions: &[Transaction],
) -> Result<String> {
    let encoded_txs: Vec<String> = transactions
        .iter()
        .map(|tx| {
            let bytes = bincode::serialize(tx).unwrap();
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes)
        })
        .collect();

    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "sendBundle",
        "params": [encoded_txs]
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
        anyhow::bail!("Jito sendBundle failed ({}): {}", status, body);
    }

    let result: serde_json::Value = response.json().await
        .context("failed to parse Jito response")?;

    if let Some(error) = result.get("error") {
        anyhow::bail!("Jito bundle error: {}", error);
    }

    let bundle_id = result["result"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    tracing::info!(bundle_id = %bundle_id, txs = transactions.len(), "bundle submitted to Jito");
    Ok(bundle_id)
}

/// Send a single liquidation transaction as a Jito bundle.
///
/// This is the primary entry point for the executor. It:
/// 1. Appends a tip instruction to the transaction
/// 2. Signs it
/// 3. Sends as a single-transaction bundle
///
/// If Jito is disabled, falls back to standard RPC sendTransaction.
pub async fn submit_liquidation(
    jito_config: &JitoConfig,
    rpc: &solana_client::rpc_client::RpcClient,
    instructions: Vec<Instruction>,
    tip_lamports: u64,
    payer: &Keypair,
) -> Result<String> {
    let recent_blockhash = rpc.get_latest_blockhash()
        .context("failed to get blockhash")?;

    if jito_config.enabled {
        // Jito bundle path
        let tx = build_tipped_transaction(instructions, tip_lamports, payer, recent_blockhash);
        let bundle_id = send_bundle(jito_config, &[tx]).await?;
        Ok(bundle_id)
    } else {
        // Standard RPC fallback (no tip, no atomicity guarantee)
        tracing::warn!("Jito disabled — submitting via standard RPC (no MEV protection)");
        let tx = Transaction::new_signed_with_payer(
            &instructions,
            Some(&payer.pubkey()),
            &[payer],
            recent_blockhash,
        );
        let sig = rpc.send_and_confirm_transaction(&tx)
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
        let ix = tip_instruction(&payer, 0); // try 0 tip
        // Should enforce minimum of 1000 lamports
        let amount = u64::from_le_bytes(ix.data[4..12].try_into().unwrap());
        assert_eq!(amount, MIN_TIP_LAMPORTS);
    }

    #[test]
    fn jito_config_from_env() {
        let config = JitoConfig::default();
        assert!(config.enabled);
        assert!(config.endpoint.contains("jito.wtf"));
    }
}
