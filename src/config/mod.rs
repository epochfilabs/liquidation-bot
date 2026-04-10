use anyhow::{Context, Result};
use serde::Deserialize;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    /// Solana RPC endpoint
    pub rpc_url: String,

    /// Yellowstone gRPC endpoint
    pub grpc_url: String,

    /// gRPC auth token (Helius / Triton)
    pub grpc_token: Option<String>,

    /// Kamino Lend market address
    pub kamino_market: String,

    /// Kamino Lend program ID
    #[serde(default = "default_klend_program")]
    pub klend_program_id: String,

    /// Liquidator keypair path
    #[serde(default)]
    pub liquidator_keypair_path: String,

    /// Minimum profit threshold in lamports to trigger liquidation
    #[serde(default = "default_min_profit")]
    pub min_profit_lamports: u64,

    /// Supabase project URL
    #[serde(default)]
    pub supabase_url: Option<String>,

    /// Supabase service role key (for server-side inserts)
    #[serde(default)]
    pub supabase_service_role_key: Option<String>,
}

fn default_klend_program() -> String {
    "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD".to_string()
}

fn default_min_profit() -> u64 {
    10_000 // 0.00001 SOL
}

impl AppConfig {
    /// Load config from config.toml, then override with environment variables.
    /// Call `dotenvy::dotenv()` before this to load .env file.
    pub fn load(path: &str) -> Result<Self> {
        let mut config: AppConfig = if let Ok(contents) = std::fs::read_to_string(path) {
            toml::from_str(&contents).context("failed to parse config.toml")?
        } else {
            AppConfig {
                rpc_url: String::new(),
                grpc_url: String::new(),
                grpc_token: None,
                kamino_market: "7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF".to_string(),
                klend_program_id: default_klend_program(),
                liquidator_keypair_path: String::new(),
                min_profit_lamports: default_min_profit(),
                supabase_url: None,
                supabase_service_role_key: None,
            }
        };

        // Override with env vars
        if let Ok(v) = std::env::var("SOLANA_RPC_URL") {
            config.rpc_url = v;
        }
        if let Ok(v) = std::env::var("YELLOWSTONE_GRPC_ENDPOINT") {
            config.grpc_url = v;
        }
        if let Ok(v) = std::env::var("YELLOWSTONE_GRPC_TOKEN") {
            config.grpc_token = Some(v);
        }
        if let Ok(v) = std::env::var("KAMINO_MARKET") {
            config.kamino_market = v;
        }
        if let Ok(v) = std::env::var("KLEND_PROGRAM_ID") {
            config.klend_program_id = v;
        }
        if let Ok(v) = std::env::var("LIQUIDATOR_KEYPAIR_PATH") {
            config.liquidator_keypair_path = v;
        }
        if let Ok(v) = std::env::var("MIN_PROFIT_LAMPORTS") {
            if let Ok(n) = v.parse() {
                config.min_profit_lamports = n;
            }
        }
        if let Ok(v) = std::env::var("SUPABASE_URL") {
            config.supabase_url = Some(v);
        }
        if let Ok(v) = std::env::var("SUPABASE_SERVICE_ROLE_KEY") {
            config.supabase_service_role_key = Some(v);
        }

        Ok(config)
    }

    pub fn klend_program_pubkey(&self) -> Result<Pubkey> {
        Ok(Pubkey::from_str(&self.klend_program_id)?)
    }

    pub fn kamino_market_pubkey(&self) -> Result<Pubkey> {
        Ok(Pubkey::from_str(&self.kamino_market)?)
    }

    pub fn has_supabase(&self) -> bool {
        self.supabase_url.is_some() && self.supabase_service_role_key.is_some()
    }
}
