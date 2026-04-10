//! MarginFi v2 instruction builders.
//!
//! Program: MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA
//!
//! MarginFi flash loans use start/end instructions that set a flag on the
//! MarginfiAccount. Health is only checked at end_flashloan.
//!
//! Liquidation requires the liquidator to have their own MarginfiAccount.

use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use super::marginfi::PROGRAM_ID;

fn marginfi_program() -> Pubkey {
    PROGRAM_ID.parse().unwrap()
}

fn spl_token_program() -> Pubkey {
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".parse().unwrap()
}

/// Discriminator for lending_account_liquidate.
const LIQUIDATE_DISC: [u8; 8] = [0xd6, 0xa9, 0x97, 0xd5, 0xfb, 0xa7, 0x56, 0xdb];

/// Discriminator for lending_account_start_flashloan.
const START_FLASHLOAN_DISC: [u8; 8] = {
    // sha256("global:lending_account_start_flashloan")[..8]
    [0x49, 0xf0, 0xad, 0x60, 0x79, 0xd6, 0x1b, 0x86]
};

/// Discriminator for lending_account_end_flashloan.
const END_FLASHLOAN_DISC: [u8; 8] = {
    // sha256("global:lending_account_end_flashloan")[..8]
    [0x36, 0xd0, 0x72, 0xef, 0xc0, 0x0a, 0x28, 0x0b]
};

/// Build `lending_account_start_flashloan` instruction.
///
/// Data: [disc(8)] [end_index: u64 LE] — index of end_flashloan ix in the tx
pub fn start_flashloan(
    end_index: u64,
    marginfi_account: &Pubkey,
    signer: &Pubkey,
) -> Instruction {
    let mut data = Vec::with_capacity(16);
    data.extend_from_slice(&START_FLASHLOAN_DISC);
    data.extend_from_slice(&end_index.to_le_bytes());

    let accounts = vec![
        AccountMeta::new(*marginfi_account, false),
        AccountMeta::new_readonly(*signer, true),
    ];

    Instruction {
        program_id: marginfi_program(),
        accounts,
        data,
    }
}

/// Build `lending_account_end_flashloan` instruction.
///
/// Data: [disc(8)]
/// Remaining accounts: observation Bank accounts for health check
pub fn end_flashloan(
    marginfi_account: &Pubkey,
    signer: &Pubkey,
    observation_banks: &[Pubkey],
) -> Instruction {
    let mut accounts = vec![
        AccountMeta::new(*marginfi_account, false),
        AccountMeta::new_readonly(*signer, true),
    ];

    // Remaining accounts: Bank accounts for health observation
    for bank in observation_banks {
        accounts.push(AccountMeta::new_readonly(*bank, false));
    }

    Instruction {
        program_id: marginfi_program(),
        accounts,
        data: END_FLASHLOAN_DISC.to_vec(),
    }
}

/// Build `lending_account_liquidate` instruction.
///
/// Data: [disc(8)] [asset_amount: u64 LE]
///
/// The liquidator must have a MarginfiAccount with the same group.
pub fn lending_account_liquidate(
    asset_amount: u64,
    group: &Pubkey,
    asset_bank: &Pubkey,
    liab_bank: &Pubkey,
    liquidator_marginfi_account: &Pubkey,
    signer: &Pubkey,
    liquidatee_marginfi_account: &Pubkey,
    liab_bank_liquidity_vault_authority: &Pubkey,
    liab_bank_liquidity_vault: &Pubkey,
    liab_bank_insurance_vault: &Pubkey,
    remaining_accounts: &[AccountMeta],
) -> Instruction {
    let mut data = Vec::with_capacity(16);
    data.extend_from_slice(&LIQUIDATE_DISC);
    data.extend_from_slice(&asset_amount.to_le_bytes());

    let mut accounts = vec![
        AccountMeta::new_readonly(*group, false),                        // 0: group
        AccountMeta::new(*asset_bank, false),                            // 1: asset_bank
        AccountMeta::new(*liab_bank, false),                             // 2: liab_bank
        AccountMeta::new(*liquidator_marginfi_account, false),           // 3: liquidator_marginfi_account
        AccountMeta::new_readonly(*signer, true),                        // 4: authority (signer)
        AccountMeta::new(*liquidatee_marginfi_account, false),           // 5: liquidatee
        AccountMeta::new_readonly(*liab_bank_liquidity_vault_authority, false), // 6: vault auth PDA
        AccountMeta::new(*liab_bank_liquidity_vault, false),             // 7: liquidity vault
        AccountMeta::new(*liab_bank_insurance_vault, false),             // 8: insurance vault
        AccountMeta::new_readonly(spl_token_program(), false),           // 9: token_program
    ];

    // Remaining accounts: [liab_mint?, asset_oracle, liab_oracle, liquidator_obs_banks..., liquidatee_obs_banks...]
    accounts.extend_from_slice(remaining_accounts);

    Instruction {
        program_id: marginfi_program(),
        accounts,
        data,
    }
}

/// Derive the liquidity vault authority PDA for a Bank.
/// Seeds: ["liquidity_vault_auth", bank_pubkey]
pub fn derive_liquidity_vault_authority(bank: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"liquidity_vault_auth", bank.as_ref()],
        &marginfi_program(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liquidate_instruction_layout() {
        let group = Pubkey::new_unique();
        let asset_bank = Pubkey::new_unique();
        let liab_bank = Pubkey::new_unique();
        let liquidator_acc = Pubkey::new_unique();
        let signer = Pubkey::new_unique();
        let liquidatee = Pubkey::new_unique();
        let vault_auth = Pubkey::new_unique();
        let vault = Pubkey::new_unique();
        let insurance = Pubkey::new_unique();

        let ix = lending_account_liquidate(
            1_000_000,
            &group, &asset_bank, &liab_bank,
            &liquidator_acc, &signer, &liquidatee,
            &vault_auth, &vault, &insurance,
            &[], // no remaining accounts
        );

        assert_eq!(ix.data.len(), 16); // 8 disc + 8 amount
        assert_eq!(ix.accounts.len(), 10);
        assert!(ix.accounts[4].is_signer); // authority
        assert_eq!(&ix.data[..8], &LIQUIDATE_DISC);
    }

    #[test]
    fn flashloan_instruction_layout() {
        let account = Pubkey::new_unique();
        let signer = Pubkey::new_unique();

        let start = start_flashloan(3, &account, &signer);
        assert_eq!(start.data.len(), 16); // 8 disc + 8 end_index
        assert_eq!(start.accounts.len(), 2);

        let end = end_flashloan(&account, &signer, &[Pubkey::new_unique()]);
        assert_eq!(end.data.len(), 8); // just disc
        assert_eq!(end.accounts.len(), 3); // account + signer + 1 observation bank
    }
}
