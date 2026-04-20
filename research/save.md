# Save (Solend) — Liquidation Research

## Program Identity

| Field | Value |
|---|---|
| Program ID | **`So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo`** |
| Framework | SPL token-lending (NOT Anchor) |
| Source repo | [solendprotocol/solana-program-library](https://github.com/solendprotocol/solana-program-library) (branch: `mainnet`) |
| IDL | **None** — no Anchor IDL. Borsh-encoded with u8 instruction tag. |
| Carbon decoder | **Custom `InstructionDecoder` required.** `carbon-cli parse` will not work. |

### Program ID Discrepancy

> **IMPORTANT**: The existing bot code at `src/protocols/save.rs:16` uses `SLendK7ySfcEzyaFqy93gDnD3RtrpXJcnRwb6zFHJSh`. This is **incorrect**. No results exist for this address in any Save/Solend context. The canonical program ID is `So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo`, confirmed by the solend-sdk crate, Solana Explorer, and Save docs.
>
> Additionally, the comment on line 3 says `So1endDq2YkqhipRh3WViPa8hFvz0XP1PV7qidbGAiN` which is also a typo (trailing letters differ).

For the indexer, use: **`So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo`**

## Instruction Encoding

Save uses **SPL token-lending Borsh encoding**: the first byte is a `u8` instruction tag, NOT an 8-byte Anchor discriminator.

### Full LendingInstruction Enum

| Tag | Variant | Arguments |
|---|---|---|
| 0 | InitLendingMarket | owner, quote_currency |
| 1 | SetLendingMarketOwnerAndConfig | new_owner |
| 2 | InitReserve | liquidity_amount, config |
| 3 | RefreshReserve | (none) |
| 4 | DepositReserveLiquidity | liquidity_amount: u64 |
| 5 | RedeemReserveCollateral | collateral_amount: u64 |
| 6 | InitObligation | (none) |
| 7 | RefreshObligation | (none) |
| 8 | DepositObligationCollateral | collateral_amount: u64 |
| 9 | WithdrawObligationCollateral | collateral_amount: u64 |
| 10 | BorrowObligationLiquidity | liquidity_amount: u64 |
| 11 | RepayObligationLiquidity | liquidity_amount: u64 |
| 12 | **LiquidateObligation** | **liquidity_amount: u64** |
| 13 | FlashLoan | amount: u64 |
| 14 | DepositReserveLiquidityAndObligationCollateral | liquidity_amount: u64 |
| 15 | WithdrawObligationCollateralAndRedeemReserveCollateral | collateral_amount: u64 |
| 16 | UpdateReserveConfig | config: ReserveConfig |
| **17** | **LiquidateObligationAndRedeemReserveCollateral** | **liquidity_amount: u64** |
| 18 | RedeemFees | (none) |
| **19** | **FlashBorrowReserveLiquidity** | **liquidity_amount: u64** |
| **20** | **FlashRepayReserveLiquidity** | **liquidity_amount: u64, borrow_instruction_index: u8** |
| 21+ | ForgiveDebt, UpdateMarketMetadata, SetObligationCloseabilityStatus, DonateToReserve | (mainnet-only extensions) |

> **BUG in existing bot**: `save_instructions.rs` uses **tag 15** for `LiquidateObligationAndRedeemReserveCollateral`. Tag 15 is actually `WithdrawObligationCollateralAndRedeemReserveCollateral`. The correct tag is **17**.

## Liquidation Instructions

### LiquidateObligation (tag 12) — simple version

Does NOT redeem collateral. Liquidator receives cTokens.

- **Data**: `[12u8] [liquidity_amount: u64 LE]` — 9 bytes total
- **Accounts (11)**:

| # | Account | Mut | Signer | Description |
|---|---|---|---|---|
| 0 | source_liquidity | **yes** | no | Liquidator's repay token account |
| 1 | destination_collateral | **yes** | no | Liquidator receives cTokens |
| 2 | repay_reserve | **yes** | no | Repay reserve (must be refreshed) |
| 3 | repay_reserve_liquidity_supply | **yes** | no | Reserve's liquidity vault |
| 4 | withdraw_reserve | no | no | Withdraw reserve (must be refreshed) |
| 5 | withdraw_reserve_collateral_supply | **yes** | no | Reserve's collateral vault |
| 6 | obligation | **yes** | no | Obligation (must be refreshed) |
| 7 | lending_market | no | no | Lending market |
| 8 | lending_market_authority | no | no | PDA: `[lending_market.key()]` |
| 9 | user_transfer_authority | no | **yes** | Liquidator signer |
| 10 | token_program | no | no | SPL Token program |

### LiquidateObligationAndRedeemReserveCollateral (tag 17) — preferred

Redeems cTokens to underlying in the same instruction. This is the standard liquidation path.

- **Data**: `[17u8] [liquidity_amount: u64 LE]` — 9 bytes total
- **Accounts (15)**:

| # | Account | Mut | Signer | Description |
|---|---|---|---|---|
| 0 | source_liquidity | **yes** | no | Liquidator's repay token account |
| 1 | destination_collateral | **yes** | no | Liquidator's cToken account (intermediate) |
| 2 | destination_liquidity | **yes** | no | Liquidator receives underlying collateral |
| 3 | repay_reserve | **yes** | no | Repay reserve (refreshed) |
| 4 | repay_reserve_liquidity_supply | **yes** | no | Repay reserve's liquidity vault |
| 5 | withdraw_reserve | **yes** | no | Withdraw reserve (refreshed) |
| 6 | withdraw_reserve_collateral_mint | **yes** | no | cToken mint of withdraw reserve |
| 7 | withdraw_reserve_collateral_supply | **yes** | no | Withdraw reserve's collateral vault |
| 8 | withdraw_reserve_liquidity_supply | **yes** | no | Withdraw reserve's liquidity vault |
| 9 | withdraw_reserve_liquidity_fee_receiver | **yes** | no | Fee receiver for protocol liquidation fee |
| 10 | obligation | **yes** | no | Obligation (refreshed) |
| 11 | lending_market | no | no | Lending market |
| 12 | lending_market_authority | no | no | PDA: `[lending_market.key()]` |
| 13 | user_transfer_authority | no | **yes** | Liquidator signer |
| 14 | token_program | no | no | SPL Token program |

## Liquidation Bonus / Close Factor

### Per-reserve config fields

| Field | Type | Reserve Offset | Description |
|---|---|---|---|
| `liquidation_bonus` | `u8` | ~260 | Percentage (e.g., 5 = 5%). `bonus_rate = 1.0 + liquidation_bonus/100` |
| `liquidation_threshold` | `u8` | ~258 | LTV threshold triggering liquidation |
| `protocol_liquidation_fee` | `u8` | ~329 | % of bonus taken by protocol |

### Close factor

- `LIQUIDATION_CLOSE_FACTOR: u8 = 20` — 20% of obligation debt can be repaid per call
- `LIQUIDATION_CLOSE_AMOUNT: u64 = 2` — below this borrowed_amount_wads, close entire dust position
- `MAX_LIQUIDATABLE_VALUE_AT_ONCE: u64 = 500_000` — caps max quote-currency value per call
- When obligation exceeds `super_unhealthy_borrow_value`, a higher close factor (possibly 100%) may apply

The close factor was temporarily reduced to 1% during the 2022 whale crisis via governance.

### Bonus computation

```
repay_value_usd = liquidity_amount * debt_price
bonus_rate = 1.0 + liquidation_bonus / 100   (e.g., 1.05 for 5%)
collateral_value_usd = repay_value_usd * bonus_rate
collateral_amount = collateral_value_usd / collateral_price
protocol_fee = collateral_bonus_portion * protocol_liquidation_fee / 100
```

Typical values: 5% bonus for major assets, reduced to 2% for SOL during high stress.

### Where the bonus lands

- Liquidator receives `collateral_amount` of the underlying (via cToken redemption for tag 17)
- Protocol fee is sent to `withdraw_reserve_liquidity_fee_receiver`
- **For indexer profit reconstruction**: `liquidator_profit = value(destination_liquidity_post - destination_liquidity_pre) - value(source_liquidity_pre - source_liquidity_post)`

## Oracle Setup

Save uses **both Pyth and Switchboard**, with Pyth as primary and Switchboard as fallback.

### Per-reserve oracle accounts

Each reserve stores (within the `ReserveLiquidity` sub-struct):
- `pyth_oracle_pubkey` — offset 65 from reserve data start (32 bytes)
- `switchboard_oracle_pubkey` — offset 97 from reserve data start (32 bytes)
- Mainnet extension: `extra_oracle_pubkey` in ReserveConfig supporting `Pyth`, `PythPull`, `Switchboard`, `SbOnDemand`

### Price resolution

`get_price()` returns Pyth price if non-zero, otherwise falls back to Switchboard. Validation: at least one oracle must be non-null.

### Critical for indexing

Oracle accounts are **NOT** passed in the liquidation instruction. They are passed to `RefreshReserve` (tag 3), which caches prices in the reserve/obligation state. The liquidation reads cached prices.

**RefreshReserve accounts**: reserve (mut), pyth_oracle, switchboard_oracle, clock

**To get price at liquidation slot**: Read the reserve account's cached market value, or read the oracle account at that slot directly.

## Obligation Layout

### Published SDK (solend-sdk 0.1.0): OBLIGATION_LEN = 1300

| Offset | Field | Type | Size |
|---|---|---|---|
| 0 | version | u8 | 1 |
| 1 | last_update.slot | u64 | 8 |
| 9 | last_update.stale | bool | 1 |
| 10 | lending_market | Pubkey | 32 |
| 42 | owner | Pubkey | 32 |
| 74 | deposited_value | Decimal (u128, WAD-scaled) | 16 |
| 90 | borrowed_value | Decimal (u128, WAD-scaled) | 16 |
| 106 | allowed_borrow_value | Decimal | 16 |
| 122 | unhealthy_borrow_value | Decimal | 16 |
| 138 | padding | [u8; 64] | 64 |
| 202 | deposits_len | u8 | 1 |
| 203 | borrows_len | u8 | 1 |
| 204 | data_flat start | — | variable |

### Mainnet-deployed version (extended, uses padding region)

| Offset | Field | Type | Size |
|---|---|---|---|
| 138 | super_unhealthy_borrow_value | Decimal (u128) | 16 |
| 154 | borrowing_isolated_asset | bool (u8) | 1 |
| 155 | deposits_len | u8 | 1 |
| 156 | borrows_len | u8 | 1 |
| 157 | data_flat start | — | variable |

### ObligationCollateral (per deposit entry)

**SDK says 88 bytes (56 data + 32 padding). Mainnet may use 56 bytes without padding.**

| Offset | Field | Type | Size |
|---|---|---|---|
| 0 | deposit_reserve | Pubkey | 32 |
| 32 | deposited_amount | u64 | 8 |
| 40 | market_value | Decimal (u128) | 16 |
| 56 | padding | [u8; 32] | 32 (may be absent on mainnet) |

### ObligationLiquidity (per borrow entry)

**SDK says 112 bytes (80 data + 32 padding). Mainnet may use 80 bytes without padding.**

| Offset | Field | Type | Size |
|---|---|---|---|
| 0 | borrow_reserve | Pubkey | 32 |
| 32 | cumulative_borrow_rate_wads | Decimal (u128) | 16 |
| 48 | borrowed_amount_wads | Decimal (u128) | 16 |
| 64 | market_value | Decimal (u128) | 16 |
| 80 | padding | [u8; 32] | 32 (may be absent on mainnet) |

> **Critical verification needed**: The indexer must validate ObligationCollateral and ObligationLiquidity sizes against actual on-chain data. If the mainnet program uses 56/80 (no padding), the SDK's 88/112 would cause misalignment. Parse `deposits_len` and `borrows_len` from the correct offset (155/156 for mainnet) and verify the total data length matches.

### Decimal encoding (WAD-scaled)

All value fields (`deposited_value`, `borrowed_value`, etc.) are u128 scaled by `WAD = 10^18`:
```
usd_value = u128_value as f64 / 1_000_000_000_000_000_000.0
```

## Flash Loans

### FlashBorrowReserveLiquidity (tag 19)

- **Data**: `[19u8] [liquidity_amount: u64 LE]` — 9 bytes
- **Accounts (7)**:

| # | Account | Mut | Signer | Description |
|---|---|---|---|---|
| 0 | source_liquidity | **yes** | no | Reserve's liquidity supply vault |
| 1 | destination_liquidity | **yes** | no | Borrower's token account |
| 2 | reserve | **yes** | no | Reserve account |
| 3 | lending_market | no | no | Lending market |
| 4 | lending_market_authority | no | no | PDA |
| 5 | instructions_sysvar | no | no | SysvarInstructions |
| 6 | token_program | no | no | SPL Token |

### FlashRepayReserveLiquidity (tag 20)

- **Data**: `[20u8] [liquidity_amount: u64 LE] [borrow_instruction_index: u8]` — 10 bytes
- **Accounts (9)**:

| # | Account | Mut | Signer | Description |
|---|---|---|---|---|
| 0 | source_liquidity | **yes** | no | Repayer's token account |
| 1 | destination_liquidity | **yes** | no | Reserve's liquidity supply vault |
| 2 | reserve_liquidity_fee_receiver | **yes** | no | Fee receiver |
| 3 | host_fee_receiver | **yes** | no | Host fee receiver (often same as fee receiver) |
| 4 | reserve | **yes** | no | Reserve account |
| 5 | lending_market | no | no | Lending market |
| 6 | user_transfer_authority | no | **yes** | Signer |
| 7 | instructions_sysvar | no | no | SysvarInstructions |
| 8 | token_program | no | no | SPL Token |

### Fee structure

Flash loan fee: per-reserve config at `fees.flash_loan_fee_wad`. Example: 0.3%, split 80% protocol / 20% host. Only one flash borrow per transaction allowed. CPI-based flash loans are blocked.

### Standard liquidation tx layout

```
ix[0]  RefreshReserve (repay reserve, with oracle accounts)
ix[1]  RefreshReserve (withdraw reserve, with oracle accounts)
ix[2]  RefreshObligation
ix[3]  FlashBorrowReserveLiquidity (tag 19)
ix[4]  LiquidateObligationAndRedeemReserveCollateral (tag 17)
ix[5]  FlashRepayReserveLiquidity (tag 20)
```

Save does **NOT** have built-in flash loans like Kamino does — it uses its own separate flash loan mechanism. Jupiter swaps may be included when collateral != debt token.

## Program Upgrade History

Save/Solend launched in 2021. The program has been actively maintained:

- The mainnet branch has instructions beyond published SDK (0.1.0): `ForgiveDebt`, `UpdateMarketMetadata`, `SetObligationCloseabilityStatus`, `DonateToReserve` (tags 21+)
- The Obligation struct gained `super_unhealthy_borrow_value` (offset 138), `borrowing_isolated_asset` (offset 154), and `closeable` fields — stored in what the SDK considers padding
- The old `LiquidateObligation` (tag 12, returns cTokens) still exists but tag 17 (with redeem) is preferred
- The old `FlashLoan` (tag 13, CPI-based) was replaced by FlashBorrow/FlashRepay (tags 19/20, sysvar-based)
- Rate limiter added for market outflows
- Extra oracle support: `PythPull`, `SbOnDemand` added to the extra oracle type

**The instruction tag numbering has remained stable** — tag 17 has always been `LiquidateObligationAndRedeemReserveCollateral`. The argument (u64 liquidity_amount) has not changed. Account ordering has remained stable.

**For the 2-year backfill**: The main risk is the Obligation struct layout — the padding region usage may differ between on-chain versions. The indexer should detect the obligation version from byte 0 and parse accordingly.

## Custom Carbon Decoder Requirements

Since Save has no Anchor IDL, the indexer needs a hand-written `InstructionDecoder`:

1. Read byte 0 as the instruction tag
2. Match on tags 12, 17, 19, 20 for liquidation-related instructions
3. Parse arguments: simple Borsh — tag (u8) + u64 LE, optionally u8 for flash repay
4. Map accounts by position (hardcoded per tag)
5. Handle both tag 12 (11 accounts) and tag 17 (15 accounts)
6. Implement `AccountDecoder` for Obligation, Reserve (SPL token-lending layout, no discriminator)
