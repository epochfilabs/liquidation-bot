//! Jupiter Lend instruction builders for flash loan liquidation.
//!
//! Flash Loan Program: jupgfSgfuAXv4B6R2Uxu85Z1qdzgju79s6MfZekN6XS (zero fees)
//! Vaults Program:     jupr81YtYssSyPt8jbnGuiWon5f6x9TcDEFxYe3Bdzi

use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    sysvar,
};

use super::jupiter_lend::{FLASH_LOAN_PROGRAM_ID, VAULTS_PROGRAM_ID, LENDING_PROGRAM_ID};

/// Flash loan borrow discriminator.
const FLASH_BORROW_DISC: [u8; 8] = [103, 19, 78, 24, 240, 9, 135, 63];

/// Flash loan payback discriminator.
const FLASH_PAYBACK_DISC: [u8; 8] = [213, 47, 153, 137, 84, 243, 94, 232];

/// Liquidate instruction discriminator.
const LIQUIDATE_DISC: [u8; 8] = [223, 179, 226, 125, 48, 46, 39, 74];

/// ATA program.
fn ata_program() -> Pubkey {
    "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL".parse().unwrap()
}

fn system_program() -> Pubkey {
    solana_sdk::system_program::ID
}

fn spl_token_program() -> Pubkey {
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".parse().unwrap()
}

/// Accounts needed for Jupiter Lend flash loan operations.
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

/// Build `flashloan_borrow` instruction.
///
/// Data: [disc(8)] [amount(u64 LE)]
pub fn flash_borrow(
    amount: u64,
    signer: &Pubkey,
    signer_borrow_token_account: &Pubkey,
    flash_accounts: &JupiterFlashLoanAccounts,
) -> Instruction {
    let flash_program: Pubkey = FLASH_LOAN_PROGRAM_ID.parse().unwrap();

    let mut data = Vec::with_capacity(16);
    data.extend_from_slice(&FLASH_BORROW_DISC);
    data.extend_from_slice(&amount.to_le_bytes());

    let accounts = vec![
        AccountMeta::new(*signer, true),                                          // signer
        AccountMeta::new(flash_accounts.flashloan_admin, false),                  // flashloan_admin
        AccountMeta::new(*signer_borrow_token_account, false),                    // signer_borrow_token_account
        AccountMeta::new_readonly(flash_accounts.mint, false),                    // mint
        AccountMeta::new(flash_accounts.flashloan_token_reserves_liquidity, false), // reserves
        AccountMeta::new(flash_accounts.flashloan_borrow_position_on_liquidity, false), // borrow pos
        AccountMeta::new_readonly(flash_accounts.rate_model, false),              // rate_model
        AccountMeta::new(flash_accounts.vault, false),                            // vault
        AccountMeta::new_readonly(flash_accounts.liquidity, false),               // liquidity
        AccountMeta::new_readonly(flash_accounts.liquidity_program, false),       // liquidity_program
        AccountMeta::new_readonly(spl_token_program(), false),                    // token_program
        AccountMeta::new_readonly(ata_program(), false),                          // ata_program
        AccountMeta::new_readonly(system_program(), false),                       // system_program
        AccountMeta::new_readonly(sysvar::instructions::ID, false),               // instruction_sysvar
    ];

    Instruction {
        program_id: flash_program,
        accounts,
        data,
    }
}

