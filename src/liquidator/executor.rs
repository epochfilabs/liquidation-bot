//! Unified cross-protocol liquidation executor.
//!
//! Uses the FlashLoanProvider trait to select the cheapest flash loan source
//! for each liquidation. Jupiter Lend (0% fee) is preferred over Kamino (0.001%).
//!
//! Transaction layout:
//!   [ATA creation ixs (if needed)]
//!   flash_borrow (from cheapest provider)
//!   protocol-specific liquidate instruction
//!   [optional: Jupiter swap if collateral ≠ debt token]
//!   flash_repay (to same provider)

use anyhow::{bail, Context, Result};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::{read_keypair_file, Keypair, Signer},
    transaction::Transaction,
};

use crate::config::AppConfig;
use crate::db::{self, LiquidationStatus, NewLiquidationRecord, SupabaseClient, UpdateLiquidationResult};
use crate::flash_loan::{self, FlashLoanProvider, FlashLoanProviderKind};
use crate::liquidator::instructions as kamino_ix;
use crate::liquidator::reserve as kamino_reserve;
use crate::protocols::{self, ProtocolKind, Positions};
use crate::protocols::jupiter_lend::{self};
use crate::protocols::jupiter_lend_instructions as jup_ix;
use crate::protocols::save_instructions as save_ix;
use crate::protocols::marginfi_bank;
use crate::protocols::marginfi_instructions as mfi_ix;

/// Context for a liquidation across any protocol.
pub struct LiquidationParams {
    pub protocol: ProtocolKind,
    pub position_pubkey: Pubkey,
    pub health: protocols::HealthResult,
    pub positions: Positions,
}

