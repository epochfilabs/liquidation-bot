# Solana Liquidation Service — Strategy & Execution Plan

## Thesis

Solana lending liquidation is a real but bursty market. Kamino's published data shows 70,822 events in February 2026 followed by 551 in March — a 99.2% drop. This is not a steady monthly cash-flow business. It's a crash-driven, asymmetric opportunity that requires:

1. A measurement layer that tells you when to trade and when to sit
2. A narrow execution bot with strict EV filters and loss caps
3. Expansion into adjacent strategies only after the base is proven

## Revenue reality

Based on verified data (Kamino monthly reports) and structural analysis:

| Condition | Monthly Revenue (solo operator) | Probability |
|---|---|---|
| Calm market | $60-2,500 | ~7 months/year |
| Volatile market | $2,600-15,000 | ~3 months/year |
| Major crash | $3,000-45,000 | ~2 months/year |
| **Annualized estimate** | **$15K-120K** | |

80%+ of annual revenue comes from 2-3 crash months. The measurement layer is what keeps you from bleeding during the other 9-10 months.

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
- [ ] Connect backfill to Old Faithful CAR files for Kamino historical data
  - Download one epoch CAR file (~500GB, free from files.old-faithful.net)
  - Run `faithful-cli rpc` locally
  - Point backfill binary at local RPC
- [ ] Run Tier 0 backfill: process one epoch of Kamino data end-to-end
- [ ] Verify: indexed January 2026 numbers match Kamino's published report
  - Target: 14,355 events, $15.2M seized, $15.0M repaid (within 5% tolerance)
- [ ] If mismatch: debug decoder/processor, fix, re-run until reconciled

### Step 1.2 — Add Jupiter Lend ingestion (Week 1-2)

- [ ] Run backfill for Jupiter Lend vaults program over the same epoch
- [ ] Record: liquidation event count, seized amounts, bonus pool
- [ ] Compare against any available DefiLlama fee data for validation
- [ ] Note: Jupiter Lend lacks published reports — your indexer becomes the source of truth

### Step 1.3 — Build the replay/simulation harness (Week 2-3)

For each historical liquidation event in ClickHouse, compute:

```
simulated_pnl = liquidation_bonus
              - flash_loan_fee (Kamino: ~0.001%, Jupiter Lend: 0%)
              - estimated_jito_tip (use $2-5 as baseline)
              - base_tx_fee (5000 lamports ≈ $0.001)
```

Build as a SQL query + small Rust binary that:
- [ ] Reads all liquidation events from ClickHouse
- [ ] Computes simulated PnL per event
- [ ] Buckets by size: <$1K, $1K-$10K, $10K-$100K, $100K+
- [ ] Outputs: win rate, average PnL, total PnL per bucket
- [ ] Identifies the **minimum event size** that is net-positive after tips

Expected finding (based on Kamino February data):
- Sub-$1K events (67,825 of 70,822 = 96%): net negative after tips
- $1K-$10K events (2,596): marginally positive
- $10K+ events (401): clearly positive — this is where the money is

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

**Duration:** 4-6 weeks
**Cost:** ~$20-50/month (cheap VPS + Jito tips)
**Goal:** Learn your true capture rate and true cost to win.

### Step 2.1 — Deploy Kamino live bot (Week 1-2)

