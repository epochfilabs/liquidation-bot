-- ============================================================================
-- Solana Liquidation Indexer — ClickHouse Schema
-- Migration 001: Initial schema
--
-- Fixes applied from self-critique review:
--   1. LowCardinality(Nullable(String)) wrapping order corrected
--   2. reserves_snapshots ORDER BY includes `role`
--   3. obligations_snapshots ORDER BY includes `block_time` prefix
--   4. mv_success_failure_ratio split into two MVs feeding one target
--   5. mv_top_liquidators uses AggregatingMergeTree
--   6. liquidation_threshold_pct widened to UInt16
--   7. owner in obligations_snapshots made Nullable
--   8. Denormalized obligation size onto liquidations table
--   9. tx_metadata ORDER BY includes block_time
-- ============================================================================

-- -------------------------------------------------------
-- liquidations
-- -------------------------------------------------------

CREATE TABLE IF NOT EXISTS liquidations
(
    -- Identity
    venue                       LowCardinality(String),
    program_id                  FixedString(44),
    slot                        UInt64,
    block_time                  DateTime64(3, 'UTC'),
    tx_signature                FixedString(88),
    ix_index                    UInt16,
    inner_ix_index              Nullable(UInt16),

    -- Participants
    liquidator                  FixedString(44),
    liquidatee                  Nullable(FixedString(44)),       -- NULL for Jupiter Lend (tick-based)
    obligation                  FixedString(44),
    market                      FixedString(44),

    -- Collateral & Debt
    collateral_reserve          FixedString(44),
    debt_reserve                FixedString(44),
    collateral_mint             FixedString(44),
    debt_mint                   FixedString(44),
    repay_amount                UInt128,
    withdraw_amount             UInt128,

    -- USD Values (enriched — nullable because oracle may be unavailable)
    repay_amount_usd            Nullable(Decimal64(6)),
    collateral_seized_usd       Nullable(Decimal64(6)),
    liquidator_profit_usd       Nullable(Decimal64(6)),
    collateral_price            Nullable(Decimal128(12)),
    debt_price                  Nullable(Decimal128(12)),

    -- Denormalized obligation size (avoids join for size-based filtering)
    obligation_deposited_usd    Nullable(Decimal64(6)),
    obligation_borrowed_usd     Nullable(Decimal64(6)),

    -- Bonus & Fees
    liquidation_bonus_bps       Nullable(UInt32),
    close_factor_pct            Nullable(UInt16),               -- Max % of debt repayable
    protocol_fee_amount         Nullable(UInt128),              -- Protocol fee from bonus (Kamino, Save)
    insurance_fee_amount        Nullable(UInt128),              -- Insurance fee (MarginFi)

    -- Transaction metadata
    tx_fee_lamports             UInt64,
    priority_fee_lamports       UInt64,
    jito_tip_lamports           Nullable(UInt64),
    compute_units_consumed      UInt32,

    -- Bundling
    used_flashloan              Bool,
    flashloan_source            LowCardinality(Nullable(String)),  -- kamino, jupiter_lend, marginfi, save, external
    used_jupiter_swap           Bool,

    -- Venue-specific (nullable — only populated for relevant venue)
    liquidation_reason          LowCardinality(Nullable(String)),  -- Kamino only
    tick_start                  Nullable(Int32),                    -- Jupiter Lend only
    tick_end                    Nullable(Int32),                    -- Jupiter Lend only
    absorbed_bad_debt           Nullable(Bool),                     -- Jupiter Lend only

    -- Instruction data
    raw_ix_data                 String,

    -- Housekeeping
    ingested_at                 DateTime DEFAULT now()
)
ENGINE = ReplacingMergeTree(ingested_at)
PARTITION BY toYYYYMM(block_time)
ORDER BY (venue, block_time, tx_signature, ix_index);


-- -------------------------------------------------------
-- failed_liquidation_attempts
-- -------------------------------------------------------

