//! Round-trip test: decode real mainnet Kamino liquidation transactions.

use std::fs;
use std::path::Path;

use klend_decoder::instructions;
use solana_sdk::pubkey::Pubkey;

/// Parse a transaction fixture JSON and extract klend instructions.
fn load_fixture(path: &str) -> Vec<(Vec<u8>, Vec<Pubkey>)> {
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

    let klend_program: Pubkey = "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD"
        .parse()
        .unwrap();

    let mut results = Vec::new();
    for ix in message["instructions"].as_array().unwrap() {
        let prog_idx = ix["programIdIndex"].as_u64().unwrap() as usize;
        if account_keys[prog_idx] != klend_program {
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
fn decode_kamino_fixtures() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures/kamino");

    if !fixtures_dir.exists() {
        eprintln!("No Kamino fixtures found, skipping");
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
    let mut liquidations = 0;

    for entry in &entries {
        let path = entry.path();
        let fixture_name = path.file_stem().unwrap().to_str().unwrap();
        let ixs = load_fixture(path.to_str().unwrap());

        for (data, accounts) in &ixs {
            total_ixs += 1;
            match instructions::decode(data, accounts) {
                Ok(Some(ix)) => {
                    decoded_ixs += 1;
                    if ix.is_liquidation() {
                        liquidations += 1;
                        // Verify accessor methods don't panic
                        let _ = ix.liquidator();
                        let _ = ix.obligation();
                        let _ = ix.lending_market();
                        let _ = ix.liquidity_amount();
                        eprintln!(
                            "  {} decoded {} (amount={})",
                            fixture_name,
                            ix.kind(),
                            ix.liquidity_amount()
                        );
                    } else {
                        eprintln!("  {} decoded {} (flash loan)", fixture_name, ix.kind());
                    }
                }
                Ok(None) => {
                    // Not a liquidation/flash loan instruction — expected for refreshes etc.
                }
                Err(e) => {
                    panic!("Failed to decode instruction in {}: {}", fixture_name, e);
                }
            }
        }
    }

    eprintln!(
        "\nKamino fixtures: {} total klend ixs, {} decoded (liquidation/flash), {} liquidations",
        total_ixs, decoded_ixs, liquidations
    );
}
