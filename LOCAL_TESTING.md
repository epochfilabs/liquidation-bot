# Local Testing Guide — Solana Liquidation Indexer

## Prerequisites

- Docker Desktop installed and running
- Rust toolchain (`cargo`)
- The `.env` file with `SOLANA_RPC_URL` (Triton RPC) for live backfill tests

## Quick Start

```bash
# Run everything: start ClickHouse, apply schema, process fixtures, write to DB, verify
./scripts/local-test.sh
```

Or step by step:

```bash
# 1. Start ClickHouse
docker compose up -d

# 2. Apply schema
./scripts/local-test.sh --schema

# 3. Run fixture validation (no ClickHouse needed)
cargo test -p backfill --test validation -- --nocapture

# 4. Write fixtures to ClickHouse and query results
CLICKHOUSE_URL=http://localhost:8123 \
CLICKHOUSE_DATABASE=liquidation_indexer \
CLICKHOUSE_USER=default \
CLICKHOUSE_PASSWORD=dev \
cargo test -p backfill --test local_integration write_fixtures_to_clickhouse -- --nocapture

# 5. Browse the database
./scripts/clickhouse-shell.sh
```

## ClickHouse Access

| Method | Command |
|---|---|
| Interactive shell | `./scripts/clickhouse-shell.sh` |
| Single query | `./scripts/clickhouse-shell.sh "SELECT count() FROM liquidations"` |
| HTTP (curl) | `curl 'http://localhost:8123/?database=liquidation_indexer&user=default&password=dev' -d 'YOUR SQL'` |
| Docker exec | `docker compose exec clickhouse clickhouse-client --database=liquidation_indexer --password=dev` |

**Credentials:** user=`default`, password=`dev`, database=`liquidation_indexer`, port=`8123` (HTTP) / `9000` (native).

## Test Fixtures

12 real + synthetic mainnet transaction fixtures under `tests/fixtures/`:

| Venue | Fixtures | Type | Source |
|---|---|---|---|
| **Kamino** | 3 | Failed liquidation attempts (error 6016: ObligationHealthy) | Real mainnet via Triton RPC |
| **Save** | 3 | Successful liquidations (`LiquidateWithoutReceivingCtokens`, tag 12 via CPI) | Real mainnet via Triton RPC |
| **MarginFi** | 3 | Successful liquidations (`lendingAccountLiquidate`) | Synthetic (real discriminator + account structure) |
| **Jupiter Lend** | 3 | Successful liquidations (`liquidate` with absorb/transfer_type) | Synthetic (real discriminator + account structure) |

**Why synthetic?** MarginFi and Jupiter Lend liquidation events are extremely rare on recent mainnet. After scanning 6000+ transactions across both protocols, zero successful liquidations were found in the accessible history window. The synthetic fixtures use correct program IDs, Anchor discriminators, and account layouts to validate the full decoder → processor → ClickHouse pipeline.

**Why are Save liquidations CPI inner instructions?** Real-world Save liquidators use wrapper programs that call Save's `LiquidateObligation` (tag 12) via CPI. The liquidation instruction appears as an inner instruction, not a top-level instruction. The processors scan both top-level and inner instructions to handle this.

Total fixture size: ~232KB (well under 100MB).

## Test Commands

### Fixture processing only (no ClickHouse)

```bash
# Process all fixtures through decoders + processors, validate every field
cargo test -p backfill --test validation -- --nocapture
```

Expected output:
```
[kamino]       ISSUES — 0 liquidations, 3 failed attempts, 3 validation issues
[marginfi]     ISSUES — 3 liquidations, 0 failed attempts, 3 validation issues
[save]         ISSUES — 3 liquidations, 0 failed attempts, 3 validation issues
[jupiter-lend] PASS   — 3 liquidations, 0 failed attempts, 0 validation issues

VALIDATION ISSUES (9 total):
  All 9 are: liquidatee — NULL (expected: requires obligation account data read)
```

