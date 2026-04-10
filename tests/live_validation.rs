//! Live validation test: fetches real obligation and reserve accounts from
//! mainnet and validates that our byte-offset deserialization produces
//! sensible values.
//!
//! Requires: SOLANA_RPC_URL env var (or .env file)
//!
//! Run: cargo test --test live_validation -- --nocapture

use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
};
use std::str::FromStr;

/// The actual mainnet klend program ID (production).
const KLEND_PROGRAM: &str = "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD";
const KAMINO_MAIN_MARKET: &str = "7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF";

/// Reserve account size: 8-byte discriminator + 8616 struct = 8624 bytes.
const RESERVE_ACCOUNT_SIZE: usize = 8624;

fn get_rpc() -> Option<RpcClient> {
    let _ = dotenvy::dotenv();
    let url = std::env::var("SOLANA_RPC_URL").ok()?;
    if url.is_empty() {
        return None;
    }
    Some(RpcClient::new_with_commitment(url, CommitmentConfig::confirmed()))
}

fn dummy_config() -> liquidation_bot::config::AppConfig {
    liquidation_bot::config::AppConfig {
        rpc_url: String::new(),
        grpc_url: String::new(),
        grpc_token: None,
        kamino_market: KAMINO_MAIN_MARKET.to_string(),
        klend_program_id: KLEND_PROGRAM.to_string(),
        liquidator_keypair_path: String::new(),
        min_profit_lamports: 0,
        supabase_url: None,
        supabase_service_role_key: None,
    }
}

#[test]
fn validate_reserve_parsing_against_live_data() {
    let rpc = match get_rpc() {
        Some(r) => r,
        None => {
            eprintln!("SOLANA_RPC_URL not set, skipping live validation");
            return;
        }
    };

    let program_id = Pubkey::from_str(KLEND_PROGRAM).unwrap();

    // Fetch reserves by dataSize (8624 bytes = Reserve account)
    use solana_client::rpc_filter::RpcFilterType;

    let filters = vec![
        RpcFilterType::DataSize(RESERVE_ACCOUNT_SIZE as u64),
    ];

    println!("Fetching reserves from mainnet (program={})...", KLEND_PROGRAM);
    let accounts = rpc
        .get_program_accounts_with_config(
            &program_id,
            solana_client::rpc_config::RpcProgramAccountsConfig {
                filters: Some(filters),
                account_config: solana_client::rpc_config::RpcAccountInfoConfig {
                    encoding: Some(solana_account_decoder::UiAccountEncoding::Base64),
                    ..Default::default()
                },
                with_context: None,
                sort_results: None,
            },
        )
        .expect("failed to fetch reserves");

    println!("Fetched {} reserve accounts", accounts.len());
    assert!(!accounts.is_empty(), "no reserves found");

    let market_pubkey = Pubkey::from_str(KAMINO_MAIN_MARKET).unwrap();
    let mut main_market_reserves = 0;

    for (pubkey, account) in &accounts {
        let data = &account.data;

        let reserve = liquidation_bot::liquidator::reserve::parse_reserve(pubkey, data);
        assert!(reserve.is_ok(), "reserve parse failed for {}: {:?}", pubkey, reserve.err());
        let reserve = reserve.unwrap();

        // Check if this reserve belongs to the main market
        // lending_market is at struct offset 24 + 8 disc = 32
        let lending_market_bytes: [u8; 32] = data[32..64].try_into().unwrap();
        let lending_market = Pubkey::new_from_array(lending_market_bytes);
        if lending_market != market_pubkey {
            continue; // Skip reserves from other markets
        }
        main_market_reserves += 1;

        // Sanity checks
        assert_ne!(reserve.accounts.liquidity_mint, Pubkey::default(),
            "liquidity_mint is zero for {} (offset error?)", pubkey);
        assert_ne!(reserve.accounts.liquidity_supply_vault, Pubkey::default(),
            "supply_vault is zero for {} (offset error?)", pubkey);
        assert_ne!(reserve.accounts.liquidity_fee_vault, Pubkey::default(),
            "fee_vault is zero for {} (offset error?)", pubkey);
        assert_ne!(reserve.accounts.collateral_mint, Pubkey::default(),
            "collateral_mint is zero for {} (offset error?)", pubkey);

        // Token program should be SPL Token or Token-2022
        let spl_token: Pubkey = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".parse().unwrap();
        let token_2022: Pubkey = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb".parse().unwrap();
        assert!(
            reserve.accounts.token_program == spl_token || reserve.accounts.token_program == token_2022,
            "unexpected token_program for {}: {} (offset error?)",
            pubkey, reserve.accounts.token_program
        );

        // Liquidation params should be reasonable
        assert!(
            reserve.liquidation_threshold_pct <= 100,
            "liquidation_threshold_pct={} for {} (offset error?)",
            reserve.liquidation_threshold_pct, pubkey
        );
        assert!(
            reserve.max_liquidation_bonus_bps <= 10000,
            "max_liquidation_bonus_bps={} for {} (offset error?)",
            reserve.max_liquidation_bonus_bps, pubkey
        );

        println!(
            "  {} mint={} avail={} threshold={}% bonus={}-{}bps",
            pubkey,
            &reserve.accounts.liquidity_mint.to_string()[..8],
            reserve.available_liquidity,
            reserve.liquidation_threshold_pct,
            reserve.min_liquidation_bonus_bps,
            reserve.max_liquidation_bonus_bps,
        );
    }

    println!("\nValidated {} reserves for main market (of {} total)", main_market_reserves, accounts.len());
    assert!(main_market_reserves >= 3, "expected at least 3 main market reserves");
}

