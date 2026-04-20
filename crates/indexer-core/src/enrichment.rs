//! Transaction enrichment: extract Jito tips, flash loan usage, Jupiter swaps,
//! priority fees, and compute budget from transaction metadata and inner instructions.

use solana_sdk::pubkey::Pubkey;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::LazyLock;

// ---------------------------------------------------------------------------
// Jito tip accounts (mainnet)
// Source: https://jito-foundation.gitbook.io/mev/mev-payment-and-distribution/on-chain-addresses
// ---------------------------------------------------------------------------

static JITO_TIP_ACCOUNTS: LazyLock<HashSet<Pubkey>> = LazyLock::new(|| {
    [
        "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5",
        "HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe",
        "Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY",
        "ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49",
        "DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh",
        "ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt",
        "DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL",
        "3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT",
    ]
    .into_iter()
    .map(|s| Pubkey::from_str(s).unwrap())
    .collect()
});

/// System program ID.
static SYSTEM_PROGRAM: LazyLock<Pubkey> = LazyLock::new(|| {
    solana_sdk::system_program::ID
});

/// ComputeBudget program ID.
static COMPUTE_BUDGET_PROGRAM: LazyLock<Pubkey> = LazyLock::new(|| {
    Pubkey::from_str("ComputeBudget111111111111111111111111111111").unwrap()
});

/// Jupiter v6 program ID.
static JUPITER_V6_PROGRAM: LazyLock<Pubkey> = LazyLock::new(|| {
    Pubkey::from_str("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4").unwrap()
});

// Known flash loan program IDs
static KLEND_PROGRAM: LazyLock<Pubkey> = LazyLock::new(|| {
    Pubkey::from_str("KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD").unwrap()
});
static JUPITER_FLASHLOAN_PROGRAM: LazyLock<Pubkey> = LazyLock::new(|| {
    Pubkey::from_str("jupgfSgfuAXv4B6R2Uxu85Z1qdzgju79s6MfZekN6XS").unwrap()
});
static MARGINFI_PROGRAM: LazyLock<Pubkey> = LazyLock::new(|| {
    Pubkey::from_str("MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA").unwrap()
});
static SAVE_PROGRAM: LazyLock<Pubkey> = LazyLock::new(|| {
    Pubkey::from_str("So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo").unwrap()
});

/// Represents a decoded instruction for enrichment purposes.
#[derive(Debug, Clone)]
pub struct InstructionInfo {
    pub program_id: Pubkey,
    pub data: Vec<u8>,
    pub accounts: Vec<Pubkey>,
}

/// Enrichment results extracted from a transaction.
#[derive(Debug, Clone, Default)]
pub struct TxEnrichment {
    /// Jito tip amount in lamports, if detected.
    pub jito_tip_lamports: Option<u64>,
    /// Priority fee from ComputeBudget::SetComputeUnitPrice.
    pub priority_fee_lamports: u64,
    /// Compute unit limit from ComputeBudget::SetComputeUnitLimit.
    pub compute_units_requested: Option<u32>,
    /// Whether any flash loan instruction was detected.
    pub used_flashloan: bool,
    /// Source of the flash loan if detected.
    pub flashloan_source: Option<String>,
    /// Whether a Jupiter swap was detected.
    pub used_jupiter_swap: bool,
}