CREATE TABLE IF NOT EXISTS failed_liquidation_attempts
(
    -- Same structure as liquidations
    venue                       LowCardinality(String),
    program_id                  FixedString(44),
    slot                        UInt64,
    block_time                  DateTime64(3, 'UTC'),
    tx_signature                FixedString(88),
    ix_index                    UInt16,
    inner_ix_index              Nullable(UInt16),

    liquidator                  FixedString(44),
    liquidatee                  Nullable(FixedString(44)),
    obligation                  FixedString(44),
    market                      FixedString(44),

    collateral_reserve          FixedString(44),
    debt_reserve                FixedString(44),
    collateral_mint             FixedString(44),
    debt_mint                   FixedString(44),
    repay_amount                Nullable(UInt128),              -- Nullable: may not be extractable from failed tx
    withdraw_amount             Nullable(UInt128),              -- Nullable: always 0 or NULL for failures

    repay_amount_usd            Nullable(Decimal64(6)),
    collateral_seized_usd       Nullable(Decimal64(6)),
    liquidator_profit_usd       Nullable(Decimal64(6)),
    collateral_price            Nullable(Decimal128(12)),
    debt_price                  Nullable(Decimal128(12)),

    obligation_deposited_usd    Nullable(Decimal64(6)),
    obligation_borrowed_usd     Nullable(Decimal64(6)),

    liquidation_bonus_bps       Nullable(UInt32),
    close_factor_pct            Nullable(UInt16),
    protocol_fee_amount         Nullable(UInt128),
    insurance_fee_amount        Nullable(UInt128),

    tx_fee_lamports             UInt64,
    priority_fee_lamports       UInt64,
    jito_tip_lamports           Nullable(UInt64),
    compute_units_consumed      UInt32,

    used_flashloan              Bool,
    flashloan_source            LowCardinality(Nullable(String)),
    used_jupiter_swap           Bool,

    liquidation_reason          LowCardinality(Nullable(String)),
    tick_start                  Nullable(Int32),
    tick_end                    Nullable(Int32),
    absorbed_bad_debt           Nullable(Bool),

    raw_ix_data                 String,

    -- Error info (additional columns)
    error_code                  Nullable(UInt32),
    error_message               LowCardinality(Nullable(String)),

    ingested_at                 DateTime DEFAULT now()
)
ENGINE = ReplacingMergeTree(ingested_at)
PARTITION BY toYYYYMM(block_time)
ORDER BY (venue, block_time, tx_signature, ix_index);


-- -------------------------------------------------------
-- obligations_snapshots
-- -------------------------------------------------------

CREATE TABLE IF NOT EXISTS obligations_snapshots
(
    venue                       LowCardinality(String),
    slot                        UInt64,
    block_time                  DateTime64(3, 'UTC'),
    tx_signature                FixedString(88),       -- FK to liquidations
    ix_index                    UInt16,                 -- FK to liquidations

    obligation                  FixedString(44),
    owner                       Nullable(FixedString(44)),  -- NULL for Jupiter Lend
    market                      FixedString(44),

    -- Health
    deposited_value_usd         Nullable(Decimal64(6)),
    borrowed_value_usd          Nullable(Decimal64(6)),
    ltv                         Nullable(Float64),
    unhealthy_ltv               Nullable(Float64),
    health_factor               Nullable(Float64),          -- MarginFi only

    -- Positions (JSON arrays — variable-length per venue)
    deposits                    String,     -- JSON: [{"reserve":"...","mint":"...","amount":123,"value_usd":1.23}]
    borrows                     String,     -- JSON: [{"reserve":"...","mint":"...","amount_sf":123,"value_usd":1.23}]

    -- Raw
    obligation_data_b64         Nullable(String),

    ingested_at                 DateTime DEFAULT now()
)
ENGINE = ReplacingMergeTree(ingested_at)
PARTITION BY toYYYYMM(block_time)
ORDER BY (venue, block_time, tx_signature, ix_index);


-- -------------------------------------------------------
-- reserves_snapshots
-- -------------------------------------------------------

CREATE TABLE IF NOT EXISTS reserves_snapshots
(
    venue                       LowCardinality(String),
    slot                        UInt64,
    block_time                  DateTime64(3, 'UTC'),
    reserve                     FixedString(44),
    market                      FixedString(44),
    mint                        FixedString(44),
    role                        LowCardinality(String),     -- 'repay' or 'withdraw'

    -- Liquidity
    available_liquidity         Nullable(UInt128),
    total_borrows               Nullable(UInt128),
    utilization_pct             Nullable(Decimal64(4)),

    -- Config
    liquidation_threshold_bps   Nullable(UInt16),           -- Basis points (UInt16 supports up to 65535)
    liquidation_bonus_bps       Nullable(UInt32),
    max_liquidation_bonus_bps   Nullable(UInt32),
    protocol_liquidation_fee_bps Nullable(UInt16),          -- Basis points
    flash_loan_fee_bps          Nullable(UInt32),

    -- Price
    oracle_price                Nullable(Decimal128(12)),
    oracle_source               LowCardinality(Nullable(String)),

    -- Venue extension data (JSON — avoids per-venue column sprawl)
    -- MarginFi: {"asset_share_value": 1.05, "liability_share_value": 1.02, ...}
    -- Kamino: {"elevation_group": 0, ...}
    -- Save: {"liquidation_bonus_pct": 5, ...}
    venue_ext                   Nullable(String),           -- JSON for venue-specific fields

    reserve_data_b64            Nullable(String),
    ingested_at                 DateTime DEFAULT now()
)
ENGINE = ReplacingMergeTree(ingested_at)
PARTITION BY toYYYYMM(block_time)
ORDER BY (venue, slot, reserve, role);


-- -------------------------------------------------------
-- tx_metadata
-- -------------------------------------------------------

