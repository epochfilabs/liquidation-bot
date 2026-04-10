//! Multi-protocol support for flash loan liquidations.
//!
//! Each lending protocol implements the `LendingProtocol` trait, providing:
//! - Account discriminator detection
//! - Health evaluation from raw account data
//! - Position parsing (deposits/borrows)
//! - Flash loan liquidation transaction building

pub mod kamino;
pub mod marginfi;
pub mod save;
pub mod jupiter_lend;
pub mod jupiter_lend_instructions;
pub mod save_instructions;
pub mod marginfi_bank;
pub mod marginfi_instructions;

use anyhow::Result;
use solana_sdk::{
    instruction::Instruction,
    pubkey::Pubkey,
};

/// Common health result across all protocols.
#[derive(Debug, Clone)]
pub struct HealthResult {
    pub current_ltv: f64,
    pub unhealthy_ltv: f64,
    pub is_liquidatable: bool,
    pub deposited_value_usd: f64,
    pub borrowed_value_usd: f64,
}

/// A deposit position.
#[derive(Debug, Clone)]
pub struct DepositPosition {
    pub reserve: Pubkey,
    pub mint: Option<Pubkey>,
    pub amount: u64,
    pub market_value_usd: f64,
}

/// A borrow position.
#[derive(Debug, Clone)]
pub struct BorrowPosition {
    pub reserve: Pubkey,
    pub mint: Option<Pubkey>,
    pub amount_sf: u128,
    pub market_value_usd: f64,
}

/// Parsed positions from a protocol's obligation/account.
#[derive(Debug, Clone)]
pub struct Positions {
    pub deposits: Vec<DepositPosition>,
    pub borrows: Vec<BorrowPosition>,
    pub market: Pubkey,
    pub owner: Pubkey,
}

/// Instructions for a flash loan liquidation.
#[derive(Debug)]
pub struct FlashLoanLiquidationIxs {
    /// ATA creation instructions (if needed).
    pub setup_ixs: Vec<Instruction>,
    /// Flash borrow instruction.
    pub flash_borrow_ix: Instruction,
    /// Liquidation instruction.
    pub liquidate_ix: Instruction,
    /// Flash repay instruction.
    pub flash_repay_ix: Instruction,
    /// The index of flash_borrow within the final transaction
    /// (setup_ixs.len()), needed by some protocols for introspection.
    pub borrow_ix_index: u8,
}

/// Identifies which protocol an account belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolKind {
    Kamino,
    Save,
    MarginFi,
    JupiterLend,
}

impl std::fmt::Display for ProtocolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Kamino => write!(f, "kamino"),
            Self::Save => write!(f, "save"),
            Self::MarginFi => write!(f, "marginfi"),
            Self::JupiterLend => write!(f, "jupiter_lend"),
        }
    }
}

/// All known lending protocol program IDs.
pub fn protocol_program_ids() -> Vec<(ProtocolKind, Pubkey)> {
    vec![
        (
            ProtocolKind::Kamino,
            "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD"
                .parse()
                .unwrap(),
        ),
        (
            ProtocolKind::Save,
            "SLendK7ySfcEzyaFqy93gDnD3RtrpXJcnRwb6zFHJSh"
                .parse()
                .unwrap(),
        ),
        (
            ProtocolKind::MarginFi,
            "MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA"
                .parse()
                .unwrap(),
        ),
        (
            ProtocolKind::JupiterLend,
            // Jupiter Lend Vaults program (handles liquidation)
            "jupr81YtYssSyPt8jbnGuiWon5f6x9TcDEFxYe3Bdzi"
                .parse()
                .unwrap(),
        ),
    ]
}

/// Identify which protocol owns an account by its owner program.
pub fn identify_protocol(owner_program: &Pubkey) -> Option<ProtocolKind> {
    for (kind, program_id) in protocol_program_ids() {
        if owner_program == &program_id {
            return Some(kind);
        }
    }
    None
}

/// Trait that each lending protocol implements.
pub trait LendingProtocol: Send + Sync {
    /// Protocol identifier.
    fn kind(&self) -> ProtocolKind;

    /// Program ID for this protocol.
    fn program_id(&self) -> Pubkey;

    /// Check if raw account data is an obligation/position account for this protocol.
    fn is_position_account(&self, data: &[u8]) -> bool;

    /// Evaluate the health of a position from raw account data.
    fn evaluate_health(&self, data: &[u8]) -> Result<HealthResult>;

    /// Parse deposit/borrow positions from raw account data.
    fn parse_positions(&self, data: &[u8]) -> Result<Positions>;
}
