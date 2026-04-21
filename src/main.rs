mod config;
mod db;
mod decoder;
pub mod flash_loan;
mod grpc;
pub mod jito;
mod liquidator;
mod obligation;
mod protocols;
pub mod risk;

use std::collections::HashMap;
use std::sync::Arc;

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

    // --- Supabase audit trail (optional) ---
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

    // --- Protocol handlers ---
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

    // --- Flash loan providers (cheapest first) ---
    let rpc = solana_client::rpc_client::RpcClient::new_with_commitment(
        cfg.rpc_url.clone(),
        solana_sdk::commitment_config::CommitmentConfig::confirmed(),
    );
    let flash_providers = flash_loan::init::initialize_providers(&cfg, &rpc)?;

    // --- Risk management ---
    let risk_config = risk::RiskConfig::from_env();
    let daily_tracker = Arc::new(risk::DailyTracker::new());
    tracing::info!(
        min_repay = risk_config.min_repay_amount,
        min_bonus_usd = risk_config.min_estimated_bonus_usd,
        daily_cap_lamports = risk_config.daily_tip_cap_lamports,
        max_tip_per_tx = risk_config.max_tip_per_tx_lamports,
        bonus_rate = risk_config.estimated_bonus_rate,
        "risk config loaded"
    );

    // --- Jito bundle submission ---
    let jito_config = jito::JitoConfig::from_env();
    tracing::info!(
        endpoint = %jito_config.endpoint,
        enabled = jito_config.enabled,
        "jito config loaded"
    );

    // --- gRPC stream ---
    let mut stream = grpc::subscribe_all_protocols(&cfg).await?;

    tracing::info!("entering event loop — listening for liquidatable positions");

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

                // Step 1: Is this a position account?
                if !handler.is_position_account(&data) {
                    continue;
                }

                // Step 2: Is it liquidatable?
                let health = match handler.evaluate_health(&data) {
                    Ok(h) => h,
                    Err(_) => continue,
                };

                if !health.is_liquidatable {
                    continue;
                }

                // Step 3: Parse positions
                let positions = match handler.parse_positions(&data) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!(error = %e, "failed to parse positions");
                        continue;
                    }
                };

                // Step 4: Find the repay amount for EV filter
                let repay_pos = match positions.borrows.iter()
                    .max_by(|a, b| a.market_value_usd.partial_cmp(&b.market_value_usd).unwrap())
                {
                    Some(p) => p,
                    None => continue,
                };

                // Estimate repay amount in token units
                let sf_shift: u128 = 1u128 << 60;
                let repay_amount_raw = match protocol {
                    ProtocolKind::Kamino => (repay_pos.amount_sf / sf_shift) as u64,
                    ProtocolKind::Save => {
                        let wad: u128 = 1_000_000_000_000_000_000;
                        (repay_pos.amount_sf / wad) as u64
                    }
                    ProtocolKind::JupiterLend => repay_pos.amount_sf as u64,
                    ProtocolKind::MarginFi => repay_pos.amount_sf as u64,
                };

                // Step 5: EV filter — should we submit?
                // Use 6 decimals and $1.0 as conservative stablecoin estimate
                // TODO: use real oracle price from Pyth for non-stablecoin debt
                let token_decimals = 6u8;
                let token_price_usd = 1.0;

                let ev_decision = risk::evaluate_opportunity(
                    &risk_config,
                    repay_amount_raw,
                    token_decimals,
                    token_price_usd,
                    &daily_tracker,
                );

                match &ev_decision {
                    risk::EvDecision::Submit { estimated_bonus_usd, recommended_tip_lamports } => {
                        tracing::warn!(
                            protocol = %protocol,
                            position = %pubkey,
                            ltv = %format!("{:.4}", health.current_ltv),
                            borrowed = %format!("${:.2}", health.borrowed_value_usd),
                            estimated_bonus = %format!("${:.2}", estimated_bonus_usd),
                            tip = recommended_tip_lamports,
                            "submitting liquidation"
                        );

                        let params = liquidator::executor::LiquidationParams {
                            protocol,
                            position_pubkey: pubkey,
                            health,
                            positions,
                        };

                        // Build the liquidation instructions via executor
                        // The executor handles flash loan wrapping internally
                        match liquidator::executor::execute_liquidation(
                            &cfg,
                            &params,
                            &flash_providers,
                            supabase.as_ref(),
                        ).await {
                            Ok(()) => {
                                daily_tracker.record_success();
                                daily_tracker.record_tip(*recommended_tip_lamports);
                                tracing::info!(
                                    protocol = %protocol,
                                    position = %pubkey,
                                    "liquidation succeeded"
                                );
                            }
                            Err(e) => {
                                daily_tracker.record_failure();
                                daily_tracker.record_tip(*recommended_tip_lamports);
                                tracing::error!(
                                    error = %e,
                                    protocol = %protocol,
                                    position = %pubkey,
                                    "liquidation failed"
                                );
                            }
                        }

                        // Log daily stats periodically
                        let (spent, successes, failures, skips) = daily_tracker.stats();
                        if (successes + failures) % 10 == 0 && (successes + failures) > 0 {
                            let sol_price = 140.0;
                            tracing::info!(
                                spent_usd = format!("${:.2}", spent as f64 * sol_price / 1e9),
                                successes = successes,
                                failures = failures,
                                skips = skips,
                                "daily stats"
                            );
                        }
                    }
                    risk::EvDecision::SkipTooSmall { repay_amount, min_required } => {
                        daily_tracker.record_skip();
                        tracing::debug!(
                            protocol = %protocol,
                            position = %pubkey,
                            repay = repay_amount,
                            min = min_required,
                            "skipped: too small"
                        );
                    }
                    risk::EvDecision::SkipLowEv { estimated_bonus_usd, min_required } => {
                        daily_tracker.record_skip();
                        tracing::debug!(
                            protocol = %protocol,
                            position = %pubkey,
                            bonus = format!("${:.2}", estimated_bonus_usd),
                            min = format!("${:.2}", min_required),
                            "skipped: low EV"
                        );
                    }
                    risk::EvDecision::SkipDailyCapReached { spent_today, cap } => {
                        tracing::warn!(
                            spent = spent_today,
                            cap = cap,
                            "daily loss cap reached — pausing submissions"
                        );
                    }
                }
            }
        }
    }

    Ok(())
}
