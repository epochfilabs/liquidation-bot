//! Save (Solend) instruction builders for flash loan liquidation.
//!
//! Program: So1endDq2YkqhipRh3WViPa8hFvz0XP1PV7qidbGAiN
//!
//! Save uses SPL token-lending style instructions (not Anchor).
//! Instructions are identified by a u8 tag, not an 8-byte discriminator.
//!
//! Flash borrow: instruction tag 19
//! Flash repay:  instruction tag 20
//! Liquidate and redeem: instruction tag 15

use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    sysvar,
};

use super::save::PROGRAM_ID;

fn save_program() -> Pubkey {
    PROGRAM_ID.parse().unwrap()
}

fn spl_token_program() -> Pubkey {
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".parse().unwrap()
}

/// Accounts derived from a Save reserve account.
#[derive(Debug, Clone)]
pub struct SaveReserveAccounts {
    pub reserve: Pubkey,
    pub liquidity_mint: Pubkey,
    pub liquidity_supply: Pubkey,
    pub liquidity_fee_receiver: Pubkey,
    pub collateral_mint: Pubkey,
    pub collateral_supply: Pubkey,
    pub token_program: Pubkey,
}

/// Build `FlashBorrowReserveLiquidity` instruction (tag 19).
///
/// Data: [19u8] [liquidity_amount: u64 LE]
pub fn flash_borrow_reserve_liquidity(
    liquidity_amount: u64,
    reserve: &SaveReserveAccounts,
    lending_market: &Pubkey,
    lending_market_authority: &Pubkey,
    user_destination_liquidity: &Pubkey,
) -> Instruction {
    let mut data = Vec::with_capacity(9);
    data.push(19); // instruction tag
    data.extend_from_slice(&liquidity_amount.to_le_bytes());

    let accounts = vec![
        AccountMeta::new(reserve.liquidity_supply, false),              // source_liquidity
        AccountMeta::new(*user_destination_liquidity, false),           // destination_liquidity
        AccountMeta::new(reserve.reserve, false),                       // reserve
        AccountMeta::new_readonly(*lending_market, false),              // lending_market
        AccountMeta::new_readonly(*lending_market_authority, false),    // lending_market_authority
        AccountMeta::new_readonly(sysvar::instructions::ID, false),    // instructions_sysvar
        AccountMeta::new_readonly(spl_token_program(), false),         // token_program
    ];

    Instruction {
        program_id: save_program(),
        accounts,
        data,
    }
}

/// Build `FlashRepayReserveLiquidity` instruction (tag 20).
///
/// Data: [20u8] [liquidity_amount: u64 LE] [borrow_instruction_index: u8]
pub fn flash_repay_reserve_liquidity(
    liquidity_amount: u64,
    borrow_instruction_index: u8,
    user_source_liquidity: &Pubkey,
    reserve: &SaveReserveAccounts,
    lending_market: &Pubkey,
    user_transfer_authority: &Pubkey,
) -> Instruction {
    let mut data = Vec::with_capacity(10);
    data.push(20); // instruction tag
    data.extend_from_slice(&liquidity_amount.to_le_bytes());
    data.push(borrow_instruction_index);

    let accounts = vec![
        AccountMeta::new(*user_source_liquidity, false),                // source_liquidity
        AccountMeta::new(reserve.liquidity_supply, false),              // destination_liquidity
        AccountMeta::new(reserve.liquidity_fee_receiver, false),        // fee_receiver
        AccountMeta::new(reserve.liquidity_fee_receiver, false),        // host_fee_receiver (same as fee)
        AccountMeta::new(reserve.reserve, false),                       // reserve
        AccountMeta::new_readonly(*lending_market, false),              // lending_market
        AccountMeta::new_readonly(*user_transfer_authority, true),      // user_transfer_authority (signer)
        AccountMeta::new_readonly(sysvar::instructions::ID, false),    // instructions_sysvar
        AccountMeta::new_readonly(spl_token_program(), false),         // token_program
    ];

    Instruction {
        program_id: save_program(),
        accounts,
        data,
    }
}

