# Canonical Data Model — Solana Liquidation Indexer

## Design Principles

1. **One row per on-chain event**, not one row per concept. A liquidation that touches two reserves produces one `liquidations` row, not two.
2. **Nullable over fabricated.** If a field can't be populated for a venue (e.g., `liquidatee` for Jupiter Lend), it's `Nullable`, not filled with a sentinel.
3. **Pubkeys as FixedString(44).** Base58-encoded Solana pubkeys are always 32–44 chars. `FixedString(44)` avoids the overhead of variable-length `String` at the cost of trailing null padding for shorter keys.
4. **Amounts as UInt128.** Solana token amounts can exceed u64 when WAD-scaled (Save) or SF-scaled (Kamino). UInt128 covers all venues without lossy conversion.
5. **USD values as Decimal64(6).** Six decimal places ($0.000001 precision) — sufficient for liquidation profit analysis. Derived from oracle prices at the liquidation slot.
6. **LowCardinality for categorical columns.** `venue`, `flashloan_source`, `liquidation_reason` have <20 distinct values — LowCardinality gives dictionary encoding.
7. **ReplacingMergeTree for idempotency.** The `(tx_signature, ix_index)` pair uniquely identifies an instruction. Re-processing the same epoch/block writes the same row, which `ReplacingMergeTree` deduplicates on merge.

## Table: `liquidations`

One row per **successful** liquidation instruction execution.

### Primary Key / Ordering

```
ENGINE = ReplacingMergeTree(ingested_at)
PARTITION BY toYYYYMM(block_time)
ORDER BY (venue, block_time, tx_signature, ix_index)
```

### Columns

| Column | Type | Nullable | Description |
|---|---|---|---|
| **Identity** | | | |
| `venue` | `LowCardinality(String)` | no | `kamino`, `jupiter_lend`, `marginfi`, `save` |
| `program_id` | `FixedString(44)` | no | Program that executed the liquidation |
| `slot` | `UInt64` | no | Solana slot number |
| `block_time` | `DateTime64(3, 'UTC')` | no | Block timestamp (ms precision) |
| `tx_signature` | `FixedString(88)` | no | Base58 tx signature |
| `ix_index` | `UInt16` | no | Instruction index within the transaction |
| `inner_ix_index` | `Nullable(UInt16)` | yes | If liquidation was a CPI inner instruction |
| **Participants** | | | |
| `liquidator` | `FixedString(44)` | no | Liquidator signer pubkey |
| `liquidatee` | `Nullable(FixedString(44))` | yes | Obligation owner / marginfi account authority. **NULL for Jupiter Lend** (tick-based, no per-position liquidatee). |
| `obligation` | `FixedString(44)` | no | Obligation / MarginfiAccount / VaultConfig pubkey |
| `market` | `FixedString(44)` | no | Lending market / MarginFi group / vault_config PDA |
| **Collateral & Debt** | | | |
| `collateral_reserve` | `FixedString(44)` | no | Withdraw reserve / asset bank / vault_config |
| `debt_reserve` | `FixedString(44)` | no | Repay reserve / liability bank / vault_config |
| `collateral_mint` | `FixedString(44)` | no | Mint of collateral token |
| `debt_mint` | `FixedString(44)` | no | Mint of debt token |
| `repay_amount` | `UInt128` | no | Debt repaid (native units). For MarginFi: derived from balance changes, not instruction arg. |
| `withdraw_amount` | `UInt128` | no | Collateral seized (native units). From pre/post token balances. |
| **USD Values (enriched)** | | | |
| `repay_amount_usd` | `Nullable(Decimal64(6))` | yes | Debt repaid in USD. Null if oracle price unavailable. |
| `collateral_seized_usd` | `Nullable(Decimal64(6))` | yes | Collateral value in USD |
| `liquidator_profit_usd` | `Nullable(Decimal64(6))` | yes | `collateral_seized_usd - repay_amount_usd - flash_loan_fee_usd` |
| `collateral_price` | `Nullable(Decimal64(12))` | yes | Price per smallest unit at liquidation slot |
| `debt_price` | `Nullable(Decimal64(12))` | yes | Price per smallest unit at liquidation slot |
| **Bonus & Fees** | | | |
| `liquidation_bonus_bps` | `Nullable(UInt32)` | yes | Effective bonus in basis points |
| `protocol_fee_amount` | `Nullable(UInt128)` | yes | Protocol fee taken from bonus (Kamino, Save) |
| `insurance_fee_amount` | `Nullable(UInt128)` | yes | Insurance fee (MarginFi only) |
| **Transaction metadata** | | | |
| `tx_fee_lamports` | `UInt64` | no | Base transaction fee |
| `priority_fee_lamports` | `UInt64` | no | Compute budget priority fee |
| `jito_tip_lamports` | `Nullable(UInt64)` | yes | Jito tip if detected |
| `compute_units_consumed` | `UInt32` | no | CU used |
| **Bundling** | | | |
| `used_flashloan` | `Bool` | no | Whether a flash loan was used |
| `flashloan_source` | `Nullable(LowCardinality(String))` | yes | `kamino`, `jupiter_lend`, `marginfi`, `save`, `external` |
| `used_jupiter_swap` | `Bool` | no | Whether a Jupiter DEX swap was included |
| **Venue-specific** | | | |
| `liquidation_reason` | `Nullable(LowCardinality(String))` | yes | Kamino: `ltv_exceeded`, `deleveraging`, `debt_maturity`, `obligation_order` |
| `tick_start` | `Nullable(Int32)` | yes | Jupiter Lend: start tick of liquidated range |
| `tick_end` | `Nullable(Int32)` | yes | Jupiter Lend: end tick of liquidated range |
| `absorbed_bad_debt` | `Nullable(Bool)` | yes | Jupiter Lend: whether absorption phase ran |
| **Instruction data** | | | |
| `raw_ix_data` | `String` | no | Hex-encoded raw instruction data |
| **Housekeeping** | | | |
| `ingested_at` | `DateTime DEFAULT now()` | no | When this row was written (ReplacingMergeTree version) |

