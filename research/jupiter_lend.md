# Jupiter Lend — Liquidation Research

## Program Identity

Jupiter Lend is a **multi-program system** built on Fluid/Instadapp technology. Six programs, all Anchor:

| Program | Program ID | IDL Version | Role |
|---|---|---|---|
| **Vaults** | `jupr81YtYssSyPt8jbnGuiWon5f6x9TcDEFxYe3Bdzi` | 0.1.2 | Position management, liquidation, tick/branch orderbook |
| **Liquidity** | `jupeiUmn818Jg1ekPURTpr4mFo29p46vygyykFJ3wZC` | 0.1.2 | Token reserves, supply/borrow operations |
| **Lending** | `jup3YeL8QhtSx1e253b2FDvsMNC87fDrgQZivbrndc9` | 0.1.0 | Lending market rate model |
| **Oracle** | `jupnw4B6Eqs7ft6rxpzYLJZYSnrpRgPcr589n5Kv4oc` | 0.1.3 | Price aggregation (Pyth, Chainlink, Redstone) |
| **Flash Loan** | `jupgfSgfuAXv4B6R2Uxu85Z1qdzgju79s6MfZekN6XS` | 0.1.0 | Zero-fee flash loans |
| **Lending Reward Rate Model** | `jup7TthsMgcR9Y3L277b8Eo9uboVSmu1utkuXHNUKar` | 0.1.0 | Reward distribution rate model |

### IDL Sources

All six IDLs at:
```
https://raw.githubusercontent.com/jup-ag/jupiter-lend/refs/heads/main/target/idl/{vaults,liquidity,lending,oracle,flashloan,lending_reward_rate_model}.json
```