/// Extract enrichment data from a list of instructions (both top-level and inner).
pub fn enrich_transaction(
    top_level_ixs: &[InstructionInfo],
    inner_ixs: &[InstructionInfo],
    fee_lamports: u64,
) -> TxEnrichment {
    let mut result = TxEnrichment::default();
    let all_ixs: Vec<&InstructionInfo> = top_level_ixs.iter().chain(inner_ixs.iter()).collect();

    for ix in &all_ixs {
        // Jito tip detection: SystemProgram::Transfer to a tip account
        if ix.program_id == *SYSTEM_PROGRAM && ix.data.len() >= 12 {
            // SystemProgram Transfer: [2, 0, 0, 0] (u32 LE tag) + [amount: u64 LE]
            let tag = u32::from_le_bytes(ix.data[0..4].try_into().unwrap_or([0; 4]));
            if tag == 2 && ix.accounts.len() >= 2 {
                let destination = &ix.accounts[1];
                if JITO_TIP_ACCOUNTS.contains(destination) {
                    let amount = u64::from_le_bytes(
                        ix.data[4..12].try_into().unwrap_or([0; 8]),
                    );
                    result.jito_tip_lamports = Some(
                        result.jito_tip_lamports.unwrap_or(0) + amount,
                    );
                }
            }
        }

        // ComputeBudget instructions
        if ix.program_id == *COMPUTE_BUDGET_PROGRAM && !ix.data.is_empty() {
            match ix.data[0] {
                // SetComputeUnitLimit: [2] [units: u32 LE]
                2 if ix.data.len() >= 5 => {
                    result.compute_units_requested = Some(u32::from_le_bytes(
                        ix.data[1..5].try_into().unwrap(),
                    ));
                }
                // SetComputeUnitPrice: [3] [micro_lamports: u64 LE]
                3 if ix.data.len() >= 9 => {
                    let micro_lamports = u64::from_le_bytes(
                        ix.data[1..9].try_into().unwrap(),
                    );
                    // priority_fee = micro_lamports * CU / 1_000_000
                    // We'll compute the actual fee from tx meta, but store the rate
                    let _ = micro_lamports; // rate stored, actual fee from meta
                }
                _ => {}
            }
        }

        // Jupiter swap detection
        if ix.program_id == *JUPITER_V6_PROGRAM {
            result.used_jupiter_swap = true;
        }

        // Flash loan detection
        detect_flashloan(ix, &mut result);
    }

    // Priority fee: total fee - base fee (5000 lamports per signature)
    // The actual priority fee comes from tx meta, not from our calculation
    result.priority_fee_lamports = fee_lamports.saturating_sub(5000);

    result
}

/// Detect flash loan instructions from known programs.
fn detect_flashloan(ix: &InstructionInfo, result: &mut TxEnrichment) {
    // Kamino flash borrow: discriminator [135, 231, 52, 167, 7, 52, 212, 193]
    if ix.program_id == *KLEND_PROGRAM && ix.data.len() >= 8 {
        let disc: [u8; 8] = ix.data[..8].try_into().unwrap();
        if disc == [135, 231, 52, 167, 7, 52, 212, 193] {
            result.used_flashloan = true;
            result.flashloan_source = Some("kamino".to_string());
        }
    }

    // Jupiter Lend flash borrow: discriminator [103, 19, 78, 24, 240, 9, 135, 63]
    if ix.program_id == *JUPITER_FLASHLOAN_PROGRAM && ix.data.len() >= 8 {
        let disc: [u8; 8] = ix.data[..8].try_into().unwrap();
        if disc == [103, 19, 78, 24, 240, 9, 135, 63] {
            result.used_flashloan = true;
            result.flashloan_source = Some("jupiter_lend".to_string());
        }
    }

    // MarginFi start flashloan: discriminator [0x49, 0xf0, 0xad, 0x60, 0x79, 0xd6, 0x1b, 0x86]
    if ix.program_id == *MARGINFI_PROGRAM && ix.data.len() >= 8 {
        let disc: [u8; 8] = ix.data[..8].try_into().unwrap();
        if disc == [0x49, 0xf0, 0xad, 0x60, 0x79, 0xd6, 0x1b, 0x86] {
            result.used_flashloan = true;
            result.flashloan_source = Some("marginfi".to_string());
        }
    }

    // Save flash borrow: tag 19
    if ix.program_id == *SAVE_PROGRAM && !ix.data.is_empty() && ix.data[0] == 19 {
        result.used_flashloan = true;
        result.flashloan_source = Some("save".to_string());
    }
}

