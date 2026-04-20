//! Save (Solend) instruction decoding.
//!
//! All instructions use a u8 tag as the first byte, followed by Borsh-encoded
//! arguments. This module decodes the liquidation-related instructions:
//!
//! - Tag 12: LiquidateObligation (returns cTokens)
//! - Tag 17: LiquidateObligationAndRedeemReserveCollateral (returns underlying)
//! - Tag 19: FlashBorrowReserveLiquidity
//! - Tag 20: FlashRepayReserveLiquidity

use solana_sdk::pubkey::Pubkey;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Instruction tags
// ---------------------------------------------------------------------------

/// Full LendingInstruction tag enum. We only decode liquidation-related variants
/// but include all tags for completeness and to support filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum InstructionTag {
    InitLendingMarket = 0,
    SetLendingMarketOwnerAndConfig = 1,
    InitReserve = 2,
    RefreshReserve = 3,
    DepositReserveLiquidity = 4,
    RedeemReserveCollateral = 5,
    InitObligation = 6,
    RefreshObligation = 7,
    DepositObligationCollateral = 8,
    WithdrawObligationCollateral = 9,
    BorrowObligationLiquidity = 10,
    RepayObligationLiquidity = 11,
    LiquidateObligation = 12,
    FlashLoan = 13,
    DepositReserveLiquidityAndObligationCollateral = 14,
    WithdrawObligationCollateralAndRedeemReserveCollateral = 15,
    UpdateReserveConfig = 16,
    LiquidateObligationAndRedeemReserveCollateral = 17,
    RedeemFees = 18,
    FlashBorrowReserveLiquidity = 19,
    FlashRepayReserveLiquidity = 20,
}

impl InstructionTag {
    pub fn from_u8(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Self::InitLendingMarket),
            1 => Some(Self::SetLendingMarketOwnerAndConfig),
            2 => Some(Self::InitReserve),
            3 => Some(Self::RefreshReserve),
            4 => Some(Self::DepositReserveLiquidity),
            5 => Some(Self::RedeemReserveCollateral),
            6 => Some(Self::InitObligation),
            7 => Some(Self::RefreshObligation),
            8 => Some(Self::DepositObligationCollateral),
            9 => Some(Self::WithdrawObligationCollateral),
            10 => Some(Self::BorrowObligationLiquidity),
            11 => Some(Self::RepayObligationLiquidity),
            12 => Some(Self::LiquidateObligation),
            13 => Some(Self::FlashLoan),
            14 => Some(Self::DepositReserveLiquidityAndObligationCollateral),
            15 => Some(Self::WithdrawObligationCollateralAndRedeemReserveCollateral),
            16 => Some(Self::UpdateReserveConfig),
            17 => Some(Self::LiquidateObligationAndRedeemReserveCollateral),
            18 => Some(Self::RedeemFees),
            19 => Some(Self::FlashBorrowReserveLiquidity),
            20 => Some(Self::FlashRepayReserveLiquidity),
            _ => None,
        }
    }

    /// Whether this tag represents a liquidation instruction.
    pub fn is_liquidation(&self) -> bool {
        matches!(
            self,
            Self::LiquidateObligation | Self::LiquidateObligationAndRedeemReserveCollateral
        )
    }

    /// Whether this tag is relevant to the indexer (liquidation + flash loan).
    pub fn is_indexer_relevant(&self) -> bool {
        matches!(
            self,
            Self::LiquidateObligation
                | Self::LiquidateObligationAndRedeemReserveCollateral
                | Self::FlashBorrowReserveLiquidity
                | Self::FlashRepayReserveLiquidity
                | Self::RefreshReserve
                | Self::RefreshObligation
        )
    }
}

impl std::fmt::Display for InstructionTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

// ---------------------------------------------------------------------------
// Decode errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("instruction data is empty")]
    EmptyData,
    #[error("unknown instruction tag: {0}")]
    UnknownTag(u8),
    #[error("instruction data too short: got {got} bytes, need {need}")]
    DataTooShort { got: usize, need: usize },
    #[error("wrong number of accounts: got {got}, need {need} for {instruction}")]
    WrongAccountCount {
        got: usize,
        need: usize,
        instruction: String,
    },
}

// ---------------------------------------------------------------------------
// Decoded instruction types
// ---------------------------------------------------------------------------

/// Decoded LiquidateObligation (tag 12).
/// Liquidator receives cTokens (not redeemed to underlying).
#[derive(Debug, Clone)]
pub struct LiquidateObligation {
    pub liquidity_amount: u64,
    pub accounts: LiquidateObligationAccounts,
}

