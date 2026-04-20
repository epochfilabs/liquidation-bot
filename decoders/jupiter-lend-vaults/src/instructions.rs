//! Jupiter Lend Vaults instruction decoding.
//!
//! Liquidation instruction: `liquidate`
//!   - Discriminator: [223, 179, 226, 125, 48, 46, 39, 74]
//!   - Args: debt_amt (u64), col_per_unit_debt (u128), absorb (bool),
//!           transfer_type (Option<TransferType>), remaining_accounts_indices (Vec<u8>)
//!   - 26 fixed accounts + remaining (oracle sources, branches, ticks)
//!
//! Jupiter Lend is tick-based. There is no per-position "liquidatee".
//! The instruction liquidates a range of ticks on the vault's orderbook.

use solana_sdk::pubkey::Pubkey;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Discriminators
// ---------------------------------------------------------------------------

pub const LIQUIDATE_DISC: [u8; 8] = [223, 179, 226, 125, 48, 46, 39, 74];

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("instruction data too short: got {got}, need >= {need}")]
    DataTooShort { got: usize, need: usize },
    #[error("unknown discriminator")]
    UnknownDiscriminator,
    #[error("wrong account count: got {got}, need >= {need} for {instruction}")]
    WrongAccountCount {
        got: usize,
        need: usize,
        instruction: String,
    },
    #[error("invalid transfer_type variant: {0}")]
    InvalidTransferType(u8),
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferType {
    Skip = 0,
    Direct = 1,
    Claim = 2,
}

impl TransferType {
    pub fn from_u8(v: u8) -> Result<Self, DecodeError> {
        match v {
            0 => Ok(Self::Skip),
            1 => Ok(Self::Direct),
            2 => Ok(Self::Claim),
            _ => Err(DecodeError::InvalidTransferType(v)),
        }
    }
}

/// Liquidation instruction arguments.
#[derive(Debug, Clone)]
pub struct LiquidateArgs {
    /// Amount of debt to repay.
    pub debt_amt: u64,
    /// Slippage protection: min collateral per unit debt (1e15 precision).
    pub col_per_unit_debt: u128,
    /// Whether to absorb bad-debt ticks above max_tick.
    pub absorb: bool,
    /// Transfer type: Skip (sim only), Direct, or Claim.
    pub transfer_type: Option<TransferType>,
    /// Indices into remaining accounts for oracle_sources, branches, ticks, tick_has_debt.
    pub remaining_accounts_indices: Vec<u8>,
}

/// Liquidation instruction accounts (26 fixed + remaining).
#[derive(Debug, Clone)]
pub struct LiquidateAccounts {
    pub signer: Pubkey,
    pub signer_token_account: Pubkey,
    pub to: Pubkey,
    pub to_token_account: Pubkey,
    pub vault_config: Pubkey,
    pub vault_state: Pubkey,
    pub supply_token: Pubkey,
    pub borrow_token: Pubkey,
    pub oracle: Pubkey,
    pub new_branch: Pubkey,
    pub supply_token_reserves_liquidity: Pubkey,
    pub borrow_token_reserves_liquidity: Pubkey,
    pub vault_supply_position_on_liquidity: Pubkey,
    pub vault_borrow_position_on_liquidity: Pubkey,
    pub supply_rate_model: Pubkey,
    pub borrow_rate_model: Pubkey,
    pub supply_token_claim_account: Pubkey,
    pub liquidity: Pubkey,
    pub liquidity_program: Pubkey,
    pub vault_supply_token_account: Pubkey,
    pub vault_borrow_token_account: Pubkey,
    pub supply_token_program: Pubkey,
    pub borrow_token_program: Pubkey,
    pub system_program: Pubkey,
    pub associated_token_program: Pubkey,
    pub oracle_program: Pubkey,
    /// Remaining: oracle sources, branches, ticks, tick_has_debt.
    pub remaining: Vec<Pubkey>,
}

/// Decoded Jupiter Lend Vaults instruction.
#[derive(Debug, Clone)]
pub enum VaultsInstruction {
    Liquidate {
        args: LiquidateArgs,
        accounts: LiquidateAccounts,
    },
}

impl VaultsInstruction {
    pub fn is_liquidation(&self) -> bool {
        matches!(self, Self::Liquidate { .. })
    }

    /// The signer (liquidator).
    pub fn liquidator(&self) -> &Pubkey {
        match self {
            Self::Liquidate { accounts, .. } => &accounts.signer,
        }
    }

    /// The vault config (acts as "obligation" equivalent).
    pub fn vault_config(&self) -> &Pubkey {
        match self {
            Self::Liquidate { accounts, .. } => &accounts.vault_config,
        }
    }

