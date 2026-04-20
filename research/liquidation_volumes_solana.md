# Solana Lending Liquidations — Dune Analysis

Date: 2026-04-21
Scope: Kamino klend, MarginFi v2, Save (Solend). Lending markets only; perps/DEX out of scope.

## TL;DR

- Recent 7-day liquidation **counts** (2026-04-14 → 2026-04-20):
  - Save/Solend: **1,455**
  - MarginFi:    **565**
  - Kamino klend: **305**
- Last 30 days (approx): Save ~13,400 · Kamino ~10,300 · MarginFi ~2,200.
- **These are instruction counts, not USD volumes.** USD requires per-protocol decode + price joins (see Next steps).

## Dune table coverage

| Protocol | Decoded on Dune? | Table(s) used | Notes |
|---|---|---|---|
| Kamino klend | ✅ via `solana.instruction_calls_decoded` | filter `executing_account = 'KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD'`, `instruction_name IN ('liquidateObligationAndRedeemReserveCollateralV2', 'liquidateObligationAndRedeemReserveCollateral')` | No dedicated `kamino_solana.*` lending tables — only `kamino_solana.yvaults_*` (Kamino Liquidity). Lending must go through the canonical decoded table. |
| MarginFi v2 | ✅ | `marginfi_solana.marginfi_call_lendingaccountliquidate` (has `assetAmount`, `account_assetBank`) | Columns prefixed `call_*`. The event table `marginfi_evt_lendingaccountliquidateevent` exists but was **empty** at time of query. |
| Save / Solend | ❌ not decoded | `solana.instruction_calls`, filter program `So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo`, raw discriminator byte `0x0C` (LiquidateObligation) or `0x13` (LiquidateObligationAndRedeemReserveCollateral) | Count-only without additional parsing of `data` and `account_arguments`. |

## Daily counts — highlights

### Cascade days (signal of stress events)
- **Kamino**: Apr 1 (4,483) + Apr 2 (5,146) = ~9,600 in 48h vs. 5–30/day baseline. Likely a sharp drawdown / oracle event.
- **Save**: Mar 22 (2,258), Apr 10 (2,150), Apr 12 (2,191).
- **MarginFi**: Apr 5 (549), Mar 30 (459), Mar 16 (291), Apr 16 (256), Apr 14 (243).

### Baseline
- Kamino: 5–30/day; spikes rare but extreme.
- MarginFi: 10–250/day; many dust liquidations.
- Save: 50–500/day with frequent cascades.

## Saved Dune queries

- `dune.com/queries/7346835` — Kamino + MarginFi daily counts (180d)
- `dune.com/queries/7346913` — Save daily counts (30d, raw discriminator)
- `dune.com/queries/7346775` — Kamino klend instruction discovery (used once to map instruction names)
- `dune.com/queries/7346804` — Save raw program activity probe

Credits used this session: ~282 on community plan.

## Caveats

1. **Counts ≠ volume.** MarginFi often has many small liquidations; Kamino tends to fewer, larger ones. Count comparison overstates MarginFi/Save relative to USD-weighted ranking.
2. **Save is unfiltered instruction calls.** Failed transactions are excluded by Dune's canonical tables, but a single obligation may be liquidated over multiple consecutive instructions, inflating counts relative to "unique liquidation events."
3. **Kamino klend V1 vs V2:** V2 instruction dominates (~10k/30d); V1 only 2 calls in last 30d.
4. **MarginFi event table empty** — had to use call table. Means `liquidateePreHealth`/`liquidateePostHealth` are not available for scale/severity.
5. `solana.instruction_calls` over 180 days for a single program timed out at 30 min on medium tier — Save query had to be narrowed to 30 days.

## Next steps (to get true USD volumes)

### Kamino klend
- Decode `liquidityAmount` from `data` bytes `[8:16]` (u64 LE, Anchor layout after 8-byte discriminator).
- Identify repay reserve liquidity mint from `account_arguments` (index depends on V1 vs V2 IDL — V2: `account_arguments[5]` is the `repayReserveLiquidityMint`; verify against a sample tx).
- Join to `prices.usd` on `(contract_address = mint, minute = date_trunc('minute', block_time))`.

### MarginFi
- `account_assetBank` is a bank PDA, not a mint. Build a bank→mint map from `marginfi_solana.marginfi_call_lendingpooladdbank` (or equivalent init events) and join.
- `assetAmount` is in asset-mint native units; divide by mint decimals then multiply by USD price.

### Save / Solend
- Decode liability amount from `data[1:9]` (u64 LE, no discriminator padding — Solend uses single-byte tags).
- Liability reserve is in `account_arguments[4]` for LiquidateObligation, different for the combined instruction — verify from Solend program source.
- Map reserve → liability mint via reserve account state (requires RPC or a pre-built lookup; Dune has no Solend reserve table).

### Alternative: reconstruct from SPL transfers
- For each liquidation tx, intersect `tokens_solana.transfers` in-tx with known reserve vault addresses (liquidator → reserve = repay leg). Avoids per-protocol decode but requires a reserve-vault lookup table per protocol and can double-count if multiple liquidations share a tx.

## Open questions

- Do we want a rolling 365-day view? Currently only 30–180d.
- Jupiter Perps and DefiTuna were explicitly deprioritized this turn — revisit if scope widens beyond lending.
- Kamino klend IDL V2 account layout should be pinned down from the actual program IDL in `idls/` rather than inferred, before committing the price-join query.
