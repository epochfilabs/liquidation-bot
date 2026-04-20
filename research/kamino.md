# Kamino Lend (klend) — Liquidation Research

## Program Identity

| Field | Value |
|---|---|
| Program ID | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` |
| Framework | Anchor |
| Source repo | [Kamino-Finance/klend](https://github.com/Kamino-Finance/klend) |
| IDL source | [klend-sdk `src/idl/klend.json`](https://github.com/Kamino-Finance/klend-sdk/blob/master/src/idl/klend.json) |
| Carbon decoder | `carbon-kamino-lending-decoder` exists on crates.io (pre-built) |
| IDL on-chain | Not guaranteed; use the SDK repo IDL |

## Liquidation Instructions

Two variants exist. Both share the same argument list but differ in account layout.

### Arguments (v1 and v2 identical)

| Name | Type | Description |
|---|---|---|
| `liquidity_amount` | `u64` | Amount of debt token the liquidator repays |
| `min_acceptable_received_liquidity_amount` | `u64` | Slippage protection: minimum collateral received after redemption |
| `max_allowed_ltv_override_percent` | `u64` | Staging-only override; ignored on mainnet unless liquidator == obligation owner |

### v1: `liquidateObligationAndRedeemReserveCollateral`

Original instruction. Requires that `refreshReserve`, `refreshObligation`, and `refreshObligationFarmsForReserve` appear earlier in the same transaction, verified via instruction sysvar introspection (`check_refresh_ixs!`).

**Accounts (20 fixed + remaining):**

| # | Account | Mut | Signer | Description |
|---|---|---|---|---|
| 0 | `liquidator` | no | **yes** | Liquidator authority |
| 1 | `obligation` | **yes** | no | Obligation being liquidated |
| 2 | `lendingMarket` | no | no | Lending market the obligation belongs to |
| 3 | `lendingMarketAuthority` | no | no | PDA: seeds = `[b"lma", lending_market.key()]` |
| 4 | `repayReserve` | **yes** | no | Reserve for the debt being repaid |
| 5 | `repayReserveLiquidityMint` | no | no | Mint of the repay reserve's liquidity token |
| 6 | `repayReserveLiquiditySupply` | **yes** | no | Vault holding repay reserve's liquidity |
| 7 | `withdrawReserve` | **yes** | no | Reserve for the collateral being seized |
| 8 | `withdrawReserveLiquidityMint` | no | no | Mint of the withdraw reserve's liquidity token |
| 9 | `withdrawReserveCollateralMint` | **yes** | no | cToken mint of the withdraw reserve |
| 10 | `withdrawReserveCollateralSupply` | **yes** | no | Vault holding withdraw reserve's cTokens |
| 11 | `withdrawReserveLiquiditySupply` | **yes** | no | Vault holding withdraw reserve's liquidity |
| 12 | `withdrawReserveLiquidityFeeReceiver` | **yes** | no | Fee vault of the withdraw reserve |
| 13 | `userSourceLiquidity` | **yes** | no | Liquidator's token account for the repay token |
| 14 | `userDestinationCollateral` | **yes** | no | Liquidator's cToken account (intermediate) |
| 15 | `userDestinationLiquidity` | **yes** | no | Liquidator's account for redeemed collateral |
| 16 | `collateralTokenProgram` | no | no | SPL Token (non-2022) |
| 17 | `repayLiquidityTokenProgram` | no | no | Token program for repay (supports Token-2022) |
| 18 | `withdrawLiquidityTokenProgram` | no | no | Token program for withdrawal (supports Token-2022) |
| 19 | `instructionSysvarAccount` | no | no | Sysvar Instructions |
| remaining | deposit reserves | no | no | All other deposit reserves in the obligation (elevation group tracking) |

### v2: `liquidateObligationAndRedeemReserveCollateralV2`

Introduced in **v1.10.0 (2025-02-21)**. Avoids transaction introspection by inlining farm-refresh accounts. Same 20 accounts as v1, plus:

| Account | Mut | Description |
|---|---|---|
| `collateralFarmsAccounts.obligationFarmUserState` | **yes** | Optional |
| `collateralFarmsAccounts.reserveFarmState` | **yes** | Optional |
| `debtFarmsAccounts.obligationFarmUserState` | **yes** | Optional |
| `debtFarmsAccounts.reserveFarmState` | **yes** | Optional |
| `farmsProgram` | no | Farms program ID |

Reserve/obligation refresh is still a prerequisite — v2 only removes the farm-refresh introspection requirement.

## Liquidation Bonus Computation

The bonus is **dynamic**, not a fixed percentage. Computed in `calculate_liquidation_bonus()`.

### Per-reserve config fields

| Field | Type | Description |
|---|---|---|
| `min_liquidation_bonus_bps` | `u16` | Floor for the bonus (basis points) |
| `max_liquidation_bonus_bps` | `u16` | Ceiling for the bonus |
| `bad_debt_liquidation_bonus_bps` | `u16` | Bonus at insolvency (LTV >= 99%) |
| `protocol_liquidation_fee_pct` | `u8` | % of the bonus taken by the protocol (0-100) |

### Algorithm (standard LTV-exceeded liquidation)

1. `unhealthy_factor = user_ltv - max_allowed_ltv`
2. `max_bonus = max(collateral_reserve.max_liq_bonus_bps, debt_reserve.max_liq_bonus_bps)` — capped by elevation group if applicable
3. `min_bonus = max(collateral_reserve.min_liq_bonus_bps, debt_reserve.min_liq_bonus_bps)`
4. `effective_min = max(min_bonus, unhealthy_factor)` — bonus grows with how underwater the position is
5. `collared_bonus = min(effective_min, max_bonus)`
6. `final_bonus = min(collared_bonus, 1.0 - user_no_bf_ltv)` — never push into bad debt

Near-insolvency path (no_bf_ltv >= 0.99): uses `bad_debt_liquidation_bonus_bps`.

### Where the bonus lands

- Liquidator repays `repay_amount` of debt, receives collateral worth `repay_amount * (1 + bonus)`
- Protocol fee: `ceil(bonus_portion * protocol_liquidation_fee_pct / 100)`, minimum 1 lamport
- Fee is transferred from `userDestinationLiquidity` to `withdrawReserveLiquidityFeeReceiver`
- **For indexer profit reconstruction**: `liquidator_profit = post_balance(userDestinationLiquidity) - pre_balance(userSourceLiquidity) - flash_loan_fee`

### Close factor

Controlled by `LendingMarket` fields:

| Field | Type | Description |
|---|---|---|
| `liquidation_max_debt_close_factor_pct` | `u8` | Max % of total debt liquidatable at once (e.g., 50) |
| `insolvency_risk_unhealthy_ltv_pct` | `u8` | Above this LTV, close factor = 100% |
| `min_full_liquidation_value_threshold` | `u64` | Below this borrowed value, full liquidation required |
| `max_liquidatable_debt_market_value_at_once` | `u64` | Hard cap on $ value per liquidation |

### Additional liquidation reasons

Beyond standard LTV-exceeded, klend supports:
- `IndividualDeleveraging` — obligation marked for deleveraging, bonus ramps over time
- `MarketWideDeleveraging` — reserve limit crossed
- `ReserveDebtMaturityReached` — fixed-term debt matured
- `ObligationBorrowDebtTermReached` — individual borrow term expired
- `ObligationOrder` — user-set stop-loss/take-profit, bonus interpolated from order parameters

Each has its own bonus logic. The indexer should capture the `LiquidationReason` from the instruction data or events.

## Oracle Setup

Three providers plus Kamino's own oracle aggregator (Scope):

### Per-reserve oracle config (in `TokenInfo`)

| Struct | Fields |
|---|---|
| `PythConfiguration` | `price: Pubkey` |
| `SwitchboardConfiguration` | `price_aggregator: Pubkey`, `twap_aggregator: Pubkey` |
| `ScopeConfiguration` | `price_feed: Pubkey`, `price_chain: [u16; 4]`, `twap_chain: [u16; 4]` |

Plus heuristic bounds: `max_twap_divergence_bps`, `max_age_price_seconds`, `max_age_twap_seconds`.

### Critical for indexing

Oracle accounts are **NOT** passed to the liquidation instruction. They are passed to `refreshReserve`, which writes `market_price_sf` (u128, 2^60-scaled) into the Reserve account. The liquidation reads that cached price.

**To get the price at liquidation slot**: Read `Reserve.liquidity.market_price_sf` from the reserve account state at that slot, OR read the oracle account at that slot directly.

## Flash Loan Bundling

### Kamino flash loan instructions

- `flashBorrowReserveLiquidity` — borrow from klend
- `flashRepayReserveLiquidity` — repay with fee

These must be a matched pair in the same transaction (sysvar introspection). Flash loan fee: configurable per reserve (typically 0.001%).

### Standard liquidation transaction layout

```
ix[0]  refreshReserve (repay reserve, with oracle accounts)
ix[1]  refreshReserve (withdraw reserve, with oracle accounts)
ix[2]  refreshObligation
ix[3]  flashBorrowReserveLiquidity (borrow debt token)
ix[4]  liquidateObligationAndRedeemReserveCollateral
ix[5]  [optional] Jupiter swap (collateral -> debt token)
ix[6]  flashRepayReserveLiquidity (repay flash loan + fee)
```

Jupiter swaps are common when collateral != debt token. Jito bundles are standard for atomicity.

## Program Upgrade History (2024-04 to 2026-04)

| Version | Date | Breaking Changes |
|---|---|---|
| **v1.10.0** | 2025-02-21 | **V2 instructions added** (deposit, withdraw, borrow, repay, liquidate). Removes global unhealthy borrow value. Removes tx introspection for v2 variants. |
| v1.11.0 | 2025-02-21 | Stricter CollateralExchangeRate rounding |
| v1.12.0 | 2025-06-02 | Global config, obligation orders |
| v1.12.5 | 2025-11-07 | Unconditional obligation orders, configurable min deleveraging bonus |
| v1.12.6 | 2025-11-14 | Multiple reserves of same mint on same market |
| v1.13.0 | 2026-02-04 | Borrow orders |
| v1.14.0 | 2026-02-26 | Withdraw queues |
| v1.16.0 | 2026-03-23 | Throttle term-based liquidations |
| v1.17.0 | 2026-03-31 | Cancel withdraw ticket, early repay penalty |
| v1.18.0 | 2026-04-13 | Reserve emergency mode |

**Key layout implication**: v1.10.0 is the only version that introduced new liquidation instruction variants. Both v1 and v2 coexist — the indexer must decode both. The current IDL has **69 instructions**. Instruction discriminators (8-byte SHA256 prefix) did not change for v1.

**For the 2-year backfill**: Before ~Feb 2025, only v1 liquidation instruction exists. After, both v1 and v2 may appear. The discriminator uniquely identifies which variant was used.
