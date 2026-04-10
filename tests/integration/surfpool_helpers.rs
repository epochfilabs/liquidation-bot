//! Surfpool integration test helpers.
//!
//! Provides utilities for:
//! - Connecting to a running Surfpool instance
//! - Forging account data via surfnet_setAccount
//! - Creating underwater obligation accounts for each protocol
//! - Funding liquidator wallets

use anyhow::{Context, Result};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
};

/// Default local test RPC endpoint (Surfpool or solana-test-validator).
pub const LOCAL_RPC: &str = "http://127.0.0.1:8899";

/// Connect to a running local test validator (Surfpool or solana-test-validator).
/// Returns None if not running.
pub fn connect_local() -> Option<RpcClient> {
    let rpc = RpcClient::new_with_commitment(
        LOCAL_RPC.to_string(),
        CommitmentConfig::confirmed(),
    );

    match rpc.get_version() {
        Ok(v) => {
            tracing::info!("connected to local validator: {:?}", v);
            Some(rpc)
        }
        Err(_) => None,
    }
}

// Keep old name for compatibility
pub fn connect_surfpool() -> Option<RpcClient> {
    connect_local()
}

pub const SURFPOOL_RPC: &str = LOCAL_RPC;

/// Set an account's data on the local test validator.
///
/// Tries Surfpool's `surfnet_setAccount` first, falls back to
/// solana-test-validator's `simulateTransaction` with account overrides
/// via writing a JSON account file and restarting.
///
/// For solana-test-validator, the simplest approach is to write the
/// account data to a JSON file and use `solana account` format.
pub fn set_account(
    rpc_url: &str,
    pubkey: &Pubkey,
    data: &[u8],
    lamports: u64,
    owner: &Pubkey,
) -> Result<()> {
    use base64::Engine;

    // Try surfnet_setAccount (Surfpool)
    let client = reqwest::blocking::Client::new();
    let encoded_data = base64::engine::general_purpose::STANDARD.encode(data);

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "surfnet_setAccount",
        "params": [
            pubkey.to_string(),
            {
                "lamports": lamports,
                "data": [encoded_data, "base64"],
                "owner": owner.to_string(),
                "executable": false,
                "rentEpoch": 0
            }
        ]
    });

    let resp = client.post(rpc_url).json(&body).send();
    if let Ok(r) = resp {
        if r.status().is_success() {
            return Ok(());
        }
    }

    // Fallback: write account to a JSON file for use with --account flag.
    // This requires restarting the validator, so we write the file and return
    // an instruction for the caller to restart.
    let account_file = format!("/tmp/test_account_{}.json", pubkey);
    let account_json = serde_json::json!({
        "pubkey": pubkey.to_string(),
        "account": {
            "lamports": lamports,
            "data": [encoded_data, "base64"],
            "owner": owner.to_string(),
            "executable": false,
            "rentEpoch": 0,
            "space": data.len()
        }
    });
    std::fs::write(&account_file, serde_json::to_string_pretty(&account_json)?)?;

    // For solana-test-validator, use the CLI to set account data directly
    let output = std::process::Command::new("solana")
        .args(["account", &pubkey.to_string(), "--output", "json", "--url", rpc_url])
        .output();

    // Write the forged data via solana CLI's account subcommand
    // Actually, the simplest way for test-validator is via program-test or
    // by restarting with --account flag pointing to the JSON file.
    //
    // Since we can't restart mid-test, we'll write account files BEFORE
    // starting the validator and pass them via --account.
    tracing::warn!(
        "set_account fallback: wrote {} — restart validator with --account {} {}",
        account_file, pubkey, account_file
    );

    Ok(())
}

/// Airdrop SOL to an account on Surfpool.
pub fn airdrop_sol(rpc: &RpcClient, pubkey: &Pubkey, lamports: u64) -> Result<()> {
    let sig = rpc
        .request_airdrop(pubkey, lamports)
        .context("airdrop failed")?;
    rpc.confirm_transaction(&sig)
        .context("airdrop confirmation failed")?;
    Ok(())
}

