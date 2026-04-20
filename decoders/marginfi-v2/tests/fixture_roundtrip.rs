//! Round-trip test: decode real mainnet MarginFi v2 liquidation transactions.

use std::fs;
use std::path::Path;

use marginfi_v2_decoder::instructions;
use solana_sdk::pubkey::Pubkey;

fn load_marginfi_instructions(path: &str) -> Vec<(Vec<u8>, Vec<Pubkey>)> {
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

    let marginfi_program: Pubkey = "MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA"
        .parse()
        .unwrap();

    let mut results = Vec::new();
    for ix in message["instructions"].as_array().unwrap() {
        let prog_idx = ix["programIdIndex"].as_u64().unwrap() as usize;
        if account_keys[prog_idx] != marginfi_program {
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
fn decode_marginfi_fixtures() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures/marginfi");

    if !fixtures_dir.exists() {
        eprintln!("No MarginFi fixtures found, skipping");
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
        let ixs = load_marginfi_instructions(path.to_str().unwrap());

        for (data, accounts) in &ixs {
            total_ixs += 1;
            match instructions::decode(data, accounts) {
                Ok(Some(ix)) => {
                    decoded_ixs += 1;
                    if ix.is_liquidation() {
                        liquidations += 1;
                        let _ = ix.liquidator().expect("liquidator should be present");
                        let _ = ix.liquidatee_account().expect("liquidatee should be present");
                        let _ = ix.group().expect("group should be present");
                        eprintln!(
                            "  {} decoded {} (asset_amount={})",
                            fixture_name,
                            ix.kind(),
                            match &ix {
                                instructions::MarginfiInstruction::Liquidate { args, .. } =>
                                    args.asset_amount,
                                _ => 0,
                            }
                        );
                    } else {
                        eprintln!("  {} decoded {}", fixture_name, ix.kind());
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
        "\nMarginFi fixtures: {} total MFI ixs, {} decoded, {} liquidations",
        total_ixs, decoded_ixs, liquidations
    );
}