CREATE TABLE IF NOT EXISTS tx_metadata
(
    tx_signature                FixedString(88),
    slot                        UInt64,
    block_time                  DateTime64(3, 'UTC'),
    succeeded                   Bool,
    fee_lamports                UInt64,
    priority_fee_lamports       UInt64,
    jito_tip_lamports           Nullable(UInt64),
    compute_units_consumed      UInt32,
    compute_units_requested     Nullable(UInt32),
    num_instructions            UInt16,
    num_inner_instructions      UInt16,
    signers                     Array(FixedString(44)),
    fee_payer                   FixedString(44),
    uses_address_lookup_table   Bool,

    ingested_at                 DateTime DEFAULT now()
)
ENGINE = ReplacingMergeTree(ingested_at)
PARTITION BY toYYYYMM(block_time)
ORDER BY (block_time, tx_signature);


-- -------------------------------------------------------
-- _indexer_progress
-- -------------------------------------------------------

CREATE TABLE IF NOT EXISTS _indexer_progress
(
    venue                       LowCardinality(String),
    source                      LowCardinality(String),     -- 'old_faithful', 'grpc_realtime'
    last_epoch                  Nullable(UInt32),
    last_slot                   UInt64,
    last_signature              Nullable(FixedString(88)),
    backfill_state              LowCardinality(String),     -- 'pending', 'in_progress', 'complete', 'failed'
    rows_liquidations           UInt64,
    rows_failed                 UInt64,
    rows_total_processed        UInt64,
    error_message               Nullable(String),
    updated_at                  DateTime DEFAULT now()
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (venue, source);


-- ============================================================================
-- Materialized Views
-- ============================================================================

-- -------------------------------------------------------
-- mv_daily_volume: Daily liquidation USD volume per venue
-- -------------------------------------------------------

CREATE MATERIALIZED VIEW IF NOT EXISTS mv_daily_volume
ENGINE = SummingMergeTree()
ORDER BY (venue, day)
AS SELECT
    venue,
    toDate(block_time) AS day,
    count() AS liquidation_count,
    sum(repay_amount_usd) AS total_repay_usd,
    sum(collateral_seized_usd) AS total_collateral_usd,
    sum(liquidator_profit_usd) AS total_profit_usd
FROM liquidations
WHERE repay_amount_usd IS NOT NULL
GROUP BY venue, day;


-- -------------------------------------------------------
-- mv_top_liquidators: Top liquidators by profit
-- Uses AggregatingMergeTree to correctly handle min/max timestamps
-- -------------------------------------------------------

CREATE MATERIALIZED VIEW IF NOT EXISTS mv_top_liquidators
ENGINE = AggregatingMergeTree()
ORDER BY (venue, liquidator)
AS SELECT
    venue,
    liquidator,
    countState() AS liquidation_count,
    sumState(liquidator_profit_usd) AS total_profit_usd,
    sumState(jito_tip_lamports) AS total_tips_lamports,
    minState(block_time) AS first_seen,
    maxState(block_time) AS last_seen
FROM liquidations
GROUP BY venue, liquidator;


-- -------------------------------------------------------
-- mv_tip_profit_ratio: Tip as fraction of profit, by venue and quarter
-- Only includes Jito-tipped transactions
-- -------------------------------------------------------

CREATE MATERIALIZED VIEW IF NOT EXISTS mv_tip_profit_ratio
ENGINE = SummingMergeTree()
ORDER BY (venue, quarter)
AS SELECT
    venue,
    toStartOfQuarter(block_time) AS quarter,
    count() AS tipped_count,
    sum(jito_tip_lamports) AS total_tips_lamports,
    sum(liquidator_profit_usd) AS total_profit_usd,
    sum(tx_fee_lamports + priority_fee_lamports) AS total_fees_lamports
FROM liquidations
WHERE jito_tip_lamports IS NOT NULL
GROUP BY venue, quarter;


-- -------------------------------------------------------
-- mv_success_failure_monthly: Combined success/failure counts
-- FIX: Two separate MVs feed one target table (UNION ALL not allowed in MV)
-- -------------------------------------------------------

CREATE TABLE IF NOT EXISTS _success_failure_monthly
(
    venue                       LowCardinality(String),
    month                       Date,
    succeeded                   Bool,
    count                       UInt64
)
ENGINE = SummingMergeTree()
ORDER BY (venue, month, succeeded);

CREATE MATERIALIZED VIEW IF NOT EXISTS mv_success_monthly
TO _success_failure_monthly
AS SELECT
    venue,
    toStartOfMonth(block_time) AS month,
    true AS succeeded,
    count() AS count
FROM liquidations
GROUP BY venue, month;

CREATE MATERIALIZED VIEW IF NOT EXISTS mv_failure_monthly
TO _success_failure_monthly
AS SELECT
    venue,
    toStartOfMonth(block_time) AS month,
    false AS succeeded,
    count() AS count
FROM failed_liquidation_attempts
GROUP BY venue, month;