    /// The oracle account.
    pub fn oracle(&self) -> &Pubkey {
        match self {
            Self::Liquidate { accounts, .. } => &accounts.oracle,
        }
    }
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

pub fn identify(data: &[u8]) -> Result<bool, DecodeError> {
    if data.len() < 8 {
        return Err(DecodeError::DataTooShort {
            got: data.len(),
            need: 8,
        });
    }
    let disc: [u8; 8] = data[..8].try_into().unwrap();
    Ok(disc == LIQUIDATE_DISC)
}

pub fn decode(
    data: &[u8],
    accounts: &[Pubkey],
) -> Result<Option<VaultsInstruction>, DecodeError> {
    if !identify(data)? {
        return Ok(None);
    }

    // Minimum data: 8 disc + 8 debt_amt + 16 col_per_unit + 1 absorb = 33
    if data.len() < 33 {
        return Err(DecodeError::DataTooShort {
            got: data.len(),
            need: 33,
        });
    }

    if accounts.len() < 26 {
        return Err(DecodeError::WrongAccountCount {
            got: accounts.len(),
            need: 26,
            instruction: "liquidate".into(),
        });
    }

    let debt_amt = read_u64(data, 8);
    let col_per_unit_debt = read_u128(data, 16);
    let absorb = data[32] != 0;

    // Parse transfer_type (Option<enum>): byte 33
    let mut offset = 33;
    let transfer_type = if offset < data.len() {
        let option_tag = data[offset];
        offset += 1;
        if option_tag == 0 {
            None
        } else if offset < data.len() {
            let variant = TransferType::from_u8(data[offset])?;
            offset += 1;
            Some(variant)
        } else {
            None
        }
    } else {
        None
    };

    // Parse remaining_accounts_indices (Borsh Vec<u8>: 4-byte LE len + bytes)
    let remaining_accounts_indices = if offset + 4 <= data.len() {
        let len = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        if offset + len <= data.len() {
            data[offset..offset + len].to_vec()
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    let args = LiquidateArgs {
        debt_amt,
        col_per_unit_debt,
        absorb,
        transfer_type,
        remaining_accounts_indices,
    };

    let accts = LiquidateAccounts {
        signer: accounts[0],
        signer_token_account: accounts[1],
        to: accounts[2],
        to_token_account: accounts[3],
        vault_config: accounts[4],
        vault_state: accounts[5],
        supply_token: accounts[6],
        borrow_token: accounts[7],
        oracle: accounts[8],
        new_branch: accounts[9],
        supply_token_reserves_liquidity: accounts[10],
        borrow_token_reserves_liquidity: accounts[11],
        vault_supply_position_on_liquidity: accounts[12],
        vault_borrow_position_on_liquidity: accounts[13],
        supply_rate_model: accounts[14],
        borrow_rate_model: accounts[15],
        supply_token_claim_account: accounts[16],
        liquidity: accounts[17],
        liquidity_program: accounts[18],
        vault_supply_token_account: accounts[19],
        vault_borrow_token_account: accounts[20],
        supply_token_program: accounts[21],
        borrow_token_program: accounts[22],
        system_program: accounts[23],
        associated_token_program: accounts[24],
        oracle_program: accounts[25],
        remaining: accounts[26..].to_vec(),
    };

    Ok(Some(VaultsInstruction::Liquidate {
        args,
        accounts: accts,
    }))
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

fn read_u128(data: &[u8], offset: usize) -> u128 {
    u128::from_le_bytes(data[offset..offset + 16].try_into().unwrap())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_liquidate_data(debt_amt: u64, col_per_unit: u128, absorb: bool) -> Vec<u8> {
        let mut data = Vec::with_capacity(42);
        data.extend_from_slice(&LIQUIDATE_DISC);
        data.extend_from_slice(&debt_amt.to_le_bytes());
        data.extend_from_slice(&col_per_unit.to_le_bytes());
        data.push(if absorb { 1 } else { 0 });
        // transfer_type: Some(Direct)
        data.push(1); // Some
        data.push(1); // Direct
        // remaining_accounts_indices: [0, 1, 2, 3]
        data.extend_from_slice(&4u32.to_le_bytes());
        data.extend_from_slice(&[0, 1, 2, 3]);
        data
    }

    #[test]
    fn decode_liquidate() {
        let data = make_liquidate_data(1_000_000, 500_000, false);
        let accounts: Vec<Pubkey> = (0..30).map(|_| Pubkey::new_unique()).collect();
        let result = decode(&data, &accounts).unwrap().unwrap();

        if let VaultsInstruction::Liquidate { args, accounts: accts } = &result {
            assert_eq!(args.debt_amt, 1_000_000);
            assert_eq!(args.col_per_unit_debt, 500_000);
            assert!(!args.absorb);
            assert_eq!(args.transfer_type, Some(TransferType::Direct));
            assert_eq!(args.remaining_accounts_indices, vec![0, 1, 2, 3]);
            assert_eq!(accts.signer, accounts[0]);
            assert_eq!(accts.vault_config, accounts[4]);
            assert_eq!(accts.oracle, accounts[8]);
            assert_eq!(accts.remaining.len(), 4);
        }
    }

    #[test]
    fn decode_with_absorb() {
        let data = make_liquidate_data(2_000_000, 0, true);
        let accounts: Vec<Pubkey> = (0..26).map(|_| Pubkey::new_unique()).collect();
        let result = decode(&data, &accounts).unwrap().unwrap();

        if let VaultsInstruction::Liquidate { args, .. } = result {
            assert!(args.absorb);
            assert_eq!(args.debt_amt, 2_000_000);
        }
    }

    #[test]
    fn non_liquidate_returns_none() {
        let data = vec![0u8; 40];
        let accounts: Vec<Pubkey> = (0..26).map(|_| Pubkey::new_unique()).collect();
        assert!(decode(&data, &accounts).unwrap().is_none());
    }

    #[test]
    fn too_few_accounts_errors() {
        let data = make_liquidate_data(1_000_000, 0, false);
        let accounts: Vec<Pubkey> = (0..20).map(|_| Pubkey::new_unique()).collect();
        assert!(decode(&data, &accounts).is_err());
    }

    #[test]
    fn identify_works() {
        let mut data = vec![0u8; 8];
        data[..8].copy_from_slice(&LIQUIDATE_DISC);
        assert!(identify(&data).unwrap());
        assert!(!identify(&[0u8; 8]).unwrap());
    }
}
