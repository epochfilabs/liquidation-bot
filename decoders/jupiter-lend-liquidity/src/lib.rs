//! Jupiter Lend liquidity program decoder stub.
//!
//! Program ID: jupeiUmn818Jg1ekPURTpr4mFo29p46vygyykFJ3wZC
//!
//! This crate provides program identification for CPI inner instruction
//! walking during liquidation event reconstruction. Full instruction
//! decoding will be added when processing CPI chains.

use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::sync::LazyLock;

pub static PROGRAM_ID: LazyLock<Pubkey> = LazyLock::new(|| {
    Pubkey::from_str("jupeiUmn818Jg1ekPURTpr4mFo29p46vygyykFJ3wZC").unwrap()
});

pub fn is_jupiter_lend_liquidity_instruction(program_id: &Pubkey) -> bool {
    program_id == &*PROGRAM_ID
}
