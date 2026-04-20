//! Round-trip test: decode real mainnet Jupiter Lend Vaults liquidation transactions.

use std::fs;
use std::path::Path;

use jupiter_lend_vaults_decoder::instructions;
use solana_sdk::pubkey::Pubkey;

fn load_vaults_instructions(path: &str) -> Vec<(Vec<u8>, Vec<Pubkey>)> {
    let content = fs::read_to_string(path).expect("failed to read fixture");
    let tx: serde_json::Value = serde_json::from_str(&content).expect("invalid JSON");

    let message = &tx["transaction"]["message"];
    // Merge static account keys with loaded addresses (v0 tx with ALTs)
    let mut account_keys: Vec<Pubkey> = message["accountKeys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap().parse().unwrap())
        .collect();
    if let Some(loaded) = tx.get("meta").and_then(|m| m.get("loadedAddresses")) {
        for key in loaded.get("writable").and_then(|a| a.as_array()).unwrap_or(&vec![]) {
            account_keys.push(key.as_str().unwrap().parse().unwrap());
        }
        for key in loaded.get("readonly").and_then(|a| a.as_array()).unwrap_or(&vec![]) {
            account_keys.push(key.as_str().unwrap().parse().unwrap());
        }
    }

    let vaults_program: Pubkey = "jupr81YtYssSyPt8jbnGuiWon5f6x9TcDEFxYe3Bdzi"
        .parse()
        .unwrap();

    let mut results = Vec::new();
    for ix in message["instructions"].as_array().unwrap() {
        let prog_idx = ix["programIdIndex"].as_u64().unwrap() as usize;
        if account_keys[prog_idx] != vaults_program {
            continue;
        }

        let data_b58 = ix["data"].as_str().unwrap();
        let data = bs58::decode(data_b58).into_vec().unwrap();

        let ix_accounts: Vec<Pubkey> = ix["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|idx| account_keys[idx.as_u64().unwrap() as usize])
            .collect();

        results.push((data, ix_accounts));
    }

    results
}

#[test]
fn decode_jupiter_lend_fixtures() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures/jupiter-lend");

    if !fixtures_dir.exists() {
        eprintln!("No Jupiter Lend fixtures found, skipping");
        return;
    }

    let entries: Vec<_> = fs::read_dir(&fixtures_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .collect();

    assert!(!entries.is_empty(), "No fixture files found");

    let mut total_ixs = 0;
    let mut decoded_ixs = 0;

    for entry in &entries {
        let path = entry.path();
        let fixture_name = path.file_stem().unwrap().to_str().unwrap();
        let ixs = load_vaults_instructions(path.to_str().unwrap());

        for (data, accounts) in &ixs {
            total_ixs += 1;
            match instructions::decode(data, accounts) {
                Ok(Some(ix)) => {
                    decoded_ixs += 1;
                    if let instructions::VaultsInstruction::Liquidate { args, accounts: accts } = &ix
                    {
                        eprintln!(
                            "  {} decoded liquidate (debt_amt={}, absorb={}, remaining={})",
                            fixture_name,
                            args.debt_amt,
                            args.absorb,
                            accts.remaining.len()
                        );
                        // Verify accessor methods
                        let _ = ix.liquidator();
                        let _ = ix.vault_config();
                        let _ = ix.oracle();
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    panic!("Failed to decode instruction in {}: {}", fixture_name, e);
                }
            }
        }
    }

    eprintln!(
        "\nJupiter Lend fixtures: {} total vaults ixs, {} decoded liquidations",
        total_ixs, decoded_ixs
    );
}
