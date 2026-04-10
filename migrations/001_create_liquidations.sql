-- Liquidation bot audit trail
-- Run this against your Supabase project via the SQL editor

CREATE TABLE IF NOT EXISTS liquidation_attempts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- Obligation info
    obligation_pubkey TEXT NOT NULL,
    obligation_owner TEXT NOT NULL,
    lending_market TEXT NOT NULL,

    -- Position info
    repay_reserve TEXT NOT NULL,
    repay_mint TEXT NOT NULL,
    withdraw_reserve TEXT NOT NULL,
    withdraw_mint TEXT NOT NULL,

    -- Health at time of detection
    ltv_at_detection DOUBLE PRECISION NOT NULL,
    unhealthy_ltv DOUBLE PRECISION NOT NULL,

    -- Liquidation params
    repay_amount BIGINT NOT NULL,
    liquidation_bonus_bps INTEGER NOT NULL,
    flash_loan_fee_fraction DOUBLE PRECISION NOT NULL,

    -- Profitability estimate (USD)
    estimated_gross_profit_usd DOUBLE PRECISION NOT NULL,
    estimated_net_profit_usd DOUBLE PRECISION NOT NULL,

    -- Execution result
    status TEXT NOT NULL DEFAULT 'pending',
    -- pending | submitted | confirmed | failed | skipped
    tx_signature TEXT,
    error_message TEXT,

    -- Actual results (filled after confirmation)
    actual_profit_usd DOUBLE PRECISION,
    sol_fee_paid BIGINT,
    slot_submitted BIGINT,
    slot_confirmed BIGINT
);

-- Indexes for common queries
CREATE INDEX IF NOT EXISTS idx_liquidations_status ON liquidation_attempts(status);
CREATE INDEX IF NOT EXISTS idx_liquidations_created ON liquidation_attempts(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_liquidations_obligation ON liquidation_attempts(obligation_pubkey);
CREATE INDEX IF NOT EXISTS idx_liquidations_signature ON liquidation_attempts(tx_signature);

-- ROI summary view
CREATE OR REPLACE VIEW liquidation_roi_summary AS
SELECT
    COUNT(*) AS total_attempts,
    COUNT(*) FILTER (WHERE status = 'confirmed') AS successful,
    COUNT(*) FILTER (WHERE status = 'failed') AS failed,
    COUNT(*) FILTER (WHERE status = 'skipped') AS skipped,
    ROUND(
        COUNT(*) FILTER (WHERE status = 'confirmed')::NUMERIC
        / NULLIF(COUNT(*) FILTER (WHERE status IN ('confirmed', 'failed')), 0) * 100,
        2
    ) AS success_rate_pct,
    ROUND(SUM(estimated_net_profit_usd) FILTER (WHERE status = 'confirmed')::NUMERIC, 4) AS total_estimated_profit_usd,
    ROUND(SUM(actual_profit_usd) FILTER (WHERE status = 'confirmed')::NUMERIC, 4) AS total_actual_profit_usd,
    ROUND(SUM(sol_fee_paid) FILTER (WHERE status = 'confirmed')::NUMERIC / 1e9, 6) AS total_sol_fees,
    MIN(created_at) AS first_attempt,
    MAX(created_at) AS last_attempt
FROM liquidation_attempts;

-- Daily PnL view
CREATE OR REPLACE VIEW liquidation_daily_pnl AS
SELECT
    DATE_TRUNC('day', created_at) AS day,
    COUNT(*) AS attempts,
    COUNT(*) FILTER (WHERE status = 'confirmed') AS successful,
    COUNT(*) FILTER (WHERE status = 'failed') AS failed,
    ROUND(SUM(estimated_net_profit_usd) FILTER (WHERE status = 'confirmed')::NUMERIC, 4) AS estimated_profit_usd,
    ROUND(SUM(actual_profit_usd) FILTER (WHERE status = 'confirmed')::NUMERIC, 4) AS actual_profit_usd,
    ROUND(SUM(sol_fee_paid) FILTER (WHERE status = 'confirmed')::NUMERIC / 1e9, 6) AS sol_fees
FROM liquidation_attempts
GROUP BY DATE_TRUNC('day', created_at)
ORDER BY day DESC;

-- Per-obligation breakdown
CREATE OR REPLACE VIEW liquidation_by_obligation AS
SELECT
    obligation_pubkey,
    obligation_owner,
    COUNT(*) AS attempts,
    COUNT(*) FILTER (WHERE status = 'confirmed') AS successful,
    ROUND(SUM(estimated_net_profit_usd) FILTER (WHERE status = 'confirmed')::NUMERIC, 4) AS total_profit_usd,
    MAX(created_at) AS last_attempt
FROM liquidation_attempts
GROUP BY obligation_pubkey, obligation_owner
ORDER BY total_profit_usd DESC NULLS LAST;
