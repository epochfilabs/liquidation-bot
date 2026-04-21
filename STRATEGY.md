# Solana Liquidation Service — Strategy & Execution Plan

## Thesis

Solana lending liquidation is a real but bursty market. Kamino's published data shows 70,822 events in February 2026 followed by 551 in March — a 99.2% drop. This is not a steady monthly cash-flow business. It's a crash-driven, asymmetric opportunity that requires:

1. A measurement layer that tells you when to trade and when to sit
2. A narrow execution bot with strict EV filters and loss caps
3. Expansion into adjacent strategies only after the base is proven

## Revenue scenarios

The following are **scenario ranges, not forecasts.** They depend on assumptions about crash frequency, capture rate, and tip discipline that can only be validated by running the measurement layer. Treat them as planning brackets, not operating targets.

| Condition | Monthly Revenue (solo operator) | Assumed frequency |
|---|---|---|
| Calm market | $60-2,500 | ~7 months/year |
| Volatile market | $2,600-15,000 | ~3 months/year |
| Major crash | $3,000-45,000 | ~2 months/year |

**Scenario range if assumptions hold: $15K-120K/year.** The wide band reflects uncertainty in capture rate and crash frequency. 80%+ of annual revenue would come from 2-3 crash months. The measurement layer is what keeps you from bleeding during the other 9-10 months.

---

## Scope and sequencing

**Primary focus: Solana lending liquidation.** Kamino and Jupiter Lend are the first two venues, taken through measurement (Phase 1) and live execution (Phase 2).

**Next-venue decision: MarginFi vs. Morpho on Base.** After Kamino + Jupiter Lend are proven live, one of these two is promoted based on measured simulated PnL and net-of-effort profile (see Step 2.3). The loser is parked, not killed — revisit if its conditions change.

**Phase 3 expansion** (Drift keepers, arb) remains unchanged regardless of the MarginFi/Morpho choice.

---

## Phase 1 — Prove the measurement layer

**Duration:** 3-4 weeks
**Cost:** $0 (no infrastructure beyond dev machine, free Old Faithful data)
**Goal:** Answer one question reliably: *where is real, net-positive opportunity?*

### Step 1.1 — Complete the Kamino indexer (Week 1)

What exists today:
- Kamino decoder (v1 + v2 liquidation instructions) ✅
- Jupiter Lend vaults decoder ✅
- Save, MarginFi decoders ✅
- ClickHouse schema (6 tables, 4 materialized views) ✅
- Backfill pipeline (block fetcher → processors → writer) ✅
- Docker Compose for local ClickHouse ✅

What to build:
- [ ] **Step A: Validate counts via Dune Analytics (free, ~1 hour)**
  - Query Kamino program transactions for January 2026
  - Confirm: 14,355 liquidation events, $15.2M seized match Kamino's published report
  - This validates the target before spending any credits
- [ ] **Step B: Get raw transaction data for the replay harness**
  - **Primary approach: Dune export + targeted RPC.** Use Dune to extract the ~14,355 liquidation transaction signatures (free tier). Then fetch only those specific transactions via Triton RPC `getTransaction` — 14,355 calls ≈ ~$7 in credits, ~287MB of data.
  - **Alternative: Triton gRPC historical replay** — check with Triton support whether Dragon's Mouth supports historical slot ranges. If yes, this is the most efficient path: filtered by program ID, no wasted calls.
  - **Not recommended: Old Faithful CAR files.** Each epoch is ~500GB and contains ~100M transactions. Your useful data is 0.01% of that. Downloading 500GB to extract 287MB is not practical.
- [ ] Feed raw transactions through the backfill pipeline into ClickHouse
- [ ] Verify: indexed numbers match Kamino's published report (within 5% tolerance)
- [ ] If mismatch: debug decoder/processor, fix, re-run until reconciled

### Step 1.2 — Add Jupiter Lend ingestion (Week 1-2)

- [ ] Same approach: Dune query for Jupiter Lend Vaults liquidation signatures, then targeted `getTransaction` calls
- [ ] Record: liquidation event count, seized amounts, bonus pool
- [ ] Sanity-check against DefiLlama fee data — but note that DefiLlama's Jupiter Lend fees are mostly borrow-interest protocol revenue, not a clean liquidation bonus pool. Use as a directional sanity check, not a liquidation benchmark.
- [ ] Your indexer becomes the source of truth for Jupiter Lend liquidation volume. No public protocol reports exist to validate against.

