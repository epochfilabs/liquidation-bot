# Project Status — April 21, 2026 (Late Night)

## Latest: Flash Loan Provider Trait + Executor Rewrite

Built a pluggable `FlashLoanProvider` trait system and rewired the executor:

```
src/flash_loan/
├── mod.rs       — FlashLoanProvider trait, select_provider(), build_flash_loan_tx()
├── kamino.rs    — KaminoFlashLoanProvider (0.001% fee)
└── jupiter.rs   — JupiterFlashLoanProvider (0% fee — preferred)
```

The executor now:
1. Takes a list of flash loan providers ordered by preference (cheapest first)
2. For each liquidation, calls `select_provider(&providers, &debt_mint)` to pick the cheapest
3. Builds the full atomic tx: `[setup ATAs] → flash_borrow → liquidate → [swap] → flash_repay`
4. Falls back to no-flash-loan mode if no provider supports the mint

Adding new flash loan sources (Save, MarginFi, or any future protocol) is one `impl FlashLoanProvider`.

---

## Phase 1 Progress — Measurement Layer

### What's done

| Step | Status | Detail |
|---|---|---|
| **1.1A** Validate Kamino counts via Dune | ✅ **Done** | Dune query confirmed 14,355 successful + 28,389 failed = 42,768 total. Exact match to Kamino's published January report. |
| **1.1B** Backfill Kamino into ClickHouse | ✅ **Done** | All 42,768 signatures downloaded via Dune API, fetched via Triton RPC with 10x concurrency (~35 min), loaded into ClickHouse. |
| **1.2** Jupiter Lend ingestion | ✅ **Done** | 2,891 successful + 19,939 failed = 22,830 total. 10 unique liquidators. |
| **1.2+** MarginFi ingestion | ✅ **Done** | 8,799 successful + 1,924 failed = 10,723 total. 9 unique liquidators. |
| **1.2+** Save ingestion | ✅ **Done** | 1,145 successful + 2,168 failed = 3,313 total. 17 unique liquidators. |
| **1.3** Replay/simulation harness | ⚠️ **Partially done via Dune** | Actual per-event P&L computed for top 4 Kamino operators using Dune `tokens_solana.transfers` + `prices.usd`. Not yet built as a local Rust binary / SQL query. |
| **1.4** Dashboards | ⚠️ **Ad-hoc queries done** | Multiple analytical queries run against ClickHouse. No formal Grafana setup yet. |
| **1.5** February + March validation | ❌ **Not started** | Need to repeat the Dune export + backfill process for Feb and Mar. |

### ClickHouse data inventory

| Table | Rows | Venues |
|---|---|---|
| `liquidations` | 27,190 | Kamino (14,355), MarginFi (8,799), Jupiter Lend (2,891), Save (1,145) |
| `failed_liquidation_attempts` | 52,447 | Kamino (28,416), Jupiter Lend (19,939), Save (2,168), MarginFi (1,924) |
| `tx_metadata` | 79,637 | All venues |
| `obligations_snapshots` | 0 | Not populated (needs account data reads) |
| `reserves_snapshots` | 0 | Not populated (needs account data reads) |

### Oracle / USD price status

**USD prices are NOT in ClickHouse.** All `_usd` and `_price` columns remain NULL.

However, we successfully computed actual per-event profit using **Dune's `prices.usd` table** joined with `tokens_solana.transfers`. This approach works and produced the profitability analysis in `research/liquidator_profitability.md`. Two paths forward:

1. **Continue using Dune for P&L analysis** — free/cheap, already proven, minute-level prices available. Suitable for research and replay simulation.
2. **Build price enrichment into the local pipeline** — fetch prices from Pyth/Birdeye API and write to ClickHouse. Needed for real-time bot decision-making in Phase 2, but not blocking for Phase 1 completion.

### Data source costs incurred