/// Build `LiquidateObligationAndRedeemReserveCollateral` instruction (tag 15).
///
/// Data: [15u8] [liquidity_amount: u64 LE]
pub fn liquidate_obligation_and_redeem(
    liquidity_amount: u64,
    user_source_liquidity: &Pubkey,
    user_destination_collateral: &Pubkey,
    user_destination_liquidity: &Pubkey,
    repay_reserve: &SaveReserveAccounts,
    withdraw_reserve: &SaveReserveAccounts,
    obligation: &Pubkey,
    lending_market: &Pubkey,
    lending_market_authority: &Pubkey,
    user_transfer_authority: &Pubkey,
) -> Instruction {
    let mut data = Vec::with_capacity(9);
    data.push(15); // instruction tag
    data.extend_from_slice(&liquidity_amount.to_le_bytes());

    let accounts = vec![
        AccountMeta::new(*user_source_liquidity, false),                // source_liquidity
        AccountMeta::new(*user_destination_collateral, false),          // destination_collateral
        AccountMeta::new(*user_destination_liquidity, false),           // destination_liquidity
        AccountMeta::new(repay_reserve.reserve, false),                 // repay_reserve
        AccountMeta::new(repay_reserve.liquidity_supply, false),        // repay_reserve_liquidity_supply
        AccountMeta::new(withdraw_reserve.reserve, false),              // withdraw_reserve
        AccountMeta::new(withdraw_reserve.collateral_mint, false),      // withdraw_reserve_collateral_mint
        AccountMeta::new(withdraw_reserve.collateral_supply, false),    // withdraw_reserve_collateral_supply
        AccountMeta::new(withdraw_reserve.liquidity_supply, false),     // withdraw_reserve_liquidity_supply
        AccountMeta::new(withdraw_reserve.liquidity_fee_receiver, false), // withdraw_reserve_fee_receiver
        AccountMeta::new(*obligation, false),                           // obligation
        AccountMeta::new_readonly(*lending_market, false),              // lending_market
        AccountMeta::new_readonly(*lending_market_authority, false),    // lending_market_authority
        AccountMeta::new_readonly(*user_transfer_authority, true),      // user_transfer_authority (signer)
        AccountMeta::new_readonly(spl_token_program(), false),         // token_program
    ];

    Instruction {
        program_id: save_program(),
        accounts,
        data,
    }
}

/// Derive the Save lending market authority PDA.
/// Seeds: [lending_market_pubkey]
pub fn derive_lending_market_authority(lending_market: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[lending_market.as_ref()], &save_program())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_reserve() -> SaveReserveAccounts {
        SaveReserveAccounts {
            reserve: Pubkey::new_unique(),
            liquidity_mint: Pubkey::new_unique(),
            liquidity_supply: Pubkey::new_unique(),
            liquidity_fee_receiver: Pubkey::new_unique(),
            collateral_mint: Pubkey::new_unique(),
            collateral_supply: Pubkey::new_unique(),
            token_program: spl_token_program(),
        }
    }

    #[test]
    fn flash_borrow_layout() {
        let reserve = dummy_reserve();
        let market = Pubkey::new_unique();
        let authority = Pubkey::new_unique();
        let dest = Pubkey::new_unique();

        let ix = flash_borrow_reserve_liquidity(1_000_000, &reserve, &market, &authority, &dest);
        assert_eq!(ix.data.len(), 9); // 1 tag + 8 amount
        assert_eq!(ix.data[0], 19);
        assert_eq!(ix.accounts.len(), 7);
    }

    #[test]
    fn flash_repay_layout() {
        let reserve = dummy_reserve();
        let market = Pubkey::new_unique();
        let source = Pubkey::new_unique();
        let auth = Pubkey::new_unique();

        let ix = flash_repay_reserve_liquidity(1_000_000, 0, &source, &reserve, &market, &auth);
        assert_eq!(ix.data.len(), 10); // 1 tag + 8 amount + 1 index
        assert_eq!(ix.data[0], 20);
        assert_eq!(ix.accounts.len(), 9);
        assert!(ix.accounts[6].is_signer); // user_transfer_authority
    }

    #[test]
    fn liquidate_layout() {
        let repay = dummy_reserve();
        let withdraw = dummy_reserve();
        let obligation = Pubkey::new_unique();
        let market = Pubkey::new_unique();
        let authority = Pubkey::new_unique();
        let signer = Pubkey::new_unique();
        let src = Pubkey::new_unique();
        let dst_col = Pubkey::new_unique();
        let dst_liq = Pubkey::new_unique();

        let ix = liquidate_obligation_and_redeem(
            500_000, &src, &dst_col, &dst_liq,
            &repay, &withdraw, &obligation, &market, &authority, &signer,
        );
        assert_eq!(ix.data.len(), 9); // 1 tag + 8 amount
        assert_eq!(ix.data[0], 15);
        assert_eq!(ix.accounts.len(), 15);
        assert!(ix.accounts[13].is_signer); // user_transfer_authority
    }
}