The only validation issue across all venues is `liquidatee = NULL` — this field requires reading the obligation account data at the liquidation slot, which is a documented enrichment step (Phase 3.5).

### Full pipeline test (with ClickHouse)

```bash
# Process fixtures and write to ClickHouse
CLICKHOUSE_URL=http://localhost:8123 \
CLICKHOUSE_DATABASE=liquidation_indexer \
CLICKHOUSE_USER=default \
CLICKHOUSE_PASSWORD=dev \
cargo test -p backfill --test local_integration -- --nocapture
```

Expected: `ClickHouse write complete: 9 liquidations, 3 failed, 12 tx_meta`

### Full workspace test suite

```bash
cargo test --workspace
```

Expected: 122+ tests, 0 failures. Includes decoder unit tests, fixture round-trips, live mainnet validation, and pipeline integration tests.

### Live backfill (uses RPC credits)

```bash
# Backfill a specific slot range (e.g., around known Kamino liquidation slots)
SOLANA_RPC_URL=<your-triton-rpc> \
CLICKHOUSE_URL=http://localhost:8123 \
CLICKHOUSE_DATABASE=liquidation_indexer \
CLICKHOUSE_USER=default \
CLICKHOUSE_PASSWORD=dev \
BACKFILL_START_SLOT=414544140 \
BACKFILL_END_SLOT=414544150 \
RUST_LOG=info \
cargo run -p backfill
```

## Browsing the Database

### Interactive shell

```bash
./scripts/clickhouse-shell.sh
```

### Useful queries

```sql
-- Table overview
SHOW TABLES;

-- Row counts
SELECT 'liquidations' AS tbl, count() AS rows FROM liquidations
UNION ALL SELECT 'failed_attempts', count() FROM failed_liquidation_attempts
UNION ALL SELECT 'tx_metadata', count() FROM tx_metadata;

-- Liquidations per venue
SELECT venue, count() AS total FROM liquidations GROUP BY venue ORDER BY total DESC;

-- Failed attempts with error breakdown
SELECT venue, error_code, error_message, count() AS occurrences
FROM failed_liquidation_attempts
GROUP BY venue, error_code, error_message
ORDER BY occurrences DESC;

-- Full detail of all liquidation events
SELECT
    venue,
    slot,
    substring(tx_signature, 1, 20) AS sig,
    repay_amount,
    withdraw_amount,
    used_flashloan,
    flashloan_source,
    jito_tip_lamports,
    inner_ix_index
FROM liquidations
ORDER BY venue, slot
FORMAT PrettyCompact;

-- Full detail of a single event (vertical format is easier to read)
SELECT * FROM liquidations LIMIT 1 FORMAT Vertical;

-- Check for null/empty fields that shouldn't be
SELECT
    countIf(liquidator = '') AS empty_liquidator,
    countIf(obligation = '') AS empty_obligation,
    countIf(market = '') AS empty_market,
    countIf(collateral_reserve = '') AS empty_collateral_reserve,
    countIf(debt_reserve = '') AS empty_debt_reserve,
    countIf(raw_ix_data = '') AS empty_raw_data,
    countIf(program_id = '') AS empty_program_id
FROM liquidations;

-- Flash loan usage per venue
SELECT venue, used_flashloan, flashloan_source, count() AS total
FROM liquidations
GROUP BY venue, used_flashloan, flashloan_source
ORDER BY venue;

-- Jito tips analysis
SELECT
    venue,
    countIf(jito_tip_lamports IS NOT NULL) AS tipped,
    countIf(jito_tip_lamports IS NULL) AS untipped,
    avg(jito_tip_lamports) AS avg_tip
FROM liquidations
GROUP BY venue;

-- Transaction metadata
SELECT
    substring(tx_signature, 1, 20) AS sig,
    slot, succeeded, fee_lamports, priority_fee_lamports,
    compute_units_consumed, num_instructions, uses_address_lookup_table
FROM tx_metadata
ORDER BY slot
FORMAT PrettyCompact;
```

### Full sanity query suite

