-- ============================================================================
-- Sanity Queries — Solana Liquidation Indexer
--
-- Run these against the ClickHouse database to validate data quality.
-- Any reviewer can run them without code changes.
-- ============================================================================

-- -------------------------------------------------------
-- 1. Top 20 liquidators by realized profit per venue
-- -------------------------------------------------------

SELECT
    venue,
    liquidator,
    countMerge(liquidation_count) AS total_liquidations,
    sumMerge(total_profit_usd) AS profit_usd,
    sumMerge(total_tips_lamports) AS tips_lamports,
    minMerge(first_seen) AS first_seen,
    maxMerge(last_seen) AS last_seen
FROM mv_top_liquidators
GROUP BY venue, liquidator
ORDER BY profit_usd DESC
LIMIT 20;


-- -------------------------------------------------------
-- 2. Daily liquidation USD volume stacked by venue
-- -------------------------------------------------------

SELECT
    day,
    venue,
    liquidation_count,
    total_repay_usd,
    total_collateral_usd,
    total_profit_usd
FROM mv_daily_volume
ORDER BY day DESC, venue
LIMIT 120;  -- ~30 days * 4 venues


-- -------------------------------------------------------
-- 3. Median Jito tip as a fraction of liquidator profit,
--    bucketed by venue and quarter
-- -------------------------------------------------------

SELECT
    venue,
    toStartOfQuarter(block_time) AS quarter,
    count() AS tipped_liquidations,
    median(jito_tip_lamports) AS median_tip_lamports,
    median(liquidator_profit_usd) AS median_profit_usd,
    median(
        CASE
            WHEN liquidator_profit_usd > 0
            THEN (jito_tip_lamports * 0.00000000067) / liquidator_profit_usd  -- tip in USD / profit in USD
            ELSE NULL
        END
    ) AS median_tip_profit_ratio
FROM liquidations
WHERE jito_tip_lamports IS NOT NULL
  AND liquidator_profit_usd IS NOT NULL
GROUP BY venue, quarter
ORDER BY quarter DESC, venue;


-- -------------------------------------------------------
-- 4. Failed-vs-successful ratio per venue per month
-- -------------------------------------------------------

SELECT
    venue,
    month,
    sumIf(count, succeeded = true) AS successful,
    sumIf(count, succeeded = false) AS failed,
    sumIf(count, succeeded = true) + sumIf(count, succeeded = false) AS total,
    round(sumIf(count, succeeded = false) * 100.0
        / (sumIf(count, succeeded = true) + sumIf(count, succeeded = false)), 2) AS failure_pct
FROM _success_failure_monthly
GROUP BY venue, month
ORDER BY month DESC, venue;


-- -------------------------------------------------------
-- 5. Liquidation latency distribution
--    (block_time minus obligation.last_update_slot → approximate seconds)
--    Uses obligations_snapshots to check how stale the obligation was
-- -------------------------------------------------------

SELECT
    l.venue,
    toStartOfMonth(l.block_time) AS month,
    count() AS liquidations,
    quantile(0.5)(l.slot - o.slot) AS p50_slot_delta,
    quantile(0.9)(l.slot - o.slot) AS p90_slot_delta,
    -- Approximate seconds (400ms per slot)
    quantile(0.5)(l.slot - o.slot) * 0.4 AS p50_seconds,
    quantile(0.9)(l.slot - o.slot) * 0.4 AS p90_seconds
FROM liquidations l
LEFT JOIN obligations_snapshots o
    ON l.tx_signature = o.tx_signature AND l.ix_index = o.ix_index
GROUP BY l.venue, month
ORDER BY month DESC, l.venue;


-- -------------------------------------------------------
-- 6. Collateral/debt pair dominance — which pairs get
--    liquidated most, by count and USD volume
-- -------------------------------------------------------

SELECT
    venue,
    collateral_mint,
    debt_mint,
    count() AS liquidation_count,
    sum(repay_amount_usd) AS total_repay_usd,
    sum(liquidator_profit_usd) AS total_profit_usd,
    avg(liquidation_bonus_bps) AS avg_bonus_bps
FROM liquidations
WHERE repay_amount_usd IS NOT NULL
GROUP BY venue, collateral_mint, debt_mint
ORDER BY total_repay_usd DESC
LIMIT 50;


-- -------------------------------------------------------
-- 7. Jito tip effectiveness — do higher tips correlate
--    with higher success rates?
-- -------------------------------------------------------

WITH tips AS (
    SELECT
        venue,
        jito_tip_lamports,
        true AS succeeded
    FROM liquidations
    WHERE jito_tip_lamports IS NOT NULL

    UNION ALL

    SELECT
        venue,
        jito_tip_lamports,
        false AS succeeded
    FROM failed_liquidation_attempts
    WHERE jito_tip_lamports IS NOT NULL
)
SELECT
    venue,
    multiIf(
        jito_tip_lamports < 10000, '<10K',
        jito_tip_lamports < 100000, '10K-100K',
        jito_tip_lamports < 1000000, '100K-1M',
        jito_tip_lamports < 10000000, '1M-10M',
        '>10M'
    ) AS tip_bucket,
    countIf(succeeded = true) AS successes,
    countIf(succeeded = false) AS failures,
    round(countIf(succeeded = true) * 100.0 / count(), 2) AS success_rate_pct
FROM tips
GROUP BY venue, tip_bucket
ORDER BY venue, tip_bucket;


-- -------------------------------------------------------
-- 8. Searcher concentration — how many unique liquidators
--    account for 80% of profit per venue?
-- -------------------------------------------------------

WITH ranked AS (
    SELECT
        venue,
        liquidator,
        sumMerge(total_profit_usd) AS profit,
        sum(sumMerge(total_profit_usd)) OVER (PARTITION BY venue ORDER BY sumMerge(total_profit_usd) DESC) AS cumulative_profit,
        sum(sumMerge(total_profit_usd)) OVER (PARTITION BY venue) AS total_profit
    FROM mv_top_liquidators
    GROUP BY venue, liquidator
)
SELECT
    venue,
    countIf(cumulative_profit <= total_profit * 0.8) + 1 AS liquidators_for_80pct_profit,
    count() AS total_unique_liquidators
FROM ranked
GROUP BY venue;


-- -------------------------------------------------------
-- 9. Indexer coverage check
-- -------------------------------------------------------

SELECT
    venue,
    source,
    backfill_state,
    last_epoch,
    last_slot,
    rows_liquidations,
    rows_failed,
    rows_total_processed,
    updated_at
FROM _indexer_progress
ORDER BY venue, source;


-- -------------------------------------------------------
-- 10. Data freshness — most recent liquidation per venue
-- -------------------------------------------------------

SELECT
    venue,
    max(block_time) AS latest_liquidation,
    max(slot) AS latest_slot,
    dateDiff('minute', max(block_time), now()) AS minutes_behind
FROM liquidations
GROUP BY venue
ORDER BY venue;