| Source | Usage | Cost |
|---|---|---|
| Dune Analytics | ~8 queries, ~1,200 credits | Free tier |
| Dune API (CSV export) | 3 downloads (~8MB total) | API key usage |
| Triton RPC (getTransaction) | ~80,000 calls across all venues | Against prepaid balance |

---

## Key Findings

### Liquidator profitability (verified with real oracle data)

Full analysis in `research/liquidator_profitability.md`. Summary:

| Operator | Strategy | Net P&L (Jan 2026) | Win Rate | Avg/Tx |
|---|---|---|---|---|
| **8t7ZN** | Pre-funded, selective, low tips | **+$84,849** | 100% | +$143 |
| **LionX** | Pre-funded, cross-venue, high volume | **+$36,611** | 99.9% | +$42 |
| **evoxx** | 100% flash loan, high volume | **+$10,725** | 96.6% | +$3 |
| **4NUiC** | Flash loan, overtipping | **-$18,686** | 9% | -$187 |

**Critical correction:** Earlier estimate of 4NUiC's profit (+$47K) was wrong — they actually lost $19K. The error: Jito tips are paid via SOL transfers to tip accounts, not via priority fees. Our enrichment code detects this in `jito_tip_lamports`, but the initial analysis looked at the wrong field (`priority_fee_lamports`).

### Strategy insights from data

1. **Pre-funded capital beats flash loans** — 8t7ZN (pre-funded) makes $143/tx. evoxx (100% flash loan) makes $3/tx. 4NUiC (81% flash loan + overtipping) loses $187/tx.

2. **Selectivity beats volume** — 8t7ZN does 591 trades for $85K profit. evoxx does 3,253 trades for $11K profit. Fewer, better-targeted trades win.

3. **Jito tips are the largest cost** — not flash loan fees, not gas. 4NUiC's $17K in Jito tips exceeded their entire bonus revenue.

4. **Uncontested positions are the opportunity** — 93% of 8t7ZN's wins had zero competition in the same slot.

5. **Niche stablecoins reduce competition but don't eliminate tip costs** — PYUSD/AUSD/bSOL debt pairs have 12-17 competitors vs 171 for USDC, but the tip cost remains if you use Jito bundles.

### Venue comparison (January 2026, all from Dune)

| Venue | Successful | Failed | Total | Unique Liquidators |
|---|---|---|---|---|
| Kamino | 14,355 | 28,389 | 42,744 | 178 |
| MarginFi | 8,796 | 1,924 | 10,720 | 9 |
| Jupiter Lend | 2,888 | 19,939 | 22,827 | 10 |
| Save | 1,142 | 2,168 | 3,310 | 17 |

### Infrastructure analysis

- **Validator not justified** for liquidation alone. An RPC node ($1,200/month) provides the same detection advantage at Phase 3 scale.
- **Current setup (Triton managed)** is sufficient for Phases 1-2 at $200-400/month.
- **Self-hosted RPC node** becomes worthwhile when gRPC costs exceed $1,200/month (multi-venue live monitoring + arb).

---

## What needs to happen next

### Immediate next steps (completing Phase 1)

**1. Build the replay harness as a Dune query (not local Rust binary)**

The Dune approach is proven and cheaper than building local oracle enrichment. Create a single Dune query that:
- Joins liquidation instructions with `tokens_solana.transfers` and `prices.usd`
- Computes net P&L per event (inflows - outflows)
- Buckets by size and competition level
- Outputs the minimum profitable event size and optimal tip strategy

This replaces Step 1.3's local Rust binary with a Dune-native approach. The query pattern is proven — we ran it for 4 operators already.

**2. Run the replay across ALL Kamino liquidators (not just top 4)**

The 4-operator analysis covers ~5,765 of 14,355 events. The remaining ~8,590 events from 174 other operators would reveal the full distribution of profitability. This is one Dune query, ~300 credits.

**3. Backfill February and March**

Repeat the Dune export + Triton RPC + ClickHouse backfill for February (70,822 events) and March (551 events). February is the stress test — 70K events in a month. March validates calm-market behavior.

