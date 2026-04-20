# Architecture

Cross-venue flash loan liquidation bot for Solana lending protocols. Rust/Tokio, ~4750 LOC.

## How It Works

```
Yellowstone gRPC (Triton/Helius)
        │
        │  streams account updates for all 4 protocol programs
        ▼
┌─────────────────────────────────────────────────────┐
│  main.rs — event loop                               │
│                                                     │
│  for each account update:                           │
│    1. identify protocol (Kamino/Jupiter/Save/MFi)   │
│    2. check discriminator → is this a position?     │
│    3. evaluate health → is it liquidatable?          │
│    4. parse deposits & borrows                      │
│    5. route to executor                             │
└─────────────┬───────────────────────────────────────┘
              │
              ▼
┌─────────────────────────────────────────────────────┐
│  executor.rs — cross-protocol liquidation           │
│                                                     │
│  Kamino positions:                                  │
│    Kamino flash_borrow → Kamino liquidate →          │
│    Kamino flash_repay (all klend program)            │
│                                                     │
│  Jupiter / Save / MarginFi positions:               │
│    Kamino flash_borrow → target protocol liquidate → │
│    Kamino flash_repay                                │
│                                                     │
│  Before submit: profitability check                 │
│  After submit:  log to Supabase                     │
└─────────────────────────────────────────────────────┘
```

The core insight: **Kamino has the deepest flash loan liquidity on Solana**, so we always borrow from Kamino and liquidate on whichever protocol has the underwater position.

## File Map

### Entry Points

| File | Purpose |
|---|---|
| `src/main.rs` | Tokio async entry. Loads config, inits Supabase, starts gRPC stream, runs event loop. |
| `src/lib.rs` | Re-exports all modules as `liquidation_bot` crate for tests. |

### Configuration

| File | Purpose |
|---|---|
| `src/config/mod.rs` | `AppConfig` struct. Loads from `config.toml`, overrides with env vars (`SOLANA_RPC_URL`, `YELLOWSTONE_GRPC_ENDPOINT`, `SUPABASE_URL`, etc). |
| `.env` | Secrets: RPC URL, gRPC token, Supabase key. Never committed. |
| `config.toml` | Non-secret defaults: market address, program ID, profit threshold. |

### Protocol Layer (`src/protocols/`)

This is the multi-venue abstraction. Each protocol implements the `LendingProtocol` trait.

| File | Purpose |
|---|---|
| `mod.rs` | `LendingProtocol` trait definition, `ProtocolKind` enum, program ID registry, `identify_protocol()` dispatcher. |
| `kamino.rs` | Wraps `obligation/health.rs` and `obligation/positions.rs` into the trait. Account size: 3344 bytes. |
| `jupiter_lend.rs` | Parses 71-byte NFT-based positions. Tick-based LTV: `(1.0015)^tick`. Also parses VaultConfig (219B) and VaultState (127B). |
| `jupiter_lend_instructions.rs` | Instruction builders: `flashloan_borrow`, `flashloan_payback` (zero fee), `liquidate` (26 accounts). |
| `save.rs` | Parses SPL token-lending style Obligation (variable size, WAD-scaled). No Anchor discriminator — uses version byte. |
| `save_instructions.rs` | Instruction builders: tag 19 (flash borrow), tag 20 (flash repay), tag 15 (liquidate + redeem). |
| `marginfi.rs` | Parses MarginfiAccount (2312B). 16 Balance slots with i80F48 share values. Health requires Bank lookups. |
| `marginfi_bank.rs` | Parses Bank accounts (1864B). Extracts share values, maintenance weights, vault addresses. |
| `marginfi_instructions.rs` | Instruction builders: `lending_account_liquidate`, `start_flashloan`, `end_flashloan`. |

**Adding a new protocol:** Create `<name>.rs` (implement `LendingProtocol`) and `<name>_instructions.rs`, add to `mod.rs` registry, add program ID to `protocol_program_ids()`.

### Kamino-Specific Deserialization (`src/obligation/`)

These predate the protocol abstraction layer and are used directly by the Kamino adapter and the legacy executor.

