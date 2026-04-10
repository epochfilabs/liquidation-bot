//! Jupiter Lend protocol implementation.
//!
//! Built on Fluid/Instadapp technology. Uses NFT-based positions and a
//! tick-based liquidation system.
//!
//! Program IDs:
//!   Vaults:     jupr81YtYssSyPt8jbnGuiWon5f6x9TcDEFxYe3Bdzi
//!   Flash Loan: jupgfSgfuAXv4B6R2Uxu85Z1qdzgju79s6MfZekN6XS
//!   Lending:    jup3YeL8QhtSx1e253b2FDvsMNC87fDrgQZivbrndc9
//!
//! Position accounts are 71 bytes. VaultConfig 219 bytes. VaultState 127 bytes.
//! Discriminators and offsets validated against 43K+ live mainnet positions.

use anyhow::{bail, Result};
use solana_sdk::pubkey::Pubkey;

use super::{
    BorrowPosition, DepositPosition, HealthResult, LendingProtocol, Positions, ProtocolKind,
};

pub const VAULTS_PROGRAM_ID: &str = "jupr81YtYssSyPt8jbnGuiWon5f6x9TcDEFxYe3Bdzi";
pub const FLASH_LOAN_PROGRAM_ID: &str = "jupgfSgfuAXv4B6R2Uxu85Z1qdzgju79s6MfZekN6XS";
pub const LENDING_PROGRAM_ID: &str = "jup3YeL8QhtSx1e253b2FDvsMNC87fDrgQZivbrndc9";

/// Position account discriminator.
const POSITION_DISCRIMINATOR: [u8; 8] = [0xaa, 0xbc, 0x8f, 0xe4, 0x7a, 0x40, 0xf7, 0xd0];

/// Position account size (validated: 71 bytes).
const POSITION_ACCOUNT_SIZE: usize = 71;

/// Position struct layout (after 8-byte discriminator):
///   +8:  vault_id (u16, 2)
///   +10: nft_id (u32, 4)
///   +14: position_mint (Pubkey, 32)
///   +46: is_supply_only_position (u8, 1)
///   +47: tick (i32, 4)
///   +51: tick_id (u32, 4)
///   +55: supply_amount (u64, 8)
///   +63: dust_debt_amount (u64, 8)
mod position_offsets {
    pub const VAULT_ID: usize = 8;
    pub const NFT_ID: usize = 10;
    pub const POSITION_MINT: usize = 14;
    pub const IS_SUPPLY_ONLY: usize = 46;
    pub const TICK: usize = 47;
    pub const SUPPLY_AMOUNT: usize = 55;
    pub const DUST_DEBT: usize = 63;
}

/// VaultConfig struct layout (219 bytes, validated):
///   +8:  vault_id (u16)
///   +14: collateral_factor (u16, /1000)
///   +16: liquidation_threshold (u16, /1000)
///   +18: liquidation_max_limit (u16, /1000)
///   +22: liquidation_penalty (u16, /10000)
///   +154: supply_token (Pubkey)
///   +186: borrow_token (Pubkey)
pub const VAULT_CONFIG_SIZE: usize = 219;

mod vault_config_offsets {
    pub const VAULT_ID: usize = 8;
    pub const COLLATERAL_FACTOR: usize = 14;
    pub const LIQUIDATION_THRESHOLD: usize = 16;
    pub const LIQUIDATION_MAX_LIMIT: usize = 18;
    pub const LIQUIDATION_PENALTY: usize = 22;
    pub const SUPPLY_TOKEN: usize = 154;
    pub const BORROW_TOKEN: usize = 186;
}

/// VaultState struct layout (127 bytes, validated):
///   +8:  vault_id (u16)
///   +10: branch_liquidated (u8)
///   +11: topmost_tick (i32)
///   +23: total_supply (u64)
///   +31: total_borrow (u64)
///   +39: total_positions (u32)
pub const VAULT_STATE_SIZE: usize = 127;

mod vault_state_offsets {
    pub const VAULT_ID: usize = 8;
    pub const TOPMOST_TICK: usize = 11;
    pub const TOTAL_SUPPLY: usize = 23;
    pub const TOTAL_BORROW: usize = 31;
}

/// Parsed position data.
#[derive(Debug, Clone)]
pub struct JupiterPosition {
    pub vault_id: u16,
    pub nft_id: u32,
    pub position_mint: Pubkey,
    pub is_supply_only: bool,
    pub tick: i32,
    pub supply_amount: u64,
    pub dust_debt_amount: u64,
}