Source code: [jup-ag/jupiter-lend](https://github.com/jup-ag/jupiter-lend) + [Code4rena audit repo](https://github.com/code-423n4/2026-02-jupiter-lend).

## Liquidation Instruction

Lives in the **Vaults program**. Instruction name: `liquidate`.

### Arguments

| Name | Type | Description |
|---|---|---|
| `debt_amt` | `u64` | Amount of debt to repay |
| `col_per_unit_debt` | `u128` | Slippage protection: min collateral per unit debt (1e15 precision) |
| `absorb` | `bool` | Whether to absorb bad-debt ticks above `max_tick` first |
| `transfer_type` | `Option<TransferType>` | `SKIP=0`, `DIRECT=1`, `CLAIM=2` |
| `remaining_accounts_indices` | `Vec<u8>` | 4 indices into remaining accounts for oracle_sources, branches, ticks, tick_has_debt |

### Instruction Data Layout

```
[0..8]    discriminator: [223, 179, 226, 125, 48, 46, 39, 74]
[8..16]   debt_amt (u64 LE)
[16..32]  col_per_unit_debt (u128 LE)
[32]      absorb (u8: 0=false, 1=true)
[33..]    transfer_type (Option<enum>: 0=None, or 1+variant)
[..]      remaining_accounts_indices (Borsh Vec<u8>: 4-byte LE len + 4 index bytes)
```

### Accounts (26 fixed + remaining)

| # | Account | Mut | Signer | Description |
|---|---|---|---|---|
| 0 | `signer` | **yes** | **yes** | Liquidator wallet |
| 1 | `signer_token_account` | **yes** | no | Liquidator's borrow token ATA |
| 2 | `to` | no | no | Collateral recipient (usually = signer) |
| 3 | `to_token_account` | **yes** | no | Recipient's supply token ATA (init_if_needed) |
| 4 | `vault_config` | **yes** | no | VaultConfig PDA |
| 5 | `vault_state` | **yes** | no | VaultState PDA |
| 6 | `supply_token` | no | no | Supply token mint |
| 7 | `borrow_token` | no | no | Borrow token mint |
| 8 | `oracle` | no | no | Oracle account (oracle program PDA) |
| 9 | `new_branch` | **yes** | no | Branch account for liquidation branch |
| 10 | `supply_token_reserves_liquidity` | **yes** | no | TokenReserve on liquidity program (supply) |
| 11 | `borrow_token_reserves_liquidity` | **yes** | no | TokenReserve on liquidity program (borrow) |
| 12 | `vault_supply_position_on_liquidity` | **yes** | no | UserSupplyPosition on liquidity |
| 13 | `vault_borrow_position_on_liquidity` | **yes** | no | UserBorrowPosition on liquidity |
| 14 | `supply_rate_model` | no | no | Rate model account |
| 15 | `borrow_rate_model` | no | no | Rate model account |
| 16 | `supply_token_claim_account` | **yes** | no | Optional (program ID as placeholder OK) |
| 17 | `liquidity` | no | no | Liquidity PDA |
| 18 | `liquidity_program` | no | no | Liquidity program ID |
| 19 | `vault_supply_token_account` | **yes** | no | Vault's supply token ATA |
| 20 | `vault_borrow_token_account` | **yes** | no | Vault's borrow token ATA |
| 21 | `supply_token_program` | no | no | SPL Token / Token-2022 |
| 22 | `borrow_token_program` | no | no | SPL Token / Token-2022 |
| 23 | `system_program` | no | no | System Program |
| 24 | `associated_token_program` | no | no | ATA program |
| 25 | `oracle_program` | no | no | Oracle program ID |
| remaining | dynamic | varies | no | Oracle sources, branches, ticks, tick_has_debt (indexed by `remaining_accounts_indices`) |

## Tick-Based Liquidation Model

Jupiter Lend does **not** liquidate individual positions. It liquidates ranges of ticks on a vault's orderbook.

### How it works

1. Each position has a `tick` value encoding its debt/collateral ratio: `ratio ≈ 1.0015^tick`. Higher tick = more debt relative to collateral.
2. The oracle returns a `liquidation_tick` — the threshold above which positions are underwater.
3. The `liquidate` instruction iterates **downward** from `VaultState.topmost_tick` toward `liquidation_tick`.
4. For each tick with debt:
   - Calculates debt and collateral using the tick's ratio
   - Partially or fully liquidates the tick
   - Updates `debt_factor` on a `Branch` account to track partial liquidation
5. All positions at liquidated ticks are affected proportionally via the branch's `debt_factor`.

### Absorption phase

If `absorb=true` and `topmost_tick > max_tick` (derived from `VaultConfig.liquidation_max_limit`):
- All debt above `max_tick` is absorbed as bad debt
- Socialized across the vault via `VaultState.absorbed_debt_amount` and `absorbed_col_amount`

### Key implication for indexing

**There is no specific "liquidatee" in the liquidation transaction.** The instruction liquidates a range of ticks, not a specific position. Individual position holders discover their positions were liquidated when they next interact with the protocol.

To determine affected positions:
1. Parse `LogLiquidateInfo` event for `start_tick` and `end_tick`
2. Find all Position accounts with `tick` in that range for the vault
3. Use branch `debt_factor` to compute each position's loss

For the `liquidations` table, the `liquidatee` field should be `NULL` for Jupiter Lend. The `obligation` field maps to the vault_config PDA.

## Liquidation Penalty

| Field | Type | Location | Description |
|---|---|---|---|
| `liquidation_penalty` | `u16` | VaultConfig | Penalty in basis points (100 = 1%). Values as low as 0.1% (10). |
| `MAX_LIQUIDATION_PENALTY` | const | — | 9970 (99.7%) |
| `collateral_factor` | `u16` | VaultConfig | /1000. e.g., 800 = 80% max borrow LTV |
| `liquidation_threshold` | `u16` | VaultConfig | /1000. e.g., 900 = 90% (liquidation trigger) |
| `liquidation_max_limit` | `u16` | VaultConfig | /1000. e.g., 950 = 95% (bad debt absorption) |

**Application**: Folded into `col_per_debt` during oracle price calculation:
```
col_per_debt = (raw_col_per_debt * (1e4 + penalty)) / 1e4
```

**Destination**: The entire penalty goes to the liquidator as excess collateral. The protocol does not take a separate cut.

**Profit reconstruction**: `liquidator_profit = collateral_received_value - debt_repaid_value`. Parse from pre/post token balance changes on `signer_token_account` (borrow) and `to_token_account` (supply).

## CPI Call Chain

A single `liquidate` call triggers:

```
Vaults::liquidate
  ├── CPI → Oracle::get_exchange_rate_liquidate
  │         └── Reads Pyth/Chainlink/Redstone feeds from remaining accounts
  │         └── Returns u128 exchange rate (1e15 precision)
  ├── [Optional: absorption — state updates only, no CPI]
  ├── [Main liquidation loop — tick/branch state updates, no CPI]
  ├── CPI → Liquidity::pre_operate (borrow token — refresh exchange prices)
  ├── SPL Token transfer: liquidator → vault_borrow_token_account
  ├── CPI → Liquidity::operate (borrow_amount = -actual_debt, reduces vault debt)
  ├── CPI → Liquidity::operate (supply_amount = -actual_col, withdraws collateral)
  └── Emit LogLiquidate { signer, col_amount, debt_amount, to }
  └── Emit LogLiquidateInfo { vault_id, start_tick, end_tick }
```

**For indexing**: Walk inner instructions at depth 1-2 to capture:
- Oracle exchange rate from the oracle CPI
- Actual supply/borrow amounts from liquidity operate CPIs
- `LogLiquidate` and `LogLiquidateInfo` events from program logs
- `LogAbsorb` event if absorption occurred

## Oracle Setup

Jupiter Lend has its **own oracle program** (`jupnw4B6Eqs7ft6rxpzYLJZYSnrpRgPcr589n5Kv4oc`).

### Oracle account structure

Each `Oracle` account contains a `nonce` (u16), `bump`, and a vector of `Sources`:

```rust
pub struct Sources {
    pub source: Pubkey,         // e.g., Pyth price account
    pub invert: bool,           // invert the rate
    pub multiplier: u128,       // scaling multiplier
    pub divisor: u128,          // scaling divisor
    pub source_type: SourceType,
}
```

**SourceType variants**: `Pyth`, `StakePool`, `MsolPool`, `Redstone`, `Chainlink`, `SinglePool`, `JupLend`, `ChainlinkDataStreams`

### Price at liquidation slot

The vaults program CPIs `oracle::get_exchange_rate_liquidate(nonce)` passing the Oracle account + remaining accounts (actual price feeds). The oracle reads each source, applies multiplier/divisor/invert, returns a composite exchange rate as u128 with 15 decimal precision.

For Pyth specifically: validates `VerificationLevel::Full`, checks `MAX_AGE_LIQUIDATE`, applies `CONFIDENCE_SCALE_FACTOR_LIQUIDATE`.

**For indexing**: Oracle source accounts are in the remaining accounts. Parse inner CPI instructions to extract which feeds were used.

## Flash Loan Bundling

### Flash loan instructions (zero fee)

- `flashloan_borrow(amount: u64)` — 14 accounts, discriminator `[103, 19, 78, 24, 240, 9, 135, 63]`
- `flashloan_payback(amount: u64)` — 14 accounts, discriminator `[213, 47, 153, 137, 84, 243, 94, 232]`

Fee is configurable but currently set to **zero**.

### Typical liquidation tx layout

```
ix[0]  flashloan_borrow(debt_amount)
ix[1]  liquidate(debt_amt, col_per_unit_debt, absorb, transfer_type, indices)
ix[2]  Jupiter DEX swap (collateral → borrow token)    [if tokens differ]
ix[3]  flashloan_payback(debt_amount)
```

Address Lookup Tables (ALTs) are commonly used since total account count can exceed 64.

## Program Upgrade History

Jupiter Lend launched in private beta ~July 2025, public beta late 2025. Code4rena audit: February 2026.

| Program | Version | Notes |
|---|---|---|
| Vaults | 0.1.2 | Stable |
| Liquidity | 0.1.2 | Stable |
| Oracle | **0.1.3** | **Upgraded** — added `ChainlinkDataStreams` source type |
| Lending | 0.1.0 | Stable |
| Flashloan | 0.1.0 | Stable |
| Reward Rate Model | 0.1.0 | Stable |

All account structs use `#[account(zero_copy)]` with `#[repr(C, packed)]`, so byte layouts are ABI-stable. No layout-breaking changes documented. The oracle program's `SourceType` enum extension is the main risk for future decoder breakage.

**For the 2-year backfill**: Jupiter Lend is the youngest protocol. It only has data from ~mid-2025 onward, so the backfill window is ~1 year.

## Position NFT Model

Each borrow position is an NFT. The `Position` account (71 bytes) stores:
- `vault_id: u16`
- `nft_id: u32`
- `position_mint: Pubkey` (the NFT mint)
- `is_supply_only_position: bool`
- `tick: i32`
- `supply_amount: u64`
- `dust_debt_amount: u64`

The NFT holder is the position owner. When ticks are liquidated, the position's effective value changes based on the branch's `debt_factor`, but the NFT itself is retained.

### Dead address simulation

If `to == ADDRESS_DEAD` (all zeros) in the liquidate instruction, the program emits a diagnostic and reverts with `VaultLiquidationResult`. This is a simulation/query mechanism used by bots to check liquidation profitability.