| File | Purpose |
|---|---|
| `health.rs` | Reads 4 u128 scaled-fraction fields from raw obligation bytes. LTV = `bf_adjusted_debt / deposited_value`. Liquidatable when `debt >= unhealthy_borrow`. |
| `positions.rs` | Parses 8 deposit slots (136B each) and 5 borrow slots (200B each). Extracts reserve pubkeys, amounts, market values. |

### Liquidation Execution (`src/liquidator/`)

| File | Purpose |
|---|---|
| `mod.rs` | `execute_liquidation()` — Kamino-native path with full profitability check and Supabase logging. |
| `executor.rs` | `execute_cross_protocol()` — routes to protocol-specific liquidation builders. Kamino delegated to `mod.rs`, others build standalone liquidate instructions. |
| `flash_loan.rs` | `build_liquidation_tx()` — fetches all on-chain state (obligation, reserves, market), calculates repay amount, builds ATAs, constructs the 3-ix atomic tx (flash_borrow + liquidate + flash_repay). |
| `instructions.rs` | Raw Anchor instruction builders for klend: `flash_borrow_reserve_liquidity`, `flash_repay_reserve_liquidity`, `liquidate_obligation_and_redeem_reserve_collateral`. Discriminators verified against on-chain program. |
| `reserve.rs` | Parses Kamino Reserve (8624B) and LendingMarket accounts. Extracts liquidity vaults, fee receivers, oracle config, liquidation parameters. |
| `profitability.rs` | `estimate_profit()` — calculates expected profit from liquidation bonus minus flash loan fee minus protocol fee minus tx cost. Returns `ProfitEstimate` with `is_profitable` flag. |

### Infrastructure

| File | Purpose |
|---|---|
| `src/grpc/mod.rs` | Yellowstone gRPC subscription. Spawns async task, filters by all protocol program owners, sends `PositionUpdate` to mpsc channel. |
| `src/decoder/mod.rs` | Anchor discriminator computation for Kamino Obligation accounts. SHA256-based. |
| `src/db/mod.rs` | Supabase PostgREST client. Inserts/updates `liquidation_attempts` table. Fire-and-forget — never blocks the liquidation path. |

## On-Chain Account Layouts

All deserialization is **raw byte offset reads** — no CPI crates. This avoids solana-sdk version conflicts. Offsets were validated against live mainnet data.

### Why No CPI Crates

The `kamino-lend` crate pins solana-sdk 1.x. `yellowstone-grpc-client` needs 2.x. Using raw bytes with validated offsets lets us stay on modern solana-sdk without dependency hell.

### Key Offsets (Kamino Obligation, 3344 bytes)

```
+0     discriminator (8B, sha256("account:Obligation")[..8])
+8     tag (u64)
+16    last_update (16B)
+32    lending_market (Pubkey)
+64    owner (Pubkey)
+96    deposits[8] (8 × 136B = 1088B)
         +0  deposit_reserve (Pubkey)
         +32 deposited_amount (u64)
         +40 market_value_sf (u128)
+1184  lowest_reserve_deposit_liquidation_ltv (u64)
+1192  deposited_value_sf (u128)          ← total collateral USD value
+1208  borrows[5] (5 × 200B = 1000B)
         +0  borrow_reserve (Pubkey)
         +88 borrowed_amount_sf (u128)
         +104 market_value_sf (u128)
+2208  borrow_factor_adjusted_debt_sf (u128) ← total debt USD value
+2224  borrowed_assets_market_value_sf (u128)
+2240  allowed_borrow_value_sf (u128)
+2256  unhealthy_borrow_value_sf (u128)   ← liquidation threshold
+2277  elevation_group (u8)
```

All `_sf` fields are u128 with 2^60 fixed-point scaling. On Solana BPF, u128 alignment is **8 bytes** (not 16).

### Key Offsets (Kamino Reserve, 8624 bytes)

