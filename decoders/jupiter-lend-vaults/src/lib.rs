//! Jupiter Lend Vaults program decoder.
//!
//! Program ID: jupr81YtYssSyPt8jbnGuiWon5f6x9TcDEFxYe3Bdzi
//!
//! Contains the `liquidate` instruction. Jupiter Lend is tick-based: liquidation
//! operates on ranges of ticks, not individual positions. There is no specific
//! "liquidatee" in the transaction.
//!
//! IDL source: https://github.com/jup-ag/jupiter-lend/blob/main/target/idl/vaults.json

pub mod instructions;

use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::sync::LazyLock;

pub static PROGRAM_ID: LazyLock<Pubkey> = LazyLock::new(|| {
    Pubkey::from_str("jupr81YtYssSyPt8jbnGuiWon5f6x9TcDEFxYe3Bdzi").unwrap()
});

pub fn is_vaults_instruction(program_id: &Pubkey) -> bool {
    program_id == &*PROGRAM_ID
}
