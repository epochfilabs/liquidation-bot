# Phase 2 Report — Data Modelling

## What was built

### Files produced

```
schema/
├── MODEL.md                        — 500+ line canonical data model documentation
└── migrations/
    └── 001_initial_schema.sql      — 280 line ClickHouse DDL (6 tables + 4 materialized views)

analysis/
└── sanity.sql                      — 10 validation queries
```

### Tables

| Table | Engine | Purpose | Partition | Order By |
|---|---|---|---|---|
| `liquidations` | ReplacingMergeTree | Successful liquidations | `toYYYYMM(block_time)` | `(venue, block_time, tx_signature, ix_index)` |
| `failed_liquidation_attempts` | ReplacingMergeTree | Failed liquidation txs | `toYYYYMM(block_time)` | `(venue, block_time, tx_signature, ix_index)` |
| `obligations_snapshots` | ReplacingMergeTree | Borrower state at liquidation | `toYYYYMM(block_time)` | `(venue, block_time, tx_signature, ix_index)` |
| `reserves_snapshots` | ReplacingMergeTree | Reserve/bank state at liquidation | `toYYYYMM(block_time)` | `(venue, slot, reserve, role)` |
| `tx_metadata` | ReplacingMergeTree | Transaction-level info | `toYYYYMM(block_time)` | `(block_time, tx_signature)` |
| `_indexer_progress` | ReplacingMergeTree | Backfill cursor per venue | None | `(venue, source)` |

### Materialized views

| View | Engine | Source | Purpose |
|---|---|---|---|
| `mv_daily_volume` | SummingMergeTree | `liquidations` | Daily USD volume per venue |
| `mv_top_liquidators` | **AggregatingMergeTree** | `liquidations` | Top liquidators with correct min/max timestamps |
| `mv_tip_profit_ratio` | SummingMergeTree | `liquidations` (Jito-tipped only) | Tip effectiveness per quarter |
| `mv_success_monthly` + `mv_failure_monthly` → `_success_failure_monthly` | SummingMergeTree | `liquidations` + `failed_liquidation_attempts` | Monthly success/failure ratio (two MVs feeding one target) |

### Sanity queries (10 total)

1. Top 20 liquidators by profit per venue
2. Daily liquidation USD volume stacked by venue
3. Median Jito tip as fraction of profit by venue/quarter
4. Failed vs. successful ratio per venue per month
5. Liquidation latency distribution (slot delta)
6. Collateral/debt pair dominance
7. Jito tip effectiveness by bucket
8. Searcher concentration (how many for 80% of profit)
9. Indexer coverage check
10. Data freshness (most recent per venue)

## Self-critique process

Ran a dedicated review pass in the role of "skeptical data modeller." Found 15 issues:

- **2 critical**: Invalid MV syntax (UNION ALL in MV), corrupted timestamps in SummingMergeTree
- **3 high**: Wrong Nullable wrapping order, missing ORDER BY columns, missing Nullable
- **5 medium**: Venue-specific column leak, type sizing, missing denormalized fields
- **5 low**: Performance, precision, and ergonomic improvements

All critical and high issues were fixed in the DDL. Medium issues were either fixed or documented as accepted risk with a boundary condition for when to revisit.

## Key design decisions

1. **`venue_ext` JSON column in reserves_snapshots** instead of per-venue columns. MarginFi needs `asset_share_value`, `liability_share_value`, `maint_asset_weight`, `maint_liability_weight`; Kamino needs elevation group info; Save needs raw bonus config. Rather than 10+ nullable columns, a single JSON string holds venue-specific extensions. Queries like `JSONExtractFloat(venue_ext, 'asset_share_value')` work in ClickHouse for ad-hoc analysis.

2. **Denormalized obligation size on liquidations table.** `obligation_deposited_usd` and `obligation_borrowed_usd` are copied from the obligation snapshot to avoid the most common join (filtering by position size). This trades ~16 bytes/row for eliminating a join in 80% of analytical queries.

3. **`Decimal128(12)` for prices instead of `Decimal64(12)`.** Tokens priced in unusual units (e.g., price per smallest unit of a 0-decimal token) can exceed Decimal64's 6-digit integer range. Decimal128 gives 26 integer digits — overkill but safe.

4. **`Float64` for ratios instead of `Decimal64(6)`.** LTV, health_factor, and utilization are ratios (0.0–1.0 or similar). Float64 is natural, cheaper, and avoids the fixed-point overhead. No financial rounding concerns since these are analytical, not accounting values.

## What surprised me

1. **ClickHouse materialized views can't UNION ALL across tables.** This is a fundamental limitation — each MV must have exactly one source table. The workaround (two MVs writing to one target) is standard but not obvious.

2. **SummingMergeTree silently sums ALL non-key numeric columns on merge.** If you put `min(block_time)` as a column in a SummingMergeTree, after a background merge you get `timestamp_a + timestamp_b` — a garbage value. AggregatingMergeTree with explicit `State` functions is the correct pattern.

3. **The venue-specific column problem is harder than it looks.** Five columns today is fine. But each protocol has 3-5 unique fields, and new features (Kamino obligation orders, MarginFi e-mode, Jupiter absorption) keep adding more. The `venue_ext` JSON escape valve is intentional — it lets the schema remain stable while venue-specific analysis evolves.

## Open questions

1. **Should `raw_ix_data` be hex or base64?** Hex is human-readable but 2x the size. Base64 is compact. The DDL currently documents it as hex; may switch to base64 for storage efficiency.

2. **Competing attempts correlation.** The self-critique flagged `competing_attempts_count` as missing. Computing this requires a self-join on `(obligation, slot)` across both `liquidations` and `failed_liquidation_attempts`. This is feasible as a query but expensive to materialize. Deferred to Phase 5 as an analysis query rather than a stored column.

3. **Leader/validator identity.** Useful for tip strategy but requires the leader schedule (available from `getLeaderSchedule` RPC). Could be added as a column in `tx_metadata` during enrichment. Deferred.
