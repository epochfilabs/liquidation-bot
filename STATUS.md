# Project Status — April 22, 2026

## Bot is structurally complete

The full pipeline from detection to submission is wired end-to-end:

```
gRPC stream (Yellowstone)
  → account update for Kamino / Jupiter Lend / Save / MarginFi
  → is_position_account()           filter noise
  → evaluate_health()               is it liquidatable?
  → parse_positions()               extract deposits/borrows
  → evaluate_opportunity()           EV filter
      → SkipTooSmall                 below $5K repay
      → SkipLowEv                   bonus < $10
      → SkipDailyCapReached          spent > $50/day
      → Submit                       proceed
  → execute_liquidation()            build tx
      → select_provider()            Jupiter (0%) or Kamino (0.001%)
      → build_flash_loan_tx()        borrow → liquidate → repay
  → submit_liquidation() via Jito    atomic bundle with tip
  → record to Supabase + DailyTracker
```

### New modules built

| Module | File | Tests | Purpose |
|---|---|---|---|
| **FlashLoanProvider trait** | `src/flash_loan/mod.rs` | — | Pluggable flash loan interface. `select_provider()` picks cheapest. |
| **Kamino provider** | `src/flash_loan/kamino.rs` | — | 0.001% fee. Registers reserves by liquidity mint. |
| **Jupiter provider** | `src/flash_loan/jupiter.rs` | — | 0% fee. Registers mints with derived PDAs. |
| **Provider auto-init** | `src/flash_loan/init.rs` | — | Fetches on-chain Kamino reserves + derives Jupiter PDAs at startup. |
| **EV filter** | `src/risk/mod.rs` | 5 | Min repay size, min bonus, tip recommendation. |
| **Daily loss cap** | `src/risk/mod.rs` | — | Atomic tracker, auto-resets at midnight UTC. |
| **Jito bundles** | `src/jito/mod.rs` | 5 | `send_bundle()`, tip instructions, 8 tip accounts. |
| **Executor rewrite** | `src/liquidator/executor.rs` | — | Unified entry point for all 4 venues, uses flash loan trait + Jito. |

**Total tests: 43 in main crate + 80+ across workspace = 123+ tests, 0 failures.**

### Configuration (all via environment variables)

```bash
# Risk management
MIN_REPAY_AMOUNT=5000000            # $5K minimum event size (6-decimal tokens)
MIN_ESTIMATED_BONUS_USD=10          # $10 minimum estimated profit
DAILY_TIP_CAP_LAMPORTS=357142857    # ~$50/day at $140/SOL
MAX_TIP_PER_TX_LAMPORTS=10000000    # ~$1.40/tx max tip
ESTIMATED_BONUS_RATE=0.011          # 1.1% (Kamino January average)

# Jito
JITO_ENDPOINT=https://frankfurt.mainnet.block-engine.jito.wtf/api/v1/bundles
JITO_ENABLED=true                   # false = fallback to standard RPC

# Flash loans auto-initialize from on-chain data
# Jupiter Lend (0% fee) preferred, Kamino (0.001% fee) fallback
```

---

## Indexer Data Summary

### ClickHouse (local, Docker)

| Table | Rows |
|---|---|
| `liquidations` | 32,360 |
| `failed_liquidation_attempts` | 99,117 |
| `tx_metadata` | 131,477 |

### By venue and month

| Venue | Jan Liquidations | Jan Failed | Feb Liquidations | Oracle Prices |
|---|---|---|---|---|
| **Kamino** | 14,355 | 28,413 | Not yet | 97% ($15.3M repaid) |
| **Jupiter Lend** | 2,888 | 19,939 | 4,344 (partial) | 71% ($13.5M repaid) |
| **MarginFi** | 8,796 | 1,924 | Not yet | 0% (empty mints) |
| **Save** | 1,142 | 2,168 | Not yet | 0% (empty mints) |

### Verified operator P&L (from Dune token transfer analysis)

| Operator | Strategy | Net P&L (Jan) | Win Rate | Avg/Tx |
|---|---|---|---|---|
| **8t7ZN** | Pre-funded, selective | **+$84,849** | 100% | +$143 |
| **LionX** | Pre-funded, cross-venue | **+$36,611** | 99.9% | +$42 |
| **evoxx** | Flash loan, high volume | **+$10,725** | 96.6% | +$3.30 |
| **4NUiC** | Flash loan, overtipping | **-$18,686** | 9% | -$187 |

Full analysis: `research/liquidator_profitability.md`

---

## Key Findings

### Strategy (from data)

1. **Jupiter Lend is the best venue to target first.** $4,660 avg event (4.4x Kamino), 10 competitors (vs 172), zero-fee flash loans.

2. **Flash loan + minimal tipping works.** evoxx made $10,725 in January using 100% flash loans on 3,253 trades at $3.30 avg profit per tx. Starting capital: ~$300 (gas + ATA rent).

