//! Flash loan liquidation transaction builder.
//!
//! Constructs an atomic transaction:
//!   ix[0]: flash_borrow_reserve_liquidity — borrow the repay token
//!   ix[1]: liquidate_obligation_and_redeem_reserve_collateral — repay debt, seize collateral
//!   ix[2]: flash_repay_reserve_liquidity — repay the flash loan (principal + fee)
//!
//! Profit = received collateral value - flash loan principal - flash loan fee

use anyhow::{bail, Context, Result};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::{read_keypair_file, Keypair, Signer},
    transaction::Transaction,
};

use crate::config::AppConfig;
use crate::obligation::{health::HealthResult, positions};
use super::instructions::{self, LiquidateParams, ReserveAccounts};
use super::reserve::{self, ReserveData, LendingMarketData};

/// Scale factor for klend's u128 scaled fractions (2^60).
const SF_SHIFT: u128 = 1u128 << 60;

/// Context gathered for executing a liquidation.
#[derive(Debug)]
pub struct LiquidationContext {
    pub obligation_pubkey: Pubkey,
    pub obligation_data: Vec<u8>,
    pub positions: positions::ObligationPositions,
    pub repay_reserve: ReserveData,
    pub withdraw_reserve: ReserveData,
    pub lending_market: LendingMarketData,
    pub lending_market_pubkey: Pubkey,
    pub lending_market_authority: Pubkey,
    pub repay_amount: u64,
    pub min_receive_amount: u64,
}

/// Gather all on-chain data needed for a liquidation, then build the transaction.
pub async fn build_liquidation_tx(
    config: &AppConfig,
    obligation_pubkey: &Pubkey,
    _health: &HealthResult,
) -> Result<(Transaction, Keypair)> {
    let rpc = RpcClient::new_with_commitment(
        config.rpc_url.clone(),
        CommitmentConfig::confirmed(),
    );

    let liquidator_keypair = read_keypair_file(&config.liquidator_keypair_path)
        .map_err(|e| anyhow::anyhow!("failed to read keypair: {}", e))?;

    // 1. Fetch obligation account
    let obligation_account = rpc
        .get_account(obligation_pubkey)
        .context("failed to fetch obligation account")?;
    let obligation_data = obligation_account.data;

    // 2. Parse positions to find which reserves are involved
    let obligation_positions = positions::parse_positions(&obligation_data)?;

    if obligation_positions.borrows.is_empty() {
        bail!("obligation has no borrows — nothing to liquidate");
    }
    if obligation_positions.deposits.is_empty() {
        bail!("obligation has no deposits — no collateral to seize");
    }

    // 3. Select the highest-value borrow (repay) and deposit (withdraw)
    let repay_position = obligation_positions
        .borrows
        .iter()
        .max_by_key(|b| b.market_value_sf)
        .unwrap();
    let withdraw_position = obligation_positions
        .deposits
        .iter()
        .max_by_key(|d| d.market_value_sf)
        .unwrap();

    // 4. Fetch reserve accounts
    let repay_reserve_account = rpc
        .get_account(&repay_position.reserve)
        .context("failed to fetch repay reserve")?;
    let repay_reserve = reserve::parse_reserve(&repay_position.reserve, &repay_reserve_account.data)?;

    let withdraw_reserve_account = rpc
        .get_account(&withdraw_position.reserve)
        .context("failed to fetch withdraw reserve")?;
    let withdraw_reserve =
        reserve::parse_reserve(&withdraw_position.reserve, &withdraw_reserve_account.data)?;

    // 5. Fetch lending market for liquidation params
    let lending_market_pubkey = obligation_positions.lending_market;
    let lending_market_account = rpc
        .get_account(&lending_market_pubkey)
        .context("failed to fetch lending market")?;
    let lending_market = reserve::parse_lending_market(&lending_market_account.data)?;

    // 6. Derive PDA
    let program_id = config.klend_program_pubkey()?;
    let (lending_market_authority, _) =
        instructions::derive_lending_market_authority(&lending_market_pubkey, &program_id);

    // 7. Calculate liquidation amount
    let repay_amount = calculate_repay_amount(
        repay_position.borrowed_amount_sf,
        &lending_market,
        &repay_reserve,
    );

    if repay_amount == 0 {
        bail!("calculated repay amount is zero");
    }

    // Min receive = 0 for now (accept any collateral — can tighten for MEV protection)
    let min_receive_amount = 0u64;

    let ctx = LiquidationContext {
        obligation_pubkey: *obligation_pubkey,
        obligation_data,
        positions: obligation_positions,
        repay_reserve,
        withdraw_reserve,
        lending_market,
        lending_market_pubkey,
        lending_market_authority,
        repay_amount,
        min_receive_amount,
    };

    let tx = build_tx(&ctx, &liquidator_keypair, &program_id, &rpc)?;
    Ok((tx, liquidator_keypair))
}

