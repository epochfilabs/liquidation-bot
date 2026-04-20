//! MarginFi v2 instruction decoding.
//!
//! Liquidation: `lendingAccountLiquidate`
//!   - Discriminator: [0xd6, 0xa9, 0x97, 0xd5, 0xfb, 0xa7, 0x56, 0xdb]
//!   - Arg: asset_amount (u64) — collateral to seize, NOT debt amount
//!   - 10 fixed accounts + remaining (oracles, observation banks)
//!
//! Flash loan: `lendingAccountStartFlashloan` / `lendingAccountEndFlashloan`

use sha2::{Digest, Sha256};
use solana_sdk::pubkey::Pubkey;
use std::sync::LazyLock;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Discriminators
// ---------------------------------------------------------------------------

fn anchor_disc(name: &str) -> [u8; 8] {
    let hash = Sha256::digest(name.as_bytes());
    let mut disc = [0u8; 8];
    disc.copy_from_slice(&hash[..8]);
    disc
}

/// Known discriminator for lendingAccountLiquidate.
pub const LIQUIDATE_DISC: [u8; 8] = [0xd6, 0xa9, 0x97, 0xd5, 0xfb, 0xa7, 0x56, 0xdb];

static START_FLASHLOAN_DISC: LazyLock<[u8; 8]> =
    LazyLock::new(|| anchor_disc("global:lending_account_start_flashloan"));

static END_FLASHLOAN_DISC: LazyLock<[u8; 8]> =
    LazyLock::new(|| anchor_disc("global:lending_account_end_flashloan"));

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
}

// ---------------------------------------------------------------------------
// Instruction types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarginfiInstructionKind {
    Liquidate,
    StartFlashloan,
    EndFlashloan,
}

impl std::fmt::Display for MarginfiInstructionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Liquidate => write!(f, "lendingAccountLiquidate"),
            Self::StartFlashloan => write!(f, "lendingAccountStartFlashloan"),
            Self::EndFlashloan => write!(f, "lendingAccountEndFlashloan"),
        }
    }
}

/// Liquidation instruction arguments.
#[derive(Debug, Clone)]
pub struct LiquidateArgs {
    /// Amount of collateral asset to seize (native units).
    /// NOTE: This is collateral, not debt. Unique to MarginFi.
    pub asset_amount: u64,
}

/// Liquidation fixed accounts (10).
#[derive(Debug, Clone)]
pub struct LiquidateAccounts {
    pub marginfi_group: Pubkey,
    pub asset_bank: Pubkey,
    pub liab_bank: Pubkey,
    pub liquidator_marginfi_account: Pubkey,
    pub authority: Pubkey,
    pub liquidatee_marginfi_account: Pubkey,
    pub bank_liquidity_vault_authority: Pubkey,
    pub bank_liquidity_vault: Pubkey,
    pub bank_insurance_vault: Pubkey,
    pub token_program: Pubkey,
    /// Remaining accounts: [liab_mint_ai?, asset_oracle, liab_oracle,
    ///   liquidator_obs_banks..., liquidatee_obs_banks...]
    pub remaining: Vec<Pubkey>,
}

/// Start flashloan arguments.
#[derive(Debug, Clone)]
pub struct StartFlashloanArgs {
    pub end_index: u64,
}

/// Start flashloan accounts.
#[derive(Debug, Clone)]
pub struct StartFlashloanAccounts {
    pub marginfi_account: Pubkey,
    pub authority: Pubkey,
    pub ixs_sysvar: Pubkey,
}

/// End flashloan accounts.
#[derive(Debug, Clone)]
pub struct EndFlashloanAccounts {
    pub marginfi_account: Pubkey,
    pub authority: Pubkey,
    /// Remaining: observation bank accounts for health check.
    pub remaining: Vec<Pubkey>,
}

/// Decoded MarginFi instruction.
#[derive(Debug, Clone)]
pub enum MarginfiInstruction {
    Liquidate {
        args: LiquidateArgs,
        accounts: LiquidateAccounts,
    },
    StartFlashloan {
        args: StartFlashloanArgs,
        accounts: StartFlashloanAccounts,
    },
    EndFlashloan {
        accounts: EndFlashloanAccounts,
    },
}