- [ ] Set up Yellowstone gRPC stream (Triton Dragon's Mouth) filtered on Kamino program ID
- [ ] Connect to Jito Block Engine for bundle submission
- [ ] Implement strict EV filter from Phase 1 findings:
  - Skip events below minimum profitable size threshold
  - Skip if estimated bonus < estimated tip + fees
- [ ] Set daily loss cap: $50/day maximum tip spend on failed attempts
- [ ] Log every:
  - Candidate detected (obligation pubkey, LTV, estimated bonus)
  - Simulated PnL before submission
  - Bid/tip amount
  - Outcome: landed/failed
  - Time delta: detection → submission → landed slot
  - If failed: why (ObligationHealthy, outbid, timeout)

### Step 2.2 — Jupiter Lend in shadow mode (Week 2-3)

- [ ] Add Jupiter Lend program to the gRPC subscription
- [ ] Detect liquidatable positions
- [ ] Simulate full transaction (flash borrow → liquidate → repay)
- [ ] Log candidate + simulated PnL
- [ ] Do NOT submit — shadow mode only
- [ ] After 30 days of shadow data: decide whether to go live

### Step 2.3 — Add MarginFi to indexer (Week 3-4)

- [ ] Run MarginFi backfill through existing decoder
- [ ] Measure: actual liquidation event count and fee pool at current $38M TVL
- [ ] If indexed data shows >$500/month simulated PnL: add to live bot
- [ ] If not: park it, focus engineering time elsewhere

### Step 2.4 — Analyze first month of live data (Week 4-6)

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
- [ ] You know whether Jupiter Lend deserves live execution
- [ ] You have measured your detection-to-landing latency vs winners
- [ ] Daily loss cap has never been breached (discipline check)

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
- [ ] **Deploy on relaunch day one** — competition vacuum is the opportunity

### Step 3.2 — Evaluate arb (only after Phase 2 proves execution)

- [ ] The gRPC stream already sees every account update needed for arb detection
- [ ] Adding DEX price monitoring is incremental work on existing infrastructure
- [ ] But: arb is the most competitive MEV lane — thousands of bots, sub-100ms races
- [ ] Only pursue if Phase 2 shows your infrastructure is competitive on latency
- [ ] Start with less-competitive arb paths (long-tail pairs, smaller DEXes)

### Step 3.3 — Expand venue coverage based on data

Only add venues where the indexer shows real opportunity:

| Venue | Add when... |
|---|---|
| Save | Indexer shows >$500/month liquidation bonus pool |
| Loopscale | Confirms permissionless liquidation + variable penalty is live |
| Phoenix Perps | Leaves private beta + publishes liquidation docs |
| Pacifica | Opens permissionless keeper access (unlikely given architecture) |

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

### Phase 3 (multi-strategy)

```
Dedicated server ($120/month, Hetzner Frankfurt or Tokyo)
├── Bot binary (liquidation + arb/keeper)
├── Yellowstone gRPC
├── Jito Block Engine
├── ClickHouse (local)
├── Grafana dashboards
└── Budget: scales with proven revenue
```

**Rule: don't upgrade infrastructure until revenue justifies it.** A $20 VPS is enough for Phase 2. Only move to Hetzner when monthly revenue consistently exceeds $500.

---

## What NOT to do

1. **Don't build for all 11 protocols at once.** 6 of them have $0 liquidation opportunity (Jupiter Perps, Pacifica, Phoenix, DefiTuna, Project 0, and likely Loopscale). Engineering time on dead venues is pure waste.

2. **Don't skip Phase 1.** The urge to start trading immediately is the main risk. Without the measurement layer, you can't distinguish "the market is quiet" from "my bot is broken."

3. **Don't assume execution revenue arrives quickly.** March 2026 produced $610K total seized on Kamino — the entire liquidator bonus pool was ~$7K for the month, split 166 ways. That's $42 per liquidator. You need to be prepared for months like that.

4. **Don't spend real money on tips until you've validated simulated PnL.** Shadow mode and replay exist for a reason.

5. **Don't treat this as a standalone company.** At $15K-120K/year estimated revenue, this is a profitable side operation or one strategy in a broader MEV stack — not a salary.

---

## Decision points

| After... | Decide... |
|---|---|
| Phase 1 validation | Is the minimum profitable event size achievable? If no events above that threshold exist, pivot to pure arb. |
| 30 days of Phase 2 live | Is win rate >5% on filtered opportunities? If not, investigate latency vs tip problems. |
| 30 days of Jupiter Lend shadow | Does simulated PnL justify going live? Threshold: >$500/month. |
| Drift relaunch announced | Deploy keeper bot within 24 hours of relaunch. Pre-built, pre-tested. |
| Phase 2 net positive for 3 months | Upgrade to dedicated server. Add arb. |
| Phase 2 net negative for 3 months | Pause execution, keep indexer running, wait for market conditions to change. |

---

## Timeline

```
Week 1-2:   Indexer backfill + Kamino validation
Week 3-4:   Replay harness + Jupiter Lend ingestion + dashboard
Week 5-6:   Narrow Kamino live bot + Jupiter Lend shadow
Week 7-8:   Analyze first live data, tune filters
Week 9-10:  MarginFi decision (data-driven), Drift prep begins
Week 11-12: Jupiter Lend live/no-go decision
Week 13+:   Drift deploy (when live), arb evaluation
```

**Measure first. Trade narrowly second. Expand third.**
