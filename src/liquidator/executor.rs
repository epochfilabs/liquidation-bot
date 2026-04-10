//! Unified cross-protocol liquidation executor.
//!
//! Strategy: use Kamino flash loans as the liquidity source (deepest on Solana),
//! then liquidate on any protocol (Kamino, Jupiter Lend, Save, MarginFi).
//!
//! Transaction layout:
//!   [ATA creation ixs (if needed)]
//!   Kamino flash_borrow_reserve_liquidity(repay_amount)
//!   Protocol-specific liquidate instruction
//!   Kamino flash_repay_reserve_liquidity(repay_amount)

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

/// Execute a cross-protocol liquidation:
/// Kamino flash loan → protocol-specific liquidate → Kamino flash repay.
pub async fn execute_cross_protocol(
    config: &AppConfig,
    params: &LiquidationParams,
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

    let repay_pos = params.positions.borrows.iter()
        .max_by(|a, b| a.market_value_usd.partial_cmp(&b.market_value_usd).unwrap())
        .unwrap();
    let withdraw_pos = params.positions.deposits.iter()
        .max_by(|a, b| a.market_value_usd.partial_cmp(&b.market_value_usd).unwrap())
        .unwrap();

    // For Kamino-native liquidations, use the existing executor path
    if params.protocol == ProtocolKind::Kamino {
        return execute_kamino_native(config, params, supabase).await;
    }

    // For cross-protocol: we need a Kamino reserve that holds the same token
    // as the target protocol's debt token. This requires finding a matching
    // Kamino reserve by mint.
    //
    // Build the protocol-specific liquidation instruction
    let liquidate_ix = match params.protocol {
        ProtocolKind::JupiterLend => {
            build_jupiter_liquidate_ix(&rpc, params, &liquidator_pubkey)?
        }
        ProtocolKind::Save => {
            build_save_liquidate_ix(&rpc, params, &liquidator_pubkey)?
        }
        ProtocolKind::MarginFi => {
            build_marginfi_liquidate_ix(&rpc, params, &liquidator_pubkey)?
        }
        ProtocolKind::Kamino => unreachable!(),
    };

    // Log to DB
    let record_id = db::new_record_id();
    let db_record = NewLiquidationRecord {
        id: record_id.clone(),
        obligation_pubkey: params.position_pubkey.to_string(),
        obligation_owner: params.positions.owner.to_string(),
        lending_market: params.positions.market.to_string(),
        repay_reserve: repay_pos.reserve.to_string(),
        repay_mint: repay_pos.mint.map_or("unknown".into(), |m| m.to_string()),
        withdraw_reserve: withdraw_pos.reserve.to_string(),
        withdraw_mint: withdraw_pos.mint.map_or("unknown".into(), |m| m.to_string()),
        ltv_at_detection: params.health.current_ltv,
        unhealthy_ltv: params.health.unhealthy_ltv,
        repay_amount: 0, // filled per-protocol
        liquidation_bonus_bps: 0,
        flash_loan_fee_fraction: 0.0,
        estimated_gross_profit_usd: 0.0,
        estimated_net_profit_usd: 0.0,
        status: LiquidationStatus::Pending.to_string(),
        error_message: None,
    };

    if let Some(sb) = supabase {
        let _ = sb.insert_liquidation(&db_record).await;
    }

    // TODO: Find matching Kamino reserve for flash loan, build full tx:
    //   [ATAs] + kamino_flash_borrow + liquidate_ix + kamino_flash_repay
    // For now, submit the liquidation instruction standalone (requires
    // the liquidator to already hold the repay token).

    tracing::info!(
        protocol = %params.protocol,
        position = %params.position_pubkey,
        "cross-protocol liquidation built — submitting"
    );

    let recent_blockhash = rpc.get_latest_blockhash()?;
    let tx = Transaction::new_signed_with_payer(
        &[liquidate_ix],
        Some(&liquidator_pubkey),
        &[&liquidator_keypair],
        recent_blockhash,
    );

    // Update DB: submitted
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
                "cross-protocol liquidation confirmed"
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
            tracing::error!(protocol = %params.protocol, error = %e, "liquidation tx failed");
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

/// Kamino-native liquidation (flash borrow + liquidate + flash repay, all on klend).
async fn execute_kamino_native(
    config: &AppConfig,
    params: &LiquidationParams,
    supabase: Option<&SupabaseClient>,
) -> Result<()> {
    // Delegate to existing Kamino executor
    let kamino_health = crate::obligation::health::evaluate(
        // We need the raw data — for now re-fetch
        &RpcClient::new_with_commitment(config.rpc_url.clone(), CommitmentConfig::confirmed())
            .get_account(&params.position_pubkey)
            .context("failed to re-fetch kamino obligation")?
            .data,
        config,
    )?;
    crate::liquidator::execute_liquidation(config, &params.position_pubkey, &kamino_health, supabase).await
}

/// Build Jupiter Lend liquidation instruction.
fn build_jupiter_liquidate_ix(
    rpc: &RpcClient,
    params: &LiquidationParams,
    liquidator: &Pubkey,
) -> Result<Instruction> {
    // Parse the position to get vault_id
    let pos_account = rpc.get_account(&params.position_pubkey)
        .context("failed to fetch jupiter position")?;
    let position = jupiter_lend::parse_position(&pos_account.data)?;

    // Derive and fetch VaultConfig
    let vaults_program: Pubkey = jupiter_lend::VAULTS_PROGRAM_ID.parse().unwrap();
    let (vault_config_pda, _) = Pubkey::find_program_address(
        &[b"vault_config", &position.vault_id.to_le_bytes()],
        &vaults_program,
    );
    let vault_config_account = rpc.get_account(&vault_config_pda)
        .context("failed to fetch jupiter vault config")?;
    let vault_config = jupiter_lend::parse_vault_config(&vault_config_account.data)?;

    // Derive VaultState
    let (vault_state_pda, _) = Pubkey::find_program_address(
        &[b"vault_state", &position.vault_id.to_le_bytes()],
        &vaults_program,
    );

    // Derive other PDAs needed for the liquidate instruction
    let lending_program: Pubkey = jupiter_lend::LENDING_PROGRAM_ID.parse().unwrap();

    // Liquidity program account
    let (liquidity_pda, _) = Pubkey::find_program_address(
        &[b"liquidity"],
        &lending_program,
    );

    // For the remaining accounts (reserves, positions on liquidity, rate models, etc.)
    // we need to derive them from the vault's supply/borrow tokens.
    // These are PDAs on the lending program.
    let (supply_reserves, _) = Pubkey::find_program_address(
        &[b"token_reserves", vault_config.supply_token.as_ref()],
        &lending_program,
    );
    let (borrow_reserves, _) = Pubkey::find_program_address(
        &[b"token_reserves", vault_config.borrow_token.as_ref()],
        &lending_program,
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
        &[b"rate_model", vault_config.supply_token.as_ref()],
        &lending_program,
    );
    let (borrow_rate_model, _) = Pubkey::find_program_address(
        &[b"rate_model", vault_config.borrow_token.as_ref()],
        &lending_program,
    );
    // Branch account for liquidation
    let (new_branch, _) = Pubkey::find_program_address(
        &[b"branch", &position.vault_id.to_le_bytes(), &0u32.to_le_bytes()],
        &vaults_program,
    );

    // Vault token accounts (ATAs of the vault for supply/borrow tokens)
    let vault_supply_ata = spl_associated_token_account(
        &vault_config_pda,
        &vault_config.supply_token,
    );
    let vault_borrow_ata = spl_associated_token_account(
        &vault_config_pda,
        &vault_config.borrow_token,
    );

    // Liquidator's ATAs
    let liquidator_borrow_ata = spl_associated_token_account(liquidator, &vault_config.borrow_token);
    let liquidator_supply_ata = spl_associated_token_account(liquidator, &vault_config.supply_token);

    // Oracle from vault config
    let oracle = read_pubkey_at(&vault_config_account.data, 26); // offset 26 in VaultConfig

    // Oracle program (typically Pyth or Switchboard)
    let oracle_program = read_pubkey_at(&vault_config_account.data, 122);

    let debt_amount = position.dust_debt_amount;

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
        debt_amount,
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

    // Fetch reserve accounts to get liquidity supply, fee receiver, collateral mint/supply
    let repay_reserve_data = rpc.get_account(&repay_pos.reserve)
        .context("failed to fetch save repay reserve")?;
    let withdraw_reserve_data = rpc.get_account(&withdraw_pos.reserve)
        .context("failed to fetch save withdraw reserve")?;

    let repay_reserve = parse_save_reserve_accounts(&repay_pos.reserve, &repay_reserve_data.data)?;
    let withdraw_reserve = parse_save_reserve_accounts(&withdraw_pos.reserve, &withdraw_reserve_data.data)?;

    let (lending_market_authority, _) =
        save_ix::derive_lending_market_authority(&params.positions.market);

    // Calculate repay amount from borrowed_amount_wads
    let wad: u128 = 1_000_000_000_000_000_000;
    let repay_amount = (repay_pos.amount_sf / wad) as u64;
    let repay_amount = repay_amount.min(repay_amount / 2); // max 50% close factor for Save

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
///
/// MarginFi requires the liquidator to have a MarginfiAccount in the same group.
/// The liquidator_marginfi_account must be created beforehand (one-time setup).
fn build_marginfi_liquidate_ix(
    rpc: &RpcClient,
    params: &LiquidationParams,
    liquidator: &Pubkey,
) -> Result<Instruction> {
    // Find the highest-value asset (collateral) and liability (debt) balances
    let asset_pos = params.positions.deposits.iter()
        .max_by(|a, b| a.market_value_usd.partial_cmp(&b.market_value_usd).unwrap())
        .ok_or_else(|| anyhow::anyhow!("no deposits for marginfi liquidation"))?;
    let liab_pos = params.positions.borrows.iter()
        .max_by(|a, b| a.market_value_usd.partial_cmp(&b.market_value_usd).unwrap())
        .ok_or_else(|| anyhow::anyhow!("no borrows for marginfi liquidation"))?;

    let asset_bank_pubkey = asset_pos.reserve;
    let liab_bank_pubkey = liab_pos.reserve;

    // Fetch Bank accounts for vault addresses
    let liab_bank_account = rpc.get_account(&liab_bank_pubkey)
        .context("failed to fetch marginfi liab bank")?;
    let liab_bank = marginfi_bank::parse_bank(&liab_bank_account.data)?;

    let asset_bank_account = rpc.get_account(&asset_bank_pubkey)
        .context("failed to fetch marginfi asset bank")?;
    let asset_bank = marginfi_bank::parse_bank(&asset_bank_account.data)?;

    // Derive liquidity vault authority PDA
    let (liab_vault_authority, _) = mfi_ix::derive_liquidity_vault_authority(&liab_bank_pubkey);

    // The liquidator needs a MarginfiAccount — derive it from a PDA or
    // use a pre-created one. For now, derive a deterministic PDA.
    let marginfi_program: Pubkey = crate::protocols::marginfi::PROGRAM_ID.parse().unwrap();
    let (liquidator_mfi_account, _) = Pubkey::find_program_address(
        &[b"marginfi_account", params.positions.market.as_ref(), liquidator.as_ref()],
        &marginfi_program,
    );

    // Calculate liquidation amount: take up to the full asset balance
    // MarginFi liquidation is specified in collateral asset amount
    let asset_amount = asset_pos.amount;

    // Remaining accounts: asset_oracle, liab_oracle, then observation banks
    // for both liquidator and liquidatee
    // For simplicity, we include the asset and liab banks as observation accounts
    let remaining = vec![
        // asset oracle (from bank config — would need to fetch, use placeholder)
        // For now we skip oracle accounts — they'll need to be resolved from Bank.config
        AccountMeta::new_readonly(asset_bank_pubkey, false), // liquidator obs: asset bank
        AccountMeta::new_readonly(liab_bank_pubkey, false),  // liquidator obs: liab bank
        AccountMeta::new_readonly(asset_bank_pubkey, false), // liquidatee obs: asset bank
        AccountMeta::new_readonly(liab_bank_pubkey, false),  // liquidatee obs: liab bank
    ];

    Ok(mfi_ix::lending_account_liquidate(
        asset_amount,
        &params.positions.market, // group
        &asset_bank_pubkey,
        &liab_bank_pubkey,
        &liquidator_mfi_account,
        liquidator,
        &params.position_pubkey, // liquidatee marginfi account
        &liab_vault_authority,
        &liab_bank.liquidity_vault,
        &liab_bank.insurance_vault,
        &remaining,
    ))
}

/// Parse Save reserve accounts from raw data.
/// Save reserves use SPL token-lending layout (no Anchor discriminator).
fn parse_save_reserve_accounts(
    reserve_pubkey: &Pubkey,
    data: &[u8],
) -> Result<save_ix::SaveReserveAccounts> {
    // Save Reserve layout (SPL token-lending):
    //   0:    version (1)
    //   1:    last_update (9)
    //   10:   lending_market (32)
    //   42:   liquidity.mint_pubkey (32)
    //   74:   liquidity.supply_pubkey (32)  — token account holding the liquidity
    //   106:  liquidity.fee_receiver (32)
    //   138:  ... more liquidity fields
    //   Then collateral fields
    //   The exact layout depends on the version, but key fields are stable.
    if data.len() < 300 {
        bail!("save reserve too small: {} bytes", data.len());
    }

    let liquidity_mint = read_pubkey_at(data, 42);
    let liquidity_supply = read_pubkey_at(data, 74);
    let fee_receiver = read_pubkey_at(data, 106);

    // Collateral fields (after liquidity section, ~offset 226 for SPL token-lending v1)
    // This varies by version — collateral.mint is at a known offset
    // For Solend v2, collateral section starts around offset 226
    let collateral_mint = read_pubkey_at(data, 226);
    let collateral_supply = read_pubkey_at(data, 258);

    Ok(save_ix::SaveReserveAccounts {
        reserve: *reserve_pubkey,
        liquidity_mint,
        liquidity_supply,
        liquidity_fee_receiver: fee_receiver,
        collateral_mint,
        collateral_supply,
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
