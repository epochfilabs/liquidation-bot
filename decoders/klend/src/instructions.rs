//! Kamino Lend instruction decoding.
//!
//! Liquidation instructions:
//!   - liquidateObligationAndRedeemReserveCollateral (v1)
//!   - liquidateObligationAndRedeemReserveCollateralV2
//!
//! Flash loan instructions:
//!   - flashBorrowReserveLiquidity
//!   - flashRepayReserveLiquidity

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

static LIQUIDATE_V1_DISC: LazyLock<[u8; 8]> = LazyLock::new(|| {
    anchor_disc("global:liquidate_obligation_and_redeem_reserve_collateral")
});

static LIQUIDATE_V2_DISC: LazyLock<[u8; 8]> = LazyLock::new(|| {
    anchor_disc("global:liquidate_obligation_and_redeem_reserve_collateral_v2")
});

static FLASH_BORROW_DISC: LazyLock<[u8; 8]> =
    LazyLock::new(|| anchor_disc("global:flash_borrow_reserve_liquidity"));

static FLASH_REPAY_DISC: LazyLock<[u8; 8]> =
    LazyLock::new(|| anchor_disc("global:flash_repay_reserve_liquidity"));

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("instruction data too short: got {got}, need >= {need}")]
    DataTooShort { got: usize, need: usize },
    #[error("unknown discriminator: {0:?}")]
    UnknownDiscriminator([u8; 8]),
    #[error("wrong number of accounts: got {got}, need >= {need} for {instruction}")]
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
pub enum KlendInstructionKind {
    LiquidateV1,
    LiquidateV2,
    FlashBorrow,
    FlashRepay,
}

impl std::fmt::Display for KlendInstructionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LiquidateV1 => write!(f, "liquidateObligationAndRedeemReserveCollateral"),
            Self::LiquidateV2 => write!(f, "liquidateObligationAndRedeemReserveCollateralV2"),
            Self::FlashBorrow => write!(f, "flashBorrowReserveLiquidity"),
            Self::FlashRepay => write!(f, "flashRepayReserveLiquidity"),
        }
    }
}

/// Liquidation instruction arguments (v1 and v2 share the same args).
#[derive(Debug, Clone)]
pub struct LiquidateArgs {
    pub liquidity_amount: u64,
    pub min_acceptable_received_liquidity_amount: u64,
    pub max_allowed_ltv_override_percent: u64,
}

/// Liquidation v1 accounts (20 fixed + remaining deposit reserves).
#[derive(Debug, Clone)]
pub struct LiquidateV1Accounts {
    pub liquidator: Pubkey,
    pub obligation: Pubkey,
    pub lending_market: Pubkey,
    pub lending_market_authority: Pubkey,
    pub repay_reserve: Pubkey,
    pub repay_reserve_liquidity_mint: Pubkey,
    pub repay_reserve_liquidity_supply: Pubkey,
    pub withdraw_reserve: Pubkey,
    pub withdraw_reserve_liquidity_mint: Pubkey,
    pub withdraw_reserve_collateral_mint: Pubkey,
    pub withdraw_reserve_collateral_supply: Pubkey,
    pub withdraw_reserve_liquidity_supply: Pubkey,
    pub withdraw_reserve_liquidity_fee_receiver: Pubkey,
    pub user_source_liquidity: Pubkey,
    pub user_destination_collateral: Pubkey,
    pub user_destination_liquidity: Pubkey,
    pub collateral_token_program: Pubkey,
    pub repay_liquidity_token_program: Pubkey,
    pub withdraw_liquidity_token_program: Pubkey,
    pub instruction_sysvar: Pubkey,
    /// Additional deposit reserves passed as remaining accounts.
    pub remaining_deposit_reserves: Vec<Pubkey>,
}

/// Flash borrow arguments.
#[derive(Debug, Clone)]
pub struct FlashBorrowArgs {
    pub liquidity_amount: u64,
}

/// Flash borrow accounts (12 fixed).
#[derive(Debug, Clone)]
pub struct FlashBorrowAccounts {
    pub user_transfer_authority: Pubkey,
    pub lending_market_authority: Pubkey,
    pub lending_market: Pubkey,
    pub reserve: Pubkey,
    pub reserve_liquidity_mint: Pubkey,
    pub reserve_liquidity_supply: Pubkey,
    pub user_destination_liquidity: Pubkey,
    pub reserve_liquidity_fee_receiver: Pubkey,
    pub referrer_token_state: Pubkey,
    pub referrer_account: Pubkey,
    pub instruction_sysvar: Pubkey,
    pub token_program: Pubkey,
}

