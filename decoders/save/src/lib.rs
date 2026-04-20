//! Save (Solend) instruction decoder.
//!
//! Program ID: So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo
//!
//! Save uses SPL token-lending style Borsh encoding. The first byte is a u8
//! instruction tag, NOT an 8-byte Anchor discriminator. This decoder is
//! hand-written from the solendprotocol/solana-program-library mainnet branch.
//!
//! Source: https://github.com/solendprotocol/solana-program-library
//!         branch: mainnet
//!         file: token-lending/program/src/instruction.rs

pub mod instructions;
pub mod accounts;

use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::sync::LazyLock;

/// Save (Solend) program ID (mainnet production).
pub static PROGRAM_ID: LazyLock<Pubkey> = LazyLock::new(|| {
    Pubkey::from_str("So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo").unwrap()
});

/// Check whether an instruction belongs to the Save program.
pub fn is_save_instruction(program_id: &Pubkey) -> bool {
    program_id == &*PROGRAM_ID
}
