//! Save (Solend) instruction builders for flash-loan liquidation.
//!
//! Save uses SPL token-lending-style instructions (not Anchor). Instructions
//! are identified by a `u8` tag, not an 8-byte discriminator.
//!
//! - Flash borrow: tag 19
//! - Flash repay:  tag 20
//! - Liquidate & redeem: tag 15

use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    sysvar,
};

use super::PROGRAM_ID;

pub const SPL_TOKEN_PROGRAM: Pubkey =
    solana_sdk::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

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

/// `FlashBorrowReserveLiquidity` (tag 19). Data: `[19u8] [liquidity_amount: u64 LE]`.
pub fn flash_borrow_reserve_liquidity(
    liquidity_amount: u64,
    reserve: &SaveReserveAccounts,
    lending_market: &Pubkey,
    lending_market_authority: &Pubkey,
    user_destination_liquidity: &Pubkey,
) -> Instruction {
    let mut data = Vec::with_capacity(9);
    data.push(19);
    data.extend_from_slice(&liquidity_amount.to_le_bytes());

    let accounts = vec![
        AccountMeta::new(reserve.liquidity_supply, false),
        AccountMeta::new(*user_destination_liquidity, false),
        AccountMeta::new(reserve.reserve, false),
        AccountMeta::new_readonly(*lending_market, false),
        AccountMeta::new_readonly(*lending_market_authority, false),
        AccountMeta::new_readonly(sysvar::instructions::ID, false),
        AccountMeta::new_readonly(SPL_TOKEN_PROGRAM, false),
    ];

    Instruction {
        program_id: PROGRAM_ID,
        accounts,
        data,
    }
}

/// `FlashRepayReserveLiquidity` (tag 20). Data: `[20u8] [liquidity_amount: u64 LE] [borrow_ix_idx: u8]`.
pub fn flash_repay_reserve_liquidity(
    liquidity_amount: u64,
    borrow_instruction_index: u8,
    user_source_liquidity: &Pubkey,
    reserve: &SaveReserveAccounts,
    lending_market: &Pubkey,
    user_transfer_authority: &Pubkey,
) -> Instruction {
    let mut data = Vec::with_capacity(10);
    data.push(20);
    data.extend_from_slice(&liquidity_amount.to_le_bytes());
    data.push(borrow_instruction_index);

    let accounts = vec![
        AccountMeta::new(*user_source_liquidity, false),
        AccountMeta::new(reserve.liquidity_supply, false),
        AccountMeta::new(reserve.liquidity_fee_receiver, false),
        AccountMeta::new(reserve.liquidity_fee_receiver, false),
        AccountMeta::new(reserve.reserve, false),
        AccountMeta::new_readonly(*lending_market, false),
        AccountMeta::new_readonly(*user_transfer_authority, true),
        AccountMeta::new_readonly(sysvar::instructions::ID, false),
        AccountMeta::new_readonly(SPL_TOKEN_PROGRAM, false),
    ];

    Instruction {
        program_id: PROGRAM_ID,
        accounts,
        data,
    }
}

/// `LiquidateObligationAndRedeemReserveCollateral` (tag 15). Data: `[15u8] [liquidity_amount: u64 LE]`.
#[allow(clippy::too_many_arguments)]
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
    data.push(15);
    data.extend_from_slice(&liquidity_amount.to_le_bytes());

    let accounts = vec![
        AccountMeta::new(*user_source_liquidity, false),
        AccountMeta::new(*user_destination_collateral, false),
        AccountMeta::new(*user_destination_liquidity, false),
        AccountMeta::new(repay_reserve.reserve, false),
        AccountMeta::new(repay_reserve.liquidity_supply, false),
        AccountMeta::new(withdraw_reserve.reserve, false),
        AccountMeta::new(withdraw_reserve.collateral_mint, false),
        AccountMeta::new(withdraw_reserve.collateral_supply, false),
        AccountMeta::new(withdraw_reserve.liquidity_supply, false),
        AccountMeta::new(withdraw_reserve.liquidity_fee_receiver, false),
        AccountMeta::new(*obligation, false),
        AccountMeta::new_readonly(*lending_market, false),
        AccountMeta::new_readonly(*lending_market_authority, false),
        AccountMeta::new_readonly(*user_transfer_authority, true),
        AccountMeta::new_readonly(SPL_TOKEN_PROGRAM, false),
    ];

    Instruction {
        program_id: PROGRAM_ID,
        accounts,
        data,
    }
}

/// Derive the Save lending-market authority PDA. Seeds: `[lending_market]`.
pub fn derive_lending_market_authority(lending_market: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[lending_market.as_ref()], &PROGRAM_ID)
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
            token_program: SPL_TOKEN_PROGRAM,
        }
    }

    #[test]
    fn flash_borrow_layout() {
        let reserve = dummy_reserve();
        let market = Pubkey::new_unique();
        let authority = Pubkey::new_unique();
        let dest = Pubkey::new_unique();

        let ix = flash_borrow_reserve_liquidity(1_000_000, &reserve, &market, &authority, &dest);
        assert_eq!(ix.data.len(), 9);
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
        assert_eq!(ix.data.len(), 10);
        assert_eq!(ix.data[0], 20);
        assert_eq!(ix.accounts.len(), 9);
        assert!(ix.accounts[6].is_signer);
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
            500_000, &src, &dst_col, &dst_liq, &repay, &withdraw, &obligation, &market,
            &authority, &signer,
        );
        assert_eq!(ix.data.len(), 9);
        assert_eq!(ix.data[0], 15);
        assert_eq!(ix.accounts.len(), 15);
        assert!(ix.accounts[13].is_signer);
    }
}
