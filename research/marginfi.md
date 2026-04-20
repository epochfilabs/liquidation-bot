# MarginFi v2 — Liquidation Research

## Program Identity

| Field | Value |
|---|---|
| Program ID | `MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA` |
| Framework | Anchor |
| Source repo | [mrgnlabs/marginfi-v2](https://github.com/mrgnlabs/marginfi-v2) |
| IDL source | Attached to [GitHub releases](https://github.com/mrgnlabs/marginfi-v2/releases) or extracted from `@mrgnlabs/marginfi-client-v2` npm package |
| IDL on-chain | May work via `solana idl fetch`; releases advise using the attached IDL |
| Program version const | `3` |

## Liquidation Instruction

### `lendingAccountLiquidate`

### Arguments

| Name | Type | Description |
|---|---|---|
| `asset_amount` | `u64` | Amount of collateral asset to seize (native units) |

### Named Accounts (10 fixed)

| # | Account | Mut | Signer | Description |
|---|---|---|---|---|
| 0 | `marginfi_group` | no | no | MarginfiGroup; must not be paused |
| 1 | `asset_bank` | **yes** | no | Bank for the collateral asset; `has_one = group` |
| 2 | `liab_bank` | **yes** | no | Bank for the liability; `has_one = group`; standard asset_tag only |
| 3 | `liquidator_marginfi_account` | **yes** | no | Liquidator's MarginfiAccount; same group; not frozen/receivership |
| 4 | `authority` | no | **yes** | Must match liquidator account's authority |
| 5 | `liquidatee_marginfi_account` | **yes** | no | Liquidatee's MarginfiAccount; same group; not in receivership |
| 6 | `bank_liquidity_vault_authority` | no | no | PDA: `[b"liquidity_vault_auth", liab_bank.key()]` |
| 7 | `bank_liquidity_vault` | **yes** | no | PDA: `[b"liquidity_vault", liab_bank.key()]` |
| 8 | `bank_insurance_vault` | **yes** | no | PDA: `[b"insurance_vault", liab_bank.key()]` |
| 9 | `token_program` | no | no | SPL Token / Token-2022 interface |

### Remaining Accounts (complex, ordered)

```
[
  liab_mint_ai,                     // ONLY if Token-2022 mint
  asset_oracle_ai,                  // Oracle(s) for the asset bank
  liab_oracle_ai,                   // Oracle(s) for the liability bank
  liquidator_observation_ais...,    // [bank, oracle_1, ..., oracle_N] per position
  liquidatee_observation_ais...,    // [bank, oracle_1, ..., oracle_N] per position
]
```

The number of accounts per bank depends on `OracleSetup`:
- `Fixed`: 1 account (bank only, no oracle)
- `PythPushOracle` / `SwitchboardPull`: 2 accounts (bank + oracle)
- `FixedKamino` / `FixedDrift` / `FixedJuplend`: 2 accounts (bank + integration account)
- Staked/integration variants: varies

**Breaking change (v0.1.8-rc3, March 2025):** Banks in remaining accounts must be passed as **writable**.

## Liquidation Mechanics

### Health Model

```
health = SUM(asset_value_i * maint_asset_weight_i) - SUM(liability_value_j * maint_liability_weight_j)
```

- Asset prices use **low bias** (price - confidence interval)
- Liability prices use **high bias** (price + confidence interval)
- Account is liquidatable when `health <= 0`

### Liquidation Flow

1. **Pre-check**: `check_pre_liquidation_condition` verifies `health <= 0` at maintenance level
2. Liquidator specifies `asset_amount` of collateral to seize
3. Protocol computes liability amount using oracle prices and discount factors
4. Balance transfers occur via **share accounting** (not token transfers between user accounts):
   - Liquidatee's asset shares decrease, liquidator's asset shares increase
   - Liquidatee's liability shares decrease, liquidator's liability shares increase
5. Insurance fee transferred as SPL tokens from liquidity vault to insurance vault
6. **Post-check**: `check_post_liquidation_condition` verifies:
   - Liquidatee health **improved** (strictly better than pre)
   - Liquidatee health **remains <= 0** (cannot over-liquidate past healthy)
   - Liquidator account remains healthy (init-level health check)

### Fees and Discount

**Constants (hardcoded in program):**

| Constant | Value | Description |
|---|---|---|
| `LIQUIDATION_LIQUIDATOR_FEE` | 2.5% (0.025) | Liquidator's discount |
| `LIQUIDATION_INSURANCE_FEE` | 2.5% (0.025) | Protocol insurance cut |
| `LIQUIDATION_BONUS_FEE_MINIMUM` | 5% (0.05) | Total minimum premium |

**Formulas:**
```
liquidator_discount = 1 - LIQUIDATION_LIQUIDATOR_FEE = 0.975
final_discount      = 1 - (LIQUIDATION_LIQUIDATOR_FEE + LIQUIDATION_INSURANCE_FEE) = 0.95

liab_amount_liquidator = (asset_amount * asset_price * liquidator_discount) / liab_price
liab_amount_final      = (asset_amount * asset_price * final_discount) / liab_price
insurance_fee          = liab_amount_liquidator - liab_amount_final
```

The liquidator pays `liab_amount_liquidator` of debt; the liquidatee gets `liab_amount_final` credited. The difference goes to insurance.

**For indexer profit reconstruction**: The liquidator's profit is the difference between the value of collateral received (asset_amount at asset_price) and the liability absorbed (liab_amount_liquidator at liab_price), minus the 2.5% discount they already received. Net: `asset_amount * asset_price * LIQUIDATION_LIQUIDATOR_FEE`.

### Close Factor

**No explicit close factor.** Liquidation is bounded by:
- Liquidatee's available collateral (`pre_balance >= asset_amount`)
- Post-condition: health must remain <= 0 (prevents over-liquidation)
- `LIQUIDATION_CLOSEOUT_DOLLAR_THRESHOLD`: $5 — below this, special handling
- The post-condition check is the de facto close factor — the liquidator can only take enough to improve health without crossing zero

## Bank Account Structure

Each asset in MarginFi has a `Bank` account (1864 bytes, 8-byte Anchor discriminator).

### Key fields

| Field | Type | Offset | Description |
|---|---|---|---|
| `mint` | `Pubkey` | 8 | Token mint |
| `mint_decimals` | `u8` | 40 | Decimal places |
| `group` | `Pubkey` | 41 | Parent MarginfiGroup |
| `asset_share_value` | `WrappedI80F48` | 80 | Multiplier: shares → underlying (assets) |
| `liability_share_value` | `WrappedI80F48` | 96 | Multiplier: shares → underlying (liabilities) |
| `liquidity_vault` | `Pubkey` | 112 | PDA token account for deposits |
| `insurance_vault` | `Pubkey` | 146 | PDA token account for insurance fund |
| `config.maint_asset_weight` | `WrappedI80F48` | 312 | Maintenance weight for assets |
| `config.maint_liability_weight` | `WrappedI80F48` | 344 | Maintenance weight for liabilities |

### Share-to-amount conversion

```rust
asset_amount = asset_shares * asset_share_value
liability_amount = liability_shares * liability_share_value
```

Share values grow over time as interest accrues (similar to cToken exchange rates).

### WrappedI80F48 encoding

`i128` in little-endian with 48-bit fractional part:
```
f64_value = i128_value as f64 / (1i128 << 48) as f64
```

## MarginfiAccount (Liquidatee/Liquidator Account)

Size: 2312 bytes (verified against mainnet). Layout:

| Offset | Field | Size | Description |
|---|---|---|---|
| 0 | discriminator | 8 | Anchor discriminator |
| 8 | group | 32 | MarginfiGroup pubkey |
| 40 | authority | 32 | Account owner (wallet) |
| 72 | balances | 16 * 136 = 2176 | Array of 16 Balance entries |

### Balance entry (136 bytes each)

| Offset | Field | Type | Size |
|---|---|---|---|
| 0 | active | u8/bool | 1 |
| 1 | bank_pk | Pubkey | 32 |
| 33 | pad0 | [u8; 7] | 7 |
| 40 | asset_shares | WrappedI80F48 | 16 |
| 56 | liability_shares | WrappedI80F48 | 16 |
| 72 | emissions_outstanding | WrappedI80F48 | 16 |
| 88 | last_update | u64 | 8 |
| 96 | padding | [u64; 5] | 40 |

## Oracle Setup

MarginFi supports **Pyth and Switchboard**, plus integration-specific oracle types.

### OracleSetup Enum

| Variant | Accounts | Description |
|---|---|---|
| `PythPushOracle` | 1 oracle | Primary for most banks |
| `SwitchboardPull` | 1 oracle | Switchboard on-demand |
| `Fixed` | 0 | Static fixed price (stablecoins) |
| `StakedWithPythPush` | oracle + exchange rate | Staked assets |
| `KaminoPythPush` | oracle + Kamino reserve | Kamino LP via Pyth |
| `KaminoSwitchboardPull` | oracle + Kamino reserve | Kamino LP via Switchboard |
| `DriftPythPull` | oracle + Drift spot market | Drift integration |
| `SolendPythPull` | oracle + Solend reserve | Solend integration |
| `FixedKamino` | Kamino reserve only | Fixed + Kamino exchange rate |
| `FixedDrift` | Drift spot market only | Fixed + Drift exchange rate |
| `FixedJuplend` | JupLend state only | Fixed + JupLend exchange rate |
| `JuplendPythPull` | oracle + JupLend state | JupLend + Pyth |

Oracle accounts are passed in the remaining accounts of the liquidation instruction. The ordering follows: `[bank, oracle_1, ..., oracle_N]` per position, with `N` determined by `OracleSetup`.

**For indexing**: To get the price at liquidation, parse the oracle accounts from the remaining accounts. The `OracleSetup` type for each bank determines which oracle adapter to use. Alternatively, read the Bank account at that slot and compute the asset value directly.

## Insurance Vault

Each Bank has its own insurance vault (PDA: `[b"insurance_vault", bank_key]`).

During liquidation, the insurance fee is an SPL token transfer from `bank_liquidity_vault` (liability bank) to `bank_insurance_vault`. The `bank_insurance_vault` account is a named mutable account in the instruction.

Fractional dust below 1 lamport is tracked in `collected_insurance_fees_outstanding` on the Bank.

## Flash Loan

### Instructions

**`lending_account_start_flashloan`**
- Accounts: `marginfi_account` (mut), `authority` (signer), `ixs_sysvar`
- Data: `[disc(8)] [end_index: u64 LE]` — index of `end_flashloan` in the tx
- Sets `IN_FLASHLOAN_FLAG` on the MarginfiAccount

**`lending_account_end_flashloan`**
- Accounts: `marginfi_account` (mut), `authority` (signer) + remaining (observation banks)
- Data: `[disc(8)]`
- Unsets flag, runs init-level health check

### Liquidation + flash loan pattern

```
ix[0]  start_flashloan(end_index=6)
ix[1]  lending_account_deposit (deposit liability token into liquidator's marginfi account)
ix[2]  lending_account_liquidate (execute liquidation)
ix[3]  lending_account_withdraw (withdraw received collateral)
ix[4]  Jupiter/Raydium swap (collateral → liability token)
ix[5]  lending_account_repay (repay flash loan)
ix[6]  end_flashloan (health check)
```

The flash loan allows capital-efficient liquidation without upfront capital.

## Program Upgrade History (2024-04 to 2026-04)

| Version | Date | Breaking Changes |
|---|---|---|
| **v0.1.6-rc3** | Dec 2024 | Seven-point interest rate curve; `Fixed` oracle type; `start/end_deleverage` instructions; **explicit remaining account counts for liquidators** |
| **v0.1.7-rc3** | Jan 2025 | Drift/Solend integration banks; account freeze; field renames (`kamino_reserve` → `integration_acc_1`) |
| **v0.1.8-rc2** | Mar 2025 | Orders (stop-loss/take-profit); JupLend banks; rate limiting |
| **v0.1.8-rc3** | Mar 2025 | **Banks in remaining accounts must be writable**; reduced CU/heap for liquidation |

**Program ID has not changed.** The program imports its ID from an external crate (`pub use id_crate::ID`).

**For the 2-year backfill**: The remaining accounts ordering changed over time. Before Dec 2024, the `Fixed` oracle type didn't exist. Before Mar 2025, remaining account banks could be read-only. The indexer decoder should be robust to these variations — ideally by reading the Bank account's `OracleSetup` to determine how many remaining accounts to consume per position.

### IDL versioning risk

Each release ships a new IDL. The instruction discriminator for `lendingAccountLiquidate` should be stable (Anchor SHA256-based), but the remaining account interpretation depends on the Bank configuration at each slot. The decoder must be data-driven, not hardcoded.
