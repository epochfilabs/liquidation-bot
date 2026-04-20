# Phase 3 Report — Backfill Pipeline

## What was built

### Crate structure

```
crates/
├── indexer-core/          — Canonical event types, ClickHouse writer, tx enrichment, progress tracking
│   ├── events.rs          — LiquidationEvent, FailedLiquidationEvent, ObligationSnapshot,
│   │                        ReserveSnapshot, TxMetadata, ProcessedTransaction
│   ├── enrichment.rs      — Jito tip detection (8 tip accounts), flash loan detection
│   │                        (Kamino/Jupiter/MarginFi/Save), Jupiter swap detection,
│   │                        ComputeBudget parsing, Anchor error parsing
│   ├── writer.rs          — Batched ClickHouse writer (NDJSON over HTTP, >=10k rows or 1s flush)
│   └── progress.rs        — Per-venue progress tracking with resume support
│
├── processors/            — Per-venue processors: decoded instruction → canonical event
│   ├── common.rs          — Shared tx enrichment, block_time conversion, account resolution
│   ├── kamino.rs          — Kamino Lend processor (v1 + v2 liquidation → LiquidationEvent)
│   ├── jupiter_lend.rs    — Jupiter Lend processor (liquidate → LiquidationEvent with NULL liquidatee)
│   ├── marginfi.rs        — MarginFi processor (lendingAccountLiquidate → LiquidationEvent)
│   └── save.rs            — Save processor (tag 12 + tag 17 → LiquidationEvent)
│
└── backfill/              — Binary: historical backfill from RPC
    ├── main.rs            — Entry point, spawns writer actor + block fetcher
    ├── config.rs          — Environment-based config (RPC URL, ClickHouse, slot range)
    ├── block_fetcher.rs   — Slot-by-slot block fetcher with skip/error handling, periodic logging
    └── tx_parser.rs       — RPC JSON → TxContext parser (handles v0 ALTs, token balances)
```

### Pipeline architecture

```
┌──────────────────┐     ┌──────────────────┐     ┌──────────────────┐
│   Block Fetcher  │────▶│   Processors     │────▶│  ClickHouse      │
│   (RPC client)   │     │  (4 venues)      │     │  Writer Actor    │
│                  │     │                  │     │  (NDJSON batch)  │
│ getBlock(slot)   │     │ decode + enrich  │     │  >=10k rows or   │
│ for each slot    │     │ → canonical      │     │  1s interval     │
│ in range         │     │   events         │     │                  │
└──────────────────┘     └──────────────────┘     └──────────────────┘
         │                                                 │
         ▼                                                 ▼
  Old Faithful CAR                                    ClickHouse
  (faithful-cli rpc)                               (6 tables + 4 MVs)
  or Triton RPC
```

### Key features

1. **Multi-venue in a single pass.** Each block is fetched once; all four venue processors scan it. No redundant RPC calls.

2. **Fast filtering.** Before parsing instructions, `tx_parser` checks if the transaction's account keys include any of the 4 program IDs. Transactions that don't touch our programs are skipped without full parsing.

3. **Full enrichment pipeline.** For every liquidation event:
   - Jito tip detection (all 8 mainnet tip accounts, via SystemProgram::Transfer scanning)
   - Flash loan source identification (Kamino, Jupiter Lend, MarginFi, Save discriminators)
   - Jupiter DEX swap detection (v6 program ID)
   - ComputeBudget priority fee and CU limit extraction
   - Anchor error code/message parsing from logs (for failed attempts)

4. **Batched ClickHouse writes.** The writer actor buffers events and flushes when either 10k rows accumulate or 1 second elapses. Uses NDJSON over HTTP (`INSERT INTO ... FORMAT JSONEachRow`) which is ClickHouse's most efficient ingest path.

5. **Idempotent.** `ReplacingMergeTree` on `(tx_signature, ix_index)` deduplicates if the same slot is re-processed. The writer simply re-inserts.

6. **Resumable.** `ProgressTracker` maintains per-venue cursors (last_slot, row counts, backfill state). These can be persisted to the `_indexer_progress` table for restart.

