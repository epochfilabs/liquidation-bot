//! Jupiter Lend oracle program decoder stub.
//!
//! Program ID: jupnw4B6Eqs7ft6rxpzYLJZYSnrpRgPcr589n5Kv4oc
//!
//! This crate provides program identification for CPI inner instruction
//! walking during liquidation event reconstruction. Full instruction
//! decoding will be added when processing CPI chains.

use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::sync::LazyLock;

pub static PROGRAM_ID: LazyLock<Pubkey> = LazyLock::new(|| {
    Pubkey::from_str("jupnw4B6Eqs7ft6rxpzYLJZYSnrpRgPcr589n5Kv4oc").unwrap()
});

pub fn is_jupiter_lend_oracle_instruction(program_id: &Pubkey) -> bool {
    program_id == &*PROGRAM_ID
}