/// Parsed vault configuration.
#[derive(Debug, Clone)]
pub struct JupiterVaultConfig {
    pub vault_id: u16,
    pub collateral_factor: u16,
    pub liquidation_threshold: u16,
    pub liquidation_max_limit: u16,
    pub liquidation_penalty: u16,
    pub supply_token: Pubkey,
    pub borrow_token: Pubkey,
}

pub struct JupiterLendProtocol {
    pub program_id: Pubkey,
}

impl JupiterLendProtocol {
    pub fn new() -> Self {
        Self {
            program_id: VAULTS_PROGRAM_ID.parse().unwrap(),
        }
    }
}

impl LendingProtocol for JupiterLendProtocol {
    fn kind(&self) -> ProtocolKind {
        ProtocolKind::JupiterLend
    }

    fn program_id(&self) -> Pubkey {
        self.program_id
    }

    fn is_position_account(&self, data: &[u8]) -> bool {
        data.len() == POSITION_ACCOUNT_SIZE && data[..8] == POSITION_DISCRIMINATOR
    }

    fn evaluate_health(&self, data: &[u8]) -> Result<HealthResult> {
        let pos = parse_position(data)?;

        // Supply-only positions are never liquidatable
        if pos.is_supply_only {
            return Ok(HealthResult {
                current_ltv: 0.0,
                unhealthy_ltv: 1.0,
                is_liquidatable: false,
                deposited_value_usd: 0.0,
                borrowed_value_usd: 0.0,
            });
        }

        // A position with tick == i32::MIN has no active borrow
        if pos.tick == i32::MIN {
            return Ok(HealthResult {
                current_ltv: 0.0,
                unhealthy_ltv: 1.0,
                is_liquidatable: false,
                deposited_value_usd: pos.supply_amount as f64,
                borrowed_value_usd: 0.0,
            });
        }

        // In Jupiter Lend's tick-based system, a position is liquidatable when
        // the vault's oracle-derived liquidation tick >= the position's tick.
        // Without VaultState (which has topmost_tick) and oracle data, we can
        // detect positions that HAVE borrows and flag them for detailed on-chain
        // evaluation.
        //
        // The tick represents the debt/collateral ratio: higher tick = more debt.
        // We mark positions with active borrows (tick != i32::MIN and supply > 0)
        // as needing evaluation. The actual liquidation check happens when we
        // fetch VaultState.topmost_tick and compare.

        // Rough LTV estimate from tick: ratio = 1.0015^tick
        // This is an approximation — the exact formula uses 2^48 scaling.
        let ratio = (1.0015_f64).powi(pos.tick);
        let approx_ltv = ratio; // debt/collateral ratio

        Ok(HealthResult {
            current_ltv: approx_ltv,
            unhealthy_ltv: 1.0, // placeholder — real threshold from VaultConfig
            is_liquidatable: false, // requires VaultState comparison
            deposited_value_usd: pos.supply_amount as f64,
            borrowed_value_usd: pos.dust_debt_amount as f64,
        })
    }

    fn parse_positions(&self, data: &[u8]) -> Result<Positions> {
        let pos = parse_position(data)?;

        // Jupiter uses vault_id to link to config/state — we encode it
        // as a deterministic Pubkey for the market field.
        let vault_market = derive_vault_config_pda(pos.vault_id);

        let mut deposits = Vec::new();
        let mut borrows = Vec::new();

        if pos.supply_amount > 0 {
            deposits.push(DepositPosition {
                reserve: vault_market,
                mint: Some(pos.position_mint),
                amount: pos.supply_amount,
                market_value_usd: 0.0, // needs oracle
            });
        }

        if !pos.is_supply_only && pos.tick != i32::MIN {
            borrows.push(BorrowPosition {
                reserve: vault_market,
                mint: None,
                amount_sf: pos.dust_debt_amount as u128,
                market_value_usd: 0.0, // needs oracle
            });
        }

        Ok(Positions {
            deposits,
            borrows,
            market: vault_market,
            owner: pos.position_mint, // NFT holder owns the position
        })
    }
}

/// Parse a Position account from raw data.
pub fn parse_position(data: &[u8]) -> Result<JupiterPosition> {
    if data.len() < POSITION_ACCOUNT_SIZE {
        bail!("jupiter position too small: {} bytes", data.len());
    }
    if data[..8] != POSITION_DISCRIMINATOR {
        bail!("not a jupiter position account");
    }

    Ok(JupiterPosition {
        vault_id: read_u16(data, position_offsets::VAULT_ID),
        nft_id: read_u32(data, position_offsets::NFT_ID),
        position_mint: read_pubkey(data, position_offsets::POSITION_MINT),
        is_supply_only: data[position_offsets::IS_SUPPLY_ONLY] != 0,
        tick: read_i32(data, position_offsets::TICK),
        supply_amount: read_u64(data, position_offsets::SUPPLY_AMOUNT),
        dust_debt_amount: read_u64(data, position_offsets::DUST_DEBT),
    })
}