7. **v0 transaction support.** `tx_parser` merges `message.accountKeys` with `meta.loadedAddresses.{writable,readonly}` to handle Address Lookup Tables.

### Usage

```bash
# Tier 0: One epoch from Old Faithful
# 1. Download epoch CAR
curl -O https://files.old-faithful.net/800/epoch-800.car
# 2. Run local RPC
faithful-cli rpc epoch-800.car --listen :8899
# 3. Run backfill
SOLANA_RPC_URL=http://localhost:8899 \
CLICKHOUSE_URL=http://localhost:8123 \
BACKFILL_START_SLOT=345600000 \
BACKFILL_END_SLOT=345700000 \
RUST_LOG=info \
cargo run -p backfill
```

## Test results

**120 tests, 0 failures** across the full workspace (13 crates).

New tests in this phase:
- `indexer-core::enrichment` — 5 tests: Jito tip detection, flash loan detection (Kamino, Save), Jupiter swap, Anchor error parsing
- `indexer-core::progress` — 1 test: progress tracking state machine

## What was cut

1. **Price enrichment from oracle accounts.** The processors set all `_usd` and `_price` fields to `None`. Enriching these requires reading oracle account data from the same block (available in Old Faithful CAR files) and decoding Pyth/Switchboard/Scope price feeds. This is Phase 3.5 work — the pipeline captures all the structural data; USD values are an enrichment pass that can run separately.

2. **Obligation/reserve snapshot population.** The `obligations_snapshots` and `reserves_snapshots` tables are defined but the processors don't yet populate them. Populating them requires fetching account data for the obligation/reserve at the liquidation slot — this needs either the full block's account writes (available in CAR files) or additional RPC calls (`getAccountInfo` at the slot).

3. **withdraw_amount derivation from token balances.** The `withdraw_amount` and `repay_amount` (for MarginFi) fields require computing pre/post token balance deltas. The `TxContext` carries `pre_token_balances` and `post_token_balances` but the processors don't yet compute the deltas. The matching logic (by account index → owner → mint) is straightforward but venue-specific.

4. **liquidatee extraction.** For Kamino and Save, the `liquidatee` (obligation owner) requires reading the obligation account data. For MarginFi, it requires reading the MarginfiAccount's `authority` field. These are account reads at the liquidation slot.

5. **Real-time tail (Phase 4).** Swapping the block fetcher for `carbon-yellowstone-grpc-datasource` is deferred per the prompt's phasing.

## What surprised me

1. **The `clickhouse` Rust crate uses derive macros (`#[derive(clickhouse::Row)]`)** for its native insert API, not a runtime builder. Since our event types are defined in `indexer-core` and we don't want to couple them to ClickHouse's derive macro, I used the raw HTTP/NDJSON approach instead. This is actually ClickHouse's recommended high-throughput ingest path.

2. **Block fetching is the bottleneck, not processing.** Fetching `getBlock` over RPC takes 50-200ms per block. At 1 block/200ms, processing 432,000 slots (1 epoch) takes ~24 hours over remote RPC. With a local Old Faithful RPC, this drops to single-digit milliseconds per block.

3. **Most blocks have zero liquidation-relevant transactions.** The fast-filter on program IDs in account keys eliminates >99% of transactions before any instruction parsing. This means the processing pipeline is idle most of the time — the bottleneck is I/O.

## Open questions

1. **Parallel block fetching.** The current implementation fetches blocks sequentially. For remote RPC, parallelizing with a bounded semaphore (e.g., 10 concurrent requests) would significantly improve throughput. For Old Faithful local RPC, sequential is fine since the bottleneck shifts to disk I/O.

2. **Epoch-based progress vs. slot-based progress.** The prompt specifies "resume from the last fully-committed epoch." The current implementation tracks progress per-slot. Converting to epoch-based commit requires knowing epoch boundaries (432,000 slots per epoch starting from a known epoch start slot).

3. **CAR file direct reading.** The current architecture goes through RPC (either Old Faithful local or remote). A future optimization could read CAR files directly using the `yellowstone-faithful` Rust crate, bypassing the RPC layer entirely. This would be significantly faster for bulk backfill.