```
+128   liquidity.mint (Pubkey)
+160   liquidity.supply_vault (Pubkey)
+192   liquidity.fee_vault (Pubkey)
+224   liquidity.available_amount (u64)
+248   liquidity.market_price_sf (u128)
+408   liquidity.token_program (Pubkey)
+2560  collateral.mint (Pubkey)
+2600  collateral.supply_vault (Pubkey)
+4873  config.liquidation_threshold_pct (u8)
+4874  config.min_liquidation_bonus_bps (u16)
+4876  config.max_liquidation_bonus_bps (u16)
+5104  config.token_info.scope_configuration (48B)
+5152  config.token_info.switchboard_configuration (64B)
+5216  config.token_info.pyth_configuration.price (Pubkey)
```

### Key Offsets (Jupiter Lend Position, 71 bytes)

```
+0     discriminator (8B, [0xaa, 0xbc, 0x8f, 0xe4, 0x7a, 0x40, 0xf7, 0xd0])
+8     vault_id (u16)
+10    nft_id (u32)
+14    position_mint (Pubkey)
+46    is_supply_only (u8)
+47    tick (i32)           ← debt/collateral ratio as 1.0015^tick
+55    supply_amount (u64)
+63    dust_debt_amount (u64)
```

### Key Offsets (MarginFi Account, 2312 bytes)

```
+0     discriminator (8B, [0x43, 0xb2, 0x82, 0x6d, 0x7e, 0x72, 0x1c, 0x2a])
+8     group (Pubkey)
+40    authority (Pubkey)
+72    balances[16] (16 × 136B)
         +0  active (u8)
         +1  bank_pk (Pubkey)
         +40 asset_shares (i128, i80F48)
         +56 liability_shares (i128, i80F48)
```

### Key Offsets (MarginFi Bank, 1864 bytes)

```
+8     mint (Pubkey)
+80    asset_share_value (i128, i80F48)
+96    liability_share_value (i128, i80F48)
+112   liquidity_vault (Pubkey)
+146   insurance_vault (Pubkey)
+312   config.maint_asset_weight (i128, i80F48)
+344   config.maint_liability_weight (i128, i80F48)
```

## Database Schema

Supabase project `qanpmhczxnxgssalblpx`. Migration in `migrations/001_create_liquidations.sql`.

### Table: `liquidation_attempts`

Tracks every liquidation the bot considers — including skipped (unprofitable) and failed ones.

| Column | Type | Purpose |
|---|---|---|
| `id` | UUID | Primary key |
| `status` | TEXT | `pending` → `submitted` → `confirmed` / `failed` / `skipped` |
| `obligation_pubkey` | TEXT | Target position |
| `ltv_at_detection` | FLOAT | Health at time of detection |
| `repay_amount` | BIGINT | Debt amount repaid |
| `estimated_net_profit_usd` | FLOAT | Pre-trade estimate |
| `actual_profit_usd` | FLOAT | Post-trade actual (TODO) |
| `tx_signature` | TEXT | Solana tx sig on success |
| `error_message` | TEXT | Error detail on failure |

### Views

| View | Purpose |
|---|---|
| `liquidation_roi_summary` | Cumulative totals: success rate, total profit, total fees |
| `liquidation_daily_pnl` | Daily breakdown |
| `liquidation_by_obligation` | Per-position stats |

## Transaction Layout

### Kamino-Native Liquidation

```
ix[0]  RefreshReserve (repay reserve + oracle accounts)
ix[1]  RefreshReserve (withdraw reserve + oracle accounts)
ix[2]  RefreshObligation (obligation + all deposit/borrow reserves)
ix[3]  FlashBorrowReserveLiquidity (borrow repay token from Kamino)
ix[4]  LiquidateObligationAndRedeemReserveCollateral (repay debt, seize collateral)
ix[5]  FlashRepayReserveLiquidity (return borrowed tokens + fee)
```

Note: The Surfpool integration tests proved this layout works — flash borrow executed successfully in the in-process BPF VM. The refresh instructions are required by klend's instruction introspection check (error 6051 without them).

### Cross-Protocol Liquidation (Jupiter/Save/MarginFi)

```
ix[0]  Kamino FlashBorrowReserveLiquidity
ix[1]  <target protocol> Liquidate instruction
ix[2]  Kamino FlashRepayReserveLiquidity
```