Estimated cost: ~$14 in Triton credits (70K + 551 getTransaction calls).

**4. Run the same profitability analysis for Jupiter Lend, MarginFi, Save**

We have the data in ClickHouse. Create Dune P&L queries for the top liquidators on each venue. This tells us whether the 8t7ZN "pre-funded + selective" strategy works cross-venue or is Kamino-specific.

### After Phase 1 completion → Phase 2

**5. Decide execution strategy based on data**

The profitability analysis will determine:
- Pre-funded or flash loan? (Data says pre-funded, but verify across venues)
- Minimum event size filter? (Likely $10K+ based on Kamino data)
- Which venues to target live? (Kamino confirmed, others pending analysis)
- Tip strategy? (Data says: target uncontested, tip minimally or not at all)

**6. Deploy narrow Kamino live bot**

- gRPC stream → detect unhealthy obligations
- Filter: only events above minimum profitable size
- Filter: only events with low expected competition
- Pre-funded capital: ~$30-50K in stablecoins + token accounts
- Jito bundle submission with minimum viable tip
- $50/day loss cap
- Full logging of every candidate, submission, outcome

---

## File inventory

```
research/
├── SUMMARY.md                        ← Unified event model
├── kamino.md                         ← Kamino protocol research
├── jupiter_lend.md                   ← Jupiter Lend research
├── marginfi.md                       ← MarginFi research
├── save.md                           ← Save research
└── liquidator_profitability.md       ← NEW: Actual P&L analysis with oracle data

data/
├── kamino_jan_2026.csv               ← 42,768 signatures (3.6MB)
├── jupiter_lend_jan_2026.csv         ← 22,827 signatures (1.9MB)
├── marginfi_jan_2026.csv             ← 10,720 signatures (930KB)
└── save_jan_2026.csv                 ← 3,311 signatures (287KB)

Dune queries created:
├── 7349740  Kamino v1+v2 count (validated: 14,355 + 28,389)
├── 7349938  Kamino signature export
├── 7350235  Jupiter Lend count (2,888 + 19,939)
├── 7350237  MarginFi count (8,796 + 1,924)
├── 7350310  Jupiter Lend signature export
├── 7350314  MarginFi signature export
├── 7350430  Save count (1,142 + 2,168)
├── 7350474  Save signature export
├── 7350846  4NUiC per-event P&L (token transfers + prices)
├── 7350930  4NUiC full P&L detail (100 events)
├── 7351017  evoxx P&L summary (+$10,725)
├── 7351112  LionX P&L summary (+$36,611)
└── 7351116  8t7ZN P&L summary (+$84,849)
```

---

## Open risks

1. **Oracle price coverage gaps.** Dune's `prices.usd` doesn't have prices for all Solana tokens (cTokens, exotic mints). This introduces noise in the P&L calculation — estimated 5-10% of transfers per transaction are unpriced.

2. **February backfill is 70K events.** At 10x concurrency, ~$14 in Triton credits, ~1.5 hours. Not blocking but needs to be done.

3. **Profitability analysis covers 4 of 178 Kamino operators.** The full picture requires running the P&L query across all operators, which is one Dune query but expensive in credits (~300 credits).

4. **No real-time price feed in the bot yet.** The Dune approach works for historical analysis but the live bot needs real-time oracle prices (Pyth/Switchboard) for EV filtering. This is a Phase 2 engineering task.

5. **Pre-funded capital requirement ($30-50K) is higher than originally assumed.** The flash loan strategy looked cheaper but the data shows it's less profitable. The bot needs significant token holdings to replicate 8t7ZN's approach.

6. **The STRATEGY.md revenue scenarios need revision.** The original $15K-120K/year range was based on estimated 1.3% average bonus. Actual per-event data shows the top operator made $85K in one month on one venue. But that was a crash month — calm months may still be near-zero. February and March data needed to calibrate.
