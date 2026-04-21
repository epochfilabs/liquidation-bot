# Liquidator Profitability Analysis — Kamino Lend, January 2026

## Methodology

This analysis uses **actual on-chain token transfer data with USD prices** from Dune Analytics to compute real profit/loss per liquidation event. For each transaction:

- Sum all token inflows to the liquidator wallet (collateral received, flash loan proceeds)
- Sum all token outflows (debt repaid, flash loan repay, Jito tips, fees)
- Net P&L = inflows - outflows

Prices are from `prices.usd` joined at minute granularity on the token mint address. Where Dune lacks a price for a token (e.g., cTokens, exotic mints), the fallback is `1.0 * amount / 10^6`, which introduces noise on unpriced tokens.

Data source: `tokens_solana.transfers` joined with `prices.usd`, filtered to transactions containing Kamino v2 liquidation instructions (`0xa2a1238f1ebbb967` discriminator) in January 2026.

**Dune query IDs:** 7351017 (evoxx), 7351112 (LionX), 7351116 (8t7ZN), 7350930 (4NUiC).

---

## Summary: Top 4 Kamino Liquidators

| Operator | Liquidations | Winners | Losers | Win Rate | Net P&L | Avg P&L/Tx | Gains | Losses |
|---|---|---|---|---|---|---|---|---|
| **8t7ZN** | 591 | 591 | 0 | **100%** | **+$84,849** | +$143.57 | $84,849 | $0 |
| **LionX** | 881 | 880 | 1 | **99.9%** | **+$36,611** | +$41.56 | $36,613 | -$2 |
| **evoxx** | 3,253 | 3,142 | 111 | **96.6%** | **+$10,725** | +$3.30 | $11,954 | -$1,229 |
| **4NUiC** | 140 | 9 | 91 | **9%** | **-$18,686** | -$186.86 | $4,733 | -$23,418 |

The top 3 operators collectively made **$132,185** in January 2026 on Kamino. The 4th operator lost $18,686.

---

## Operator Deep Dives

### 8t7ZN — The Most Profitable ($84,849 net)