/// Build the atomic transaction, creating ATAs if needed.
fn build_tx(
    ctx: &LiquidationContext,
    liquidator: &Keypair,
    program_id: &Pubkey,
    rpc: &RpcClient,
) -> Result<Transaction> {
    let liquidator_pubkey = liquidator.pubkey();

    let repay_token_account = spl_associated_token_account(
        &liquidator_pubkey,
        &ctx.repay_reserve.accounts.liquidity_mint,
    );
    let withdraw_collateral_account = spl_associated_token_account(
        &liquidator_pubkey,
        &ctx.withdraw_reserve.accounts.collateral_mint,
    );
    let withdraw_liquidity_account = spl_associated_token_account(
        &liquidator_pubkey,
        &ctx.withdraw_reserve.accounts.liquidity_mint,
    );

    // Create ATA instructions for any token accounts that don't exist yet.
    // These are prepended to the transaction before the flash loan instructions.
    let mut setup_ixs = Vec::new();

    let atas_to_check = [
        (repay_token_account, ctx.repay_reserve.accounts.liquidity_mint, ctx.repay_reserve.accounts.token_program),
        (withdraw_collateral_account, ctx.withdraw_reserve.accounts.collateral_mint, ctx.withdraw_reserve.accounts.token_program),
        (withdraw_liquidity_account, ctx.withdraw_reserve.accounts.liquidity_mint, ctx.withdraw_reserve.accounts.token_program),
    ];

    for (ata, mint, _token_program) in &atas_to_check {
        if rpc.get_account(ata).is_err() {
            tracing::info!(ata = %ata, mint = %mint, "creating ATA");
            setup_ixs.push(
                spl_associated_token_account::instruction::create_associated_token_account(
                    &liquidator_pubkey,
                    &liquidator_pubkey,
                    mint,
                    &spl_token::id(),
                ),
            );
        }
    }

    // The flash borrow instruction index within the tx (after any ATA creation ixs)
    let borrow_ix_index = setup_ixs.len() as u8;

    let flash_borrow_ix = instructions::flash_borrow_reserve_liquidity(
        program_id,
        ctx.repay_amount,
        &liquidator_pubkey,
        &ctx.lending_market_pubkey,
        &ctx.lending_market_authority,
        &ctx.repay_reserve.accounts,
        &repay_token_account,
    );

    // ix[1]: Liquidate — repay debt and seize collateral
    let liquidate_ix = instructions::liquidate_obligation_and_redeem_reserve_collateral(
        program_id,
        &LiquidateParams {
            liquidity_amount: ctx.repay_amount,
            min_acceptable_received_liquidity_amount: ctx.min_receive_amount,
        },
        &liquidator_pubkey,
        &ctx.obligation_pubkey,
        &ctx.lending_market_pubkey,
        &ctx.lending_market_authority,
        &ctx.repay_reserve.accounts,
        &ctx.withdraw_reserve.accounts,
        &repay_token_account,
        &withdraw_collateral_account,
        &withdraw_liquidity_account,
    );

    // Flash repay — return borrowed tokens + fee
    let flash_repay_ix = instructions::flash_repay_reserve_liquidity(
        program_id,
        ctx.repay_amount,
        borrow_ix_index,
        &liquidator_pubkey,
        &ctx.lending_market_pubkey,
        &ctx.lending_market_authority,
        &ctx.repay_reserve.accounts,
        &repay_token_account,
    );

    let recent_blockhash = rpc.get_latest_blockhash()?;

    // Combine: setup ATAs (if any) + flash_borrow + liquidate + flash_repay
    let mut all_ixs = setup_ixs;
    all_ixs.push(flash_borrow_ix);
    all_ixs.push(liquidate_ix);
    all_ixs.push(flash_repay_ix);

    let tx = Transaction::new_signed_with_payer(
        &all_ixs,
        Some(&liquidator_pubkey),
        &[liquidator],
        recent_blockhash,
    );

    Ok(tx)
}