#[test]
fn validate_obligation_offsets_against_live_data() {
    let rpc = match get_rpc() {
        Some(r) => r,
        None => {
            eprintln!("SOLANA_RPC_URL not set, skipping live validation");
            return;
        }
    };

    let program_id = Pubkey::from_str(KLEND_PROGRAM).unwrap();
    let market_pubkey = Pubkey::from_str(KAMINO_MAIN_MARKET).unwrap();

    // Obligation accounts: filter by dataSize (3344) and lending_market at offset 32
    use solana_client::rpc_filter::{Memcmp, RpcFilterType};

    let filters = vec![
        RpcFilterType::DataSize(3344),
        RpcFilterType::Memcmp(Memcmp::new_raw_bytes(32, market_pubkey.to_bytes().to_vec())),
    ];

    println!("Fetching obligations from mainnet (market={})...", KAMINO_MAIN_MARKET);
    let accounts = rpc
        .get_program_accounts_with_config(
            &program_id,
            solana_client::rpc_config::RpcProgramAccountsConfig {
                filters: Some(filters),
                account_config: solana_client::rpc_config::RpcAccountInfoConfig {
                    encoding: Some(solana_account_decoder::UiAccountEncoding::Base64),
                    ..Default::default()
                },
                with_context: None,
                sort_results: None,
            },
        )
        .expect("failed to fetch obligations");

    println!("Fetched {} obligation accounts", accounts.len());
    assert!(!accounts.is_empty(), "no obligations found — RPC may not support getProgramAccounts");

    let config = dummy_config();
    let mut tested = 0;
    let mut with_borrows = 0;

    for (pubkey, account) in accounts.iter().take(30) {
        let data = &account.data;

        // Skip if not the expected obligation size
        if data.len() < 3344 {
            continue;
        }

        // 1. Health evaluation should not panic
        let health = match liquidation_bot::obligation::health::evaluate(data, &config) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("  SKIP {}: {}", pubkey, e);
                continue;
            }
        };

        // 2. Sanity checks
        if health.deposited_value_sf > 0 {
            assert!(
                health.current_ltv >= 0.0,
                "negative LTV for {}: {}", pubkey, health.current_ltv
            );
            assert!(
                health.current_ltv < 10.0,
                "unreasonable LTV for {}: {} (possible offset error)", pubkey, health.current_ltv
            );
            if health.unhealthy_ltv > 0.0 {
                assert!(
                    health.unhealthy_ltv < 10.0,
                    "unreasonable unhealthy LTV for {}: {}", pubkey, health.unhealthy_ltv
                );
            }
        }

        // 3. Position parsing
        let positions = liquidation_bot::obligation::positions::parse_positions(data);
        assert!(positions.is_ok(), "position parse failed for {}: {:?}", pubkey, positions.err());
        let positions = positions.unwrap();

        assert_eq!(
            positions.lending_market, market_pubkey,
            "lending_market mismatch for {} (offset error?)", pubkey
        );
        assert_ne!(positions.owner, Pubkey::default(),
            "owner is zero for {} (offset error?)", pubkey);

        if !positions.borrows.is_empty() {
            with_borrows += 1;
            for borrow in &positions.borrows {
                assert_ne!(borrow.reserve, Pubkey::default());
                assert!(borrow.borrowed_amount_sf > 0);
            }
        }

        for deposit in &positions.deposits {
            assert_ne!(deposit.reserve, Pubkey::default());
            assert!(deposit.deposited_amount > 0);
        }

        tested += 1;
        println!(
            "  [{}] {} d={} b={} ltv={:.4} liq={}",
            tested, &pubkey.to_string()[..8],
            positions.deposits.len(), positions.borrows.len(),
            health.current_ltv, health.is_liquidatable,
        );
    }

    println!("\nValidated {} obligations ({} with borrows)", tested, with_borrows);
    assert!(tested >= 5, "too few obligations tested: {}", tested);
}

#[test]
fn validate_lending_market_parsing() {
    let rpc = match get_rpc() {
        Some(r) => r,
        None => {
            eprintln!("SOLANA_RPC_URL not set, skipping live validation");
            return;
        }
    };

    let market_pubkey = Pubkey::from_str(KAMINO_MAIN_MARKET).unwrap();
    let account = rpc.get_account(&market_pubkey).expect("failed to fetch lending market");

    let market = liquidation_bot::liquidator::reserve::parse_lending_market(&account.data);
    assert!(market.is_ok(), "lending market parse failed: {:?}", market.err());
    let market = market.unwrap();

    println!(
        "LendingMarket: close_factor={}% max_debt_at_once={}",
        market.liquidation_max_debt_close_factor_pct,
        market.max_liquidatable_debt_market_value_at_once,
    );

    assert!(
        market.liquidation_max_debt_close_factor_pct > 0
            && market.liquidation_max_debt_close_factor_pct <= 100,
        "unreasonable close factor: {}",
        market.liquidation_max_debt_close_factor_pct
    );
}