```bash
# Run all 10 analysis queries
cat analysis/sanity.sql | while IFS=';' read -r query; do
  query=$(echo "$query" | sed '/^--/d' | tr '\n' ' ' | xargs)
  [ -z "$query" ] && continue
  echo "---"
  curl -s "http://localhost:8123/?database=liquidation_indexer&user=default&password=dev" \
    -d "$query FORMAT PrettyCompact" 2>/dev/null
done
```

## Architecture Summary

```
tests/fixtures/*.json
        │
        ▼
┌─────────────────────┐
│  tx_parser.rs       │  Parse JSON → TxContext
│  (handles v0 ALTs)  │  Fast-filter by program ID
└─────────┬───────────┘
          │
          ▼
┌─────────────────────┐
│  processors/        │  Decode instructions (top-level + inner CPI)
│  ├── kamino.rs      │  → klend-decoder (v1/v2 discriminator)
│  ├── save.rs        │  → save-decoder (tag 12/17, Borsh)
│  ├── marginfi.rs    │  → marginfi-v2-decoder (Anchor discriminator)
│  └── jupiter_lend.rs│  → jupiter-lend-vaults-decoder (Anchor discriminator)
└─────────┬───────────┘
          │
          ▼
┌─────────────────────┐
│  enrichment.rs      │  Jito tips (8 accounts), flash loans (4 protocols),
│                     │  Jupiter swaps, priority fees, error parsing
└─────────┬───────────┘
          │
          ▼
┌─────────────────────┐
│  writer.rs          │  Batch NDJSON → ClickHouse HTTP interface
│  (≥10k rows / 1s)  │  Idempotent (ReplacingMergeTree)
└─────────┬───────────┘
          │
          ▼
┌─────────────────────┐
│  ClickHouse         │  6 tables + 4 materialized views
│  (Docker, port 8123)│  Monthly partitions, venue-prefixed ordering
└─────────────────────┘
```

## Key Findings from Testing

1. **CPI inner instruction scanning is critical.** Save liquidations are invoked through wrapper programs — only scanning top-level instructions misses them entirely. All processors now scan both top-level and inner instructions.

2. **MarginFi and Jupiter Lend liquidations are extremely rare.** Scanning 6000+ recent mainnet transactions found zero successful liquidations for either protocol. Historical data (Old Faithful CAR files) is needed for real-world fixtures.

3. **Kamino liquidation attempts fail frequently.** All 3 real Kamino fixtures are failed attempts (error 6016: ObligationHealthy). Bots speculatively attempt liquidations and get rejected.

4. **MarginFi uses wrapper programs.** Transactions logged as "Liquidate Obligation 2" are actually from a wrapper program (`3cKREQ3Z7ioCQ4oa23uGEuzekhQWPxKiBEZ87WfaAZ5p`), not the core MarginFi program. The wrapper uses MarginFi flash loans to fund Save liquidations.

5. **DateTime serialization matters.** ClickHouse JSONEachRow rejects ISO 8601 timestamps with `Z` suffix. Must format as `YYYY-MM-DD HH:MM:SS.sss`.

## Known Limitations

| Limitation | Impact | Fix |
|---|---|---|
| `liquidatee` is NULL for Kamino/MarginFi/Save | Missing borrower identity | Requires reading obligation account data at liquidation slot (Phase 3.5 enrichment) |
| `withdraw_amount` is 0 | Missing collateral seized amount | Requires computing pre/post token balance deltas |
| `collateral_mint` / `debt_mint` empty for MarginFi/Save | Missing token identity | Requires reading Bank/Reserve account data |
| USD value fields are all NULL | No dollar-denominated analysis | Requires oracle price enrichment (Phase 3.5) |
| No real MarginFi/Jupiter Lend liquidation fixtures | Synthetic fixtures only | Need Old Faithful historical data or longer monitoring period |

## Shutting Down

```bash
docker compose down       # Stop ClickHouse (data persists)
docker compose down -v    # Stop and wipe all data
```