### Step 1.3 — Build the replay/simulation harness (Week 2-3)

For each historical liquidation event in ClickHouse, compute:

```
simulated_pnl = liquidation_bonus
              - flash_loan_fee (Kamino: ~0.001%, Jupiter Lend: 0%)
              - provisional_jito_tip (start with a rough model, replace with observed data)
              - base_tx_fee (5000 lamports)
```

**Important:** the Jito tip is not a fixed parameter. Jito's docs specify a minimum bundle tip of 1,000 lamports, but competitive opportunities are settled in ~50ms auctions where the actual cost to win is dynamic and opportunity-specific. Start with a provisional model (e.g., percentile-based from historical landed tips), then replace it with observed landed-tip data from your own Phase 2 attempts.

Build as a SQL query + small Rust binary that:
- [ ] Reads all liquidation events from ClickHouse
- [ ] Computes simulated PnL per event using the provisional tip model
- [ ] Buckets by size: <$1K, $1K-$10K, $10K-$100K, $100K+
- [ ] Outputs: win rate, average PnL, total PnL per bucket
- [ ] Identifies the **minimum event size** that is net-positive after tips

Expected finding (based on Kamino February data):
- Sub-$1K events (67,825 of 70,822 = 96%): likely net negative after tips
- $1K-$10K events (2,596): marginally positive
- $10K+ events (401): likely clearly positive — this is where the money is

### Step 1.4 — Build dashboards (Week 3)

Queries against ClickHouse (already partially built in `analysis/sanity.sql`):

- [ ] Events per day by venue (time series)
- [ ] Seized/repaid amounts by venue (time series)
- [ ] Average liquidation size by venue (time series)
- [ ] Implied liquidator bonus pool by venue by day
- [ ] Liquidator concentration (top 10 liquidators by seized amount)
- [ ] Success/failure ratio by venue
- [ ] Simulated PnL distribution histogram

Can be Grafana dashboards against ClickHouse, or simple SQL scripts you run manually.

### Step 1.5 — Validate against February and March reports (Week 3-4)

- [ ] Backfill February 2026 data
  - Target: 70,822 events, $26.0M seized
- [ ] Backfill March 2026 data
  - Target: 551 events, $610K seized
- [ ] All three months reconcile within 5% of Kamino's published numbers

### Phase 1 exit criteria

- [ ] Kamino indexed numbers reconcile to published monthly reports (Jan, Feb, Mar 2026)
- [ ] You can identify which opportunities would have been net-positive after fees/tips
- [ ] You have a ranked list of venues/markets by realized or simulated EV
- [ ] You know the minimum profitable event size (likely $5K-$10K+ on Kamino)
- [ ] Jupiter Lend data is flowing and you have a preliminary read on its volume

---

## Phase 2 — Ship a narrow live bot

**Duration:** 6-8 weeks (extended to accommodate the MarginFi vs. Morpho evaluation track)
**Cost:** ~$20-50/month (cheap VPS + Jito tips). Base RPC free tier is sufficient for the Morpho measurement work — no additional infra spend at this stage.
**Goal:** Learn your true capture rate and true cost to win, and decide the next venue based on data.

**Note on infrastructure:** a cheap VPS is sufficient for proving the pipeline and learning candidate quality, failure modes, and basic landing behavior. It is NOT sufficient to conclude "we can't compete" if the best opportunities are being outrun. Jito's low-latency auction design means a VPS test proves your strategy logic, not your peak competitiveness. Infrastructure upgrades are a Phase 3 decision informed by Phase 2 data.

### Step 2.1 — Deploy Kamino live bot (Week 1-2)

