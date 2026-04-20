-- ============================================================================
-- Interactive Query Examples for ClickHouse
-- Run with: ./scripts/clickhouse-shell.sh
-- Or paste individual queries into the shell.
-- ============================================================================

-- Show all tables
SHOW TABLES;

-- Row counts per table
SELECT 'liquidations' AS tbl, count() AS rows FROM liquidations
UNION ALL SELECT 'failed_liquidation_attempts', count() FROM failed_liquidation_attempts
UNION ALL SELECT 'obligations_snapshots', count() FROM obligations_snapshots
UNION ALL SELECT 'reserves_snapshots', count() FROM reserves_snapshots
UNION ALL SELECT 'tx_metadata', count() FROM tx_metadata;

-- All liquidation events with key fields
SELECT
    venue,
    tx_signature,
    slot,
    block_time,
    liquidator,
    obligation,
    repay_amount,
    withdraw_amount,
    used_flashloan,
    jito_tip_lamports
FROM liquidations
ORDER BY block_time DESC
LIMIT 20;

-- All failed attempts with error details
SELECT
    venue,
    tx_signature,
    slot,
    error_code,
    error_message,
    repay_amount,
    liquidator
FROM failed_liquidation_attempts
ORDER BY block_time DESC
LIMIT 20;

-- Liquidations per venue
SELECT venue, count() AS total FROM liquidations GROUP BY venue ORDER BY total DESC;

-- Failed attempts per venue with error breakdown
SELECT
    venue,
    error_code,
    error_message,
    count() AS occurrences
FROM failed_liquidation_attempts
GROUP BY venue, error_code, error_message
ORDER BY occurrences DESC;

-- Unique liquidators per venue
SELECT venue, uniq(liquidator) AS unique_liquidators FROM liquidations GROUP BY venue;

-- Average repay amount per venue
SELECT venue, avg(repay_amount) AS avg_repay, max(repay_amount) AS max_repay
FROM liquidations GROUP BY venue;

-- Jito tips analysis
SELECT
    venue,
    countIf(jito_tip_lamports IS NOT NULL) AS tipped,
    countIf(jito_tip_lamports IS NULL) AS untipped,
    avg(jito_tip_lamports) AS avg_tip,
    max(jito_tip_lamports) AS max_tip
FROM liquidations
GROUP BY venue;

-- Flash loan usage
SELECT
    venue,
    used_flashloan,
    flashloan_source,
    count() AS total
FROM liquidations
GROUP BY venue, used_flashloan, flashloan_source
ORDER BY venue, total DESC;

-- Check for null fields that shouldn't be null
SELECT
    'liquidator' AS field, countIf(liquidator = '') AS empty_count FROM liquidations
UNION ALL SELECT 'obligation', countIf(obligation = '') FROM liquidations
UNION ALL SELECT 'market', countIf(market = '') FROM liquidations
UNION ALL SELECT 'collateral_reserve', countIf(collateral_reserve = '') FROM liquidations
UNION ALL SELECT 'debt_reserve', countIf(debt_reserve = '') FROM liquidations
UNION ALL SELECT 'raw_ix_data', countIf(raw_ix_data = '') FROM liquidations;

-- Full detail of a single liquidation (replace the signature)
-- SELECT * FROM liquidations WHERE tx_signature = '...' FORMAT Vertical;
