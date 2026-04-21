//! MarginFi v2 instruction builders.
//!
//! MarginFi flash loans use start/end instructions that set a flag on the
//! `MarginfiAccount` — health is only checked at `end_flashloan`. Liquidation
//! requires the liquidator to hold their own `MarginfiAccount`.

use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use super::PROGRAM_ID;

pub const SPL_TOKEN_PROGRAM: Pubkey =
    solana_sdk::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

/// `lending_account_liquidate` discriminator.
const LIQUIDATE_DISC: [u8; 8] = [0xd6, 0xa9, 0x97, 0xd5, 0xfb, 0xa7, 0x56, 0xdb];

/// `lending_account_start_flashloan` discriminator.
const START_FLASHLOAN_DISC: [u8; 8] = [0x49, 0xf0, 0xad, 0x60, 0x79, 0xd6, 0x1b, 0x86];

/// `lending_account_end_flashloan` discriminator.
const END_FLASHLOAN_DISC: [u8; 8] = [0x36, 0xd0, 0x72, 0xef, 0xc0, 0x0a, 0x28, 0x0b];

/// `lending_account_start_flashloan`. Data: `[disc(8)] [end_index: u64 LE]`.
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
        program_id: PROGRAM_ID,
        accounts,
        data,
    }
}

/// `lending_account_end_flashloan`. Data: `[disc(8)]`. Remaining accounts are
/// the observation banks used for the health check.
pub fn end_flashloan(
    marginfi_account: &Pubkey,
    signer: &Pubkey,
    observation_banks: &[Pubkey],
) -> Instruction {
    let mut accounts = vec![
        AccountMeta::new(*marginfi_account, false),
        AccountMeta::new_readonly(*signer, true),
    ];
    accounts.extend(
        observation_banks
            .iter()
            .map(|bank| AccountMeta::new_readonly(*bank, false)),
    );

    Instruction {
        program_id: PROGRAM_ID,
        accounts,
        data: END_FLASHLOAN_DISC.to_vec(),
    }
}

/// `lending_account_liquidate`. Data: `[disc(8)] [asset_amount: u64 LE]`.
///
/// The liquidator must have their own `MarginfiAccount` in the same group.
#[allow(clippy::too_many_arguments)]
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
        AccountMeta::new_readonly(*group, false),
        AccountMeta::new(*asset_bank, false),
        AccountMeta::new(*liab_bank, false),
        AccountMeta::new(*liquidator_marginfi_account, false),
        AccountMeta::new_readonly(*signer, true),
        AccountMeta::new(*liquidatee_marginfi_account, false),
        AccountMeta::new_readonly(*liab_bank_liquidity_vault_authority, false),
        AccountMeta::new(*liab_bank_liquidity_vault, false),
        AccountMeta::new(*liab_bank_insurance_vault, false),
        AccountMeta::new_readonly(SPL_TOKEN_PROGRAM, false),
    ];
    accounts.extend_from_slice(remaining_accounts);

    Instruction {
        program_id: PROGRAM_ID,
        accounts,
        data,
    }
}

/// Derive the liquidity vault authority PDA for a Bank: `["liquidity_vault_auth", bank]`.
pub fn derive_liquidity_vault_authority(bank: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"liquidity_vault_auth", bank.as_ref()], &PROGRAM_ID)
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
            &group,
            &asset_bank,
            &liab_bank,
            &liquidator_acc,
            &signer,
            &liquidatee,
            &vault_auth,
            &vault,
            &insurance,
            &[],
        );

        assert_eq!(ix.data.len(), 16);
        assert_eq!(ix.accounts.len(), 10);
        assert!(ix.accounts[4].is_signer);
        assert_eq!(&ix.data[..8], &LIQUIDATE_DISC);
    }

    #[test]
    fn flashloan_instruction_layout() {
        let account = Pubkey::new_unique();
        let signer = Pubkey::new_unique();

        let start = start_flashloan(3, &account, &signer);
        assert_eq!(start.data.len(), 16);
        assert_eq!(start.accounts.len(), 2);

        let end = end_flashloan(&account, &signer, &[Pubkey::new_unique()]);
        assert_eq!(end.data.len(), 8);
        assert_eq!(end.accounts.len(), 3);
    }
}
