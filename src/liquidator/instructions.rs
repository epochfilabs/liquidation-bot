//! Raw instruction builders for klend flash loan and liquidation instructions.
//!
//! Built from the klend program source — no dependency on the kamino-lend crate.

use sha2::{Sha256, Digest};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    sysvar,
};
use std::sync::LazyLock;

/// klend program ID (mainnet).
pub static KLEND_PROGRAM_ID: LazyLock<Pubkey> = LazyLock::new(|| {
    "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD"
        .parse()
        .unwrap()
});

/// SPL Token program ID.
pub static SPL_TOKEN_PROGRAM: LazyLock<Pubkey> = LazyLock::new(|| {
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
        .parse()
        .unwrap()
});

fn anchor_discriminator(name: &str) -> [u8; 8] {
    let hash = Sha256::digest(name.as_bytes());
    let mut disc = [0u8; 8];
    disc.copy_from_slice(&hash[..8]);
    disc
}

/// Derive the lending market authority PDA.
/// Seeds: [b"lma", lending_market_pubkey]
pub fn derive_lending_market_authority(
    lending_market: &Pubkey,
    program_id: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"lma", lending_market.as_ref()], program_id)
}

/// Accounts needed to interact with a specific reserve.
#[derive(Debug, Clone)]
pub struct ReserveAccounts {
    pub reserve: Pubkey,
    pub liquidity_mint: Pubkey,
    pub liquidity_supply_vault: Pubkey,
    pub liquidity_fee_vault: Pubkey,
    pub collateral_mint: Pubkey,
    pub collateral_supply_vault: Pubkey,
    pub token_program: Pubkey,
}

/// Build `flash_borrow_reserve_liquidity` instruction.
///
/// Discriminator: sha256("global:flash_borrow_reserve_liquidity")[..8]
/// Data: [disc(8)] [liquidity_amount(u64 LE)]
pub fn flash_borrow_reserve_liquidity(
    program_id: &Pubkey,
    liquidity_amount: u64,
    user_transfer_authority: &Pubkey,
    lending_market: &Pubkey,
    lending_market_authority: &Pubkey,
    reserve: &ReserveAccounts,
    user_destination_liquidity: &Pubkey,
) -> Instruction {
    let disc = anchor_discriminator("global:flash_borrow_reserve_liquidity");

    let mut data = Vec::with_capacity(16);
    data.extend_from_slice(&disc);
    data.extend_from_slice(&liquidity_amount.to_le_bytes());

    let accounts = vec![
        AccountMeta::new_readonly(*user_transfer_authority, true),   // 0: signer
        AccountMeta::new_readonly(*lending_market_authority, false),  // 1
        AccountMeta::new_readonly(*lending_market, false),           // 2
        AccountMeta::new(reserve.reserve, false),                    // 3: writable
        AccountMeta::new_readonly(reserve.liquidity_mint, false),    // 4
        AccountMeta::new(reserve.liquidity_supply_vault, false),     // 5: writable
        AccountMeta::new(*user_destination_liquidity, false),        // 6: writable
        AccountMeta::new(reserve.liquidity_fee_vault, false),        // 7: writable
        AccountMeta::new(*program_id, false),                        // 8: referrer_token_state (unused → program_id)
        AccountMeta::new(*program_id, false),                        // 9: referrer_account (unused → program_id)
        AccountMeta::new_readonly(sysvar::instructions::ID, false),  // 10: sysvar instructions
        AccountMeta::new_readonly(reserve.token_program, false),     // 11: token program
    ];

    Instruction {
        program_id: *program_id,
        accounts,
        data,
    }
}

/// Build `flash_repay_reserve_liquidity` instruction.
///
/// Discriminator: sha256("global:flash_repay_reserve_liquidity")[..8]
/// Data: [disc(8)] [liquidity_amount(u64 LE)] [borrow_instruction_index(u8)]
pub fn flash_repay_reserve_liquidity(
    program_id: &Pubkey,
    liquidity_amount: u64,
    borrow_instruction_index: u8,
    user_transfer_authority: &Pubkey,
    lending_market: &Pubkey,
    lending_market_authority: &Pubkey,
    reserve: &ReserveAccounts,
    user_source_liquidity: &Pubkey,
) -> Instruction {
    let disc = anchor_discriminator("global:flash_repay_reserve_liquidity");

    let mut data = Vec::with_capacity(17);
    data.extend_from_slice(&disc);
    data.extend_from_slice(&liquidity_amount.to_le_bytes());
    data.push(borrow_instruction_index);

    let accounts = vec![
        AccountMeta::new_readonly(*user_transfer_authority, true),   // 0: signer
        AccountMeta::new_readonly(*lending_market_authority, false),  // 1
        AccountMeta::new_readonly(*lending_market, false),           // 2
        AccountMeta::new(reserve.reserve, false),                    // 3: writable
        AccountMeta::new_readonly(reserve.liquidity_mint, false),    // 4
        AccountMeta::new(reserve.liquidity_supply_vault, false),     // 5: writable (destination)
        AccountMeta::new(*user_source_liquidity, false),             // 6: writable
        AccountMeta::new(reserve.liquidity_fee_vault, false),        // 7: writable
        AccountMeta::new(*program_id, false),                        // 8: referrer_token_state (unused)
        AccountMeta::new(*program_id, false),                        // 9: referrer_account (unused)
        AccountMeta::new_readonly(sysvar::instructions::ID, false),  // 10: sysvar instructions
        AccountMeta::new_readonly(reserve.token_program, false),     // 11: token program
    ];

    Instruction {
        program_id: *program_id,
        accounts,
        data,
    }
}

