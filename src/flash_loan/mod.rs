//! Flash loan provider trait and implementations.
//!
//! Each flash loan source (Kamino, Jupiter Lend, Save, MarginFi) implements
//! the `FlashLoanProvider` trait. The executor selects the cheapest available
//! provider for each liquidation based on fee rate and available liquidity.
//!
//! Usage:
//!   let providers = vec![
//!       Box::new(JupiterFlashLoanProvider::new()) as Box<dyn FlashLoanProvider>,
//!       Box::new(KaminoFlashLoanProvider::new(config)),
//!   ];
//!   let best = select_provider(&providers, &mint, amount, &rpc)?;
//!   let (borrow_ix, repay_ix) = best.build_instructions(...)?;

pub mod kamino;
pub mod jupiter;

use anyhow::Result;
use solana_sdk::{
    instruction::Instruction,
    pubkey::Pubkey,
};

/// Flash loan fee as a fraction (e.g., 0.00001 = 0.001%).
pub type FeeRate = f64;

/// Describes a flash loan provider's capability for a specific token.
#[derive(Debug, Clone)]
pub struct FlashLoanQuote {
    /// Which provider this quote is from.
    pub provider: FlashLoanProviderKind,
    /// The token mint being borrowed.
    pub mint: Pubkey,
    /// Maximum borrowable amount (limited by reserve liquidity).
    pub max_amount: u64,
    /// Fee rate as a fraction (0.0 for Jupiter Lend, 0.00001 for Kamino).
    pub fee_rate: FeeRate,
    /// Estimated fee in token units for the requested amount.
    pub fee_amount: u64,
}

/// Identifies which flash loan provider is being used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlashLoanProviderKind {
    /// Kamino Lend: 0.001% fee, deepest liquidity on Solana.
    Kamino,
    /// Jupiter Lend: 0% fee, newer but growing.
    JupiterLend,
    /// Save (Solend): ~0.3% fee, most expensive.
    Save,
    /// MarginFi: flag-based (start/end), no direct fee but health check at end.
    MarginFi,
}

impl std::fmt::Display for FlashLoanProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Kamino => write!(f, "kamino"),
            Self::JupiterLend => write!(f, "jupiter_lend"),
            Self::Save => write!(f, "save"),
            Self::MarginFi => write!(f, "marginfi"),
        }
    }
}

/// The instructions produced by a flash loan provider.
#[derive(Debug)]
pub struct FlashLoanInstructions {
    /// The provider that generated these instructions.
    pub provider: FlashLoanProviderKind,
    /// Instruction to borrow tokens. Goes BEFORE the liquidation instruction.
    pub borrow_ix: Instruction,
    /// Instruction to repay tokens + fee. Goes AFTER the liquidation instruction.
    pub repay_ix: Instruction,
    /// Index of the borrow instruction in the final transaction.
    /// Some protocols (Kamino, Save) use instruction sysvar introspection
    /// to verify the borrow/repay pair exists in the same tx.
    /// Set this after assembling the full transaction.
    pub borrow_ix_index: u8,
}

/// Trait for flash loan providers.
///
/// Each implementation knows how to:
/// 1. Check if it can provide a flash loan for a given token + amount
/// 2. Quote the fee
/// 3. Build the borrow and repay instructions
pub trait FlashLoanProvider: Send + Sync {
    /// Provider identifier.
    fn kind(&self) -> FlashLoanProviderKind;

    /// Fee rate as a fraction. 0.0 = free, 0.00001 = 0.001%.
    fn fee_rate(&self) -> FeeRate;

    /// Check if this provider supports flash loans for the given mint.
    /// Returns None if the mint is not supported.
    fn supports_mint(&self, mint: &Pubkey) -> bool;

    /// Build borrow + repay instructions for the given amount.
    ///
    /// `signer`: the liquidator's wallet pubkey
    /// `token_account`: the liquidator's token account for this mint
    /// `mint`: the token to borrow
    /// `amount`: how much to borrow
    /// `borrow_ix_index`: the expected index of the borrow instruction in the tx
    fn build_instructions(
        &self,
        signer: &Pubkey,
        token_account: &Pubkey,
        mint: &Pubkey,
        amount: u64,
        borrow_ix_index: u8,
    ) -> Result<FlashLoanInstructions>;
}

/// Select the best (cheapest) flash loan provider for a given token and amount.
///
/// Returns the provider with the lowest fee rate that supports the mint.
/// If multiple providers have the same fee rate (e.g., both are 0%), the first
/// one in the list is chosen (so order your providers by preference).
pub fn select_provider<'a>(
    providers: &'a [Box<dyn FlashLoanProvider>],
    mint: &Pubkey,
) -> Option<&'a dyn FlashLoanProvider> {
    providers
        .iter()
        .filter(|p| p.supports_mint(mint))
        .min_by(|a, b| a.fee_rate().partial_cmp(&b.fee_rate()).unwrap())
        .map(|p| p.as_ref())
}

/// Build a complete liquidation transaction with flash loan wrapping.
///
/// Layout:
///   [setup_ixs]       — ATA creation if needed
///   [flash_borrow_ix] — borrow the debt token
///   [liquidate_ix]    — repay debt, seize collateral
///   [swap_ix]         — optional: swap collateral → debt token (via Jupiter)
///   [flash_repay_ix]  — repay flash loan + fee
pub fn build_flash_loan_tx(
    setup_ixs: Vec<Instruction>,
    flash: FlashLoanInstructions,
    liquidate_ix: Instruction,
    swap_ix: Option<Instruction>,
) -> Vec<Instruction> {
    let mut ixs = setup_ixs;
    ixs.push(flash.borrow_ix);
    ixs.push(liquidate_ix);
    if let Some(swap) = swap_ix {
        ixs.push(swap);
    }
    ixs.push(flash.repay_ix);
    ixs
}