/// Flash repay arguments.
#[derive(Debug, Clone)]
pub struct FlashRepayArgs {
    pub liquidity_amount: u64,
    pub borrow_instruction_index: u8,
}

/// Flash repay accounts (12 fixed).
#[derive(Debug, Clone)]
pub struct FlashRepayAccounts {
    pub user_transfer_authority: Pubkey,
    pub lending_market_authority: Pubkey,
    pub lending_market: Pubkey,
    pub reserve: Pubkey,
    pub reserve_liquidity_mint: Pubkey,
    pub reserve_liquidity_supply: Pubkey,
    pub user_source_liquidity: Pubkey,
    pub reserve_liquidity_fee_receiver: Pubkey,
    pub referrer_token_state: Pubkey,
    pub referrer_account: Pubkey,
    pub instruction_sysvar: Pubkey,
    pub token_program: Pubkey,
}

/// A fully decoded klend instruction.
#[derive(Debug, Clone)]
pub enum KlendInstruction {
    LiquidateV1 {
        args: LiquidateArgs,
        accounts: LiquidateV1Accounts,
    },
    LiquidateV2 {
        args: LiquidateArgs,
        accounts: LiquidateV1Accounts, // Same account structure, v2 adds farm accounts at end
    },
    FlashBorrow {
        args: FlashBorrowArgs,
        accounts: FlashBorrowAccounts,
    },
    FlashRepay {
        args: FlashRepayArgs,
        accounts: FlashRepayAccounts,
    },
}

impl KlendInstruction {
    pub fn kind(&self) -> KlendInstructionKind {
        match self {
            Self::LiquidateV1 { .. } => KlendInstructionKind::LiquidateV1,
            Self::LiquidateV2 { .. } => KlendInstructionKind::LiquidateV2,
            Self::FlashBorrow { .. } => KlendInstructionKind::FlashBorrow,
            Self::FlashRepay { .. } => KlendInstructionKind::FlashRepay,
        }
    }

    pub fn is_liquidation(&self) -> bool {
        matches!(
            self,
            Self::LiquidateV1 { .. } | Self::LiquidateV2 { .. }
        )
    }

    pub fn liquidator(&self) -> Option<&Pubkey> {
        match self {
            Self::LiquidateV1 { accounts, .. } | Self::LiquidateV2 { accounts, .. } => {
                Some(&accounts.liquidator)
            }
            _ => None,
        }
    }

    pub fn obligation(&self) -> Option<&Pubkey> {
        match self {
            Self::LiquidateV1 { accounts, .. } | Self::LiquidateV2 { accounts, .. } => {
                Some(&accounts.obligation)
            }
            _ => None,
        }
    }

    pub fn lending_market(&self) -> Option<&Pubkey> {
        match self {
            Self::LiquidateV1 { accounts, .. } | Self::LiquidateV2 { accounts, .. } => {
                Some(&accounts.lending_market)
            }
            Self::FlashBorrow { accounts, .. } => Some(&accounts.lending_market),
            Self::FlashRepay { accounts, .. } => Some(&accounts.lending_market),
        }
    }

