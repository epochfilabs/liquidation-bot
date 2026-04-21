//! Application configuration.
//!
//! [`AppConfig::load`] is the single entry point for loading config: it reads
//! an optional `config.toml`, then applies environment-variable overrides on
//! top. Risk and Jito configuration live as nested fields — there's no
//! separate `from_env()` scattered across modules.
//!
//! Environment variables take precedence over TOML, which takes precedence
//! over the defaults baked into this struct.

use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, Result};
use serde::Deserialize;
use solana_sdk::pubkey::Pubkey;

use crate::jito::{JITO_MAINNET_ENDPOINT, JitoConfig};
use crate::risk::RiskConfig;

/// Default Kamino Lend program ID on mainnet.
const DEFAULT_KLEND_PROGRAM: Pubkey =
    solana_sdk::pubkey!("KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD");

/// Default Kamino main market.
const DEFAULT_KAMINO_MARKET: Pubkey =
    solana_sdk::pubkey!("7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF");

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Solana RPC endpoint.
    pub rpc_url: String,

    /// Yellowstone gRPC endpoint.
    pub grpc_url: String,

    /// gRPC auth token (Helius / Triton).
    pub grpc_token: Option<String>,

    /// Kamino Lend market address (the market we watch for liquidations).
    #[serde(deserialize_with = "deserialize_pubkey")]
    pub kamino_market: Pubkey,

    /// Kamino Lend program ID.
    #[serde(deserialize_with = "deserialize_pubkey")]
    pub klend_program_id: Pubkey,

    /// Path to the liquidator's keypair JSON file.
    pub liquidator_keypair_path: PathBuf,

    /// Minimum profit threshold in lamports to trigger liquidation.
    pub min_profit_lamports: u64,

    /// Optional Supabase audit-trail configuration.
    pub supabase: Option<SupabaseConfig>,

    /// Risk / EV filter configuration.
    pub risk: RiskConfig,

    /// Jito bundle submission configuration.
    pub jito: JitoConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            rpc_url: String::new(),
            grpc_url: String::new(),
            grpc_token: None,
            kamino_market: DEFAULT_KAMINO_MARKET,
            klend_program_id: DEFAULT_KLEND_PROGRAM,
            liquidator_keypair_path: PathBuf::new(),
            min_profit_lamports: 10_000,
            supabase: None,
            risk: RiskConfig::default(),
            jito: JitoConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SupabaseConfig {
    pub url: String,
    pub service_role_key: String,
}

impl AppConfig {
    /// Load config from `config.toml` (if present), then apply environment
    /// variable overrides. Call `dotenvy::dotenv()` before this to pick up
    /// variables from `.env`.
    pub fn load(path: &str) -> Result<Self> {
        let mut config: AppConfig = match std::fs::read_to_string(path) {
            Ok(contents) => toml::from_str(&contents).context("failed to parse config.toml")?,
            Err(_) => AppConfig::default(),
        };

        if let Some(v) = env_override::<String>("SOLANA_RPC_URL") {
            config.rpc_url = v;
        }
        if let Some(v) = env_override::<String>("YELLOWSTONE_GRPC_ENDPOINT") {
            config.grpc_url = v;
        }
        if let Some(v) = env_override::<String>("YELLOWSTONE_GRPC_TOKEN") {
            config.grpc_token = Some(v);
        }
        if let Some(v) = env_override::<Pubkey>("KAMINO_MARKET") {
            config.kamino_market = v;
        }
        if let Some(v) = env_override::<Pubkey>("KLEND_PROGRAM_ID") {
            config.klend_program_id = v;
        }
        if let Some(v) = env_override::<PathBuf>("LIQUIDATOR_KEYPAIR_PATH") {
            config.liquidator_keypair_path = v;
        }
        if let Some(v) = env_override::<u64>("MIN_PROFIT_LAMPORTS") {
            config.min_profit_lamports = v;
        }

        // Supabase: both URL and service key must be present to enable it.
        if let (Some(url), Some(key)) = (
            env_override::<String>("SUPABASE_URL"),
            env_override::<String>("SUPABASE_SERVICE_ROLE_KEY"),
        ) {
            config.supabase = Some(SupabaseConfig {
                url,
                service_role_key: key,
            });
        }

        apply_risk_env(&mut config.risk);
        apply_jito_env(&mut config.jito);

        Ok(config)
    }

    pub fn has_supabase(&self) -> bool {
        self.supabase.is_some()
    }
}

fn deserialize_pubkey<'de, D>(deserializer: D) -> Result<Pubkey, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Pubkey::from_str(&s).map_err(serde::de::Error::custom)
}

/// Read `key` from the environment and parse it as `T`. Returns `None` if the
/// variable is unset or fails to parse (parse failures log a warning so silent
/// misconfiguration doesn't go unnoticed).
fn env_override<T: FromStr>(key: &str) -> Option<T>
where
    <T as FromStr>::Err: std::fmt::Display,
{
    let raw = std::env::var(key).ok()?;
    match raw.parse::<T>() {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(env = key, error = %e, "ignoring invalid env override");
            None
        }
    }
}

fn apply_risk_env(risk: &mut RiskConfig) {
    if let Some(v) = env_override::<u64>("MIN_REPAY_AMOUNT") {
        risk.min_repay_amount = v;
    }
    if let Some(v) = env_override::<f64>("MIN_ESTIMATED_BONUS_USD") {
        risk.min_estimated_bonus_usd = v;
    }
    if let Some(v) = env_override::<u64>("DAILY_TIP_CAP_LAMPORTS") {
        risk.daily_tip_cap_lamports = v;
    }
    if let Some(v) = env_override::<u64>("MAX_TIP_PER_TX_LAMPORTS") {
        risk.max_tip_per_tx_lamports = v;
    }
    if let Some(v) = env_override::<f64>("ESTIMATED_BONUS_RATE") {
        risk.estimated_bonus_rate = v;
    }
}

fn apply_jito_env(jito: &mut JitoConfig) {
    if let Some(v) = env_override::<String>("JITO_ENDPOINT") {
        jito.endpoint = v;
    } else if jito.endpoint.is_empty() {
        jito.endpoint = JITO_MAINNET_ENDPOINT.into();
    }
    if let Some(v) = env_override::<bool>("JITO_ENABLED") {
        jito.enabled = v;
    }
}
