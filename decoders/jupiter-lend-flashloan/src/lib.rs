//! Jupiter Lend flashloan program decoder stub.
//!
//! Program ID: jupgfSgfuAXv4B6R2Uxu85Z1qdzgju79s6MfZekN6XS
//!
//! This crate provides program identification for CPI inner instruction
//! walking during liquidation event reconstruction. Full instruction
//! decoding will be added when processing CPI chains.

use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::sync::LazyLock;

pub static PROGRAM_ID: LazyLock<Pubkey> = LazyLock::new(|| {
    Pubkey::from_str("jupgfSgfuAXv4B6R2Uxu85Z1qdzgju79s6MfZekN6XS").unwrap()
});

pub fn is_jupiter_lend_flashloan_instruction(program_id: &Pubkey) -> bool {
    program_id == &*PROGRAM_ID
}