    pub fn liquidity_amount(&self) -> u64 {
        match self {
            Self::LiquidateV1 { args, .. } | Self::LiquidateV2 { args, .. } => {
                args.liquidity_amount
            }
            Self::FlashBorrow { args, .. } => args.liquidity_amount,
            Self::FlashRepay { args, .. } => args.liquidity_amount,
        }
    }
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

/// Identify which klend instruction this is from the discriminator.
/// Returns None for non-liquidation/non-flashloan instructions.
pub fn identify(data: &[u8]) -> Result<Option<KlendInstructionKind>, DecodeError> {
    if data.len() < 8 {
        return Err(DecodeError::DataTooShort {
            got: data.len(),
            need: 8,
        });
    }
    let disc: [u8; 8] = data[..8].try_into().unwrap();
    if disc == *LIQUIDATE_V1_DISC {
        Ok(Some(KlendInstructionKind::LiquidateV1))
    } else if disc == *LIQUIDATE_V2_DISC {
        Ok(Some(KlendInstructionKind::LiquidateV2))
    } else if disc == *FLASH_BORROW_DISC {
        Ok(Some(KlendInstructionKind::FlashBorrow))
    } else if disc == *FLASH_REPAY_DISC {
        Ok(Some(KlendInstructionKind::FlashRepay))
    } else {
        Ok(None) // valid klend instruction, just not one we index
    }
}

/// Fully decode a klend instruction from data + account keys.
pub fn decode(data: &[u8], accounts: &[Pubkey]) -> Result<Option<KlendInstruction>, DecodeError> {
    let kind = match identify(data)? {
        Some(k) => k,
        None => return Ok(None),
    };

    match kind {
        KlendInstructionKind::LiquidateV1 | KlendInstructionKind::LiquidateV2 => {
            // Data: [disc(8)] [liquidity_amount(u64)] [min_received(u64)] [max_ltv_override(u64)]
            if data.len() < 32 {
                return Err(DecodeError::DataTooShort {
                    got: data.len(),
                    need: 32,
                });
            }
            let args = LiquidateArgs {
                liquidity_amount: read_u64(data, 8),
                min_acceptable_received_liquidity_amount: read_u64(data, 16),
                max_allowed_ltv_override_percent: read_u64(data, 24),
            };

            if accounts.len() < 20 {
                return Err(DecodeError::WrongAccountCount {
                    got: accounts.len(),
                    need: 20,
                    instruction: kind.to_string(),
                });
            }

            let accts = LiquidateV1Accounts {
                liquidator: accounts[0],
                obligation: accounts[1],
                lending_market: accounts[2],
                lending_market_authority: accounts[3],
                repay_reserve: accounts[4],
                repay_reserve_liquidity_mint: accounts[5],
                repay_reserve_liquidity_supply: accounts[6],
                withdraw_reserve: accounts[7],
                withdraw_reserve_liquidity_mint: accounts[8],
                withdraw_reserve_collateral_mint: accounts[9],
                withdraw_reserve_collateral_supply: accounts[10],
                withdraw_reserve_liquidity_supply: accounts[11],
                withdraw_reserve_liquidity_fee_receiver: accounts[12],
                user_source_liquidity: accounts[13],
                user_destination_collateral: accounts[14],
                user_destination_liquidity: accounts[15],
                collateral_token_program: accounts[16],
                repay_liquidity_token_program: accounts[17],
                withdraw_liquidity_token_program: accounts[18],
                instruction_sysvar: accounts[19],
                remaining_deposit_reserves: accounts[20..].to_vec(),
            };

            if kind == KlendInstructionKind::LiquidateV1 {
                Ok(Some(KlendInstruction::LiquidateV1 {
                    args,
                    accounts: accts,
                }))
            } else {
                Ok(Some(KlendInstruction::LiquidateV2 {
                    args,
                    accounts: accts,
                }))
            }
        }
        KlendInstructionKind::FlashBorrow => {
            if data.len() < 16 {
                return Err(DecodeError::DataTooShort {
                    got: data.len(),
                    need: 16,
                });
            }
            if accounts.len() < 12 {
                return Err(DecodeError::WrongAccountCount {
                    got: accounts.len(),
                    need: 12,
                    instruction: kind.to_string(),
                });
            }
            let args = FlashBorrowArgs {
                liquidity_amount: read_u64(data, 8),
            };
            let accts = FlashBorrowAccounts {
                user_transfer_authority: accounts[0],
                lending_market_authority: accounts[1],
                lending_market: accounts[2],
                reserve: accounts[3],
                reserve_liquidity_mint: accounts[4],
                reserve_liquidity_supply: accounts[5],
                user_destination_liquidity: accounts[6],
                reserve_liquidity_fee_receiver: accounts[7],
                referrer_token_state: accounts[8],
                referrer_account: accounts[9],
                instruction_sysvar: accounts[10],
                token_program: accounts[11],
            };
            Ok(Some(KlendInstruction::FlashBorrow { args, accounts: accts }))
        }
        KlendInstructionKind::FlashRepay => {
            if data.len() < 17 {
                return Err(DecodeError::DataTooShort {
                    got: data.len(),
                    need: 17,
                });
            }
            if accounts.len() < 12 {
                return Err(DecodeError::WrongAccountCount {
                    got: accounts.len(),
                    need: 12,
                    instruction: kind.to_string(),
                });
            }
            let args = FlashRepayArgs {
                liquidity_amount: read_u64(data, 8),
                borrow_instruction_index: data[16],
            };
            let accts = FlashRepayAccounts {
                user_transfer_authority: accounts[0],
                lending_market_authority: accounts[1],
                lending_market: accounts[2],
                reserve: accounts[3],
                reserve_liquidity_mint: accounts[4],
                reserve_liquidity_supply: accounts[5],
                user_source_liquidity: accounts[6],
                reserve_liquidity_fee_receiver: accounts[7],
                referrer_token_state: accounts[8],
                referrer_account: accounts[9],
                instruction_sysvar: accounts[10],
                token_program: accounts[11],
            };
            Ok(Some(KlendInstruction::FlashRepay { args, accounts: accts }))
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
    fn discriminator_values() {
        // Validated against existing bot code and on-chain data
        assert_eq!(*LIQUIDATE_V1_DISC, [177, 71, 154, 188, 226, 133, 74, 55]);
        assert_eq!(*FLASH_BORROW_DISC, [135, 231, 52, 167, 7, 52, 212, 193]);
        assert_eq!(*FLASH_REPAY_DISC, [185, 117, 0, 203, 96, 245, 180, 186]);
    }

    #[test]
    fn identify_liquidation_v1() {
        let mut data = vec![0u8; 32];
        data[..8].copy_from_slice(&*LIQUIDATE_V1_DISC);
        let kind = identify(&data).unwrap();
        assert_eq!(kind, Some(KlendInstructionKind::LiquidateV1));
    }

    #[test]
    fn identify_unknown_disc_returns_none() {
        let data = vec![0u8; 32];
        let kind = identify(&data).unwrap();
        assert_eq!(kind, None);
    }

    #[test]
    fn decode_liquidate_v1() {
        let mut data = vec![0u8; 32];
        data[..8].copy_from_slice(&*LIQUIDATE_V1_DISC);
        data[8..16].copy_from_slice(&1_000_000u64.to_le_bytes());
        data[16..24].copy_from_slice(&0u64.to_le_bytes());
        data[24..32].copy_from_slice(&0u64.to_le_bytes());

        let accounts: Vec<Pubkey> = (0..22).map(|_| Pubkey::new_unique()).collect();
        let result = decode(&data, &accounts).unwrap().unwrap();

        assert!(result.is_liquidation());
        assert_eq!(result.kind(), KlendInstructionKind::LiquidateV1);
        assert_eq!(result.liquidity_amount(), 1_000_000);
        assert_eq!(result.liquidator(), Some(&accounts[0]));
        assert_eq!(result.obligation(), Some(&accounts[1]));
        assert_eq!(result.lending_market(), Some(&accounts[2]));

        if let KlendInstruction::LiquidateV1 { accounts: accts, .. } = &result {
            assert_eq!(accts.repay_reserve, accounts[4]);
            assert_eq!(accts.withdraw_reserve, accounts[7]);
            assert_eq!(accts.remaining_deposit_reserves.len(), 2);
        }
    }

    #[test]
    fn decode_flash_borrow() {
        let mut data = vec![0u8; 16];
        data[..8].copy_from_slice(&*FLASH_BORROW_DISC);
        data[8..16].copy_from_slice(&5_000_000u64.to_le_bytes());

        let accounts: Vec<Pubkey> = (0..12).map(|_| Pubkey::new_unique()).collect();
        let result = decode(&data, &accounts).unwrap().unwrap();

        assert!(!result.is_liquidation());
        assert_eq!(result.kind(), KlendInstructionKind::FlashBorrow);
        assert_eq!(result.liquidity_amount(), 5_000_000);
    }

    #[test]
    fn decode_flash_repay() {
        let mut data = vec![0u8; 17];
        data[..8].copy_from_slice(&*FLASH_REPAY_DISC);
        data[8..16].copy_from_slice(&5_000_000u64.to_le_bytes());
        data[16] = 2; // borrow_instruction_index

        let accounts: Vec<Pubkey> = (0..12).map(|_| Pubkey::new_unique()).collect();
        let result = decode(&data, &accounts).unwrap().unwrap();

        if let KlendInstruction::FlashRepay { args, .. } = result {
            assert_eq!(args.liquidity_amount, 5_000_000);
            assert_eq!(args.borrow_instruction_index, 2);
        } else {
            panic!("expected FlashRepay");
        }
    }

    #[test]
    fn too_few_accounts_errors() {
        let mut data = vec![0u8; 32];
        data[..8].copy_from_slice(&*LIQUIDATE_V1_DISC);
        let accounts: Vec<Pubkey> = (0..10).map(|_| Pubkey::new_unique()).collect();
        assert!(decode(&data, &accounts).is_err());
    }
}
