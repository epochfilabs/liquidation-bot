//! Jupiter Lend reward program decoder stub.
//!
//! Program ID: jup7TthsMgcR9Y3L277b8Eo9uboVSmu1utkuXHNUKar
//!
//! This crate provides program identification for CPI inner instruction
//! walking during liquidation event reconstruction. Full instruction
//! decoding will be added when processing CPI chains.

use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::sync::LazyLock;

pub static PROGRAM_ID: LazyLock<Pubkey> = LazyLock::new(|| {
    Pubkey::from_str("jup7TthsMgcR9Y3L277b8Eo9uboVSmu1utkuXHNUKar").unwrap()
});

pub fn is_jupiter_lend_reward_instruction(program_id: &Pubkey) -> bool {
    program_id == &*PROGRAM_ID
}
