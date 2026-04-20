//! Kamino Lend (klend) instruction decoder.
//!
//! Program ID: KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD
//!
//! Decodes liquidation-related instructions using Anchor discriminators
//! (SHA256("global:<instruction_name>")[..8]).
//!
//! IDL source: https://github.com/Kamino-Finance/klend-sdk/blob/master/src/idl/klend.json
//! Program source: https://github.com/Kamino-Finance/klend

pub mod instructions;

use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::sync::LazyLock;

/// Kamino Lend program ID (mainnet).
pub static PROGRAM_ID: LazyLock<Pubkey> = LazyLock::new(|| {
    Pubkey::from_str("KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD").unwrap()
});

pub fn is_klend_instruction(program_id: &Pubkey) -> bool {
    program_id == &*PROGRAM_ID
}