- [ ] Set up Yellowstone gRPC stream (Triton Dragon's Mouth) filtered on Kamino program ID
- [ ] Connect to Jito Block Engine for bundle submission
- [ ] Implement strict EV filter from Phase 1 findings:
  - Skip events below minimum profitable size threshold
  - Skip if estimated bonus < estimated tip + fees
- [ ] Set daily loss cap: $50/day maximum tip spend on failed attempts
- [ ] Log every candidate and outcome (see failure-cost table below)

**Go/no-go rule for Kamino live:** if Kamino live is net negative for 30 consecutive days after EV filtering, reduce to shadow mode and keep only measurement running. Re-enable live execution when the indexer detects a volatility regime change (e.g., daily event count exceeds 500).

### Step 2.2 — Jupiter Lend in shadow mode (Week 2-3)

- [ ] Add Jupiter Lend program to the gRPC subscription
- [ ] Detect liquidatable positions
- [ ] Simulate full transaction (flash borrow → liquidate → repay)
- [ ] Log candidate + simulated PnL
- [ ] Do NOT submit — shadow mode only

**Jupiter Lend promotion criteria:** go live only if shadow-mode simulated PnL stays positive after replacing provisional tip assumptions with observed live tip costs from Kamino execution. If Kamino live data shows winning tips average $8, rerun Jupiter Lend simulations at $8 tip cost and confirm they're still positive.

### Step 2.3 — Next venue: MarginFi vs. Morpho on Base (Week 3-6)

After Kamino + Jupiter Lend are proven, pick one next venue to promote. Two candidates with fundamentally different tradeoffs — stack reuse vs. economic surface.

**Candidate A — MarginFi (Solana):**
- Reuses the entire existing stack: decoder already built, Yellowstone gRPC, Jito, ClickHouse schema
- 5% liquidation penalty (2.5% liquidator / 2.5% insurance fund) documented in program code
- No published monthly reports — competitor count and earnings claims come from third-party analysis of a single anomalous quarter (Q1 2025), not first-party reporting. Treat as "measure carefully before promoting," not high-EV by default
- Incremental work estimate: ~1 week (run existing backfill, add to gRPC subscription)

**Candidate B — Morpho on Base (EVM):**
- Larger economic surface: Morpho V1 TVL ~$6.65B protocol-wide with Base ~$2.6B (DefiLlama). Permissionless liquidation, full bonus to liquidator with no protocol fee on the incentive, and zero-fee protocol flash loans. ~5% liquidator incentive at 86% LLTV (per Morpho docs)
- Base Flashblocks (200ms pre-confirmation) provides a latency path, but competition on Base Morpho is likely already professionalized — expect battle-hardened searchers with Flashblocks-aware infra
- **Requires a second execution stack:** EVM RPC, Base sequencer/Flashblocks integration, ethers/foundry, different auction model (no Jito analog). ClickHouse schema generalizes; Solana decoders and execution paths do not
- Public Morpho API and SDKs accelerate time-to-shadow-bot
- Pre-liquidation / ADL surfaces absorb risk earlier, reducing fat-tail events
- Base-only liquidation fee pool is not publicly broken out — the $10.88M Q1 2026 Morpho V1 liquidation fee figure is protocol-wide. Your Base indexer produces the number that actually decides this
- Incremental work estimate: ~3-4 weeks for minimal indexer + Base-specific simulation harness

**Evaluation process (run both candidates in parallel):**
- [ ] **MarginFi:** run backfill through existing decoder, measure event count and simulated PnL under observed Kamino tip costs (not provisional)
- [ ] **Morpho:** build minimal Base indexer (Morpho Blue `liquidate` events, oracle reads, position health), reconcile one month against Morpho's public API, produce a Base-only liquidation fee pool number
- [ ] For each, compute: simulated monthly PnL, competition density (top-N liquidator concentration), detection-to-landing latency achievable with available infra, estimated incremental engineering cost to live
- [ ] Promote whichever shows >$500/month simulated PnL AND the better net-of-effort profile
- [ ] Park the loser; revisit if its conditions change (TVL growth, market regime shift, new Base infra)

**Default prior:** MarginFi wins on stack reuse and time-to-live; Morpho wins on economic surface and potential upside. The data decides — don't open a second execution stack unless the measured EV justifies it.

### Step 2.4 — Failure-cost table (continuous deliverable)

For each venue, maintain a running table:

| Metric | Kamino | Jupiter Lend | MarginFi |
|---|---|---|---|
| Candidates detected | | | |
| Submitted | | | |
| Landed (successful) | | | |
| Failed (ObligationHealthy) | | | |
| Failed (outbid) | | | |
| Failed (timeout/other) | | | |
| Avg tip spent on failures | | | |
| Avg EV before send | | | |
| Realized EV after send | | | |
| Net PnL (cumulative) | | | |
| Detection-to-landing latency (p50/p90) | | | |

This table is as important as the successful-liquidation data. It tells you where you're bleeding and why.

### Step 2.5 — Analyze first month of live data (Week 4-6)

After 4+ weeks of live operation, answer:
- What is my actual win rate per venue?
- What is my actual cost per attempt (tip + gas on failures)?
- What is my average revenue per successful liquidation?
- What is my net PnL by event size bucket?
- Am I losing on speed (someone detects first) or on tip (I detect but bid too low)?
- Is Jupiter Lend worth promoting from shadow to live?

### Phase 2 exit criteria

- [ ] At least one venue is net positive over a meaningful sample (100+ candidates)
- [ ] You know your win rate by opportunity size bucket
- [ ] You know whether small events (<$5K) are noise or carry value
- [ ] You know whether Jupiter Lend deserves live execution (data-driven, not narrative)
- [ ] You have measured your detection-to-landing latency vs winners
- [ ] Daily loss cap has never been breached (discipline check)
- [ ] Failure-cost table is complete and reveals whether losses are speed-driven or tip-driven
- [ ] MarginFi vs. Morpho decision made based on measured simulated PnL and effort profile; loser is parked with documented re-entry conditions

---

## Phase 3 — Expand into adjacent strategies

**Duration:** Ongoing
**Cost:** Scales with opportunity — potentially dedicated server ($120/month)
**Goal:** Add a second revenue stream that shares the same infrastructure.

### Step 3.1 — Drift keeper preparation (start during Phase 2)

- [ ] Study `drift-labs/keep-rs` open-source keeper bot code
- [ ] Understand Drift's liquidation engine:
  - Keeper reward: 0.75-3% of amount liquidated
  - Dynamic priority fees (60th percentile)
  - Health monitoring: within 100 slots freshness
  - Pyth price freshness: within 5 seconds
- [ ] Build Drift decoder (similar pattern to existing decoders)
- [ ] Pre-build Drift keeper bot — ready to deploy but not running
- [ ] Monitor Drift relaunch status (audits, timeline, announcement)
- [ ] **Deploy when relaunch is live and operationally stable, not merely announced.** The April 16, 2026 recovery update indicates audits and protocol reboot are still in progress. Prepare now, but do not budget revenue until the venue is confirmed live and processing real volume.

### Step 3.2 — Evaluate arb (only after Phase 2 proves execution)

- [ ] The gRPC stream already sees every account update needed for arb detection
- [ ] Adding DEX price monitoring is incremental work on existing infrastructure
- [ ] But: arb is the most competitive MEV lane — thousands of bots, sub-100ms races
- [ ] Helius' 2025 MEV report supports arb as a much larger market than lending liquidations at the ecosystem level, but that does not mean your own capture will be easy or stable
- [ ] Only pursue if Phase 2 shows your infrastructure is competitive on latency
- [ ] Start with less-competitive arb paths (long-tail pairs, smaller DEXes)

### Step 3.3 — Expand venue coverage based on data

Only add venues where the indexer shows real opportunity:

| Venue | Status | Add when... |
|---|---|---|
| Save | Permissionless, unverified volume | Indexer shows >$500/month liquidation bonus pool |
| Loopscale | Unproven for external liquidators — docs mention internal auction systems | Confirms permissionless external liquidation access |
| Phoenix Perps | Private beta, too early to assess | Leaves private beta + publishes liquidation docs |
| DefiTuna | Effectively closed to external liquidators — protocol is the liquidator | Opens external keeper access (unlikely based on current design) |
| Pacifica | Likely internal liquidation via matching engine | Opens permissionless keeper access (unlikely given hybrid off-chain architecture) |

### Phase 3 exit criteria

- [ ] Two live strategies share one execution stack
- [ ] One strategy provides baseline activity (arb or Drift keepers)
- [ ] One strategy provides spike upside (lending liquidation during crashes)
- [ ] The indexer feeds live parameter tuning (min size thresholds, tip levels)

---

## Infrastructure plan

### Phase 1 (measurement only)

```
Dev machine (your laptop)
├── ClickHouse (Docker, local)
├── faithful-cli rpc (local, reading CAR files)
├── Backfill binary
└── Cost: $0
```

### Phase 2 (narrow live bot)

```
Cheap VPS ($20-50/month, any provider)
├── Bot binary (gRPC listener + Jito submitter)
├── Yellowstone gRPC → Triton Dragon's Mouth
├── Jito Block Engine → nearest endpoint
├── ClickHouse (local on same VPS, or separate)
└── Budget: $50/day max tip spend (hard cap)
```

Note: this proves pipeline and strategy logic. It does not prove peak competitiveness against low-latency searchers. That determination requires Phase 2 data on win rate by latency bucket.

### Phase 3 (multi-strategy)

```
Dedicated server ($120/month, e.g. Hetzner Frankfurt or Tokyo)
├── Bot binary (liquidation + arb/keeper)
├── Yellowstone gRPC
├── Jito Block Engine
├── ClickHouse (local)
├── Grafana dashboards
└── Budget: scales with proven revenue
```

**Rule: don't upgrade infrastructure until revenue justifies it.** Only move to dedicated hardware when monthly revenue consistently exceeds $500 for 3+ months.

---

## What NOT to do

1. **Don't build for all protocols at once.** Several venues are either closed to external liquidators (Jupiter Perps, DefiTuna), too early (Phoenix Perps), or unproven for external participation (Loopscale, Pacifica). Engineering time on unproven venues is waste until the indexer confirms otherwise. Morpho on Base is a serious candidate but gated behind Step 2.3 measurement — don't open a second execution stack before Kamino + Jupiter Lend are proven live.

2. **Don't skip Phase 1.** The urge to start trading immediately is the main risk. Without the measurement layer, you can't distinguish "the market is quiet" from "my bot is broken."

3. **Don't assume execution revenue arrives quickly.** March 2026 produced $610K total seized on Kamino — the entire liquidator bonus pool was roughly $7K for the month, split across 166 liquidators. You need to be prepared for months like that.

4. **Don't spend real money on tips until you've validated simulated PnL.** Shadow mode and replay exist for a reason.

5. **Don't treat this as a standalone company.** At the current scenario range, this is a profitable side operation or one strategy in a broader MEV stack — not a full-time salary unless crash months are exceptionally good.

---

## Decision points

| After... | Decide... |
|---|---|
| Phase 1 validation | Is the minimum profitable event size achievable? If no events above that threshold exist, pivot to pure arb. |
| 30 days of Kamino live | Is win rate >5% on filtered opportunities? If net negative for 30 days, reduce to shadow mode. |
| 30 days of Jupiter Lend shadow | Does simulated PnL stay positive when using observed (not provisional) tip costs from Kamino? |
| MarginFi + Morpho indexers both reconciled | Promote whichever has higher simulated PnL AND better net-of-effort profile. Park the loser with documented re-entry conditions. |
| Drift relaunch live + stable | Deploy keeper bot. "Live and stable" means processing real volume, not just announced. |
| Phase 2 net positive for 3 months | Upgrade to dedicated server. Add arb. |
| Phase 2 net negative for 3 months | Pause execution, keep indexer running, wait for market conditions to change. |

---

## Timeline

```
Week 1-2:   Indexer backfill + Kamino validation against published reports
Week 3-4:   Replay harness + Jupiter Lend ingestion + dashboards
Week 5-6:   Narrow Kamino live bot + Jupiter Lend shadow mode
Week 7-8:   Analyze first live data, tune filters, build failure-cost table
Week 9-10:  MarginFi backfill + Morpho-on-Base indexer build (parallel tracks); Drift prep begins
Week 11-12: Jupiter Lend live/no-go decision; MarginFi vs. Morpho measurement complete → next-venue promotion decision
Week 13-14: Promoted venue → shadow → live under the same EV discipline as Kamino
Week 15+:   Drift deploy (when live and stable); arb evaluation
```

**Measure first. Trade narrowly second. Expand third.**
