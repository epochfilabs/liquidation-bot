//! Jupiter Lend instruction builders for flash-loan liquidation.
//!
//! - Flash loan program: `jupgfSgfuAXv4B6R2Uxu85Z1qdzgju79s6MfZekN6XS` (0% fee)
//! - Vaults program:     `jupr81YtYssSyPt8jbnGuiWon5f6x9TcDEFxYe3Bdzi`

use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    sysvar,
};

use super::{FLASH_LOAN_PROGRAM_ID, VAULTS_PROGRAM_ID};

pub const ATA_PROGRAM: Pubkey = solana_sdk::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
pub const SPL_TOKEN_PROGRAM: Pubkey =
    solana_sdk::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

const FLASH_BORROW_DISC: [u8; 8] = [103, 19, 78, 24, 240, 9, 135, 63];
const FLASH_PAYBACK_DISC: [u8; 8] = [213, 47, 153, 137, 84, 243, 94, 232];
const LIQUIDATE_DISC: [u8; 8] = [223, 179, 226, 125, 48, 46, 39, 74];

/// Accounts needed for Jupiter Lend flash-loan operations.
#[derive(Debug, Clone)]
pub struct JupiterFlashLoanAccounts {
    pub flashloan_admin: Pubkey,
    pub mint: Pubkey,
    pub flashloan_token_reserves_liquidity: Pubkey,
    pub flashloan_borrow_position_on_liquidity: Pubkey,
    pub rate_model: Pubkey,
    pub vault: Pubkey,
    pub liquidity: Pubkey,
    pub liquidity_program: Pubkey,
}

/// `flashloan_borrow`. Data: `[disc(8)] [amount: u64 LE]`.
pub fn flash_borrow(
    amount: u64,
    signer: &Pubkey,
    signer_borrow_token_account: &Pubkey,
    flash_accounts: &JupiterFlashLoanAccounts,
) -> Instruction {
    let mut data = Vec::with_capacity(16);
    data.extend_from_slice(&FLASH_BORROW_DISC);
    data.extend_from_slice(&amount.to_le_bytes());

    let accounts = vec![
        AccountMeta::new(*signer, true),
        AccountMeta::new(flash_accounts.flashloan_admin, false),
        AccountMeta::new(*signer_borrow_token_account, false),
        AccountMeta::new_readonly(flash_accounts.mint, false),
        AccountMeta::new(flash_accounts.flashloan_token_reserves_liquidity, false),
        AccountMeta::new(flash_accounts.flashloan_borrow_position_on_liquidity, false),
        AccountMeta::new_readonly(flash_accounts.rate_model, false),
        AccountMeta::new(flash_accounts.vault, false),
        AccountMeta::new_readonly(flash_accounts.liquidity, false),
        AccountMeta::new_readonly(flash_accounts.liquidity_program, false),
        AccountMeta::new_readonly(SPL_TOKEN_PROGRAM, false),
        AccountMeta::new_readonly(ATA_PROGRAM, false),
        AccountMeta::new_readonly(solana_sdk::system_program::ID, false),
        AccountMeta::new_readonly(sysvar::instructions::ID, false),
    ];

    Instruction {
        program_id: FLASH_LOAN_PROGRAM_ID,
        accounts,
        data,
    }
}

/// `flashloan_payback`. Data: `[disc(8)] [amount: u64 LE]`.
pub fn flash_payback(
    amount: u64,
    signer: &Pubkey,
    signer_borrow_token_account: &Pubkey,
    flash_accounts: &JupiterFlashLoanAccounts,
) -> Instruction {
    let mut data = Vec::with_capacity(16);
    data.extend_from_slice(&FLASH_PAYBACK_DISC);
    data.extend_from_slice(&amount.to_le_bytes());

    let accounts = vec![
        AccountMeta::new(*signer, true),
        AccountMeta::new(flash_accounts.flashloan_admin, false),
        AccountMeta::new(*signer_borrow_token_account, false),
        AccountMeta::new_readonly(flash_accounts.mint, false),
        AccountMeta::new(flash_accounts.flashloan_token_reserves_liquidity, false),
        AccountMeta::new(flash_accounts.flashloan_borrow_position_on_liquidity, false),
        AccountMeta::new_readonly(flash_accounts.rate_model, false),
        AccountMeta::new(flash_accounts.vault, false),
        AccountMeta::new_readonly(flash_accounts.liquidity, false),
        AccountMeta::new_readonly(flash_accounts.liquidity_program, false),
        AccountMeta::new_readonly(SPL_TOKEN_PROGRAM, false),
        AccountMeta::new_readonly(ATA_PROGRAM, false),
        AccountMeta::new_readonly(solana_sdk::system_program::ID, false),
        AccountMeta::new_readonly(sysvar::instructions::ID, false),
    ];

    Instruction {
        program_id: FLASH_LOAN_PROGRAM_ID,
        accounts,
        data,
    }
}

/// Accounts needed for Jupiter Lend liquidation.
#[derive(Debug, Clone)]
pub struct JupiterLiquidateAccounts {
    pub vault_config: Pubkey,
    pub vault_state: Pubkey,
    pub supply_token: Pubkey,
    pub borrow_token: Pubkey,
    pub oracle: Pubkey,
    pub oracle_program: Pubkey,
    pub new_branch: Pubkey,
    pub supply_token_reserves_liquidity: Pubkey,
    pub borrow_token_reserves_liquidity: Pubkey,
    pub vault_supply_position_on_liquidity: Pubkey,
    pub vault_borrow_position_on_liquidity: Pubkey,
    pub supply_rate_model: Pubkey,
    pub borrow_rate_model: Pubkey,
    pub liquidity: Pubkey,
    pub liquidity_program: Pubkey,
    pub vault_supply_token_account: Pubkey,
    pub vault_borrow_token_account: Pubkey,
    pub supply_token_program: Pubkey,
    pub borrow_token_program: Pubkey,
}