/// Accounts for LiquidateObligation (tag 12) — 11 accounts.
#[derive(Debug, Clone)]
pub struct LiquidateObligationAccounts {
    pub source_liquidity: Pubkey,
    pub destination_collateral: Pubkey,
    pub repay_reserve: Pubkey,
    pub repay_reserve_liquidity_supply: Pubkey,
    pub withdraw_reserve: Pubkey,
    pub withdraw_reserve_collateral_supply: Pubkey,
    pub obligation: Pubkey,
    pub lending_market: Pubkey,
    pub lending_market_authority: Pubkey,
    pub user_transfer_authority: Pubkey,
    pub token_program: Pubkey,
}

/// Decoded LiquidateObligationAndRedeemReserveCollateral (tag 17).
/// Preferred variant — redeems cTokens to underlying in the same instruction.
#[derive(Debug, Clone)]
pub struct LiquidateObligationAndRedeem {
    pub liquidity_amount: u64,
    pub accounts: LiquidateObligationAndRedeemAccounts,
}

/// Accounts for tag 17 — 15 accounts.
#[derive(Debug, Clone)]
pub struct LiquidateObligationAndRedeemAccounts {
    pub source_liquidity: Pubkey,
    pub destination_collateral: Pubkey,
    pub destination_liquidity: Pubkey,
    pub repay_reserve: Pubkey,
    pub repay_reserve_liquidity_supply: Pubkey,
    pub withdraw_reserve: Pubkey,
    pub withdraw_reserve_collateral_mint: Pubkey,
    pub withdraw_reserve_collateral_supply: Pubkey,
    pub withdraw_reserve_liquidity_supply: Pubkey,
    pub withdraw_reserve_liquidity_fee_receiver: Pubkey,
    pub obligation: Pubkey,
    pub lending_market: Pubkey,
    pub lending_market_authority: Pubkey,
    pub user_transfer_authority: Pubkey,
    pub token_program: Pubkey,
}

/// Decoded FlashBorrowReserveLiquidity (tag 19).
#[derive(Debug, Clone)]
pub struct FlashBorrowReserveLiquidity {
    pub liquidity_amount: u64,
    pub accounts: FlashBorrowAccounts,
}

/// Accounts for tag 19 — 7 accounts.
#[derive(Debug, Clone)]
pub struct FlashBorrowAccounts {
    pub source_liquidity: Pubkey,
    pub destination_liquidity: Pubkey,
    pub reserve: Pubkey,
    pub lending_market: Pubkey,
    pub lending_market_authority: Pubkey,
    pub instructions_sysvar: Pubkey,
    pub token_program: Pubkey,
}

/// Decoded FlashRepayReserveLiquidity (tag 20).
#[derive(Debug, Clone)]
pub struct FlashRepayReserveLiquidity {
    pub liquidity_amount: u64,
    pub borrow_instruction_index: u8,
    pub accounts: FlashRepayAccounts,
}

/// Accounts for tag 20 — 9 accounts.
#[derive(Debug, Clone)]
pub struct FlashRepayAccounts {
    pub source_liquidity: Pubkey,
    pub destination_liquidity: Pubkey,
    pub reserve_liquidity_fee_receiver: Pubkey,
    pub host_fee_receiver: Pubkey,
    pub reserve: Pubkey,
    pub lending_market: Pubkey,
    pub user_transfer_authority: Pubkey,
    pub instructions_sysvar: Pubkey,
    pub token_program: Pubkey,
}

/// A decoded Save instruction (any liquidation-relevant variant).
#[derive(Debug, Clone)]
pub enum SaveInstruction {
    LiquidateObligation(LiquidateObligation),
    LiquidateObligationAndRedeem(LiquidateObligationAndRedeem),
    FlashBorrow(FlashBorrowReserveLiquidity),
    FlashRepay(FlashRepayReserveLiquidity),
}

impl SaveInstruction {
    /// The instruction tag.
    pub fn tag(&self) -> InstructionTag {
        match self {
            Self::LiquidateObligation(_) => InstructionTag::LiquidateObligation,
            Self::LiquidateObligationAndRedeem(_) => {
                InstructionTag::LiquidateObligationAndRedeemReserveCollateral
            }
            Self::FlashBorrow(_) => InstructionTag::FlashBorrowReserveLiquidity,
            Self::FlashRepay(_) => InstructionTag::FlashRepayReserveLiquidity,
        }
    }

    /// Whether this is a liquidation instruction.
    pub fn is_liquidation(&self) -> bool {
        self.tag().is_liquidation()
    }

