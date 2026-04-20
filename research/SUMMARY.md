# Research Summary — Unified Liquidation Event Model

## Program ID Registry

| Venue | Program ID | Framework | Decoder Strategy |
|---|---|---|---|
| Kamino Lend | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` | Anchor | `carbon-kamino-lending-decoder` (pre-built on crates.io) or `carbon-cli parse` |
| Jupiter Lend — Vaults | `jupr81YtYssSyPt8jbnGuiWon5f6x9TcDEFxYe3Bdzi` | Anchor | `carbon-cli parse --idl vaults.json` |
| Jupiter Lend — Liquidity | `jupeiUmn818Jg1ekPURTpr4mFo29p46vygyykFJ3wZC` | Anchor | `carbon-cli parse --idl liquidity.json` |
| Jupiter Lend — Lending | `jup3YeL8QhtSx1e253b2FDvsMNC87fDrgQZivbrndc9` | Anchor | `carbon-cli parse --idl lending.json` |
| Jupiter Lend — Oracle | `jupnw4B6Eqs7ft6rxpzYLJZYSnrpRgPcr589n5Kv4oc` | Anchor | `carbon-cli parse --idl oracle.json` |
| Jupiter Lend — Flash Loan | `jupgfSgfuAXv4B6R2Uxu85Z1qdzgju79s6MfZekN6XS` | Anchor | `carbon-cli parse --idl flashloan.json` |
| Jupiter Lend — Reward Model | `jup7TthsMgcR9Y3L277b8Eo9uboVSmu1utkuXHNUKar` | Anchor | `carbon-cli parse --idl lending_reward_rate_model.json` |
| MarginFi v2 | `MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA` | Anchor | `carbon-cli parse --idl marginfi.json` |
| Save (Solend) | `So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo` | SPL (non-Anchor) | **Custom `InstructionDecoder`** — hand-written Borsh tag enum |

## IDL Sources

| Venue | Source | Pin Strategy |
|---|---|---|
| Kamino | [klend-sdk `src/idl/klend.json`](https://github.com/Kamino-Finance/klend-sdk/blob/master/src/idl/klend.json) | Git SHA of klend-sdk |
| Jupiter Lend (6 IDLs) | [jup-ag/jupiter-lend `target/idl/*.json`](https://github.com/jup-ag/jupiter-lend/tree/main/target/idl) | Git SHA of jupiter-lend |
| MarginFi | GitHub release artifacts or `@mrgnlabs/marginfi-client-v2` npm | Release tag |
| Save | N/A — no IDL | Source code SHA of `solendprotocol/solana-program-library` mainnet branch |

## Liquidation Instruction Comparison

| | Kamino | Jupiter Lend | MarginFi | Save |
|---|---|---|---|---|
| **Instruction name** | `liquidateObligationAndRedeemReserveCollateral` (v1) / `...V2` | `liquidate` | `lendingAccountLiquidate` | Tag 17: `LiquidateObligationAndRedeemReserveCollateral` |
| **Discriminator** | 8-byte SHA256 prefix (Anchor) | `[223,179,226,125,48,46,39,74]` | 8-byte SHA256 prefix (Anchor) | `0x11` (single byte tag) |
| **Key argument** | `liquidity_amount: u64` (debt to repay) | `debt_amt: u64` (debt to repay) | `asset_amount: u64` (**collateral to seize**) | `liquidity_amount: u64` (debt to repay) |
| **Fixed accounts** | 20 | 26 | 10 | 15 (tag 17) or 11 (tag 12) |
| **Remaining accounts** | Deposit reserves (elevation groups) | Oracle sources, branches, ticks | Oracles + observation banks (variable per OracleSetup) | None |
| **Signer position** | Account #0 | Account #0 | Account #4 (`authority`) | Account #13 |
| **Obligation/Position** | Account #1 (`obligation`) | No specific position — liquidates tick ranges | Account #5 (`liquidatee_marginfi_account`) | Account #10 (tag 17) |
| **Liquidatee identity** | `obligation.owner` (Pubkey in obligation data) | **No specific liquidatee** — tick ranges affected | Account #5 authority (Pubkey in marginfi account data) | `obligation.owner` (Pubkey in obligation data) |
| **Has v2 variant** | Yes (v1.10.0, Feb 2025) | No | No | Tag 12 (no redeem) vs Tag 17 (with redeem) |

## Bonus / Penalty / Fee Comparison

| | Kamino | Jupiter Lend | MarginFi | Save |
|---|---|---|---|---|
| **Bonus type** | Dynamic: ramps with LTV overshoot, collared by min/max bps per reserve | Fixed per vault (u16 bps) | Fixed: 2.5% liquidator + 2.5% insurance = 5% total | Fixed per reserve (u8 percentage) |
| **Protocol fee** | `protocol_liquidation_fee_pct` (% of bonus) → fee receiver | None (entire penalty to liquidator) | 2.5% → insurance vault | `protocol_liquidation_fee` (% of bonus) → fee receiver |
| **Close factor** | `liquidation_max_debt_close_factor_pct` (e.g., 50%), 100% if insolvency risk | No explicit factor — bounded by tick range | No explicit factor — bounded by post-condition (health must stay <= 0) | 20% (`LIQUIDATION_CLOSE_FACTOR`), dust closeout below 2 WAD |
| **Bad debt handling** | `bad_debt_liquidation_bonus_bps` at LTV >= 99% | `absorb=true` socializes debt above `max_tick` | Implicit via insurance vault | `super_unhealthy_borrow_value` trigger |

## Oracle Comparison

| | Kamino | Jupiter Lend | MarginFi | Save |
|---|---|---|---|---|
| **Providers** | Pyth, Switchboard, Scope (own aggregator) | Own oracle program aggregating Pyth, Chainlink, Redstone | Pyth, Switchboard + integration oracles (Kamino, Drift, JupLend, Solend) | Pyth (primary), Switchboard (fallback) |
| **Oracle in liquidation ix** | **No** — cached in reserve via `refreshReserve` | **Yes** — CPI to oracle program, feeds in remaining accounts | **Yes** — oracle accounts in remaining accounts | **No** — cached in reserve via `refreshReserve` (tag 3) |
| **Price reconstruction** | Read `Reserve.liquidity.market_price_sf` at liquidation slot | Parse inner CPI to oracle program for exchange rate | Read oracle accounts from remaining accounts | Read reserve cached price at liquidation slot |

## Flash Loan Comparison

| | Kamino | Jupiter Lend | MarginFi | Save |
|---|---|---|---|---|
| **Own flash loan** | Yes — `flashBorrow/RepayReserveLiquidity` | Yes — `flashloan_borrow/payback` (zero fee) | Yes — `start/end_flashloan` (flag-based) | Yes — tags 19/20 |
| **Fee** | ~0.001% per reserve | **Zero** | None (just health check at end) | ~0.3% per reserve |
| **Verification** | Sysvar instruction introspection | Sysvar instruction introspection | Sysvar instruction introspection | Sysvar instruction introspection |
| **Jupiter swap common** | Yes (when collateral != debt) | Yes (always, since vaults are single-pair) | Yes | Yes |

## Unified Event Model

This is the canonical shape for the `liquidations` table, covering all four venues.

### Core Fields (all venues)

| Field | Type | Source | Notes |
|---|---|---|---|
| `venue` | `LowCardinality(String)` | Derived | `"kamino"`, `"jupiter_lend"`, `"marginfi"`, `"save"` |
| `program_id` | `FixedString(44)` | Transaction | The program that executed the liquidation |
| `slot` | `UInt64` | Block | Solana slot number |
| `block_time` | `DateTime64(3)` | Block | UTC timestamp |
| `tx_signature` | `FixedString(88)` | Transaction | Base58-encoded signature |
| `ix_index` | `UInt16` | Transaction | Index of the liquidation instruction in the tx |
| `inner_ix_index` | `Nullable(UInt16)` | Transaction | If the liquidation is a CPI inner instruction |
| `succeeded` | `Bool` | Transaction | Whether the tx succeeded |

### Participants

| Field | Type | Source | Notes |
|---|---|---|---|
| `liquidator` | `FixedString(44)` | Instruction | Signer: account #0 (Kamino, Jupiter), #4 (MarginFi), #13 (Save) |
| `liquidatee` | `Nullable(FixedString(44))` | Account data | `obligation.owner` (Kamino, Save), `marginfi_account.authority` (MarginFi), **NULL** (Jupiter Lend — tick-based) |
| `obligation` | `FixedString(44)` | Instruction | Obligation/position account: #1 (Kamino), vault_config #4 (Jupiter), #5 (MarginFi), #10 (Save tag 17) |
| `market` | `FixedString(44)` | Instruction | Lending market / group / vault |

### Collateral & Debt

| Field | Type | Source | Notes |
|---|---|---|---|
| `collateral_reserve` | `FixedString(44)` | Instruction | Withdraw reserve (Kamino #7, Save #5), asset bank (MarginFi #1), vault_config (Jupiter) |
| `debt_reserve` | `FixedString(44)` | Instruction | Repay reserve (Kamino #4, Save #3), liab bank (MarginFi #2), vault_config (Jupiter) |
| `collateral_mint` | `FixedString(44)` | Instruction/Account | Mint of the collateral token |
| `debt_mint` | `FixedString(44)` | Instruction/Account | Mint of the debt token |
| `repay_amount` | `UInt128` | Instruction args | `liquidity_amount` (Kamino/Save), `debt_amt` (Jupiter), derived from `asset_amount` (MarginFi) |
| `withdraw_collateral_amount` | `UInt128` | Token balances | Post - pre balance on liquidator's collateral account |

### USD Values (enriched)

| Field | Type | Source | Notes |
|---|---|---|---|
| `repay_amount_usd` | `Decimal64(6)` | Oracle price × repay_amount | Price at liquidation slot |
| `collateral_seized_usd` | `Decimal64(6)` | Oracle price × withdraw_amount | Price at liquidation slot |
| `liquidator_profit_usd` | `Decimal64(6)` | collateral_seized - repay_amount - fees | Net profit |
| `liquidation_bonus_bps` | `UInt32` | Reserve/vault config | Effective bonus in basis points |
| `collateral_price` | `Decimal64(12)` | Oracle | Price per unit at liquidation slot |
| `debt_price` | `Decimal64(12)` | Oracle | Price per unit at liquidation slot |

### Transaction Metadata

| Field | Type | Source | Notes |
|---|---|---|---|
| `tx_fee_lamports` | `UInt64` | Transaction meta | Base transaction fee |
| `priority_fee_lamports` | `UInt64` | Transaction meta | Compute budget priority fee |
| `jito_tip_lamports` | `Nullable(UInt64)` | Inner instructions | SystemProgram::Transfer to Jito tip account |
| `compute_units_consumed` | `UInt32` | Transaction meta | CU used |
| `used_flashloan` | `Bool` | Inner instructions | Whether a flash loan was used |
| `flashloan_source` | `Nullable(LowCardinality(String))` | Inner instructions | `"kamino"`, `"jupiter"`, `"marginfi"`, `"save"`, `"external"` |
| `used_jupiter_swap` | `Bool` | Inner instructions | Whether a Jupiter swap was included |
| `raw_ix_data` | `String` | Instruction | Hex-encoded instruction data |

### Venue-Specific Nullable Fields

| Field | Type | Venue | Notes |
|---|---|---|---|
| `obligation_ltv_pre` | `Nullable(Decimal64(6))` | Kamino, Save | LTV before liquidation (from obligation snapshot) |
| `obligation_health_pre` | `Nullable(Decimal64(6))` | MarginFi | Health factor before liquidation |
| `tick_start` | `Nullable(Int32)` | Jupiter Lend | Start tick of liquidation range |
| `tick_end` | `Nullable(Int32)` | Jupiter Lend | End tick of liquidation range |
| `absorbed_bad_debt` | `Nullable(Bool)` | Jupiter Lend | Whether absorption phase ran |
| `insurance_fee_amount` | `Nullable(UInt128)` | MarginFi | Amount sent to insurance vault |
| `protocol_fee_amount` | `Nullable(UInt128)` | Kamino, Save | Protocol fee taken from bonus |
| `liquidation_reason` | `Nullable(LowCardinality(String))` | Kamino | `"ltv_exceeded"`, `"deleveraging"`, `"debt_maturity"`, `"obligation_order"` |

## Jito Tip Accounts (Mainnet)

All 8 known tip accounts — scan inner instructions for `SystemProgram::Transfer` to any of these:

```
96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5
HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe
Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY
ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49
DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh
ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt
DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL
3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT
```

Source: [Jito Foundation — On-Chain Addresses](https://jito-foundation.gitbook.io/mev/mev-payment-and-distribution/on-chain-addresses)

## Infrastructure Notes

### Carbon Framework

- **Repo**: [sevenlabs-hq/carbon](https://github.com/sevenlabs-hq/carbon), `carbon-core` v0.12.0, Rust 2021
- **CLI**: npm package `@sevenlabs-hq/carbon-cli` — `parse` command generates Rust decoder crates from Anchor IDLs
- **Pre-built decoder**: `carbon-kamino-lending-decoder` exists on crates.io
- **Custom decoders**: Save requires manual `InstructionDecoder` + `AccountDecoder` implementations
- **Datasources**: `carbon-yellowstone-grpc-datasource` (v0.9.1) for real-time; `carbon-rpc-block-crawler-datasource` for historical
- **No Old Faithful datasource exists** — use `faithful-cli rpc` to serve a local RPC from CAR files, then point `carbon-rpc-block-crawler-datasource` at it

### Old Faithful

- **URL pattern**: `https://files.old-faithful.net/<epoch>/epoch-<epoch>.car`
- **Size**: ~500 GB per epoch (~2 days)
- **Tools**: `faithful-cli rpc` runs a local RPC from CAR files; `faithful-cli index` generates indexes
- **Local RPC supports**: `getBlock`, `getTransaction`, `getSignaturesForAddress`, `getBlockTime`

### Backfill Architecture

```
Old Faithful CAR files
    → faithful-cli rpc (local RPC server)
        → carbon-rpc-block-crawler-datasource
            → Carbon Pipeline (per-venue decoders + processors)
                → ClickHouse batch writer
```

For real-time tail: swap `carbon-rpc-block-crawler-datasource` for `carbon-yellowstone-grpc-datasource` against Triton Dragon's Mouth.

## Program Upgrade Risk Matrix

| Venue | Layout Risk | Window | Mitigation |
|---|---|---|---|
| Kamino | **Medium** — v2 liquidation added Feb 2025, 18 releases in 2 years | Full 2-year | Decode both v1 and v2 discriminators; IDL covers both |
| Jupiter Lend | **Low** — `repr(C, packed)` layouts, only ~1 year of data | ~mid-2025 onward | Oracle `SourceType` enum is the main extension point |
| MarginFi | **Medium** — remaining accounts interpretation changed Dec 2024, Mar 2025 | Full 2-year | Data-driven: read Bank's `OracleSetup` to determine remaining account count |
| Save | **Low** — instruction tags stable, obligation layout padded for extensions | Full 2-year | Validate ObligationCollateral/Liquidity sizes against on-chain data (56/80 vs 88/112) |

## Critical Findings & Corrections

1. **Save program ID in existing bot is wrong.** `SLendK7ySfcEzyaFqy93gDnD3RtrpXJcnRwb6zFHJSh` → should be `So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo`
2. **Save instruction tag in existing bot is wrong.** Tag 15 used for liquidation → should be tag 17
3. **Jupiter Lend has no per-position liquidatee.** The `liquidatee` field must be `Nullable` in the unified model
4. **MarginFi's liquidation argument is collateral amount, not debt amount.** All other venues specify debt. The indexer must derive repay_amount from balance changes for MarginFi
5. **Kamino v1 vs v2 coexist from Feb 2025 onward.** The indexer must handle both discriminators
6. **No Carbon datasource for Old Faithful CAR files.** The bridge is `faithful-cli rpc` → `carbon-rpc-block-crawler-datasource`