/// Parameters for the liquidation instruction.
#[derive(Debug, Clone)]
pub struct LiquidateParams {
    pub liquidity_amount: u64,
    pub min_acceptable_received_liquidity_amount: u64,
}

/// Build `liquidate_obligation_and_redeem_reserve_collateral` (v1) instruction.
///
/// Discriminator: sha256("global:liquidate_obligation_and_redeem_reserve_collateral")[..8]
/// Data: [disc(8)] [liquidity_amount(u64)] [min_received(u64)] [max_ltv_override(u64)]
pub fn liquidate_obligation_and_redeem_reserve_collateral(
    program_id: &Pubkey,
    params: &LiquidateParams,
    liquidator: &Pubkey,
    obligation: &Pubkey,
    lending_market: &Pubkey,
    lending_market_authority: &Pubkey,
    repay_reserve: &ReserveAccounts,
    withdraw_reserve: &ReserveAccounts,
    user_source_liquidity: &Pubkey,
    user_destination_collateral: &Pubkey,
    user_destination_liquidity: &Pubkey,
) -> Instruction {
    let disc = anchor_discriminator(
        "global:liquidate_obligation_and_redeem_reserve_collateral",
    );

    let mut data = Vec::with_capacity(32);
    data.extend_from_slice(&disc);
    data.extend_from_slice(&params.liquidity_amount.to_le_bytes());
    data.extend_from_slice(&params.min_acceptable_received_liquidity_amount.to_le_bytes());
    data.extend_from_slice(&0u64.to_le_bytes()); // max_allowed_ltv_override_percent = 0

    let accounts = vec![
        AccountMeta::new_readonly(*liquidator, true),                            // 0: signer
        AccountMeta::new(*obligation, false),                                    // 1: writable
        AccountMeta::new_readonly(*lending_market, false),                       // 2
        AccountMeta::new_readonly(*lending_market_authority, false),              // 3
        AccountMeta::new(repay_reserve.reserve, false),                          // 4: writable
        AccountMeta::new_readonly(repay_reserve.liquidity_mint, false),          // 5
        AccountMeta::new(repay_reserve.liquidity_supply_vault, false),           // 6: writable
        AccountMeta::new(withdraw_reserve.reserve, false),                       // 7: writable
        AccountMeta::new_readonly(withdraw_reserve.liquidity_mint, false),       // 8
        AccountMeta::new(withdraw_reserve.collateral_mint, false),               // 9: writable
        AccountMeta::new(withdraw_reserve.collateral_supply_vault, false),       // 10: writable
        AccountMeta::new(withdraw_reserve.liquidity_supply_vault, false),        // 11: writable
        AccountMeta::new(withdraw_reserve.liquidity_fee_vault, false),           // 12: writable
        AccountMeta::new(*user_source_liquidity, false),                         // 13: writable
        AccountMeta::new(*user_destination_collateral, false),                   // 14: writable
        AccountMeta::new(*user_destination_liquidity, false),                    // 15: writable
        AccountMeta::new_readonly(*SPL_TOKEN_PROGRAM, false),                    // 16: collateral token program
        AccountMeta::new_readonly(repay_reserve.token_program, false),           // 17: repay liquidity token program
        AccountMeta::new_readonly(withdraw_reserve.token_program, false),        // 18: withdraw liquidity token program
        AccountMeta::new_readonly(sysvar::instructions::ID, false),              // 19: instruction sysvar
    ];

    Instruction {
        program_id: *program_id,
        accounts,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash_borrow_discriminator() {
        let disc = anchor_discriminator("global:flash_borrow_reserve_liquidity");
        assert_eq!(disc, [135, 231, 52, 167, 7, 52, 212, 193]);
    }

    #[test]
    fn flash_repay_discriminator() {
        let disc = anchor_discriminator("global:flash_repay_reserve_liquidity");
        assert_eq!(disc, [185, 117, 0, 203, 96, 245, 180, 186]);
    }

    #[test]
    fn liquidate_v1_discriminator() {
        let disc = anchor_discriminator(
            "global:liquidate_obligation_and_redeem_reserve_collateral",
        );
        assert_eq!(disc, [177, 71, 154, 188, 226, 133, 74, 55]);
    }

    #[test]
    fn lending_market_authority_derivation() {
        // Smoke test: derivation should not panic
        let market: Pubkey = "7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF"
            .parse()
            .unwrap();
        let (authority, bump) = derive_lending_market_authority(&market, &KLEND_PROGRAM_ID);
        assert_ne!(authority, Pubkey::default());
        assert!(bump <= 255);
    }

    #[test]
    fn flash_borrow_instruction_layout() {
        let program_id = *KLEND_PROGRAM_ID;
        let user = Pubkey::new_unique();
        let market = Pubkey::new_unique();
        let authority = Pubkey::new_unique();
        let reserve = ReserveAccounts {
            reserve: Pubkey::new_unique(),
            liquidity_mint: Pubkey::new_unique(),
            liquidity_supply_vault: Pubkey::new_unique(),
            liquidity_fee_vault: Pubkey::new_unique(),
            collateral_mint: Pubkey::new_unique(),
            collateral_supply_vault: Pubkey::new_unique(),
            token_program: *SPL_TOKEN_PROGRAM,
        };
        let dest = Pubkey::new_unique();

        let ix = flash_borrow_reserve_liquidity(
            &program_id, 1_000_000, &user, &market, &authority, &reserve, &dest,
        );

        assert_eq!(ix.data.len(), 16); // 8 disc + 8 amount
        assert_eq!(ix.accounts.len(), 12);
        assert!(ix.accounts[0].is_signer); // user_transfer_authority
    }
}