    /// The liquidator pubkey (signer).
    pub fn liquidator(&self) -> Option<&Pubkey> {
        match self {
            Self::LiquidateObligation(ix) => Some(&ix.accounts.user_transfer_authority),
            Self::LiquidateObligationAndRedeem(ix) => Some(&ix.accounts.user_transfer_authority),
            _ => None,
        }
    }

    /// The obligation pubkey being liquidated.
    pub fn obligation(&self) -> Option<&Pubkey> {
        match self {
            Self::LiquidateObligation(ix) => Some(&ix.accounts.obligation),
            Self::LiquidateObligationAndRedeem(ix) => Some(&ix.accounts.obligation),
            _ => None,
        }
    }

    /// The lending market.
    pub fn lending_market(&self) -> Option<&Pubkey> {
        match self {
            Self::LiquidateObligation(ix) => Some(&ix.accounts.lending_market),
            Self::LiquidateObligationAndRedeem(ix) => Some(&ix.accounts.lending_market),
            Self::FlashBorrow(ix) => Some(&ix.accounts.lending_market),
            Self::FlashRepay(ix) => Some(&ix.accounts.lending_market),
        }
    }
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

/// Decode a Save instruction from raw instruction data and account keys.
///
/// Only decodes liquidation-relevant instructions (tags 12, 17, 19, 20).
/// Returns `None` for other valid tags (not an error, just not relevant).
/// Returns `Err` for malformed data.
pub fn decode(data: &[u8], accounts: &[Pubkey]) -> Result<Option<SaveInstruction>, DecodeError> {
    if data.is_empty() {
        return Err(DecodeError::EmptyData);
    }

    let tag = data[0];
    let parsed_tag = InstructionTag::from_u8(tag).ok_or(DecodeError::UnknownTag(tag))?;

    if !parsed_tag.is_indexer_relevant() {
        return Ok(None);
    }

    match parsed_tag {
        InstructionTag::LiquidateObligation => {
            let liquidity_amount = read_u64(data, 1)?;
            let accts = arrange_liquidate_accounts(accounts)?;
            Ok(Some(SaveInstruction::LiquidateObligation(
                LiquidateObligation {
                    liquidity_amount,
                    accounts: accts,
                },
            )))
        }
        InstructionTag::LiquidateObligationAndRedeemReserveCollateral => {
            let liquidity_amount = read_u64(data, 1)?;
            let accts = arrange_liquidate_and_redeem_accounts(accounts)?;
            Ok(Some(SaveInstruction::LiquidateObligationAndRedeem(
                LiquidateObligationAndRedeem {
                    liquidity_amount,
                    accounts: accts,
                },
            )))
        }
        InstructionTag::FlashBorrowReserveLiquidity => {
            let liquidity_amount = read_u64(data, 1)?;
            let accts = arrange_flash_borrow_accounts(accounts)?;
            Ok(Some(SaveInstruction::FlashBorrow(
                FlashBorrowReserveLiquidity {
                    liquidity_amount,
                    accounts: accts,
                },
            )))
        }
        InstructionTag::FlashRepayReserveLiquidity => {
            let liquidity_amount = read_u64(data, 1)?;
            if data.len() < 10 {
                return Err(DecodeError::DataTooShort { got: data.len(), need: 10 });
            }
            let borrow_instruction_index = data[9];
            let accts = arrange_flash_repay_accounts(accounts)?;
            Ok(Some(SaveInstruction::FlashRepay(
                FlashRepayReserveLiquidity {
                    liquidity_amount,
                    borrow_instruction_index,
                    accounts: accts,
                },
            )))
        }
        _ => Ok(None), // RefreshReserve, RefreshObligation — relevant but not decoded here
    }
}

/// Identify the instruction tag without full decoding.
pub fn identify_tag(data: &[u8]) -> Result<InstructionTag, DecodeError> {
    if data.is_empty() {
        return Err(DecodeError::EmptyData);
    }
    InstructionTag::from_u8(data[0]).ok_or(DecodeError::UnknownTag(data[0]))
}

// ---------------------------------------------------------------------------
// Account arrangement helpers
// ---------------------------------------------------------------------------

fn arrange_liquidate_accounts(
    accounts: &[Pubkey],
) -> Result<LiquidateObligationAccounts, DecodeError> {
    if accounts.len() < 11 {
        return Err(DecodeError::WrongAccountCount {
            got: accounts.len(),
            need: 11,
            instruction: "LiquidateObligation".into(),
        });
    }
    Ok(LiquidateObligationAccounts {
        source_liquidity: accounts[0],
        destination_collateral: accounts[1],
        repay_reserve: accounts[2],
        repay_reserve_liquidity_supply: accounts[3],
        withdraw_reserve: accounts[4],
        withdraw_reserve_collateral_supply: accounts[5],
        obligation: accounts[6],
        lending_market: accounts[7],
        lending_market_authority: accounts[8],
        user_transfer_authority: accounts[9],
        token_program: accounts[10],
    })
}

fn arrange_liquidate_and_redeem_accounts(
    accounts: &[Pubkey],
) -> Result<LiquidateObligationAndRedeemAccounts, DecodeError> {
    if accounts.len() < 15 {
        return Err(DecodeError::WrongAccountCount {
            got: accounts.len(),
            need: 15,
            instruction: "LiquidateObligationAndRedeemReserveCollateral".into(),
        });
    }
    Ok(LiquidateObligationAndRedeemAccounts {
        source_liquidity: accounts[0],
        destination_collateral: accounts[1],
        destination_liquidity: accounts[2],
        repay_reserve: accounts[3],
        repay_reserve_liquidity_supply: accounts[4],
        withdraw_reserve: accounts[5],
        withdraw_reserve_collateral_mint: accounts[6],
        withdraw_reserve_collateral_supply: accounts[7],
        withdraw_reserve_liquidity_supply: accounts[8],
        withdraw_reserve_liquidity_fee_receiver: accounts[9],
        obligation: accounts[10],
        lending_market: accounts[11],
        lending_market_authority: accounts[12],
        user_transfer_authority: accounts[13],
        token_program: accounts[14],
    })
}

fn arrange_flash_borrow_accounts(
    accounts: &[Pubkey],
) -> Result<FlashBorrowAccounts, DecodeError> {
    if accounts.len() < 7 {
        return Err(DecodeError::WrongAccountCount {
            got: accounts.len(),
            need: 7,
            instruction: "FlashBorrowReserveLiquidity".into(),
        });
    }
    Ok(FlashBorrowAccounts {
        source_liquidity: accounts[0],
        destination_liquidity: accounts[1],
        reserve: accounts[2],
        lending_market: accounts[3],
        lending_market_authority: accounts[4],
        instructions_sysvar: accounts[5],
        token_program: accounts[6],
    })
}

fn arrange_flash_repay_accounts(
    accounts: &[Pubkey],
) -> Result<FlashRepayAccounts, DecodeError> {
    if accounts.len() < 9 {
        return Err(DecodeError::WrongAccountCount {
            got: accounts.len(),
            need: 9,
            instruction: "FlashRepayReserveLiquidity".into(),
        });
    }
    Ok(FlashRepayAccounts {
        source_liquidity: accounts[0],
        destination_liquidity: accounts[1],
        reserve_liquidity_fee_receiver: accounts[2],
        host_fee_receiver: accounts[3],
        reserve: accounts[4],
        lending_market: accounts[5],
        user_transfer_authority: accounts[6],
        instructions_sysvar: accounts[7],
        token_program: accounts[8],
    })
}

// ---------------------------------------------------------------------------
// Byte reading helpers
// ---------------------------------------------------------------------------

fn read_u64(data: &[u8], offset: usize) -> Result<u64, DecodeError> {
    if data.len() < offset + 8 {
        return Err(DecodeError::DataTooShort {
            got: data.len(),
            need: offset + 8,
        });
    }
    let bytes: [u8; 8] = data[offset..offset + 8].try_into().unwrap();
    Ok(u64::from_le_bytes(bytes))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_round_trip() {
        for tag in 0..=20u8 {
            let parsed = InstructionTag::from_u8(tag);
            assert!(parsed.is_some(), "tag {} should parse", tag);
            assert_eq!(parsed.unwrap() as u8, tag);
        }
        assert!(InstructionTag::from_u8(21).is_none());
        assert!(InstructionTag::from_u8(255).is_none());
    }

    #[test]
    fn liquidation_tags_correct() {
        assert!(InstructionTag::LiquidateObligation.is_liquidation());
        assert!(InstructionTag::LiquidateObligationAndRedeemReserveCollateral.is_liquidation());
        assert!(!InstructionTag::FlashBorrowReserveLiquidity.is_liquidation());
        assert!(!InstructionTag::RefreshReserve.is_liquidation());
    }

    #[test]
    fn decode_liquidate_obligation_tag_12() {
        let amount: u64 = 1_000_000;
        let mut data = vec![12u8];
        data.extend_from_slice(&amount.to_le_bytes());

        let accounts: Vec<Pubkey> = (0..11).map(|_| Pubkey::new_unique()).collect();
        let result = decode(&data, &accounts).unwrap().unwrap();

        match result {
            SaveInstruction::LiquidateObligation(ix) => {
                assert_eq!(ix.liquidity_amount, 1_000_000);
                assert_eq!(ix.accounts.source_liquidity, accounts[0]);
                assert_eq!(ix.accounts.obligation, accounts[6]);
                assert_eq!(ix.accounts.user_transfer_authority, accounts[9]);
            }
            _ => panic!("expected LiquidateObligation"),
        }
    }

    #[test]
    fn decode_liquidate_and_redeem_tag_17() {
        let amount: u64 = 2_500_000;
        let mut data = vec![17u8];
        data.extend_from_slice(&amount.to_le_bytes());

        let accounts: Vec<Pubkey> = (0..15).map(|_| Pubkey::new_unique()).collect();
        let result = decode(&data, &accounts).unwrap().unwrap();

        match result {
            SaveInstruction::LiquidateObligationAndRedeem(ref ix) => {
                assert_eq!(ix.liquidity_amount, 2_500_000);
                assert_eq!(ix.accounts.source_liquidity, accounts[0]);
                assert_eq!(ix.accounts.destination_collateral, accounts[1]);
                assert_eq!(ix.accounts.destination_liquidity, accounts[2]);
                assert_eq!(ix.accounts.obligation, accounts[10]);
                assert_eq!(ix.accounts.user_transfer_authority, accounts[13]);
                assert!(result.is_liquidation());
            }
            _ => panic!("expected LiquidateObligationAndRedeem"),
        }
    }

    #[test]
    fn decode_flash_borrow_tag_19() {
        let amount: u64 = 5_000_000;
        let mut data = vec![19u8];
        data.extend_from_slice(&amount.to_le_bytes());

        let accounts: Vec<Pubkey> = (0..7).map(|_| Pubkey::new_unique()).collect();
        let result = decode(&data, &accounts).unwrap().unwrap();

        match result {
            SaveInstruction::FlashBorrow(ix) => {
                assert_eq!(ix.liquidity_amount, 5_000_000);
                assert_eq!(ix.accounts.source_liquidity, accounts[0]);
                assert_eq!(ix.accounts.reserve, accounts[2]);
            }
            _ => panic!("expected FlashBorrow"),
        }
    }

    #[test]
    fn decode_flash_repay_tag_20() {
        let amount: u64 = 5_000_000;
        let mut data = vec![20u8];
        data.extend_from_slice(&amount.to_le_bytes());
        data.push(3); // borrow_instruction_index

        let accounts: Vec<Pubkey> = (0..9).map(|_| Pubkey::new_unique()).collect();
        let result = decode(&data, &accounts).unwrap().unwrap();

        match result {
            SaveInstruction::FlashRepay(ix) => {
                assert_eq!(ix.liquidity_amount, 5_000_000);
                assert_eq!(ix.borrow_instruction_index, 3);
                assert_eq!(ix.accounts.user_transfer_authority, accounts[6]);
            }
            _ => panic!("expected FlashRepay"),
        }
    }

    #[test]
    fn irrelevant_tag_returns_none() {
        let data = vec![4u8, 0, 0, 0, 0, 0, 0, 0, 0]; // DepositReserveLiquidity
        let accounts: Vec<Pubkey> = (0..10).map(|_| Pubkey::new_unique()).collect();
        let result = decode(&data, &accounts).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn empty_data_errors() {
        let accounts: Vec<Pubkey> = vec![];
        assert!(decode(&[], &accounts).is_err());
    }

    #[test]
    fn too_few_accounts_errors() {
        let data = vec![17u8, 0, 0, 0, 0, 0, 0, 0, 0];
        let accounts: Vec<Pubkey> = (0..10).map(|_| Pubkey::new_unique()).collect();
        let result = decode(&data, &accounts);
        assert!(result.is_err());
    }

    #[test]
    fn accessor_methods() {
        let amount: u64 = 100;
        let mut data = vec![17u8];
        data.extend_from_slice(&amount.to_le_bytes());
        let accounts: Vec<Pubkey> = (0..15).map(|_| Pubkey::new_unique()).collect();
        let ix = decode(&data, &accounts).unwrap().unwrap();

        assert_eq!(ix.liquidator(), Some(&accounts[13]));
        assert_eq!(ix.obligation(), Some(&accounts[10]));
        assert_eq!(ix.lending_market(), Some(&accounts[11]));
        assert!(ix.is_liquidation());
    }
}