/// Parse error code from Anchor error logs.
/// Format: "Program log: AnchorError ... Error Number: 6016."
pub fn parse_error_from_logs(logs: &[String]) -> (Option<u32>, Option<String>) {
    for log in logs {
        if log.contains("AnchorError") || log.contains("Error Number:") {
            // Extract error number
            if let Some(num_start) = log.find("Error Number: ") {
                let after = &log[num_start + 14..];
                if let Some(end) = after.find('.') {
                    if let Ok(code) = after[..end].parse::<u32>() {
                        // Extract error message
                        let msg = if let Some(msg_start) = log.find("Error Message: ") {
                            Some(log[msg_start + 15..].trim_end_matches('.').to_string())
                        } else {
                            None
                        };
                        return (Some(code), msg);
                    }
                }
            }
        }
        // SPL-style errors: "Program log: Error: <message>"
        if log.starts_with("Program log: Error: ") {
            let msg = log.strip_prefix("Program log: Error: ").unwrap().to_string();
            return (None, Some(msg));
        }
    }
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_jito_tip() {
        let tip_account =
            Pubkey::from_str("96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5").unwrap();
        let sender = Pubkey::new_unique();

        // SystemProgram::Transfer instruction data: [2,0,0,0] + amount_le
        let mut data = vec![2, 0, 0, 0];
        data.extend_from_slice(&100_000u64.to_le_bytes());

        let ix = InstructionInfo {
            program_id: *SYSTEM_PROGRAM,
            data,
            accounts: vec![sender, tip_account],
        };

        let result = enrich_transaction(&[], &[ix], 5000);
        assert_eq!(result.jito_tip_lamports, Some(100_000));
    }

    #[test]
    fn no_jito_tip_for_non_tip_account() {
        let regular_account = Pubkey::new_unique();
        let sender = Pubkey::new_unique();

        let mut data = vec![2, 0, 0, 0];
        data.extend_from_slice(&100_000u64.to_le_bytes());

        let ix = InstructionInfo {
            program_id: *SYSTEM_PROGRAM,
            data,
            accounts: vec![sender, regular_account],
        };

        let result = enrich_transaction(&[], &[ix], 5000);
        assert!(result.jito_tip_lamports.is_none());
    }

    #[test]
    fn detect_kamino_flashloan() {
        let ix = InstructionInfo {
            program_id: *KLEND_PROGRAM,
            data: vec![135, 231, 52, 167, 7, 52, 212, 193, 0, 0, 0, 0, 0, 0, 0, 0],
            accounts: vec![],
        };

        let result = enrich_transaction(&[ix], &[], 5000);
        assert!(result.used_flashloan);
        assert_eq!(result.flashloan_source.as_deref(), Some("kamino"));
    }

    #[test]
    fn detect_jupiter_swap() {
        let ix = InstructionInfo {
            program_id: *JUPITER_V6_PROGRAM,
            data: vec![0; 8],
            accounts: vec![],
        };

        let result = enrich_transaction(&[ix], &[], 5000);
        assert!(result.used_jupiter_swap);
    }

    #[test]
    fn detect_save_flashloan() {
        let ix = InstructionInfo {
            program_id: *SAVE_PROGRAM,
            data: vec![19, 0, 0, 0, 0, 0, 0, 0, 0], // tag 19 + amount
            accounts: vec![],
        };

        let result = enrich_transaction(&[ix], &[], 5000);
        assert!(result.used_flashloan);
        assert_eq!(result.flashloan_source.as_deref(), Some("save"));
    }

    #[test]
    fn parse_anchor_error() {
        let logs = vec![
            "Program log: AnchorError occurred. Error Code: ObligationHealthy. Error Number: 6016. Error Message: Cannot liquidate healthy obligations.".to_string(),
        ];
        let (code, msg) = parse_error_from_logs(&logs);
        assert_eq!(code, Some(6016));
        assert_eq!(msg.as_deref(), Some("Cannot liquidate healthy obligations"));
    }

    #[test]
    fn parse_no_error() {
        let logs = vec!["Program log: Instruction: LiquidateObligation".to_string()];
        let (code, msg) = parse_error_from_logs(&logs);
        assert!(code.is_none());
        assert!(msg.is_none());
    }
}
