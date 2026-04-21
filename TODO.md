# TODO — Liquidation Indexer

## Data collection remaining

### February backfill (~4 hours runtime, ~122K RPC calls, ~2.3GB bandwidth)

Approach: Dune daily single-venue queries for crash days (Feb 4-6) → Triton getTransaction.

When Dune rate limits reset:
1. Create + execute 12 queries (3 crash days × 4 venues — Kamino, Jupiter Lend, MarginFi, Save)
2. Download CSVs via Dune API (key: `VOck8JrrTzg8K0F4tGYAaPGScptpXaCj`)
3. Run backfill: `BACKFILL_SIGNATURES_FILE=<csv> BACKFILL_CONCURRENCY=10 cargo run --release -p backfill`
4. Enrich with Feb daily prices (same Dune price query, change date range)
5. Validate against Kamino Feb report: 70,822 events, $26M seized

Target: 94% of Feb volume (66,572 Kamino + est. 55K other venues = ~122K events).

### March backfill (tiny — 551 Kamino events)

Same approach, one Dune query, ~$1 in RPC credits. Low priority — March was a calm month.

### MarginFi and Save mint resolution

The decoders extract `asset_bank` / `liab_bank` pubkeys but not token mint addresses. To get USD prices:
- MarginFi: read Bank account at offset 8 (mint pubkey, 32 bytes) for each `asset_bank` and `liab_bank`
- Save: read Reserve account at offset 42 (liquidity_mint, 32 bytes)

Options:
1. Enhance processors to resolve mints from account data during backfill (requires additional RPC calls per event)
2. Build a lookup table: bank/reserve pubkey → mint. Fetch once, cache locally.
3. Use Dune `solana.account_activity` to find the mint for each bank address

### Oracle price enrichment for remaining venues

Once mints are resolved:
- Run the same `solana_daily_prices_jan_2026` Dune query approach
- Enrichment script: `scripts/enrich_prices.py` or `scripts/enrich_prices_birdeye.py`

### February daily prices

Same as January — create Dune query for Feb daily prices of SOL, USDC, USDT, JLP, JITOSOL, etc.
Download CSV, run enrichment against ClickHouse.

## Analysis remaining

### Full Kamino operator P&L (all 172 operators)

We have P&L for top 4 (8t7ZN, LionX, evoxx, 4NUiC). The all-operators query exceeded Dune resource limits.

Options:
1. Run individual queries for the next 10-20 operators (proven approach, ~300 credits each)
2. Use local ClickHouse estimated P&L: `repay_amount_usd × 0.011 - jito_tip_usd - priority_fee_usd`
3. Wait for Dune plan upgrade for the aggregate query

### Jupiter Lend operator P&L

Same Dune token-transfer approach. 10 operators — could run all 10 individually.

### Cross-venue operator analysis

LionX operates on both Kamino and Jupiter Lend. Identify other cross-venue operators.

## Code improvements

### Concurrent block fetcher (for slot-range mode)

The `block_fetcher.rs` currently fetches blocks sequentially. Add tokio concurrency (same pattern as `sig_fetcher.rs`) for slot-range backfill mode.

### MarginFi/Save mint resolution in processors

Enhance `processors/marginfi.rs` and `processors/save.rs` to:
1. Accept an optional mint lookup table
2. Resolve `collateral_mint` and `debt_mint` from bank/reserve pubkeys

### Price enrichment in the pipeline

Instead of post-hoc enrichment via scripts, add optional price lookup during processing:
- Pyth oracle price at the liquidation slot (from account data in the tx)
- Or daily price cache loaded from Dune CSV

## Dune queries created (both API keys)

### Key 1 (HQO7... — 2,500 credits used, exhausted)
- 7349740: Kamino v1+v2 Jan count ✅
- 7349938: Kamino Jan sig export ✅
- 7350235: Jupiter Lend Jan count ✅
- 7350237: MarginFi Jan count ✅
- 7350310: Jupiter Lend Jan sig export ✅
- 7350314: MarginFi Jan sig export ✅
- 7350430: Save Jan count ✅
- 7350474: Save Jan sig export ✅
- 7350846–7351116: Individual operator P&L queries ✅

### Key 2 (VOck... — active)
- 7351658: All operators P&L (exceeded resource limit)
- 7351659: Kamino Jan oracle price export ✅ (28,710 rows)
- 7351713: Jan 29-31 all operators P&L (exceeded resource limit)
- 7351884/7351960: Solana daily prices Jan ✅ (372 rows)
- 7351852: Jupiter Lend prices (timed out)
- 7351853: MarginFi prices (timed out)
- 7352422–7352431: Feb export attempts (various failures/timeouts)

## Infrastructure

### Self-hosted RPC node (Phase 3)

When monthly RPC costs exceed $1,200/month:
- Bare metal: $800-1,200/month (512GB RAM, NVMe, 10Gbps)
- Yellowstone gRPC plugin (self-compiled, free)
- Unlimited local RPC + zero-latency gRPC streaming
- No voting costs (RPC node, not validator)
