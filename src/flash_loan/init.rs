//! Flash loan provider initialization.
//!
//! Fetches on-chain account data at startup and populates the flash loan
//! providers with the reserve/mint information they need to build instructions.

use anyhow::{Context, Result};
use solana_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

use crate::config::AppConfig;
use crate::protocols::jupiter_lend;
use crate::protocols::jupiter_lend::instructions::JupiterFlashLoanAccounts;
use crate::protocols::kamino::reserve;

use super::kamino::KaminoFlashLoanProvider;
use super::jupiter::JupiterFlashLoanProvider;
use super::FlashLoanProvider;

/// Well-known Kamino Lend reserve addresses for major tokens.
/// These are the reserve accounts on the main Kamino market
/// (7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF).
///
/// TODO: Fetch dynamically by scanning all reserves on the market.
/// For now, hardcode the most common ones.
const KAMINO_RESERVES: &[&str] = &[
    "D6q6wuQSrifJKZYpR1M8R4YawnLDtDsMmWM1NbBmgJ59", // USDC
    "d4A2prbA2whesmvHaL88BH6Ewn5N4bTSU2Ze8P6Bc4Q",  // SOL
    "H3t6qZ1JkguCNTi9uzVKqQ7dvt2cum4XiXWom6Gn5e5S", // USDT
    "EVbyPKrHG6WBfm4dLxLMJpUDY43cCAcHSpV3KYjKsktW", // JitoSOL
    "FjWoZMcBMSS3Dg1MiSGPa7FMosLvmGGVmQVp4o7U2MF",  // JLP
];

/// Initialize flash loan providers by fetching on-chain account data.
///
/// Returns a Vec of providers ordered by preference (cheapest first):
/// 1. Jupiter Lend (0% fee)
/// 2. Kamino (0.001% fee)
pub fn initialize_providers(
    config: &AppConfig,
    rpc: &RpcClient,
) -> Result<Vec<Box<dyn FlashLoanProvider>>> {
    let mut providers: Vec<Box<dyn FlashLoanProvider>> = Vec::new();

    // --- Jupiter Lend (0% fee) ---
    match initialize_jupiter(rpc) {
        Ok(jupiter) => {
            let mint_count = jupiter.mint_count();
            providers.push(Box::new(jupiter));
            tracing::info!(mints = mint_count, "initialized Jupiter flash loan provider (0% fee)");
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to initialize Jupiter flash loan provider");
        }
    }

    // --- Kamino (0.001% fee) ---
    match initialize_kamino(config, rpc) {
        Ok(kamino) => {
            let reserve_count = kamino.reserve_count();
            providers.push(Box::new(kamino));
            tracing::info!(reserves = reserve_count, "initialized Kamino flash loan provider (0.001% fee)");
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to initialize Kamino flash loan provider");
        }
    }

    if providers.is_empty() {
        tracing::error!("no flash loan providers initialized — liquidations will require pre-funded capital");
    }

    Ok(providers)
}

/// Initialize Kamino flash loan provider by fetching reserve accounts.
fn initialize_kamino(
    config: &AppConfig,
    rpc: &RpcClient,
) -> Result<KaminoFlashLoanProvider> {
    let market = config.kamino_market;
    let mut provider = KaminoFlashLoanProvider::new(&market);

    for reserve_addr in KAMINO_RESERVES {
        let reserve_pubkey = Pubkey::from_str(reserve_addr)
            .context("invalid reserve address")?;

        match rpc.get_account(&reserve_pubkey) {
            Ok(account) => {
                match reserve::parse_reserve(&reserve_pubkey, &account.data) {
                    Ok(reserve_data) => {
                        let mint = reserve_data.accounts.liquidity_mint;
                        provider.add_reserve(reserve_data.accounts);
                        tracing::debug!(
                            reserve = %reserve_addr,
                            mint = %mint,
                            "registered Kamino reserve for flash loans"
                        );
                    }
                    Err(e) => {
                        tracing::debug!(reserve = %reserve_addr, error = %e, "failed to parse reserve");
                    }
                }
            }
            Err(e) => {
                tracing::debug!(reserve = %reserve_addr, error = %e, "failed to fetch reserve");
            }
        }
    }

    Ok(provider)
}

/// Initialize Jupiter Lend flash loan provider by deriving PDAs.
fn initialize_jupiter(_rpc: &RpcClient) -> Result<JupiterFlashLoanProvider> {
    let mut provider = JupiterFlashLoanProvider::new();

    let flash_program: Pubkey = jupiter_lend::FLASH_LOAN_PROGRAM_ID;
    let lending_program: Pubkey = jupiter_lend::LENDING_PROGRAM_ID;

    // Derive the flash loan admin PDA
    let (flashloan_admin, _) = Pubkey::find_program_address(
        &[b"flashloan_admin"],
        &flash_program,
    );

    // Liquidity PDA (shared across all mints)
    let (liquidity_pda, _) = Pubkey::find_program_address(
        &[b"liquidity"],
        &lending_program,
    );

    // Well-known token mints to register for flash loans
    let mints: Vec<(&str, Pubkey)> = vec![
        ("USDC", "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".parse().unwrap()),
        ("USDT", "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB".parse().unwrap()),
        ("SOL",  "So11111111111111111111111111111111111111112".parse().unwrap()),
    ];

    for (name, mint) in &mints {
        // Derive PDAs for this mint's flash loan accounts
        let (token_reserves, _) = Pubkey::find_program_address(
            &[b"token_reserves", mint.as_ref()],
            &lending_program,
        );

        let (borrow_position, _) = Pubkey::find_program_address(
            &[b"borrow_position_on_liquidity", flashloan_admin.as_ref(), mint.as_ref()],
            &flash_program,
        );

        let (rate_model, _) = Pubkey::find_program_address(
            &[b"rate_model", mint.as_ref()],
            &lending_program,
        );

        // The vault for flash loans
        let (vault, _) = Pubkey::find_program_address(
            &[b"vault", mint.as_ref()],
            &flash_program,
        );

        let accounts = JupiterFlashLoanAccounts {
            flashloan_admin,
            mint: *mint,
            flashloan_token_reserves_liquidity: token_reserves,
            flashloan_borrow_position_on_liquidity: borrow_position,
            rate_model,
            vault,
            liquidity: liquidity_pda,
            liquidity_program: lending_program,
        };

        provider.add_mint(*mint, accounts);
        tracing::debug!(token = name, mint = %mint, "registered Jupiter flash loan mint");
    }

    Ok(provider)
}