/// Parse a VaultConfig account from raw data.
pub fn parse_vault_config(data: &[u8]) -> Result<JupiterVaultConfig> {
    if data.len() < VAULT_CONFIG_SIZE {
        bail!("jupiter vault config too small: {} bytes", data.len());
    }

    Ok(JupiterVaultConfig {
        vault_id: read_u16(data, vault_config_offsets::VAULT_ID),
        collateral_factor: read_u16(data, vault_config_offsets::COLLATERAL_FACTOR),
        liquidation_threshold: read_u16(data, vault_config_offsets::LIQUIDATION_THRESHOLD),
        liquidation_max_limit: read_u16(data, vault_config_offsets::LIQUIDATION_MAX_LIMIT),
        liquidation_penalty: read_u16(data, vault_config_offsets::LIQUIDATION_PENALTY),
        supply_token: read_pubkey(data, vault_config_offsets::SUPPLY_TOKEN),
        borrow_token: read_pubkey(data, vault_config_offsets::BORROW_TOKEN),
    })
}

/// Derive VaultConfig PDA: ["vault_config", vault_id.to_le_bytes()]
fn derive_vault_config_pda(vault_id: u16) -> Pubkey {
    let program_id: Pubkey = VAULTS_PROGRAM_ID.parse().unwrap();
    let (pda, _) = Pubkey::find_program_address(
        &[b"vault_config", &vault_id.to_le_bytes()],
        &program_id,
    );
    pda
}

fn read_pubkey(data: &[u8], offset: usize) -> Pubkey {
    Pubkey::new_from_array(data[offset..offset + 32].try_into().unwrap())
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap())
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn read_i32(data: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_position(vault_id: u16, tick: i32, supply: u64, dust_debt: u64, supply_only: bool) -> Vec<u8> {
        let mut data = vec![0u8; POSITION_ACCOUNT_SIZE];
        data[..8].copy_from_slice(&POSITION_DISCRIMINATOR);
        data[8..10].copy_from_slice(&vault_id.to_le_bytes());
        data[10..14].copy_from_slice(&1u32.to_le_bytes()); // nft_id
        // position_mint = zeros (Pubkey::default)
        data[46] = if supply_only { 1 } else { 0 };
        data[47..51].copy_from_slice(&tick.to_le_bytes());
        data[55..63].copy_from_slice(&supply.to_le_bytes());
        data[63..71].copy_from_slice(&dust_debt.to_le_bytes());
        data
    }

    #[test]
    fn detects_position_account() {
        let proto = JupiterLendProtocol::new();
        let data = make_position(1, 100, 1000, 0, false);
        assert!(proto.is_position_account(&data));

        // Wrong discriminator
        let mut bad = data.clone();
        bad[0] = 0;
        assert!(!proto.is_position_account(&bad));

        // Wrong size
        assert!(!proto.is_position_account(&data[..50]));
    }

    #[test]
    fn supply_only_not_liquidatable() {
        let proto = JupiterLendProtocol::new();
        let data = make_position(1, 100, 1000, 0, true);
        let health = proto.evaluate_health(&data).unwrap();
        assert!(!health.is_liquidatable);
        assert_eq!(health.current_ltv, 0.0);
    }

    #[test]
    fn no_borrow_not_liquidatable() {
        let proto = JupiterLendProtocol::new();
        let data = make_position(1, i32::MIN, 1000, 0, false);
        let health = proto.evaluate_health(&data).unwrap();
        assert!(!health.is_liquidatable);
    }

    #[test]
    fn active_borrow_computes_ltv() {
        let proto = JupiterLendProtocol::new();
        let data = make_position(1, 3280, 1000, 50, false);
        let health = proto.evaluate_health(&data).unwrap();
        // tick=3280, ratio ≈ 1.0015^3280 ≈ 135.5
        assert!(health.current_ltv > 100.0);
    }

    #[test]
    fn parses_positions_with_borrow() {
        let proto = JupiterLendProtocol::new();
        let data = make_position(3, 1000, 5000, 100, false);
        let positions = proto.parse_positions(&data).unwrap();
        assert_eq!(positions.deposits.len(), 1);
        assert_eq!(positions.borrows.len(), 1);
        assert_eq!(positions.deposits[0].amount, 5000);
    }
}
