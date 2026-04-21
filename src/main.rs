//! Liquidation bot entry point.
//!
//! `main()` is the composition root: it loads config, initializes the protocol
//! registry, flash-loan providers, Supabase client, and risk tracker, then
//! runs the gRPC event loop. All work happens inside [`run`].

use std::sync::Arc;

use anyhow::Result;
use liquidation_bot::{
    config::AppConfig,
    db::SupabaseClient,
    flash_loan::{self, FlashLoanProvider},
    grpc::{self, PositionUpdate},
    liquidator,
    protocols::{LiquidationParams, ProtocolKind, Registry},
    risk::{self, DailyTracker, EvDecision},
};
use solana_client::rpc_client::RpcClient;
use solana_sdk::commitment_config::CommitmentConfig;
use tracing_subscriber::EnvFilter;

/// Approximate SOL price used when converting a lamport tip back to USD for
/// logging. The bot's EV math is quoted in USD, but Jito tips are lamports —
/// display conversion only, not risk-sensitive.
const SOL_PRICE_USD_DISPLAY: f64 = 140.0;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = Arc::new(AppConfig::load("config.toml")?);
    run(config).await
}

/// Run the bot: wire up everything and drive the gRPC event loop.
async fn run(config: Arc<AppConfig>) -> Result<()> {
    tracing::info!("loaded config for market {}", config.kamino_market);

    let supabase = match SupabaseClient::new(&config)? {
        Some(client) => {
            tracing::info!("supabase audit trail enabled");
            Some(client)
        }
        None => {
            tracing::warn!("supabase not configured — liquidations will not be indexed");
            None
        }
    };

    let registry = Registry::new();
    tracing::info!(
        protocols = ProtocolKind::COUNT,
        "initialized {} protocols: {}",
        ProtocolKind::COUNT,
        registry
            .iter()
            .map(|h| h.kind().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let rpc = RpcClient::new_with_commitment(config.rpc_url.clone(), CommitmentConfig::confirmed());
    let flash_providers = flash_loan::init::initialize_providers(&config, &rpc)?;

    tracing::info!(
        min_repay = config.risk.min_repay_amount,
        min_bonus_usd = config.risk.min_estimated_bonus_usd,
        daily_cap_lamports = config.risk.daily_tip_cap_lamports,
        max_tip_per_tx = config.risk.max_tip_per_tx_lamports,
        bonus_rate = config.risk.estimated_bonus_rate,
        "risk config loaded"
    );
    tracing::info!(
        endpoint = %config.jito.endpoint,
        enabled = config.jito.enabled,
        "jito config loaded"
    );

    let daily_tracker = Arc::new(DailyTracker::new());

    let mut stream = grpc::subscribe_all_protocols(&config).await?;

    tracing::info!("entering event loop — listening for liquidatable positions");

    while let Some(update) = stream.recv().await {
        process_update(
            update,
            &registry,
            &config,
            &flash_providers,
            supabase.as_ref(),
            &daily_tracker,
        )
        .await;
    }

    Ok(())
}

async fn process_update(
    update: PositionUpdate,
    registry: &Registry,
    config: &AppConfig,
    flash_providers: &[Box<dyn FlashLoanProvider>],
    supabase: Option<&SupabaseClient>,
    daily_tracker: &DailyTracker,
) {
    let PositionUpdate::AccountData {
        pubkey,
        protocol,
        data,
        ..
    } = update;

    let handler = registry.get(protocol);

    if !handler.is_position_account(&data) {
        return;
    }

    let Ok(health) = handler.evaluate_health(&data) else {
        return;
    };
    if !health.is_liquidatable {
        return;
    }

    let positions = match handler.parse_positions(&data) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, protocol = %protocol, "failed to parse positions");
            return;
        }
    };

    let Some(repay_pos) = positions
        .borrows
        .iter()
        .max_by(|a, b| a.market_value_usd.total_cmp(&b.market_value_usd))
    else {
        return;
    };

    let repay_amount_raw = handler.flash_loan_amount(repay_pos);

    // EV filter — conservative stablecoin estimate for now.
    // TODO: use a real oracle price (Pyth) for non-stablecoin debt.
    let token_decimals = 6u8;
    let token_price_usd = 1.0;

    let ev_decision = risk::evaluate_opportunity(
        &config.risk,
        repay_amount_raw,
        token_decimals,
        token_price_usd,
        daily_tracker,
    );

    match ev_decision {
        EvDecision::Submit {
            estimated_bonus_usd,
            recommended_tip_lamports,
        } => {
            handle_submit(
                protocol,
                pubkey,
                health,
                positions,
                estimated_bonus_usd,
                recommended_tip_lamports,
                config,
                flash_providers,
                supabase,
                daily_tracker,
            )
            .await;
        }
        EvDecision::SkipTooSmall {
            repay_amount,
            min_required,
        } => {
            daily_tracker.record_skip();
            tracing::debug!(
                protocol = %protocol,
                position = %pubkey,
                repay = repay_amount,
                min = min_required,
                "skipped: too small"
            );
        }
        EvDecision::SkipLowEv {
            estimated_bonus_usd,
            min_required,
        } => {
            daily_tracker.record_skip();
            tracing::debug!(
                protocol = %protocol,
                position = %pubkey,
                bonus = format!("${:.2}", estimated_bonus_usd),
                min = format!("${:.2}", min_required),
                "skipped: low EV"
            );
        }
        EvDecision::SkipDailyCapReached { spent_today, cap } => {
            tracing::warn!(
                spent = spent_today,
                cap,
                "daily loss cap reached — pausing submissions"
            );
        }
        // `EvDecision` is `#[non_exhaustive]`; any future variant is logged.
        _ => tracing::warn!("unhandled EV decision variant"),
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_submit(
    protocol: ProtocolKind,
    position_pubkey: solana_sdk::pubkey::Pubkey,
    health: liquidation_bot::protocols::HealthResult,
    positions: liquidation_bot::protocols::Positions,
    estimated_bonus_usd: f64,
    recommended_tip_lamports: u64,
    config: &AppConfig,
    flash_providers: &[Box<dyn FlashLoanProvider>],
    supabase: Option<&SupabaseClient>,
    daily_tracker: &DailyTracker,
) {
    tracing::warn!(
        protocol = %protocol,
        position = %position_pubkey,
        ltv = %format!("{:.4}", health.current_ltv),
        borrowed = %format!("${:.2}", health.borrowed_value_usd),
        estimated_bonus = %format!("${:.2}", estimated_bonus_usd),
        tip = recommended_tip_lamports,
        "submitting liquidation"
    );

    let params = LiquidationParams {
        protocol,
        position_pubkey,
        health,
        positions,
    };

    let submit_result =
        liquidator::executor::execute_liquidation(config, &params, flash_providers, supabase).await;

    match submit_result {
        Ok(()) => {
            daily_tracker.record_success();
            daily_tracker.record_tip(recommended_tip_lamports);
            tracing::info!(
                protocol = %protocol,
                position = %position_pubkey,
                "liquidation succeeded"
            );
        }
        Err(e) => {
            daily_tracker.record_failure();
            daily_tracker.record_tip(recommended_tip_lamports);
            tracing::error!(
                error = %e,
                protocol = %protocol,
                position = %position_pubkey,
                "liquidation failed"
            );
        }
    }

    // Periodic daily stats log — every 10 terminal events.
    let stats = daily_tracker.stats();
    let terminal = stats.successes + stats.failures;
    if terminal > 0 && terminal.is_multiple_of(10) {
        tracing::info!(
            spent_usd = format!("${:.2}", stats.spent_lamports as f64 * SOL_PRICE_USD_DISPLAY / 1e9),
            successes = stats.successes,
            failures = stats.failures,
            skips = stats.skips,
            "daily stats"
        );
    }
}