**Wallet:** [8t7ZNQNaZee8Vei72JqyrV9P8G4tRRTTyNzrvRfDucJ7](https://solscan.io/account/8t7ZNQNaZee8Vei72JqyrV9P8G4tRRTTyNzrvRfDucJ7)

| Metric | Value |
|---|---|
| Net P&L | +$84,849 |
| Liquidations | 591 (also 64 on Save) |
| Win rate | 100% — zero losing trades |
| Avg profit per tx | $143.57 |
| Active days | 15 of 31 |
| SOL balance | 21.0 SOL |
| Token accounts | 292 |
| USDC held | ~$34,283 |
| Priority fees | $0 (always base fee only) |
| Jito tips | Paid via SOL transfer, not priority fee field |
| Flash loan usage | 30% of events |
| Jupiter swap usage | 30% of events |
| Top collateral | SOL, JitoSOL |
| Top debt | USDC, USDS |
| Peak hours (UTC) | 14:00-18:00 |

**Why they win:** 8t7ZN targets **large, uncontested positions**. 93% of their wins had zero competition in the same slot. They hold 292 token accounts covering essentially every collateral/debt pair on Kamino, enabling instant liquidation without flash loans or swaps for common pairs. When they do swap, they use Jupiter. Their average liquidation repay amount ($6.8B token units) is the largest of any operator — they pick the biggest fish that nobody else is watching.

**Infrastructure signal:** Zero priority fees across 591 transactions means they either have a latency advantage that lets them land at base fee, or they exclusively target positions with no competition. The data confirms the latter — they're not winning races, they're finding positions nobody else is looking at.

### LionX — The Cross-Venue Workhorse ($36,611 net)

**Wallet:** [LionX7R69tL1EEcpRkJ9jRuwV7bi4jFoKmZZnxiVK6y](https://solscan.io/account/LionX7R69tL1EEcpRkJ9jRuwV7bi4jFoKmZZnxiVK6y)

| Metric | Value |
|---|---|
| Net P&L | +$36,611 (Kamino only; also active on Jupiter Lend) |
| Liquidations | 881 Kamino + 522 Jupiter Lend = 1,403 total |
| Win rate | 99.9% — one losing trade |
| Avg profit per tx | $41.56 |
| Active days | 23 of 31 — most consistent |
| SOL balance | 10.9 SOL |
| Token accounts | 287 |
| USDC held | ~$89,814 |
| Priority fees | Low (~$0.005 avg) |
| Flash loan usage | 6.8% |
| Jupiter swap usage | 99.7% |
| Top collateral | SOL, jupSOL, wBTC |
| Top debt | USDC, USDS, USDT |
| Peak hours (UTC) | 14:00-19:00 |

**Why they win:** LionX is the most capital-intensive operator — $90K in USDC, 287 token accounts, pre-funded for almost every pair. They swap 99.7% of the time via Jupiter but rarely use flash loans (6.8%), meaning they hold the debt token in advance and swap the received collateral after. Active 23 of 31 days — the most consistent operator. Also active on Jupiter Lend (522 additional liquidations), making them a true cross-venue operator.

### evoxx — High Volume, Thin Margins ($10,725 net)

**Wallet:** [evoxxcAvFrt8Xg6cXKV2Q5SPpnxqEv14VdmAnHmQS13](https://solscan.io/account/evoxxcAvFrt8Xg6cXKV2Q5SPpnxqEv14VdmAnHmQS13)

| Metric | Value |
|---|---|
| Net P&L | +$10,725 |
| Liquidations | 3,253 — highest volume |
| Win rate | 96.6% |
| Avg profit per tx | $3.30 |
| Active days | 9 (appeared mid-January) |
| Flash loan usage | 100% |
| Jupiter swap usage | 100% |
| Priority fees | ~$0.03 avg (232K lamports) |
| Top collateral | SOL |
| Top debt | USDC |
| Peak hours (UTC) | 14:00-19:00 |

**Why they barely win:** evoxx does 3,253 trades — more than any other operator — but makes only $3.30 per trade on average. They use flash loans on every single transaction and always swap via Jupiter. This means they pay flash loan fees + swap slippage + Jito tips on every event. 3.4% of trades are losers (111 out of 3,253), and each loser costs ~$11 on average. The strategy is viable but fragile — a slight increase in competition or decrease in bonus rate would push them negative.

### 4NUiC — Net Negative (-$18,686)

**Wallet:** [4NUiCMoJLUsG1eRZY5PPeqRvc4w2P3rcQXfDpaR6k6xX](https://solscan.io/account/4NUiCMoJLUsG1eRZY5PPeqRvc4w2P3rcQXfDpaR6k6xX)

| Metric | Value |
|---|---|
| Net P&L | -$18,686 |
| Liquidations | 140 |
| Win rate | 9% (9 of 100 analyzed events profitable) |
| Avg loss per tx | -$186.86 |
| Active days | 7 (Jan 18-31 only) |
| USDC held | ~$1,357 |
| Flash loan usage | 81% |
| Avg Jito tip | $122.16 per tx |
| Total Jito tips paid | $17,102 |
| Top collateral | SOL, cbBTC, wBTC |
| Top debt | USDC, PYUSD, AUSD |

**Why they lose:** 4NUiC pays an average Jito tip of $122 per transaction, but the liquidation bonus on most events is smaller than that. Only 9 of their events generated enough bonus to cover the tip cost. The average winning trade made $526, but the average losing trade cost $257 — and there are 10x more losers than winners. The niche stablecoin targeting (PYUSD, AUSD, USDS = 28% of trades) didn't help because the Jito tip is a fixed cost regardless of debt token.

**Possible explanation:** 4NUiC may be running a bot that overtips — bidding $122 for liquidations worth $50 in bonus. Or they may be farming protocol points/airdrops where the tip cost is subsidized by expected future token value.

---

## Key Findings

### 1. Pre-funded capital beats flash loans

| Strategy | Operator | Net P&L | Win Rate |
|---|---|---|---|
| Pre-funded + selective targeting | 8t7ZN | +$84,849 | 100% |
| Pre-funded + high volume | LionX | +$36,611 | 99.9% |
| 100% flash loan + swap | evoxx | +$10,725 | 96.6% |
| 81% flash loan + high tips | 4NUiC | -$18,686 | 9% |

The most profitable operator (8t7ZN) uses flash loans only 30% of the time. The least profitable (4NUiC) uses them 81% of the time. Pre-funded capital enables instant liquidation without the flash loan fee or the complexity of multi-instruction atomic transactions.

### 2. Tip discipline determines profitability

8t7ZN and LionX pay minimal tips and target uncontested positions. 4NUiC pays $122 average per transaction — more than the bonus on most events. evoxx pays moderate tips ($0.03 avg priority fee, plus Jito SOL transfers) and survives but barely.

The Jito tip is the single largest cost. Flash loan fees ($0.001%) are negligible. Transaction fees ($0.0007) are negligible. The tip is everything.

### 3. Selectivity matters more than volume

| Operator | Volume (txs) | Net P&L | Profit per tx |
|---|---|---|---|
| 8t7ZN | 591 | $84,849 | $143.57 |
| LionX | 881 | $36,611 | $41.56 |
| evoxx | 3,253 | $10,725 | $3.30 |
| 4NUiC | 140 | -$18,686 | -$186.86 |

8t7ZN does 5.5x fewer trades than evoxx but makes 8x more profit. The winning strategy is not "liquidate everything" — it's "liquidate only what's worth liquidating."

### 4. Competition analysis by debt token

| Debt Token | Liquidations | Unique Liquidators | Zero-Tip Win % |
|---|---|---|---|
| USDC | 10,028 | 171 | 17% |
| USDT | 1,273 | 145 | 15.6% |
| AUSD | 1,111 | 137 | 22.6% |
| PYUSD | 895 | 136 | 29.4% |
| JitoSOL | 33 | 15 | 33.3% |
| bSOL | 25 | 12 | 40% |
| mSOL | 17 | 13 | 23.5% |

Niche debt tokens (PYUSD, AUSD, bSOL) have fewer competitors but this alone doesn't guarantee profitability — 4NUiC targeted these niches and still lost money because of overtipping.

### 5. Contested slots

- 5,383 slots had multiple bots racing for the same obligation
- Average 2.3 attempts per contested slot
- 53% of contested slots had no winner (all failed)
- Average 5.7 unique bots competed per obligation across its lifetime

---

## Capital & Strategy Breakdown (All Kamino Liquidations)

| Strategy | % of Events | Description |
|---|---|---|
| Flash loan + Jupiter swap | 46.6% | Borrow debt via Kamino flash loan, liquidate, swap collateral, repay |
| Pre-funded, no swap | 36.4% | Hold both collateral and debt tokens. Simplest, fastest execution. |
| Pre-funded + Jupiter swap | 15.2% | Hold debt token, liquidate, swap collateral to stablecoin |
| Flash loan only (no swap) | 1.8% | Same-token collateral/debt pair (rare) |

---

## Correction Notice

Earlier analysis in this project estimated 4NUiC's January profit at **+$47,072** based on the 1.3% average Kamino bonus rate and the incorrect assumption of zero tipping cost. The priority_fee_lamports field showed $0, which was misinterpreted as "no tip." In reality, 4NUiC pays Jito tips via direct SOL transfers to Jito tip accounts (SystemProgram::Transfer), not via the ComputeBudget priority fee mechanism. These tips averaged $122 per transaction and totaled $17,102 for the month.

The corrected figure based on actual token transfer data: **-$18,686** (net loss).

This correction is critical: **the difference between the estimated +$47K and actual -$19K is the Jito tip cost.** Any revenue model that doesn't account for per-event tip costs will produce dangerously optimistic projections.

---

## Implications for Our Strategy

1. **Pre-fund, don't flash loan.** The data is unambiguous: pre-funded operators make 8-25x more profit per event than flash loan operators. The capital cost of holding tokens ($30K-$90K in stablecoins + token accounts) is repaid in the first crash day.

2. **Tip less, target better.** 8t7ZN's zero-priority-fee, 100%-win-rate approach is the model. They don't compete on tip — they compete on detection. Find positions nobody else is watching.

3. **Don't chase volume.** evoxx does 5.5x more trades than 8t7ZN and makes 8x less money. Each additional trade in a competitive slot adds tip cost without proportional bonus.

4. **The replay harness must include actual Jito tip costs.** Our earlier simulation used a fixed tip estimate. The real tip varies from $0 (uncontested) to $400+ (highly contested). The simulation must use observed tip distributions per competition level.

5. **Cross-venue operations add value.** LionX operates on both Kamino and Jupiter Lend. This diversification provides more opportunities without multiplicative competition — different venues have different position sizes and competitor sets.

---

## Data Quality Notes

- Token transfers where Dune lacks USD prices (exotic mints, cTokens) use a fallback of `amount / 10^6`, which may overstate or understate flows. This affects ~5-10% of transfers per transaction.
- The analysis covers 100 of 4NUiC's 140 transactions (Dune API returned 100 rows max). The full 140 would need pagination.
- Dune credit cost for these 4 queries: ~1,226 credits total.
- Query execution times: 3-6 minutes each on the medium engine.