impl MarginfiInstruction {
    pub fn kind(&self) -> MarginfiInstructionKind {
        match self {
            Self::Liquidate { .. } => MarginfiInstructionKind::Liquidate,
            Self::StartFlashloan { .. } => MarginfiInstructionKind::StartFlashloan,
            Self::EndFlashloan { .. } => MarginfiInstructionKind::EndFlashloan,
        }
    }

    pub fn is_liquidation(&self) -> bool {
        matches!(self, Self::Liquidate { .. })
    }

    /// The liquidator's signing authority.
    pub fn liquidator(&self) -> Option<&Pubkey> {
        match self {
            Self::Liquidate { accounts, .. } => Some(&accounts.authority),
            _ => None,
        }
    }

    /// The liquidatee's MarginfiAccount.
    pub fn liquidatee_account(&self) -> Option<&Pubkey> {
        match self {
            Self::Liquidate { accounts, .. } => Some(&accounts.liquidatee_marginfi_account),
            _ => None,
        }
    }

    /// The MarginFi group (acts as "lending market").
    pub fn group(&self) -> Option<&Pubkey> {
        match self {
            Self::Liquidate { accounts, .. } => Some(&accounts.marginfi_group),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

pub fn identify(data: &[u8]) -> Result<Option<MarginfiInstructionKind>, DecodeError> {
    if data.len() < 8 {
        return Err(DecodeError::DataTooShort {
            got: data.len(),
            need: 8,
        });
    }
    let disc: [u8; 8] = data[..8].try_into().unwrap();
    if disc == LIQUIDATE_DISC {
        Ok(Some(MarginfiInstructionKind::Liquidate))
    } else if disc == *START_FLASHLOAN_DISC {
        Ok(Some(MarginfiInstructionKind::StartFlashloan))
    } else if disc == *END_FLASHLOAN_DISC {
        Ok(Some(MarginfiInstructionKind::EndFlashloan))
    } else {
        Ok(None)
    }
}

pub fn decode(
    data: &[u8],
    accounts: &[Pubkey],
) -> Result<Option<MarginfiInstruction>, DecodeError> {
    let kind = match identify(data)? {
        Some(k) => k,
        None => return Ok(None),
    };

    match kind {
        MarginfiInstructionKind::Liquidate => {
            if data.len() < 16 {
                return Err(DecodeError::DataTooShort {
                    got: data.len(),
                    need: 16,
                });
            }
            if accounts.len() < 10 {
                return Err(DecodeError::WrongAccountCount {
                    got: accounts.len(),
                    need: 10,
                    instruction: kind.to_string(),
                });
            }
            let args = LiquidateArgs {
                asset_amount: read_u64(data, 8),
            };
            let accts = LiquidateAccounts {
                marginfi_group: accounts[0],
                asset_bank: accounts[1],
                liab_bank: accounts[2],
                liquidator_marginfi_account: accounts[3],
                authority: accounts[4],
                liquidatee_marginfi_account: accounts[5],
                bank_liquidity_vault_authority: accounts[6],
                bank_liquidity_vault: accounts[7],
                bank_insurance_vault: accounts[8],
                token_program: accounts[9],
                remaining: accounts[10..].to_vec(),
            };
            Ok(Some(MarginfiInstruction::Liquidate {
                args,
                accounts: accts,
            }))
        }
        MarginfiInstructionKind::StartFlashloan => {
            if data.len() < 16 {
                return Err(DecodeError::DataTooShort {
                    got: data.len(),
                    need: 16,
                });
            }
            if accounts.len() < 3 {
                return Err(DecodeError::WrongAccountCount {
                    got: accounts.len(),
                    need: 3,
                    instruction: kind.to_string(),
                });
            }
            Ok(Some(MarginfiInstruction::StartFlashloan {
                args: StartFlashloanArgs {
                    end_index: read_u64(data, 8),
                },
                accounts: StartFlashloanAccounts {
                    marginfi_account: accounts[0],
                    authority: accounts[1],
                    ixs_sysvar: accounts[2],
                },
            }))
        }
        MarginfiInstructionKind::EndFlashloan => {
            if accounts.len() < 2 {
                return Err(DecodeError::WrongAccountCount {
                    got: accounts.len(),
                    need: 2,
                    instruction: kind.to_string(),
                });
            }
            Ok(Some(MarginfiInstruction::EndFlashloan {
                accounts: EndFlashloanAccounts {
                    marginfi_account: accounts[0],
                    authority: accounts[1],
                    remaining: accounts[2..].to_vec(),
                },
            }))
        }
    }
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liquidate_discriminator_matches() {
        assert_eq!(LIQUIDATE_DISC, [0xd6, 0xa9, 0x97, 0xd5, 0xfb, 0xa7, 0x56, 0xdb]);
    }

    #[test]
    fn decode_liquidate() {
        let mut data = vec![0u8; 16];
        data[..8].copy_from_slice(&LIQUIDATE_DISC);
        data[8..16].copy_from_slice(&500_000u64.to_le_bytes());

        let accounts: Vec<Pubkey> = (0..14).map(|_| Pubkey::new_unique()).collect();
        let result = decode(&data, &accounts).unwrap().unwrap();

        assert!(result.is_liquidation());
        assert_eq!(result.kind(), MarginfiInstructionKind::Liquidate);

        if let MarginfiInstruction::Liquidate { args, accounts: accts } = &result {
            assert_eq!(args.asset_amount, 500_000);
            assert_eq!(accts.marginfi_group, accounts[0]);
            assert_eq!(accts.asset_bank, accounts[1]);
            assert_eq!(accts.liab_bank, accounts[2]);
            assert_eq!(accts.authority, accounts[4]);
            assert_eq!(accts.liquidatee_marginfi_account, accounts[5]);
            assert_eq!(accts.remaining.len(), 4);
        }
    }

    #[test]
    fn decode_start_flashloan() {
        let mut data = vec![0u8; 16];
        data[..8].copy_from_slice(&*START_FLASHLOAN_DISC);
        data[8..16].copy_from_slice(&5u64.to_le_bytes());

        let accounts: Vec<Pubkey> = (0..3).map(|_| Pubkey::new_unique()).collect();
        let result = decode(&data, &accounts).unwrap().unwrap();

        if let MarginfiInstruction::StartFlashloan { args, .. } = result {
            assert_eq!(args.end_index, 5);
        } else {
            panic!("expected StartFlashloan");
        }
    }

    #[test]
    fn decode_end_flashloan() {
        let data = END_FLASHLOAN_DISC.to_vec();
        let accounts: Vec<Pubkey> = (0..5).map(|_| Pubkey::new_unique()).collect();
        let result = decode(&data, &accounts).unwrap().unwrap();

        if let MarginfiInstruction::EndFlashloan { accounts: accts } = result {
            assert_eq!(accts.remaining.len(), 3);
        } else {
            panic!("expected EndFlashloan");
        }
    }

    #[test]
    fn unknown_disc_returns_none() {
        let data = vec![0u8; 16];
        let accounts: Vec<Pubkey> = (0..10).map(|_| Pubkey::new_unique()).collect();
        assert!(decode(&data, &accounts).unwrap().is_none());
    }

    #[test]
    fn accessor_methods() {
        let mut data = vec![0u8; 16];
        data[..8].copy_from_slice(&LIQUIDATE_DISC);
        let accounts: Vec<Pubkey> = (0..10).map(|_| Pubkey::new_unique()).collect();
        let ix = decode(&data, &accounts).unwrap().unwrap();
        assert_eq!(ix.liquidator(), Some(&accounts[4]));
        assert_eq!(ix.liquidatee_account(), Some(&accounts[5]));
        assert_eq!(ix.group(), Some(&accounts[0]));
    }
}
