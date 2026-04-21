mod config;
mod db;
mod decoder;
pub mod flash_loan;
mod grpc;
mod liquidator;
mod obligation;
mod protocols;

use std::collections::HashMap;

use anyhow::Result;
use tracing_subscriber::EnvFilter;

use protocols::{LendingProtocol, ProtocolKind};

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cfg = config::AppConfig::load("config.toml")?;
    tracing::info!("loaded config for market {}", cfg.kamino_market);

    let supabase = match db::SupabaseClient::new(&cfg)? {
        Some(client) => {
            tracing::info!("supabase audit trail enabled");
            Some(client)
        }
        None => {
            tracing::warn!("supabase not configured — liquidations will not be indexed");
            None
        }
    };

    let protocol_handlers: HashMap<ProtocolKind, Box<dyn LendingProtocol>> = HashMap::from([
        (
            ProtocolKind::Kamino,
            Box::new(protocols::kamino::KaminoProtocol::new()) as Box<dyn LendingProtocol>,
        ),
        (
            ProtocolKind::Save,
            Box::new(protocols::save::SaveProtocol::new()) as Box<dyn LendingProtocol>,
        ),
        (
            ProtocolKind::MarginFi,
            Box::new(protocols::marginfi::MarginFiProtocol::new()) as Box<dyn LendingProtocol>,
        ),
        (
            ProtocolKind::JupiterLend,
            Box::new(protocols::jupiter_lend::JupiterLendProtocol::new())
                as Box<dyn LendingProtocol>,
        ),
    ]);

    tracing::info!(
        "initialized {} protocols: {}",
        protocol_handlers.len(),
        protocol_handlers.keys().map(|k| k.to_string()).collect::<Vec<_>>().join(", ")
    );

    // Initialize flash loan providers (cheapest first)
    // Jupiter Lend: 0% fee — preferred when available
    // Kamino: 0.001% fee — deepest liquidity, fallback
    //
    // Providers need to be populated with reserve/mint data before they can
    // be used. Call provider.add_mint() / provider.add_reserve() after
    // fetching on-chain account data for the tokens you want to liquidate.
    let flash_providers: Vec<Box<dyn flash_loan::FlashLoanProvider>> = vec![
        Box::new(flash_loan::jupiter::JupiterFlashLoanProvider::new()),
        Box::new(flash_loan::kamino::KaminoFlashLoanProvider::new(
            &cfg.kamino_market_pubkey().unwrap_or_default(),
        )),
    ];

    tracing::info!(
        "initialized {} flash loan providers: {}",
        flash_providers.len(),
        flash_providers.iter().map(|p| p.kind().to_string()).collect::<Vec<_>>().join(", ")
    );

    let mut stream = grpc::subscribe_all_protocols(&cfg).await?;

    while let Some(update) = stream.recv().await {
        match update {
            grpc::PositionUpdate::AccountData {
                pubkey,
                protocol,
                data,
                ..
            } => {
                let Some(handler) = protocol_handlers.get(&protocol) else {
                    continue;
                };

                if !handler.is_position_account(&data) {
                    continue;
                }

                let health = match handler.evaluate_health(&data) {
                    Ok(h) => h,
                    Err(_) => continue,
                };

                if !health.is_liquidatable {
                    continue;
                }

                tracing::warn!(
                    protocol = %protocol,
                    position = %pubkey,
                    ltv = %format!("{:.4}", health.current_ltv),
                    deposited = %format!("${:.2}", health.deposited_value_usd),
                    borrowed = %format!("${:.2}", health.borrowed_value_usd),
                    "liquidatable position detected"
                );

                // Parse positions for the executor
                let positions = match handler.parse_positions(&data) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!(error = %e, "failed to parse positions");
                        continue;
                    }
                };

                let params = liquidator::executor::LiquidationParams {
                    protocol,
                    position_pubkey: pubkey,
                    health,
                    positions,
                };

                if let Err(e) = liquidator::executor::execute_liquidation(
                    &cfg,
                    &params,
                    &flash_providers,
                    supabase.as_ref(),
                ).await {
                    tracing::error!(
                        error = %e,
                        protocol = %protocol,
                        position = %pubkey,
                        "liquidation failed"
                    );
                }
            }
        }
    }

    Ok(())
}