/// Build `flashloan_payback` instruction.
///
/// Data: [disc(8)] [amount(u64 LE)]
pub fn flash_payback(
    amount: u64,
    signer: &Pubkey,
    signer_borrow_token_account: &Pubkey,
    flash_accounts: &JupiterFlashLoanAccounts,
) -> Instruction {
    let flash_program: Pubkey = FLASH_LOAN_PROGRAM_ID.parse().unwrap();

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
        AccountMeta::new_readonly(spl_token_program(), false),
        AccountMeta::new_readonly(ata_program(), false),
        AccountMeta::new_readonly(system_program(), false),
        AccountMeta::new_readonly(sysvar::instructions::ID, false),
    ];

    Instruction {
        program_id: flash_program,
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

/// Build `liquidate` instruction.
///
/// Data: [disc(8)] [debt_amt(u64)] [col_per_unit_debt(u128)] [absorb(u8)] [transfer_type option]
pub fn liquidate(
    debt_amount: u64,
    signer: &Pubkey,
    signer_borrow_token_account: &Pubkey,
    to: &Pubkey,
    to_supply_token_account: &Pubkey,
    accounts: &JupiterLiquidateAccounts,
) -> Instruction {
    let vaults_program: Pubkey = VAULTS_PROGRAM_ID.parse().unwrap();

    let mut data = Vec::with_capacity(36);
    data.extend_from_slice(&LIQUIDATE_DISC);
    data.extend_from_slice(&debt_amount.to_le_bytes());      // debt_amt: u64
    data.extend_from_slice(&0u128.to_le_bytes());             // col_per_unit_debt: 0 = no slippage protection
    data.push(0);                                              // absorb: false
    // transfer_type: Some(DIRECT) = Option tag (1) + enum variant (1)
    data.push(1); // Some
    data.push(1); // TransferType::DIRECT

    let account_metas = vec![
        AccountMeta::new(*signer, true),                                          // signer
        AccountMeta::new(*signer_borrow_token_account, false),                    // signer_token_account (borrow token)
        AccountMeta::new_readonly(*to, false),                                    // to (collateral recipient)
        AccountMeta::new(*to_supply_token_account, false),                        // to_token_account (supply token)
        AccountMeta::new_readonly(accounts.vault_config, false),                  // vault_config
        AccountMeta::new(accounts.vault_state, false),                            // vault_state
        AccountMeta::new_readonly(accounts.supply_token, false),                  // supply_token
        AccountMeta::new_readonly(accounts.borrow_token, false),                  // borrow_token
        AccountMeta::new_readonly(accounts.oracle, false),                        // oracle
        AccountMeta::new(accounts.new_branch, false),                             // new_branch
        AccountMeta::new(accounts.supply_token_reserves_liquidity, false),        // supply reserves
        AccountMeta::new(accounts.borrow_token_reserves_liquidity, false),        // borrow reserves
        AccountMeta::new(accounts.vault_supply_position_on_liquidity, false),     // vault supply pos
        AccountMeta::new(accounts.vault_borrow_position_on_liquidity, false),     // vault borrow pos
        AccountMeta::new_readonly(accounts.supply_rate_model, false),             // supply rate model
        AccountMeta::new_readonly(accounts.borrow_rate_model, false),             // borrow rate model
        AccountMeta::new(vaults_program, false),                                  // supply_token_claim_account (unused → program)
        AccountMeta::new_readonly(accounts.liquidity, false),                     // liquidity
        AccountMeta::new_readonly(accounts.liquidity_program, false),             // liquidity program
        AccountMeta::new(accounts.vault_supply_token_account, false),             // vault supply ATA
        AccountMeta::new(accounts.vault_borrow_token_account, false),             // vault borrow ATA
        AccountMeta::new_readonly(accounts.supply_token_program, false),          // supply token program
        AccountMeta::new_readonly(accounts.borrow_token_program, false),          // borrow token program
        AccountMeta::new_readonly(system_program(), false),                       // system program
        AccountMeta::new_readonly(ata_program(), false),                          // ata program
        AccountMeta::new_readonly(accounts.oracle_program, false),                // oracle program
    ];

    Instruction {
        program_id: vaults_program,
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
        assert_eq!(ix.data.len(), 16); // 8 disc + 8 amount
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
        // 8 disc + 8 debt_amt + 16 col_per_unit + 1 absorb + 2 transfer_type = 35
        assert_eq!(ix.data.len(), 35);
        assert_eq!(ix.accounts.len(), 26);
        assert!(ix.accounts[0].is_signer);
    }
}
