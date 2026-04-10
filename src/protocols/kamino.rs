//! Kamino Lend (klend) protocol implementation.
//!
//! Wraps the existing obligation/health and obligation/positions modules
//! into the LendingProtocol trait.

use anyhow::Result;
use solana_sdk::pubkey::Pubkey;

use super::{
    BorrowPosition, DepositPosition, HealthResult, LendingProtocol, Positions, ProtocolKind,
};
use crate::obligation::{health, positions};

/// Kamino Lend program ID (mainnet production).
pub const PROGRAM_ID: &str = "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD";

/// Expected obligation account size.
pub const OBLIGATION_ACCOUNT_SIZE: usize = 3344;

const SF_SHIFT: f64 = (1u128 << 60) as f64;

pub struct KaminoProtocol {
    pub program_id: Pubkey,
}

impl KaminoProtocol {
    pub fn new() -> Self {
        Self {
            program_id: PROGRAM_ID.parse().unwrap(),
        }
    }
}

impl LendingProtocol for KaminoProtocol {
    fn kind(&self) -> ProtocolKind {
        ProtocolKind::Kamino
    }

    fn program_id(&self) -> Pubkey {
        self.program_id
    }

    fn is_position_account(&self, data: &[u8]) -> bool {
        data.len() == OBLIGATION_ACCOUNT_SIZE && crate::decoder::is_obligation_account(data)
    }

    fn evaluate_health(&self, data: &[u8]) -> Result<HealthResult> {
        // Use a dummy config — the health module doesn't actually use it
        let dummy = crate::config::AppConfig {
            rpc_url: String::new(),
            grpc_url: String::new(),
            grpc_token: None,
            kamino_market: String::new(),
            klend_program_id: PROGRAM_ID.to_string(),
            liquidator_keypair_path: String::new(),
            min_profit_lamports: 0,
            supabase_url: None,
            supabase_service_role_key: None,
        };

        let h = health::evaluate(data, &dummy)?;
        Ok(HealthResult {
            current_ltv: h.current_ltv,
            unhealthy_ltv: h.unhealthy_ltv,
            is_liquidatable: h.is_liquidatable,
            deposited_value_usd: h.deposited_value_sf as f64 / SF_SHIFT,
            borrowed_value_usd: h.borrow_factor_adjusted_debt_value_sf as f64 / SF_SHIFT,
        })
    }

    fn parse_positions(&self, data: &[u8]) -> Result<Positions> {
        let p = positions::parse_positions(data)?;
        Ok(Positions {
            deposits: p
                .deposits
                .into_iter()
                .map(|d| DepositPosition {
                    reserve: d.reserve,
                    mint: None,
                    amount: d.deposited_amount,
                    market_value_usd: d.market_value_sf as f64 / SF_SHIFT,
                })
                .collect(),
            borrows: p
                .borrows
                .into_iter()
                .map(|b| BorrowPosition {
                    reserve: b.reserve,
                    mint: None,
                    amount_sf: b.borrowed_amount_sf,
                    market_value_usd: b.market_value_sf as f64 / SF_SHIFT,
                })
                .collect(),
            market: p.lending_market,
            owner: p.owner,
        })
    }
}
