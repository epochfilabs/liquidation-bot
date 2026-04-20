//! Jupiter Lend lending program decoder stub.
//!
//! Program ID: jup3YeL8QhtSx1e253b2FDvsMNC87fDrgQZivbrndc9
//!
//! This crate provides program identification for CPI inner instruction
//! walking during liquidation event reconstruction. Full instruction
//! decoding will be added when processing CPI chains.

use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::sync::LazyLock;

pub static PROGRAM_ID: LazyLock<Pubkey> = LazyLock::new(|| {
    Pubkey::from_str("jup3YeL8QhtSx1e253b2FDvsMNC87fDrgQZivbrndc9").unwrap()
});

pub fn is_jupiter_lend_lending_instruction(program_id: &Pubkey) -> bool {
    program_id == &*PROGRAM_ID
}
