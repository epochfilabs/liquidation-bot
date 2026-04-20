//! MarginFi v2 instruction decoder.
//!
//! Program ID: MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA
//!
//! Decodes liquidation and flash loan instructions from Anchor discriminators.
//! MarginFi is unique: the liquidation argument is `asset_amount` (collateral to
//! seize), not a debt amount. The liquidator must have their own MarginfiAccount.
//!
//! Source: https://github.com/mrgnlabs/marginfi-v2

pub mod instructions;

use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::sync::LazyLock;

pub static PROGRAM_ID: LazyLock<Pubkey> = LazyLock::new(|| {
    Pubkey::from_str("MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA").unwrap()
});

pub fn is_marginfi_instruction(program_id: &Pubkey) -> bool {
    program_id == &*PROGRAM_ID
}