/// Execute a liquidation on any protocol using flash loans from the cheapest provider.
///
/// The `flash_providers` list should be ordered by preference (cheapest first).
/// `select_provider` picks the cheapest that supports the debt token mint.
pub async fn execute_liquidation(
    config: &AppConfig,
    params: &LiquidationParams,
    flash_providers: &[Box<dyn FlashLoanProvider>],
    supabase: Option<&SupabaseClient>,
) -> Result<()> {
    let rpc = RpcClient::new_with_commitment(
        config.rpc_url.clone(),
        CommitmentConfig::confirmed(),
    );

    let liquidator_keypair = read_keypair_file(&config.liquidator_keypair_path)
        .map_err(|e| anyhow::anyhow!("failed to read keypair: {}", e))?;
    let liquidator_pubkey = liquidator_keypair.pubkey();

    if params.positions.borrows.is_empty() || params.positions.deposits.is_empty() {
        return Ok(());
    }

    // Select the highest-value borrow (to repay) and deposit (to seize)
    let repay_pos = params.positions.borrows.iter()
        .max_by(|a, b| a.market_value_usd.partial_cmp(&b.market_value_usd).unwrap())
        .unwrap();
    let withdraw_pos = params.positions.deposits.iter()
        .max_by(|a, b| a.market_value_usd.partial_cmp(&b.market_value_usd).unwrap())
        .unwrap();

    // Determine the debt token mint (needed for flash loan provider selection)
    let debt_mint = repay_pos.mint.unwrap_or_default();

    // Select the cheapest flash loan provider for this mint
    let flash_provider = flash_loan::select_provider(flash_providers, &debt_mint);

    let flash_provider_name = flash_provider
        .map(|p| p.kind().to_string())
        .unwrap_or_else(|| "none".to_string());

    tracing::info!(
        protocol = %params.protocol,
        position = %params.position_pubkey,
        debt_mint = %debt_mint,
        flash_provider = %flash_provider_name,
        "preparing liquidation"
    );

    // Build the protocol-specific liquidation instruction
    let liquidate_ix = match params.protocol {
        ProtocolKind::Kamino => {
            build_kamino_liquidate_ix(&rpc, config, params, &liquidator_pubkey)?
        }
        ProtocolKind::JupiterLend => {
            build_jupiter_liquidate_ix(&rpc, params, &liquidator_pubkey)?
        }
        ProtocolKind::Save => {
            build_save_liquidate_ix(&rpc, params, &liquidator_pubkey)?
        }
        ProtocolKind::MarginFi => {
            build_marginfi_liquidate_ix(&rpc, params, &liquidator_pubkey)?
        }
    };

    // Build ATA creation instructions if needed
    let setup_ixs = build_setup_ixs(&rpc, &liquidator_pubkey, &debt_mint, &withdraw_pos)?;

    // Calculate repay amount
    let repay_amount = calculate_repay_amount(params, repay_pos);

    // Derive the liquidator's token account for the debt mint
    let liquidator_token_account = spl_associated_token_account(
        &liquidator_pubkey, &debt_mint,
    );

    // Build the full transaction
    let all_ixs = if let Some(provider) = flash_provider {
        // Flash loan path: borrow → liquidate → [swap] → repay
        let borrow_ix_index = setup_ixs.len() as u8;

        let flash_ixs = provider.build_instructions(
            &liquidator_pubkey,
            &liquidator_token_account,
            &debt_mint,
            repay_amount,
            borrow_ix_index,
        )?;

        tracing::info!(
            provider = %flash_ixs.provider,
            amount = repay_amount,
            fee_rate = %provider.fee_rate(),
            "using flash loan"
        );

        flash_loan::build_flash_loan_tx(
            setup_ixs,
            flash_ixs,
            liquidate_ix,
            None, // TODO: Jupiter swap if collateral ≠ debt
        )
    } else {
        // No flash loan available — liquidator must hold the debt token
        tracing::warn!(
            debt_mint = %debt_mint,
            "no flash loan provider for this mint — submitting without flash loan"
        );
        let mut ixs = setup_ixs;
        ixs.push(liquidate_ix);
        ixs
    };

    // Log to DB
    let record_id = db::new_record_id();
    let db_record = NewLiquidationRecord {
        id: record_id.clone(),
        obligation_pubkey: params.position_pubkey.to_string(),
        obligation_owner: params.positions.owner.to_string(),
        lending_market: params.positions.market.to_string(),
        repay_reserve: repay_pos.reserve.to_string(),
        repay_mint: debt_mint.to_string(),
        withdraw_reserve: withdraw_pos.reserve.to_string(),
        withdraw_mint: withdraw_pos.mint.map_or("unknown".into(), |m| m.to_string()),
        ltv_at_detection: params.health.current_ltv,
        unhealthy_ltv: params.health.unhealthy_ltv,
        repay_amount: repay_amount as i64,
        liquidation_bonus_bps: 0,
        flash_loan_fee_fraction: flash_provider.map_or(0.0, |p| p.fee_rate()),
        estimated_gross_profit_usd: 0.0,
        estimated_net_profit_usd: 0.0,
        status: LiquidationStatus::Pending.to_string(),
        error_message: None,
    };

    if let Some(sb) = supabase {
        let _ = sb.insert_liquidation(&db_record).await;
    }

    // Sign and submit
    let recent_blockhash = rpc.get_latest_blockhash()?;
    let tx = Transaction::new_signed_with_payer(
        &all_ixs,
        Some(&liquidator_pubkey),
        &[&liquidator_keypair],
        recent_blockhash,
    );

    if let Some(sb) = supabase {
        let _ = sb.update_liquidation(&record_id, &UpdateLiquidationResult {
            status: LiquidationStatus::Submitted.to_string(),
            updated_at: db::now_iso(),
            tx_signature: None, error_message: None, actual_profit_usd: None,
            sol_fee_paid: None, slot_submitted: None, slot_confirmed: None,
        }).await;
    }

    match rpc.send_and_confirm_transaction(&tx) {
        Ok(sig) => {
            tracing::info!(
                protocol = %params.protocol,
                position = %params.position_pubkey,
                signature = %sig,
                flash_provider = %flash_provider_name,
                "liquidation confirmed"
            );
            if let Some(sb) = supabase {
                let _ = sb.update_liquidation(&record_id, &UpdateLiquidationResult {
                    status: LiquidationStatus::Confirmed.to_string(),
                    updated_at: db::now_iso(),
                    tx_signature: Some(sig.to_string()),
                    error_message: None, actual_profit_usd: None,
                    sol_fee_paid: None, slot_submitted: None, slot_confirmed: None,
                }).await;
            }
        }
        Err(e) => {
            tracing::error!(
                protocol = %params.protocol,
                position = %params.position_pubkey,
                error = %e,
                "liquidation tx failed"
            );
            if let Some(sb) = supabase {
                let _ = sb.update_liquidation(&record_id, &UpdateLiquidationResult {
                    status: LiquidationStatus::Failed.to_string(),
                    updated_at: db::now_iso(),
                    tx_signature: None,
                    error_message: Some(e.to_string()),
                    actual_profit_usd: None,
                    sol_fee_paid: None, slot_submitted: None, slot_confirmed: None,
                }).await;
            }
            return Err(e.into());
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn calculate_repay_amount(params: &LiquidationParams, repay_pos: &protocols::BorrowPosition) -> u64 {
    let sf_shift: u128 = 1u128 << 60;
    match params.protocol {
        ProtocolKind::Kamino => {
            // Kamino: borrowed_amount_sf is 2^60-scaled
            (repay_pos.amount_sf / sf_shift) as u64
        }
        ProtocolKind::Save => {
            // Save: borrowed_amount_wads is WAD-scaled (10^18)
            let wad: u128 = 1_000_000_000_000_000_000;
            (repay_pos.amount_sf / wad) as u64
        }
        ProtocolKind::JupiterLend => {
            // Jupiter: dust_debt_amount is already in native units
            repay_pos.amount_sf as u64
        }
        ProtocolKind::MarginFi => {
            // MarginFi: liability_shares, not directly usable as repay amount
            // The actual amount needs to be derived from shares × share_value
            repay_pos.amount_sf as u64
        }
    }
}

fn build_setup_ixs(
    rpc: &RpcClient,
    liquidator: &Pubkey,
    debt_mint: &Pubkey,
    withdraw_pos: &protocols::DepositPosition,
) -> Result<Vec<Instruction>> {
    let mut ixs = Vec::new();

    // Create ATA for debt token if needed
    let debt_ata = spl_associated_token_account(liquidator, debt_mint);
    if rpc.get_account(&debt_ata).is_err() {
        ixs.push(
            spl_associated_token_account::instruction::create_associated_token_account(
                liquidator, liquidator, debt_mint, &spl_token::id(),
            ),
        );
    }

    // Create ATA for collateral token if needed and mint is known
    if let Some(col_mint) = withdraw_pos.mint {
        let col_ata = spl_associated_token_account(liquidator, &col_mint);
        if rpc.get_account(&col_ata).is_err() {
            ixs.push(
                spl_associated_token_account::instruction::create_associated_token_account(
                    liquidator, liquidator, &col_mint, &spl_token::id(),
                ),
            );
        }
    }

    Ok(ixs)
}

/// Build Kamino liquidation instruction.
fn build_kamino_liquidate_ix(
    rpc: &RpcClient,
    config: &AppConfig,
    params: &LiquidationParams,
    liquidator: &Pubkey,
) -> Result<Instruction> {
    let repay_pos = params.positions.borrows.iter()
        .max_by(|a, b| a.market_value_usd.partial_cmp(&b.market_value_usd).unwrap())
        .unwrap();
    let withdraw_pos = params.positions.deposits.iter()
        .max_by(|a, b| a.market_value_usd.partial_cmp(&b.market_value_usd).unwrap())
        .unwrap();

    let repay_reserve_acct = rpc.get_account(&repay_pos.reserve)
        .context("failed to fetch repay reserve")?;
    let repay_reserve = kamino_reserve::parse_reserve(&repay_pos.reserve, &repay_reserve_acct.data)?;

    let withdraw_reserve_acct = rpc.get_account(&withdraw_pos.reserve)
        .context("failed to fetch withdraw reserve")?;
    let withdraw_reserve = kamino_reserve::parse_reserve(&withdraw_pos.reserve, &withdraw_reserve_acct.data)?;

    let market_acct = rpc.get_account(&params.positions.market)
        .context("failed to fetch lending market")?;
    let market = kamino_reserve::parse_lending_market(&market_acct.data)?;

    let program_id = config.klend_program_pubkey()?;
    let (lending_market_authority, _) =
        kamino_ix::derive_lending_market_authority(&params.positions.market, &program_id);

    let sf_shift: u128 = 1u128 << 60;
    let borrowed_amount = (repay_pos.amount_sf / sf_shift) as u64;
    let close_factor = market.liquidation_max_debt_close_factor_pct as u64;
    let repay_amount = (borrowed_amount * close_factor / 100)
        .min(repay_reserve.available_liquidity);

    let liquidator_repay_ata = spl_associated_token_account(
        liquidator, &repay_reserve.accounts.liquidity_mint,
    );
    let liquidator_collateral_ata = spl_associated_token_account(
        liquidator, &withdraw_reserve.accounts.collateral_mint,
    );
    let liquidator_withdraw_ata = spl_associated_token_account(
        liquidator, &withdraw_reserve.accounts.liquidity_mint,
    );

    Ok(kamino_ix::liquidate_obligation_and_redeem_reserve_collateral(
        &program_id,
        &kamino_ix::LiquidateParams {
            liquidity_amount: repay_amount,
            min_acceptable_received_liquidity_amount: 0,
        },
        liquidator,
        &params.position_pubkey,
        &params.positions.market,
        &lending_market_authority,
        &repay_reserve.accounts,
        &withdraw_reserve.accounts,
        &liquidator_repay_ata,
        &liquidator_collateral_ata,
        &liquidator_withdraw_ata,
    ))
}

/// Build Jupiter Lend liquidation instruction.
fn build_jupiter_liquidate_ix(
    rpc: &RpcClient,
    params: &LiquidationParams,
    liquidator: &Pubkey,
) -> Result<Instruction> {
    let pos_account = rpc.get_account(&params.position_pubkey)
        .context("failed to fetch jupiter position")?;
    let position = jupiter_lend::parse_position(&pos_account.data)?;

    let vaults_program: Pubkey = jupiter_lend::VAULTS_PROGRAM_ID.parse().unwrap();
    let (vault_config_pda, _) = Pubkey::find_program_address(
        &[b"vault_config", &position.vault_id.to_le_bytes()],
        &vaults_program,
    );
    let vault_config_account = rpc.get_account(&vault_config_pda)
        .context("failed to fetch jupiter vault config")?;
    let vault_config = jupiter_lend::parse_vault_config(&vault_config_account.data)?;

    let (vault_state_pda, _) = Pubkey::find_program_address(
        &[b"vault_state", &position.vault_id.to_le_bytes()],
        &vaults_program,
    );

    let lending_program: Pubkey = jupiter_lend::LENDING_PROGRAM_ID.parse().unwrap();
    let (liquidity_pda, _) = Pubkey::find_program_address(&[b"liquidity"], &lending_program);
    let (supply_reserves, _) = Pubkey::find_program_address(
        &[b"token_reserves", vault_config.supply_token.as_ref()], &lending_program,
    );
    let (borrow_reserves, _) = Pubkey::find_program_address(
        &[b"token_reserves", vault_config.borrow_token.as_ref()], &lending_program,
    );
    let (vault_supply_pos, _) = Pubkey::find_program_address(
        &[b"position_on_liquidity", vault_config_pda.as_ref(), vault_config.supply_token.as_ref()],
        &vaults_program,
    );
    let (vault_borrow_pos, _) = Pubkey::find_program_address(
        &[b"position_on_liquidity", vault_config_pda.as_ref(), vault_config.borrow_token.as_ref()],
        &vaults_program,
    );
    let (supply_rate_model, _) = Pubkey::find_program_address(
        &[b"rate_model", vault_config.supply_token.as_ref()], &lending_program,
    );
    let (borrow_rate_model, _) = Pubkey::find_program_address(
        &[b"rate_model", vault_config.borrow_token.as_ref()], &lending_program,
    );
    let (new_branch, _) = Pubkey::find_program_address(
        &[b"branch", &position.vault_id.to_le_bytes(), &0u32.to_le_bytes()],
        &vaults_program,
    );

    let vault_supply_ata = spl_associated_token_account(&vault_config_pda, &vault_config.supply_token);
    let vault_borrow_ata = spl_associated_token_account(&vault_config_pda, &vault_config.borrow_token);
    let liquidator_borrow_ata = spl_associated_token_account(liquidator, &vault_config.borrow_token);
    let liquidator_supply_ata = spl_associated_token_account(liquidator, &vault_config.supply_token);

    let oracle = read_pubkey_at(&vault_config_account.data, 26);
    let oracle_program = read_pubkey_at(&vault_config_account.data, 122);

    let accounts = jup_ix::JupiterLiquidateAccounts {
        vault_config: vault_config_pda,
        vault_state: vault_state_pda,
        supply_token: vault_config.supply_token,
        borrow_token: vault_config.borrow_token,
        oracle,
        oracle_program,
        new_branch,
        supply_token_reserves_liquidity: supply_reserves,
        borrow_token_reserves_liquidity: borrow_reserves,
        vault_supply_position_on_liquidity: vault_supply_pos,
        vault_borrow_position_on_liquidity: vault_borrow_pos,
        supply_rate_model,
        borrow_rate_model,
        liquidity: liquidity_pda,
        liquidity_program: lending_program,
        vault_supply_token_account: vault_supply_ata,
        vault_borrow_token_account: vault_borrow_ata,
        supply_token_program: spl_token_program(),
        borrow_token_program: spl_token_program(),
    };

    Ok(jup_ix::liquidate(
        position.dust_debt_amount,
        liquidator,
        &liquidator_borrow_ata,
        liquidator,
        &liquidator_supply_ata,
        &accounts,
    ))
}

/// Build Save (Solend) liquidation instruction.
fn build_save_liquidate_ix(
    rpc: &RpcClient,
    params: &LiquidationParams,
    liquidator: &Pubkey,
) -> Result<Instruction> {
    let repay_pos = params.positions.borrows.iter()
        .max_by(|a, b| a.market_value_usd.partial_cmp(&b.market_value_usd).unwrap())
        .ok_or_else(|| anyhow::anyhow!("no borrows"))?;
    let withdraw_pos = params.positions.deposits.iter()
        .max_by(|a, b| a.market_value_usd.partial_cmp(&b.market_value_usd).unwrap())
        .ok_or_else(|| anyhow::anyhow!("no deposits"))?;

    let repay_reserve_data = rpc.get_account(&repay_pos.reserve)
        .context("failed to fetch save repay reserve")?;
    let withdraw_reserve_data = rpc.get_account(&withdraw_pos.reserve)
        .context("failed to fetch save withdraw reserve")?;

    let repay_reserve = parse_save_reserve_accounts(&repay_pos.reserve, &repay_reserve_data.data)?;
    let withdraw_reserve = parse_save_reserve_accounts(&withdraw_pos.reserve, &withdraw_reserve_data.data)?;

    let (lending_market_authority, _) =
        save_ix::derive_lending_market_authority(&params.positions.market);

    let wad: u128 = 1_000_000_000_000_000_000;
    let repay_amount = (repay_pos.amount_sf / wad) as u64;
    let repay_amount = repay_amount / 2; // 50% close factor for Save

    let liquidator_repay_ata = spl_associated_token_account(liquidator, &repay_reserve.liquidity_mint);
    let liquidator_collateral_ata = spl_associated_token_account(liquidator, &withdraw_reserve.collateral_mint);
    let liquidator_withdraw_ata = spl_associated_token_account(liquidator, &withdraw_reserve.liquidity_mint);

    Ok(save_ix::liquidate_obligation_and_redeem(
        repay_amount,
        &liquidator_repay_ata,
        &liquidator_collateral_ata,
        &liquidator_withdraw_ata,
        &repay_reserve,
        &withdraw_reserve,
        &params.position_pubkey,
        &params.positions.market,
        &lending_market_authority,
        liquidator,
    ))
}

/// Build MarginFi liquidation instruction.
fn build_marginfi_liquidate_ix(
    rpc: &RpcClient,
    params: &LiquidationParams,
    liquidator: &Pubkey,
) -> Result<Instruction> {
    let asset_pos = params.positions.deposits.iter()
        .max_by(|a, b| a.market_value_usd.partial_cmp(&b.market_value_usd).unwrap())
        .ok_or_else(|| anyhow::anyhow!("no deposits for marginfi liquidation"))?;
    let liab_pos = params.positions.borrows.iter()
        .max_by(|a, b| a.market_value_usd.partial_cmp(&b.market_value_usd).unwrap())
        .ok_or_else(|| anyhow::anyhow!("no borrows for marginfi liquidation"))?;

    let liab_bank_account = rpc.get_account(&liab_pos.reserve)
        .context("failed to fetch marginfi liab bank")?;
    let liab_bank = marginfi_bank::parse_bank(&liab_bank_account.data)?;

    let asset_bank_account = rpc.get_account(&asset_pos.reserve)
        .context("failed to fetch marginfi asset bank")?;

    let (liab_vault_authority, _) = mfi_ix::derive_liquidity_vault_authority(&liab_pos.reserve);

    let marginfi_program: Pubkey = crate::protocols::marginfi::PROGRAM_ID.parse().unwrap();
    let (liquidator_mfi_account, _) = Pubkey::find_program_address(
        &[b"marginfi_account", params.positions.market.as_ref(), liquidator.as_ref()],
        &marginfi_program,
    );

    let remaining = vec![
        AccountMeta::new_readonly(asset_pos.reserve, false),
        AccountMeta::new_readonly(liab_pos.reserve, false),
        AccountMeta::new_readonly(asset_pos.reserve, false),
        AccountMeta::new_readonly(liab_pos.reserve, false),
    ];

    Ok(mfi_ix::lending_account_liquidate(
        asset_pos.amount,
        &params.positions.market,
        &asset_pos.reserve,
        &liab_pos.reserve,
        &liquidator_mfi_account,
        liquidator,
        &params.position_pubkey,
        &liab_vault_authority,
        &liab_bank.liquidity_vault,
        &liab_bank.insurance_vault,
        &remaining,
    ))
}

/// Parse Save reserve accounts from raw data.
fn parse_save_reserve_accounts(
    reserve_pubkey: &Pubkey,
    data: &[u8],
) -> Result<save_ix::SaveReserveAccounts> {
    if data.len() < 300 {
        bail!("save reserve too small: {} bytes", data.len());
    }
    Ok(save_ix::SaveReserveAccounts {
        reserve: *reserve_pubkey,
        liquidity_mint: read_pubkey_at(data, 42),
        liquidity_supply: read_pubkey_at(data, 74),
        liquidity_fee_receiver: read_pubkey_at(data, 106),
        collateral_mint: read_pubkey_at(data, 226),
        collateral_supply: read_pubkey_at(data, 258),
        token_program: spl_token_program(),
    })
}

fn read_pubkey_at(data: &[u8], offset: usize) -> Pubkey {
    Pubkey::new_from_array(data[offset..offset + 32].try_into().unwrap())
}

fn spl_token_program() -> Pubkey {
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".parse().unwrap()
}

fn spl_associated_token_account(wallet: &Pubkey, mint: &Pubkey) -> Pubkey {
    let ata_program: Pubkey = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL".parse().unwrap();
    let (ata, _) = Pubkey::find_program_address(
        &[wallet.as_ref(), spl_token_program().as_ref(), mint.as_ref()],
        &ata_program,
    );
    ata
}
