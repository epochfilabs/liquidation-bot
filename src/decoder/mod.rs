/// Kamino Lend account decoder.
///
/// We decode obligation and reserve accounts directly from raw bytes
/// using known struct offsets from the klend program source.
///
/// The `carbon-kamino-lending-decoder` crate can be integrated later
/// once dependency versions stabilize. For now, raw deserialization
/// gives us full control and zero version-conflict risk.

use sha2::{Sha256, Digest};
use std::sync::LazyLock;

/// Anchor discriminator for the Obligation account.
/// = sha256("account:Obligation")[..8]
pub static OBLIGATION_DISCRIMINATOR: LazyLock<[u8; 8]> = LazyLock::new(|| {
    let hash = Sha256::digest(b"account:Obligation");
    let mut disc = [0u8; 8];
    disc.copy_from_slice(&hash[..8]);
    disc
});

/// Check if raw account data has the Obligation discriminator.
pub fn is_obligation_account(data: &[u8]) -> bool {
    data.len() >= 8 && data[..8] == *OBLIGATION_DISCRIMINATOR
}
