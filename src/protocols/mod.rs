//! Multi-protocol support for flash-loan liquidations.
//!
//! Every lending protocol we support implements the [`LendingProtocol`] trait:
//! detecting position accounts from raw bytes, evaluating health, parsing
//! positions, and building the protocol-specific liquidation instruction.
//! [`Registry`] is the composition-root container that `main` uses to dispatch
//! gRPC updates to the right handler.

pub mod jupiter_lend;
pub mod kamino;
pub mod marginfi;
pub mod save;

use anyhow::Result;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey};

use crate::config::AppConfig;

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

/// Parsed positions from a protocol's obligation/position account.
#[derive(Debug, Clone)]
pub struct Positions {
    pub deposits: Vec<DepositPosition>,
    pub borrows: Vec<BorrowPosition>,
    pub market: Pubkey,
    pub owner: Pubkey,
}

/// Context passed to [`LendingProtocol::build_liquidate_ix`] and the executor.
#[derive(Debug)]
pub struct LiquidationParams {
    pub protocol: ProtocolKind,
    pub position_pubkey: Pubkey,
    pub health: HealthResult,
    pub positions: Positions,
}

/// Identifies which protocol an account belongs to.
///
/// Declared `#[non_exhaustive]` so adding a variant isn't a breaking change
/// for downstream match arms. Variant order is load-bearing: it is used as
/// a usize index into [`Registry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProtocolKind {
    Kamino = 0,
    Save = 1,
    MarginFi = 2,
    JupiterLend = 3,
}

impl ProtocolKind {
    /// Number of supported protocols. Must match the number of declared variants.
    pub const COUNT: usize = 4;
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

/// Mainnet program ID for every supported lending protocol.
pub fn protocol_program_ids() -> [(ProtocolKind, Pubkey); ProtocolKind::COUNT] {
    [
        (ProtocolKind::Kamino, kamino::PROGRAM_ID),
        (ProtocolKind::Save, save::PROGRAM_ID),
        (ProtocolKind::MarginFi, marginfi::PROGRAM_ID),
        (ProtocolKind::JupiterLend, jupiter_lend::PROGRAM_ID),
    ]
}

/// Identify which protocol owns an account by its owner program.
pub fn identify_protocol(owner_program: &Pubkey) -> Option<ProtocolKind> {
    protocol_program_ids()
        .into_iter()
        .find(|(_, pid)| pid == owner_program)
        .map(|(kind, _)| kind)
}

/// Trait that each lending protocol implements.
pub trait LendingProtocol: Send + Sync {
    /// Protocol identifier.
    fn kind(&self) -> ProtocolKind;

    /// Program ID for this protocol.
    fn program_id(&self) -> Pubkey;

    /// Check if raw account data is a position/obligation account for this protocol.
    fn is_position_account(&self, data: &[u8]) -> bool;

    /// Evaluate the health of a position from raw account data.
    fn evaluate_health(&self, data: &[u8]) -> Result<HealthResult>;

    /// Parse deposit/borrow positions from raw account data.
    fn parse_positions(&self, data: &[u8]) -> Result<Positions>;

    /// Convert a borrow's scaled-fraction `amount_sf` into flash-loan token units.
    /// Each protocol uses a different fixed-point scheme (Kamino 2^60, Save 10^18,
    /// Jupiter/MarginFi native units).
    fn flash_loan_amount(&self, borrow: &BorrowPosition) -> u64;

    /// Build the protocol-specific liquidation instruction. Allowed to make
    /// blocking RPC calls to fetch reserves, banks, or vaults.
    fn build_liquidate_ix(
        &self,
        rpc: &RpcClient,
        cfg: &AppConfig,
        params: &LiquidationParams,
        liquidator: &Pubkey,
    ) -> Result<Instruction>;
}

/// Composition-root registry that owns one handler per [`ProtocolKind`].
///
/// Dispatch is a single array index by `kind as usize` — no hash lookup.
pub struct Registry {
    handlers: [Box<dyn LendingProtocol>; ProtocolKind::COUNT],
}

impl Registry {
    pub fn new() -> Self {
        Self {
            handlers: [
                Box::new(kamino::KaminoProtocol::new()),
                Box::new(save::SaveProtocol::new()),
                Box::new(marginfi::MarginFiProtocol::new()),
                Box::new(jupiter_lend::JupiterLendProtocol::new()),
            ],
        }
    }

    /// Dispatch to the handler for `kind`.
    pub fn get(&self, kind: ProtocolKind) -> &dyn LendingProtocol {
        &*self.handlers[kind as usize]
    }

    /// Iterate every registered handler (used for logging, diagnostics).
    pub fn iter(&self) -> impl Iterator<Item = &dyn LendingProtocol> + '_ {
        self.handlers.iter().map(|h| h.as_ref())
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}