/// Calculate the repay amount based on the close factor and available liquidity.
fn calculate_repay_amount(
    borrowed_amount_sf: u128,
    market: &LendingMarketData,
    repay_reserve: &ReserveData,
) -> u64 {
    // Convert scaled fraction to actual amount
    let borrowed_amount = (borrowed_amount_sf / SF_SHIFT) as u64;

    // Close factor: what % of debt can be liquidated at once
    let close_factor_pct = market.liquidation_max_debt_close_factor_pct as u64;
    let max_repay_by_close_factor = borrowed_amount
        .checked_mul(close_factor_pct)
        .unwrap_or(u64::MAX)
        / 100;

    // Cap by available liquidity in the reserve
    let max_repay = max_repay_by_close_factor
        .min(repay_reserve.available_liquidity);

    max_repay
}

/// Derive the associated token account address.
/// Seeds: [wallet, TOKEN_PROGRAM_ID, mint]
fn spl_associated_token_account(wallet: &Pubkey, mint: &Pubkey) -> Pubkey {
    let ata_program: Pubkey = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"
        .parse()
        .unwrap();
    let token_program: Pubkey = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
        .parse()
        .unwrap();

    let (ata, _) = Pubkey::find_program_address(
        &[wallet.as_ref(), token_program.as_ref(), mint.as_ref()],
        &ata_program,
    );
    ata
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repay_amount_respects_close_factor() {
        let borrowed_sf = 1000u128 * SF_SHIFT; // 1000 tokens
        let market = LendingMarketData {
            liquidation_max_debt_close_factor_pct: 50, // 50%
            max_liquidatable_debt_market_value_at_once: u64::MAX,
        };
        let reserve = ReserveData {
            accounts: ReserveAccounts {
                reserve: Pubkey::new_unique(),
                liquidity_mint: Pubkey::new_unique(),
                liquidity_supply_vault: Pubkey::new_unique(),
                liquidity_fee_vault: Pubkey::new_unique(),
                collateral_mint: Pubkey::new_unique(),
                collateral_supply_vault: Pubkey::new_unique(),
                token_program: Pubkey::new_unique(),
            },
            available_liquidity: u64::MAX,
            borrowed_amount_sf: borrowed_sf,
            market_price_sf: 0,
            liquidation_threshold_pct: 80,
            min_liquidation_bonus_bps: 100,
            max_liquidation_bonus_bps: 500,
            protocol_liquidation_fee_pct: 0,
            flash_loan_fee_sf: 0,
        };

        let amount = calculate_repay_amount(borrowed_sf, &market, &reserve);
        assert_eq!(amount, 500); // 50% of 1000
    }

    #[test]
    fn repay_amount_capped_by_available_liquidity() {
        let borrowed_sf = 1000u128 * SF_SHIFT;
        let market = LendingMarketData {
            liquidation_max_debt_close_factor_pct: 100,
            max_liquidatable_debt_market_value_at_once: u64::MAX,
        };
        let reserve = ReserveData {
            accounts: ReserveAccounts {
                reserve: Pubkey::new_unique(),
                liquidity_mint: Pubkey::new_unique(),
                liquidity_supply_vault: Pubkey::new_unique(),
                liquidity_fee_vault: Pubkey::new_unique(),
                collateral_mint: Pubkey::new_unique(),
                collateral_supply_vault: Pubkey::new_unique(),
                token_program: Pubkey::new_unique(),
            },
            available_liquidity: 200, // only 200 available
            borrowed_amount_sf: borrowed_sf,
            market_price_sf: 0,
            liquidation_threshold_pct: 80,
            min_liquidation_bonus_bps: 100,
            max_liquidation_bonus_bps: 500,
            protocol_liquidation_fee_pct: 0,
            flash_loan_fee_sf: 0,
        };

        let amount = calculate_repay_amount(borrowed_sf, &market, &reserve);
        assert_eq!(amount, 200); // capped by available
    }

    #[test]
    fn ata_derivation_is_deterministic() {
        let wallet = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let ata1 = spl_associated_token_account(&wallet, &mint);
        let ata2 = spl_associated_token_account(&wallet, &mint);
        assert_eq!(ata1, ata2);
        assert_ne!(ata1, Pubkey::default());
    }
}