Status: The flash loan wrapping for cross-protocol is scaffolded in `executor.rs` but the Kamino reserve lookup by mint is a TODO.

## Testing

### Test Structure

| Location | Framework | What It Tests |
|---|---|---|
| `src/**/*.rs` (inline `#[cfg(test)]`) | cargo test | Unit tests for health calc, position parsing, instruction encoding, profitability, discriminators |
| `tests/live_validation.rs` | cargo test | Byte offset validation against live mainnet accounts (97K obligations, 55 reserves) |
| `tests/cross_validate_health.rs` | cargo test | Compares Rust health calc against TypeScript SDK reference values from fixture files |
| `tests/surfpool_liquidation.rs` | cargo test | Fetches real accounts, forges underwater positions in memory, builds full liquidation tx |
| `tests/surfpool-tests/` | Separate crate | In-process BPF execution via SurfnetSvm (surfpool-core). Loads klend program, submits tx, verifies execution. |

### Running Tests

```bash
# Unit + live validation (needs SOLANA_RPC_URL)
cargo test

# Surfpool in-process BPF tests (separate crate, solana 3.x)
cd tests/surfpool-tests && cargo test -- --nocapture --test-threads=1

# Generate TS SDK fixtures for cross-validation
cd ts-reference && npm install && RPC_URL=<rpc> npx ts-node snapshot-obligations.ts
```

### Why Surfpool Tests Are a Separate Crate

`surfpool-core` depends on solana 3.x. The main bot uses solana-sdk 2.x. These cannot coexist in the same Cargo.lock. The `tests/surfpool-tests/` crate has its own `Cargo.toml` and `Cargo.lock`, using the solana 3.x split crates directly. It duplicates the minimal byte-manipulation logic (forging, health checks) without importing the main crate.

## Environment Variables

| Variable | Required | Purpose |
|---|---|---|
| `SOLANA_RPC_URL` | Yes | Mainnet RPC for account fetches and tx submission |
| `YELLOWSTONE_GRPC_ENDPOINT` | Yes | gRPC streaming endpoint for real-time account updates |
| `YELLOWSTONE_GRPC_TOKEN` | Yes | Auth token for gRPC |
| `SUPABASE_URL` | No | Supabase project URL for audit trail |
| `SUPABASE_SERVICE_ROLE_KEY` | No | Supabase service role key for inserts |
| `LIQUIDATOR_KEYPAIR_PATH` | For execution | Path to JSON keypair file for the liquidator wallet |
| `MIN_PROFIT_LAMPORTS` | No | Minimum profit threshold (default: 10000) |

## Known Limitations

See `AUDIT.md` for the full technical audit. Key items:

1. **No Jito bundle support** — uses standard RPC submission, will lose to MEV bots
2. **No refresh instructions** in the Kamino tx — will fail on-chain (proven by Surfpool test)
3. **Jupiter and MarginFi `is_liquidatable` always false** — detection scaffolded but incomplete
4. **No oracle staleness checking** — may act on stale prices
5. **No gRPC reconnection** — dropped connection = blind bot
6. **Cross-protocol flash loan wrapping is a TODO** — requires Kamino reserve lookup by mint

## Protocol Program IDs

| Protocol | Program ID | Status |
|---|---|---|
| Kamino Lend | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` | Full pipeline (detect + execute) |
| Jupiter Lend Vaults | `jupr81YtYssSyPt8jbnGuiWon5f6x9TcDEFxYe3Bdzi` | Detect + instruction builders |
| Jupiter Lend Flash Loan | `jupgfSgfuAXv4B6R2Uxu85Z1qdzgju79s6MfZekN6XS` | Instruction builders only |
| Save (Solend) | `SLendK7ySfcEzyaFqy93gDnD3RtrpXJcnRwb6zFHJSh` | Detect + instruction builders |
| MarginFi v2 | `MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA` | Detect + Bank parsing + instruction builders |
| Loopscale | Unknown | Not integrated (closed source, no flash loans) |
