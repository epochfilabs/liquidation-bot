//! Jupiter Lend flash loan provider.
//!
//! Fee: 0% (zero fee flash loans)
//! Program: jupgfSgfuAXv4B6R2Uxu85Z1qdzgju79s6MfZekN6XS
//!
//! Jupiter flash loans use instruction sysvar introspection to verify
//! that flashloan_payback exists in the same transaction as flashloan_borrow.

use anyhow::{Context, Result};
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;

use crate::protocols::jupiter_lend::instructions::{self as jupiter_lend_instructions, JupiterFlashLoanAccounts};
use super::{FeeRate, FlashLoanInstructions, FlashLoanProvider, FlashLoanProviderKind};

/// Jupiter Lend flash loan provider.
///
/// Zero fees. Requires knowing the flash loan accounts for each mint
/// (flashloan_admin, reserves, rate_model, etc.). Call `add_mint` to register.
#[derive(Debug, Default)]
pub struct JupiterFlashLoanProvider {
    /// Flash loan accounts keyed by token mint.
    accounts: HashMap<Pubkey, JupiterFlashLoanAccounts>,
}

impl JupiterFlashLoanProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a mint for flash loan usage.
    ///
    /// The `JupiterFlashLoanAccounts` must be populated with the correct PDAs
    /// for the given mint. These can be derived from the Jupiter Lend program
    /// or fetched from on-chain account data.
    pub fn add_mint(&mut self, mint: Pubkey, accounts: JupiterFlashLoanAccounts) {
        self.accounts.insert(mint, accounts);
    }

    /// Get the flash loan accounts for a given mint, if registered.
    pub fn get_accounts(&self, mint: &Pubkey) -> Option<&JupiterFlashLoanAccounts> {
        self.accounts.get(mint)
    }

    /// Number of mints registered.
    pub fn mint_count(&self) -> usize {
        self.accounts.len()
    }
}

impl FlashLoanProvider for JupiterFlashLoanProvider {
    fn kind(&self) -> FlashLoanProviderKind {
        FlashLoanProviderKind::JupiterLend
    }

    fn fee_rate(&self) -> FeeRate {
        0.0 // Zero fee
    }

    fn supports_mint(&self, mint: &Pubkey) -> bool {
        self.accounts.contains_key(mint)
    }

    fn build_instructions(
        &self,
        signer: &Pubkey,
        token_account: &Pubkey,
        mint: &Pubkey,
        amount: u64,
        borrow_ix_index: u8,
    ) -> Result<FlashLoanInstructions> {
        let flash_accounts = self.accounts.get(mint)
            .context("Jupiter flash loan: mint not registered")?;

        let borrow_ix = jupiter_lend_instructions::flash_borrow(
            amount,
            signer,
            token_account,
            flash_accounts,
        );

        let repay_ix = jupiter_lend_instructions::flash_payback(
            amount,
            signer,
            token_account,
            flash_accounts,
        );

        Ok(FlashLoanInstructions {
            provider: FlashLoanProviderKind::JupiterLend,
            borrow_ix,
            repay_ix,
            borrow_ix_index,
        })
    }
}
