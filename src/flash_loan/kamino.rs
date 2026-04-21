//! Kamino Lend flash loan provider.
//!
//! Fee: ~0.001% (configurable per reserve, stored in reserve.fees.flash_loan_fee_sf)
//! Deepest liquidity on Solana. Uses instruction sysvar introspection to verify
//! that borrow and repay instructions exist in the same transaction.

use anyhow::{Context, Result};
use solana_sdk::{
    pubkey::Pubkey,
    instruction::Instruction,
};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::LazyLock;

use crate::liquidator::instructions::{self, ReserveAccounts};
use super::{FeeRate, FlashLoanInstructions, FlashLoanProvider, FlashLoanProviderKind};

static KLEND_PROGRAM: LazyLock<Pubkey> = LazyLock::new(|| {
    Pubkey::from_str("KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD").unwrap()
});

/// Kamino flash loan provider.
///
/// Requires knowing the reserve accounts for each mint (fetched from on-chain
/// or cached from a prior lookup). Call `add_reserve` to register mints.
pub struct KaminoFlashLoanProvider {
    /// Lending market pubkey.
    lending_market: Pubkey,
    /// Lending market authority PDA.
    lending_market_authority: Pubkey,
    /// Reserve accounts keyed by liquidity mint.
    reserves: HashMap<Pubkey, ReserveAccounts>,
}

impl KaminoFlashLoanProvider {
    pub fn new(lending_market: &Pubkey) -> Self {
        let (authority, _) = instructions::derive_lending_market_authority(
            lending_market,
            &KLEND_PROGRAM,
        );
        Self {
            lending_market: *lending_market,
            lending_market_authority: authority,
            reserves: HashMap::new(),
        }
    }

    /// Register a reserve for flash loan usage.
    /// The provider can only flash loan mints that have been registered.
    pub fn add_reserve(&mut self, reserve: ReserveAccounts) {
        self.reserves.insert(reserve.liquidity_mint, reserve);
    }

    /// Get the reserve for a given mint, if registered.
    pub fn get_reserve(&self, mint: &Pubkey) -> Option<&ReserveAccounts> {
        self.reserves.get(mint)
    }
}

impl FlashLoanProvider for KaminoFlashLoanProvider {
    fn kind(&self) -> FlashLoanProviderKind {
        FlashLoanProviderKind::Kamino
    }

    fn fee_rate(&self) -> FeeRate {
        0.00001 // 0.001%
    }

    fn supports_mint(&self, mint: &Pubkey) -> bool {
        self.reserves.contains_key(mint)
    }

    fn build_instructions(
        &self,
        signer: &Pubkey,
        token_account: &Pubkey,
        mint: &Pubkey,
        amount: u64,
        borrow_ix_index: u8,
    ) -> Result<FlashLoanInstructions> {
        let reserve = self.reserves.get(mint)
            .context("Kamino flash loan: mint not registered")?;

        let borrow_ix = instructions::flash_borrow_reserve_liquidity(
            &KLEND_PROGRAM,
            amount,
            signer,
            &self.lending_market,
            &self.lending_market_authority,
            reserve,
            token_account,
        );

        let repay_ix = instructions::flash_repay_reserve_liquidity(
            &KLEND_PROGRAM,
            amount,
            borrow_ix_index,
            signer,
            &self.lending_market,
            &self.lending_market_authority,
            reserve,
            token_account,
        );

        Ok(FlashLoanInstructions {
            provider: FlashLoanProviderKind::Kamino,
            borrow_ix,
            repay_ix,
            borrow_ix_index,
        })
    }
}