### Notes

- `repay_amount` for MarginFi is derived from pre/post balance changes since the instruction arg is `asset_amount` (collateral), not debt.
- `liquidatee` is NULL for Jupiter Lend because tick-based liquidation has no per-position target in the transaction.
- `collateral_reserve` and `debt_reserve` for Jupiter Lend both point to `vault_config` since the vault is a single pair.
- All `_usd` fields are nullable because oracle price enrichment may fail (oracle account not available at that slot).

---

## Table: `failed_liquidation_attempts`

Same structure as `liquidations` plus error information. One row per **failed** transaction that contained a liquidation instruction.

### Engine

```
ENGINE = ReplacingMergeTree(ingested_at)
PARTITION BY toYYYYMM(block_time)
ORDER BY (venue, block_time, tx_signature, ix_index)
```

### Additional Columns (beyond liquidations)

| Column | Type | Description |
|---|---|---|
| `error_code` | `Nullable(UInt32)` | Program error code (e.g., 6016 for Kamino's ObligationHealthy) |
| `error_message` | `Nullable(String)` | Human-readable error from logs |

### Notes

- USD value columns will mostly be NULL since the transaction failed.
- `withdraw_amount` will be 0 (no collateral seized).
- `repay_amount` reflects what was *attempted*, from instruction args.
- Critical for bot tuning: shows what searchers tried but didn't land.

---

## Table: `obligations_snapshots`

Borrower state at the slot immediately **before** or **at** the liquidation. One row per liquidation event in `liquidations`. This is what a liquidatable position looks like right before it gets hit.

### Engine

```
ENGINE = ReplacingMergeTree(ingested_at)
PARTITION BY toYYYYMM(block_time)
ORDER BY (venue, tx_signature, ix_index)
```

### Columns

| Column | Type | Nullable | Description |
|---|---|---|---|
| `venue` | `LowCardinality(String)` | no | |
| `slot` | `UInt64` | no | Slot of the liquidation |
| `block_time` | `DateTime64(3, 'UTC')` | no | |
| `tx_signature` | `FixedString(88)` | no | FK to `liquidations` |
| `ix_index` | `UInt16` | no | FK to `liquidations` |
| `obligation` | `FixedString(44)` | no | Obligation / MarginfiAccount pubkey |
| `owner` | `FixedString(44)` | no | Obligation owner wallet |
| `market` | `FixedString(44)` | no | Lending market / group |
| **Health** | | | |
| `deposited_value_usd` | `Nullable(Decimal64(6))` | yes | Total deposit value |
| `borrowed_value_usd` | `Nullable(Decimal64(6))` | yes | Total borrow value |
| `ltv` | `Nullable(Decimal64(6))` | yes | Loan-to-value ratio |
| `unhealthy_ltv` | `Nullable(Decimal64(6))` | yes | Threshold LTV for liquidation |
| `health_factor` | `Nullable(Decimal64(6))` | yes | MarginFi: weighted_assets - weighted_liabilities |
| **Positions (JSON arrays)** | | | |
| `deposits` | `String` | no | JSON array: `[{"reserve":"...","mint":"...","amount":123,"value_usd":1.23}, ...]` |
| `borrows` | `String` | no | JSON array: `[{"reserve":"...","mint":"...","amount_sf":123,"value_usd":1.23}, ...]` |
| **Raw** | | | |
| `obligation_data_b64` | `Nullable(String)` | yes | Base64-encoded raw account data (for re-analysis) |
| `ingested_at` | `DateTime DEFAULT now()` | no | |

### Notes

- `deposits` and `borrows` are JSON strings, not nested ClickHouse arrays, because position count varies (Kamino: up to 8 deposits + 5 borrows; Save: variable; MarginFi: up to 16 balances).
- `health_factor` is only populated for MarginFi (weighted asset/liability model). Kamino and Save use `ltv` / `unhealthy_ltv`.
- For Jupiter Lend, this table stores vault-level state (total_supply, total_borrow, topmost_tick) rather than per-position state, since liquidation is vault-wide.

---

## Table: `reserves_snapshots`

Reserve/bank state at the liquidation slot. One row per reserve involved in a liquidation (typically 2 per event: repay + withdraw).

### Engine

```
ENGINE = ReplacingMergeTree(ingested_at)
PARTITION BY toYYYYMM(block_time)
ORDER BY (venue, slot, reserve)
```

### Columns

| Column | Type | Nullable | Description |
|---|---|---|---|
| `venue` | `LowCardinality(String)` | no | |
| `slot` | `UInt64` | no | |
| `block_time` | `DateTime64(3, 'UTC')` | no | |
| `reserve` | `FixedString(44)` | no | Reserve / Bank pubkey |
| `market` | `FixedString(44)` | no | Lending market / group |
| `mint` | `FixedString(44)` | no | Token mint |
| `role` | `LowCardinality(String)` | no | `repay` or `withdraw` |
| **Liquidity** | | | |
| `available_liquidity` | `Nullable(UInt128)` | yes | Available tokens in the reserve |
| `total_borrows` | `Nullable(UInt128)` | yes | Total borrowed (may be SF-scaled) |
| `utilization_pct` | `Nullable(Decimal64(4))` | yes | Utilization rate (0-100) |
| **Config** | | | |
| `liquidation_threshold_pct` | `Nullable(UInt8)` | yes | LTV threshold for liquidation |
| `liquidation_bonus_bps` | `Nullable(UInt32)` | yes | Configured bonus (min or fixed) |
| `max_liquidation_bonus_bps` | `Nullable(UInt32)` | yes | Max bonus (Kamino) |
| `protocol_liquidation_fee_pct` | `Nullable(UInt8)` | yes | Protocol's cut of bonus |
| `flash_loan_fee_bps` | `Nullable(UInt32)` | yes | Flash loan fee |
| **Price** | | | |
| `oracle_price` | `Nullable(Decimal64(12))` | yes | Oracle price at this slot |
| `oracle_source` | `Nullable(LowCardinality(String))` | yes | `pyth`, `switchboard`, `scope`, `jupiter_oracle`, `chainlink` |
| **MarginFi-specific** | | | |
| `asset_share_value` | `Nullable(Decimal64(12))` | yes | Shares → underlying multiplier |
| `liability_share_value` | `Nullable(Decimal64(12))` | yes | |
| `maint_asset_weight` | `Nullable(Decimal64(6))` | yes | Maintenance weight |
| `maint_liability_weight` | `Nullable(Decimal64(6))` | yes | |
| `reserve_data_b64` | `Nullable(String)` | yes | Base64-encoded raw account data |
| `ingested_at` | `DateTime DEFAULT now()` | no | |

---

## Table: `tx_metadata`

Shared transaction-level info. One row per unique transaction signature that contains a liquidation (successful or failed). Joined from `liquidations` and `failed_liquidation_attempts`.

### Engine

```
ENGINE = ReplacingMergeTree(ingested_at)
PARTITION BY toYYYYMM(block_time)
ORDER BY (tx_signature)
```

### Columns

| Column | Type | Nullable | Description |
|---|---|---|---|
| `tx_signature` | `FixedString(88)` | no | |
| `slot` | `UInt64` | no | |
| `block_time` | `DateTime64(3, 'UTC')` | no | |
| `succeeded` | `Bool` | no | Whether the transaction succeeded |
| `fee_lamports` | `UInt64` | no | Base fee |
| `priority_fee_lamports` | `UInt64` | no | Priority fee from ComputeBudget |
| `jito_tip_lamports` | `Nullable(UInt64)` | yes | Jito bundle tip |
| `compute_units_consumed` | `UInt32` | no | CU consumed |
| `compute_units_requested` | `Nullable(UInt32)` | yes | CU limit from ComputeBudget |
| `num_instructions` | `UInt16` | no | Total instruction count |
| `num_inner_instructions` | `UInt16` | no | Total inner instruction count |
| `signers` | `Array(FixedString(44))` | no | All transaction signers |
| `fee_payer` | `FixedString(44)` | no | First signer (fee payer) |
| `uses_address_lookup_table` | `Bool` | no | Whether v0 tx with ALTs |
| `ingested_at` | `DateTime DEFAULT now()` | no | |

---

## Table: `_indexer_progress`

Per-venue cursor for backfill state. Non-replicated — single writer.

### Engine

```
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (venue, source)
```

### Columns

| Column | Type | Description |
|---|---|---|
| `venue` | `LowCardinality(String)` | Protocol name |
| `source` | `LowCardinality(String)` | `old_faithful`, `grpc_realtime` |
| `last_epoch` | `Nullable(UInt32)` | Last fully processed epoch |
| `last_slot` | `UInt64` | Last processed slot |
| `last_signature` | `Nullable(FixedString(88))` | Last processed signature |
| `backfill_state` | `LowCardinality(String)` | `pending`, `in_progress`, `complete`, `failed` |
| `rows_liquidations` | `UInt64` | Running count of liquidation rows |
| `rows_failed` | `UInt64` | Running count of failed attempt rows |
| `rows_total_processed` | `UInt64` | Total transactions processed |
| `error_message` | `Nullable(String)` | Last error if failed |
| `updated_at` | `DateTime DEFAULT now()` | |

---

## Materialized Views

### `mv_daily_volume`

Daily liquidation USD volume per venue.

```sql
CREATE MATERIALIZED VIEW mv_daily_volume
ENGINE = SummingMergeTree()
ORDER BY (venue, day)
AS SELECT
    venue,
    toDate(block_time) AS day,
    count() AS liquidation_count,
    sumIf(repay_amount_usd, repay_amount_usd IS NOT NULL) AS total_repay_usd,
    sumIf(collateral_seized_usd, collateral_seized_usd IS NOT NULL) AS total_collateral_usd,
    sumIf(liquidator_profit_usd, liquidator_profit_usd IS NOT NULL) AS total_profit_usd
FROM liquidations
GROUP BY venue, day
```

### `mv_top_liquidators`

Top liquidators by realized profit (updated incrementally).

```sql
CREATE MATERIALIZED VIEW mv_top_liquidators
ENGINE = SummingMergeTree()
ORDER BY (venue, liquidator)
AS SELECT
    venue,
    liquidator,
    count() AS liquidation_count,
    sumIf(liquidator_profit_usd, liquidator_profit_usd IS NOT NULL) AS total_profit_usd,
    sumIf(jito_tip_lamports, jito_tip_lamports IS NOT NULL) AS total_tips_lamports,
    min(block_time) AS first_seen,
    max(block_time) AS last_seen
FROM liquidations
GROUP BY venue, liquidator
```

### `mv_tip_profit_ratio`

Tip as fraction of profit, bucketed by venue and quarter.

```sql
CREATE MATERIALIZED VIEW mv_tip_profit_ratio
ENGINE = SummingMergeTree()
ORDER BY (venue, quarter)
AS SELECT
    venue,
    toStartOfQuarter(block_time) AS quarter,
    count() AS count,
    sumIf(jito_tip_lamports, jito_tip_lamports IS NOT NULL) AS total_tips,
    sumIf(liquidator_profit_usd, liquidator_profit_usd IS NOT NULL) AS total_profit_usd,
    sumIf(tx_fee_lamports + priority_fee_lamports, true) AS total_fees_lamports
FROM liquidations
WHERE jito_tip_lamports IS NOT NULL
GROUP BY venue, quarter
```

### `mv_success_failure_ratio`

Failed vs. successful ratio per venue per month.

```sql
CREATE MATERIALIZED VIEW mv_success_failure_ratio
ENGINE = SummingMergeTree()
ORDER BY (venue, month, succeeded)
AS SELECT
    venue,
    toStartOfMonth(block_time) AS month,
    true AS succeeded,
    count() AS count
FROM liquidations
GROUP BY venue, month
UNION ALL
SELECT
    venue,
    toStartOfMonth(block_time) AS month,
    false AS succeeded,
    count() AS count
FROM failed_liquidation_attempts
GROUP BY venue, month
```

---

---

## Self-Critique Review — Issues Found and Fixes Applied

The schema was reviewed by a skeptical data modeller. All critical issues were fixed in the DDL at `migrations/001_initial_schema.sql`. Summary:

| # | Issue | Severity | Fix Applied |
|---|---|---|---|
| 1 | `mv_success_failure_ratio` used UNION ALL (invalid for MV) | **Critical** | Split into two MVs (`mv_success_monthly`, `mv_failure_monthly`) feeding one target table (`_success_failure_monthly`) |
| 2 | `mv_top_liquidators` used `min/max(block_time)` in SummingMergeTree (corrupts on merge) | **Critical** | Changed to AggregatingMergeTree with `minState/maxState` |
| 3 | `Nullable(LowCardinality(String))` wrapping order inverted | **High** | Corrected to `LowCardinality(Nullable(String))` everywhere |
| 4 | `reserves_snapshots` ORDER BY missing `role` (dedup risk) | **High** | Added `role` to ORDER BY: `(venue, slot, reserve, role)` |
| 5 | `obligations_snapshots` ORDER BY missing time prefix (slow range queries) | **High** | Changed to `(venue, block_time, tx_signature, ix_index)` |
| 6 | `owner` in obligations_snapshots not Nullable (Jupiter Lend has no owner) | **Medium** | Made Nullable |
| 7 | `repay_amount`/`withdraw_amount` not Nullable in failed_attempts | **Medium** | Made Nullable in `failed_liquidation_attempts` |
| 8 | MarginFi-specific columns in reserves_snapshots (venue leak) | **Medium** | Replaced with `venue_ext` JSON column |
| 9 | `liquidation_threshold_pct` as UInt8 (too small for bps) | **Medium** | Changed to `liquidation_threshold_bps` as UInt16 |
| 10 | Missing denormalized obligation size on liquidations | **Low** | Added `obligation_deposited_usd`, `obligation_borrowed_usd` |
| 11 | Missing `close_factor_pct` | **Low** | Added to liquidations table |
| 12 | `tx_metadata` ORDER BY missing time prefix | **Low** | Changed to `(block_time, tx_signature)` |
| 13 | `collateral_price`/`debt_price` as Decimal64(12) — potential overflow | **Low** | Changed to `Decimal128(12)` |
| 14 | `ltv`/`health_factor` as Decimal64(6) — overkill for ratios | **Low** | Changed to `Float64` |
| 15 | `error_message` missing LowCardinality | **Low** | Changed to `LowCardinality(Nullable(String))` |

### Deferred issues (accepted risk)

- **Venue-specific columns in liquidations table**: 5 columns (`liquidation_reason`, `tick_start`, `tick_end`, `absorbed_bad_debt`, `insurance_fee_amount`) are venue-specific. This is acceptable at current scale but should be migrated to an extension table if more than 8 venue-specific columns accumulate.
- **Monthly partitioning**: Acceptable up to ~20M rows/month. If exceeded, switch to weekly (`toMonday(block_time)`).
- **Missing fields**: `bundle_id`, `competing_attempts_count`, `leader_identity`, `position_age_slots` were suggested but deferred — they require additional data sources (Jito bundle API, leader schedule) that are out of scope for the initial indexer.

---

## Cross-table Join Paths

| From | To | Join Key | Purpose |
|---|---|---|---|
| `liquidations` | `obligations_snapshots` | `(tx_signature, ix_index)` | Get borrower state at liquidation |
| `liquidations` | `reserves_snapshots` | `(venue, slot, collateral_reserve)` or `(venue, slot, debt_reserve)` | Get reserve config/price at liquidation |
| `liquidations` | `tx_metadata` | `(tx_signature)` | Get full tx details, all signers |
| `failed_liquidation_attempts` | `tx_metadata` | `(tx_signature)` | Same |
| `liquidations` | `_indexer_progress` | `(venue)` | Check coverage |

### Performance at 100M rows

- `liquidations` ORDER BY `(venue, block_time, ...)`: Time-range queries per venue are primary-key-aligned.
- `obligations_snapshots` ORDER BY `(venue, tx_signature, ix_index)`: Lookups by FK to liquidations are fast.
- `reserves_snapshots` ORDER BY `(venue, slot, reserve)`: Lookups by reserve at a slot are fast. May produce many rows per slot if multiple liquidations in same block — acceptable.
- `tx_metadata` ORDER BY `(tx_signature)`: Direct lookup by signature is fast.

Queries that cross-join `liquidations` with `reserves_snapshots` on `(venue, slot)` may be slow if many reserves exist per slot. Add `reserve` to the filter.

---

## Venue-Specific Field Usage Matrix

| Field | Kamino | Jupiter Lend | MarginFi | Save |
|---|---|---|---|---|
| `liquidatee` | obligation.owner | **NULL** | account.authority | obligation.owner |
| `obligation` | obligation pubkey | vault_config PDA | liquidatee marginfi account | obligation pubkey |
| `market` | lending_market | vault_config PDA | marginfi_group | lending_market |
| `collateral_reserve` | withdraw_reserve | vault_config | asset_bank | withdraw_reserve |
| `debt_reserve` | repay_reserve | vault_config | liab_bank | repay_reserve |
| `repay_amount` source | instruction arg | instruction arg | derived from balance deltas | instruction arg |
| `liquidation_reason` | populated | NULL | NULL | NULL |
| `tick_start/end` | NULL | populated | NULL | NULL |
| `absorbed_bad_debt` | NULL | populated | NULL | NULL |
| `insurance_fee_amount` | NULL | NULL | populated | NULL |
| `health_factor` (snapshots) | NULL | NULL | populated | NULL |
| `asset_share_value` (reserves) | NULL | NULL | populated | NULL |