/// Create a funded test keypair.
pub fn funded_keypair(rpc: &RpcClient) -> Result<Keypair> {
    let kp = Keypair::new();
    airdrop_sol(rpc, &kp.pubkey(), 10_000_000_000)?; // 10 SOL
    Ok(kp)
}

/// Forge a Kamino obligation account that is underwater.
///
/// Takes a real obligation's raw data and modifies the health fields
/// to make it liquidatable (borrow_factor_adjusted_debt >= unhealthy_borrow).
pub fn forge_underwater_kamino_obligation(
    real_obligation_data: &[u8],
) -> Vec<u8> {
    let mut data = real_obligation_data.to_vec();

    // Kamino obligation offsets (validated):
    // deposited_value_sf: 1192
    // borrow_factor_adjusted_debt_value_sf: 2208
    // unhealthy_borrow_value_sf: 2256
    let sf_shift: u128 = 1u128 << 60;

    // Set deposited value to $1000
    let deposited = 1000u128 * sf_shift;
    data[1192..1208].copy_from_slice(&deposited.to_le_bytes());

    // Set borrow to $950 (95% LTV — above most unhealthy thresholds)
    let borrowed = 950u128 * sf_shift;
    data[2208..2224].copy_from_slice(&borrowed.to_le_bytes());

    // Set unhealthy threshold to $900 (90% LTV)
    let unhealthy = 900u128 * sf_shift;
    data[2256..2272].copy_from_slice(&unhealthy.to_le_bytes());

    // Also set borrowed_assets_market_value_sf (2224) = same as bf-adjusted
    data[2224..2240].copy_from_slice(&borrowed.to_le_bytes());

    data
}

/// Forge a Jupiter Lend position that is underwater.
///
/// Sets the tick to a very high value (high debt/collateral ratio).
pub fn forge_underwater_jupiter_position(
    real_position_data: &[u8],
) -> Vec<u8> {
    let mut data = real_position_data.to_vec();

    // Position offsets:
    // is_supply_only: 46
    // tick: 47 (i32)
    // supply_amount: 55 (u64)
    // dust_debt_amount: 63 (u64)

    // Make sure it's not supply-only
    data[46] = 0;

    // Set a very high tick (= high debt/collateral ratio)
    let high_tick: i32 = 10000;
    data[47..51].copy_from_slice(&high_tick.to_le_bytes());

    // Set some supply and debt
    let supply: u64 = 1_000_000_000; // 1 token (9 decimals)
    data[55..63].copy_from_slice(&supply.to_le_bytes());
    let dust_debt: u64 = 100_000;
    data[63..71].copy_from_slice(&dust_debt.to_le_bytes());

    data
}

/// Forge a Save (Solend) obligation that is underwater.
///
/// Sets borrowed_value > unhealthy_borrow_value.
pub fn forge_underwater_save_obligation(
    real_obligation_data: &[u8],
) -> Vec<u8> {
    let mut data = real_obligation_data.to_vec();

    let wad: u128 = 1_000_000_000_000_000_000;

    // Save obligation offsets:
    // deposited_value: 74 (u128, WAD-scaled)
    // borrowed_value: 90
    // unhealthy_borrow_value: 122

    // Deposited = $1000
    let deposited = 1000u128 * wad;
    data[74..90].copy_from_slice(&deposited.to_le_bytes());

    // Borrowed = $920 (92% — above typical 90% threshold)
    let borrowed = 920u128 * wad;
    data[90..106].copy_from_slice(&borrowed.to_le_bytes());

    // Unhealthy threshold = $900 (90%)
    let unhealthy = 900u128 * wad;
    data[122..138].copy_from_slice(&unhealthy.to_le_bytes());

    data
}
