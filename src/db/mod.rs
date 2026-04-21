//! Supabase integration for liquidation audit trail.
//!
//! Uses the PostgREST API (Supabase's auto-generated REST layer) to
//! insert and update liquidation records. This avoids heavy DB driver
//! dependencies and works over HTTPS.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::AppConfig;

/// Supabase client for the liquidation audit trail.
#[derive(Clone)]
pub struct SupabaseClient {
    client: Client,
    base_url: String,
    api_key: String,
}

/// Status of a liquidation attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum LiquidationStatus {
    Pending,
    Submitted,
    Confirmed,
    Failed,
    Skipped,
}

impl std::fmt::Display for LiquidationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Submitted => write!(f, "submitted"),
            Self::Confirmed => write!(f, "confirmed"),
            Self::Failed => write!(f, "failed"),
            Self::Skipped => write!(f, "skipped"),
        }
    }
}

/// A liquidation attempt record for insertion.
#[derive(Debug, Serialize)]
pub struct NewLiquidationRecord {
    pub id: String,
    pub obligation_pubkey: String,
    pub obligation_owner: String,
    pub lending_market: String,
    pub repay_reserve: String,
    pub repay_mint: String,
    pub withdraw_reserve: String,
    pub withdraw_mint: String,
    pub ltv_at_detection: f64,
    pub unhealthy_ltv: f64,
    pub repay_amount: i64,
    pub liquidation_bonus_bps: i32,
    pub flash_loan_fee_fraction: f64,
    pub estimated_gross_profit_usd: f64,
    pub estimated_net_profit_usd: f64,
    pub status: String,
    pub error_message: Option<String>,
}

/// Fields to update after tx submission or confirmation.
#[derive(Debug, Serialize)]
pub struct UpdateLiquidationResult {
    pub status: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_profit_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sol_fee_paid: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot_submitted: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot_confirmed: Option<i64>,
}

/// ROI summary from the database view.
#[derive(Debug, Deserialize)]
pub struct RoiSummary {
    pub total_attempts: Option<i64>,
    pub successful: Option<i64>,
    pub failed: Option<i64>,
    pub skipped: Option<i64>,
    pub success_rate_pct: Option<f64>,
    pub total_estimated_profit_usd: Option<f64>,
    pub total_actual_profit_usd: Option<f64>,
    pub total_sol_fees: Option<f64>,
    pub first_attempt: Option<DateTime<Utc>>,
    pub last_attempt: Option<DateTime<Utc>>,
}

/// Daily PnL record.
#[derive(Debug, Deserialize)]
pub struct DailyPnl {
    pub day: DateTime<Utc>,
    pub attempts: i64,
    pub successful: i64,
    pub failed: i64,
    pub estimated_profit_usd: Option<f64>,
    pub actual_profit_usd: Option<f64>,
    pub sol_fees: Option<f64>,
}

impl SupabaseClient {
    /// Create a new Supabase client from config. Returns `Ok(None)` when no
    /// Supabase credentials are configured.
    pub fn new(config: &AppConfig) -> Result<Option<Self>> {
        let Some(supabase) = &config.supabase else {
            return Ok(None);
        };

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .context("failed to build HTTP client")?;

        Ok(Some(Self {
            client,
            base_url: supabase.url.trim_end_matches('/').to_string(),
            api_key: supabase.service_role_key.clone(),
        }))
    }

    /// PostgREST endpoint URL for a table or view.
    fn rest_url(&self, table: &str) -> String {
        format!("{}/rest/v1/{}", self.base_url, table)
    }

    /// Insert a new liquidation attempt record. Returns the record ID.
    pub async fn insert_liquidation(&self, record: &NewLiquidationRecord) -> Result<String> {
        let resp = self
            .client
            .post(self.rest_url("liquidation_attempts"))
            .header("apikey", &self.api_key)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("Prefer", "return=minimal")
            .json(record)
            .send()
            .await
            .context("failed to send insert request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("supabase insert failed ({}): {}", status, body);
        }

        Ok(record.id.clone())
    }

    /// Update a liquidation record by ID with execution results.
    pub async fn update_liquidation(
        &self,
        record_id: &str,
        update: &UpdateLiquidationResult,
    ) -> Result<()> {
        let url = format!(
            "{}?id=eq.{}",
            self.rest_url("liquidation_attempts"),
            record_id
        );

        let resp = self
            .client
            .patch(&url)
            .header("apikey", &self.api_key)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("Prefer", "return=minimal")
            .json(update)
            .send()
            .await
            .context("failed to send update request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("supabase update failed ({}): {}", status, body);
        }

        Ok(())
    }

    /// Fetch the ROI summary from the database view.
    pub async fn get_roi_summary(&self) -> Result<RoiSummary> {
        let resp = self
            .client
            .get(self.rest_url("liquidation_roi_summary"))
            .header("apikey", &self.api_key)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .context("failed to fetch ROI summary")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("supabase query failed ({}): {}", status, body);
        }

        let rows: Vec<RoiSummary> = resp.json().await.context("failed to parse ROI summary")?;
        rows.into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no ROI summary returned"))
    }

    /// Fetch daily PnL records.
    pub async fn get_daily_pnl(&self, limit: usize) -> Result<Vec<DailyPnl>> {
        let url = format!(
            "{}?order=day.desc&limit={}",
            self.rest_url("liquidation_daily_pnl"),
            limit
        );

        let resp = self
            .client
            .get(&url)
            .header("apikey", &self.api_key)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .context("failed to fetch daily PnL")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("supabase query failed ({}): {}", status, body);
        }

        resp.json().await.context("failed to parse daily PnL")
    }
}

/// Helper to create a new record ID.
pub fn new_record_id() -> String {
    Uuid::new_v4().to_string()
}

/// Helper to get the current UTC timestamp as an ISO string.
pub fn now_iso() -> String {
    Utc::now().to_rfc3339()
}