/// `liquidate`. Data: `[disc(8)] [debt_amt:u64] [col_per_unit_debt:u128] [absorb:u8] [transfer_type: Option<u8>]`.
pub fn liquidate(
    debt_amount: u64,
    signer: &Pubkey,
    signer_borrow_token_account: &Pubkey,
    to: &Pubkey,
    to_supply_token_account: &Pubkey,
    accounts: &JupiterLiquidateAccounts,
) -> Instruction {
    let mut data = Vec::with_capacity(36);
    data.extend_from_slice(&LIQUIDATE_DISC);
    data.extend_from_slice(&debt_amount.to_le_bytes());
    data.extend_from_slice(&0u128.to_le_bytes()); // col_per_unit_debt: 0 = no slippage
    data.push(0); // absorb: false
    data.push(1); // transfer_type = Some
    data.push(1); // TransferType::DIRECT

    let account_metas = vec![
        AccountMeta::new(*signer, true),
        AccountMeta::new(*signer_borrow_token_account, false),
        AccountMeta::new_readonly(*to, false),
        AccountMeta::new(*to_supply_token_account, false),
        AccountMeta::new_readonly(accounts.vault_config, false),
        AccountMeta::new(accounts.vault_state, false),
        AccountMeta::new_readonly(accounts.supply_token, false),
        AccountMeta::new_readonly(accounts.borrow_token, false),
        AccountMeta::new_readonly(accounts.oracle, false),
        AccountMeta::new(accounts.new_branch, false),
        AccountMeta::new(accounts.supply_token_reserves_liquidity, false),
        AccountMeta::new(accounts.borrow_token_reserves_liquidity, false),
        AccountMeta::new(accounts.vault_supply_position_on_liquidity, false),
        AccountMeta::new(accounts.vault_borrow_position_on_liquidity, false),
        AccountMeta::new_readonly(accounts.supply_rate_model, false),
        AccountMeta::new_readonly(accounts.borrow_rate_model, false),
        AccountMeta::new(VAULTS_PROGRAM_ID, false), // supply_token_claim_account (unused → program)
        AccountMeta::new_readonly(accounts.liquidity, false),
        AccountMeta::new_readonly(accounts.liquidity_program, false),
        AccountMeta::new(accounts.vault_supply_token_account, false),
        AccountMeta::new(accounts.vault_borrow_token_account, false),
        AccountMeta::new_readonly(accounts.supply_token_program, false),
        AccountMeta::new_readonly(accounts.borrow_token_program, false),
        AccountMeta::new_readonly(solana_sdk::system_program::ID, false),
        AccountMeta::new_readonly(ATA_PROGRAM, false),
        AccountMeta::new_readonly(accounts.oracle_program, false),
    ];

    Instruction {
        program_id: VAULTS_PROGRAM_ID,
        accounts: account_metas,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash_borrow_instruction_layout() {
        let signer = Pubkey::new_unique();
        let ata = Pubkey::new_unique();
        let flash_accounts = JupiterFlashLoanAccounts {
            flashloan_admin: Pubkey::new_unique(),
            mint: Pubkey::new_unique(),
            flashloan_token_reserves_liquidity: Pubkey::new_unique(),
            flashloan_borrow_position_on_liquidity: Pubkey::new_unique(),
            rate_model: Pubkey::new_unique(),
            vault: Pubkey::new_unique(),
            liquidity: Pubkey::new_unique(),
            liquidity_program: Pubkey::new_unique(),
        };

        let ix = flash_borrow(1_000_000, &signer, &ata, &flash_accounts);
        assert_eq!(ix.data.len(), 16);
        assert_eq!(ix.accounts.len(), 14);
        assert!(ix.accounts[0].is_signer);
        assert_eq!(&ix.data[..8], &FLASH_BORROW_DISC);
    }

    #[test]
    fn liquidate_instruction_layout() {
        let signer = Pubkey::new_unique();
        let signer_ata = Pubkey::new_unique();
        let to = Pubkey::new_unique();
        let to_ata = Pubkey::new_unique();
        let liq_accounts = JupiterLiquidateAccounts {
            vault_config: Pubkey::new_unique(),
            vault_state: Pubkey::new_unique(),
            supply_token: Pubkey::new_unique(),
            borrow_token: Pubkey::new_unique(),
            oracle: Pubkey::new_unique(),
            oracle_program: Pubkey::new_unique(),
            new_branch: Pubkey::new_unique(),
            supply_token_reserves_liquidity: Pubkey::new_unique(),
            borrow_token_reserves_liquidity: Pubkey::new_unique(),
            vault_supply_position_on_liquidity: Pubkey::new_unique(),
            vault_borrow_position_on_liquidity: Pubkey::new_unique(),
            supply_rate_model: Pubkey::new_unique(),
            borrow_rate_model: Pubkey::new_unique(),
            liquidity: Pubkey::new_unique(),
            liquidity_program: Pubkey::new_unique(),
            vault_supply_token_account: Pubkey::new_unique(),
            vault_borrow_token_account: Pubkey::new_unique(),
            supply_token_program: Pubkey::new_unique(),
            borrow_token_program: Pubkey::new_unique(),
        };

        let ix = liquidate(500_000, &signer, &signer_ata, &to, &to_ata, &liq_accounts);
        assert_eq!(ix.data.len(), 35);
        assert_eq!(ix.accounts.len(), 26);
        assert!(ix.accounts[0].is_signer);
    }
}