3. **Overtipping destroys profitability.** 4NUiC lost $18,686 because their $122 avg Jito tip exceeded the bonus on 91% of events.

4. **The EV filter is critical.** 96% of Kamino February events were sub-$1K (avg $60). At $3.30 avg profit for the evoxx-style strategy, the filter must reject events where tip cost > expected bonus.

5. **Niche debt tokens reduce competition.** PYUSD, AUSD, bSOL debt pairs have 12-17 competitors vs 171 for USDC. But 4NUiC's loss proves niche targeting alone doesn't guarantee profitability — tip discipline matters more.

### Infrastructure

- **Validator not justified** — 8t7ZN makes $85K/month without one. An RPC node ($1,200/month) is the right Phase 3 upgrade.
- **Triton RPC appears to be on an unlimited plan** — 3.1K requests and 16.5GB bandwidth observed without charges.
- **Dune free tier** (2,500 credits) exhausted. Second key active with credits. The 2-minute query timeout blocks February full export.

---

## Remaining work to go live

### Must-have

| Item | Effort | Notes |
|---|---|---|
| **Validate Jupiter flash loan PDAs** | 1 hour | The derived PDAs in `init.rs` need to be checked against on-chain accounts. One RPC call per PDA. |
| **Real oracle price in EV filter** | 2 hours | Currently hardcodes $1.0 for debt tokens. Need Pyth price feed or a cached daily price lookup. Non-stablecoin debt (JitoSOL, SOL) will be mispriced. |
| **Test against live gRPC stream** | 1 hour | Run with `RUST_LOG=info` against Triton gRPC, observe detection rate, verify no panics. Don't submit — shadow mode first. |
| **Shadow mode run** | 1 week | Log all candidates and simulated P&L without submitting. Verify the EV filter produces sensible decisions. |

### Nice-to-have before live

| Item | Effort | Notes |
|---|---|---|
| February backfill | 4 hours | ~122K getTransaction calls via Triton. Blocked by Dune export timeout for sig extraction. Alternative: use RPC getSignaturesForAddress. |
| MarginFi/Save mint resolution | 2 hours | Read Bank/Reserve account data at known offsets to extract token mints. Enables oracle price enrichment for these venues. |
| Jupiter swap integration | 4 hours | When collateral ≠ debt token, swap via Jupiter after liquidation. Currently a TODO in the flash loan tx builder. |
| Grafana dashboards | 2 hours | ClickHouse → Grafana for real-time monitoring of daily stats, P&L, event rates. |

---

## File inventory

```
src/
├── main.rs                         ← Full event loop: detect → filter → flash loan → Jito → log
├── config/mod.rs                   ← AppConfig
├── flash_loan/
│   ├── mod.rs                      ← FlashLoanProvider trait, select_provider(), build_flash_loan_tx()
│   ├── kamino.rs                   ← KaminoFlashLoanProvider (0.001% fee)
│   ├── jupiter.rs                  ← JupiterFlashLoanProvider (0% fee)
│   └── init.rs                     ← Auto-init from on-chain data at startup
├── jito/
│   └── mod.rs                      ← Jito bundle submission, tip accounts, send_bundle()
├── risk/
│   └── mod.rs                      ← EV filter, daily loss cap, DailyTracker
├── liquidator/
│   ├── executor.rs                 ← Unified executor: all 4 venues, flash loan trait, Jito submit
│   ├── flash_loan.rs               ← Kamino-native flash loan tx builder (legacy, still used)
│   ├── instructions.rs             ← klend instruction builders
│   ├── profitability.rs            ← Profit estimation
│   └── reserve.rs                  ← Reserve account parsing
├── protocols/                      ← LendingProtocol trait + 4 venue implementations
├── grpc/mod.rs                     ← Yellowstone gRPC subscription
├── db/mod.rs                       ← Supabase audit trail
├── decoder/mod.rs                  ← Obligation discriminator
└── obligation/                     ← Health evaluation + position parsing

decoders/                           ← 10 indexer decoder crates
crates/                             ← 3 indexer pipeline crates (indexer-core, processors, backfill)
schema/                             ← ClickHouse DDL + data model
research/                           ← Protocol research + liquidator profitability analysis
data/                               ← Dune exports + price CSVs (~25MB total)
scripts/                            ← local-test.sh, clickhouse-shell.sh, enrich_prices.py, fetch_sigs_rpc.py
```

---

## TODO list

Full task list: `TODO.md`

Priority order:
1. Validate Jupiter flash loan PDAs (1 hour)
2. Add real oracle price to EV filter (2 hours)
3. Shadow mode test against live gRPC (1 week observation)
4. First live submission with $50/day loss cap
5. February backfill + analysis (when convenient)
6. MarginFi/Save mint resolution (when convenient)
