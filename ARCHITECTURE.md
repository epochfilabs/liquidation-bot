# Architecture

## System Overview

Two components: a **liquidation bot** (execution) and a **liquidation indexer** (measurement). The indexer feeds intelligence to the bot. Both share decoders and infrastructure.

```
┌─────────────────────────────────────────────────────────────────────┐
│                        DATA SOURCES                                 │
│                                                                     │
│  Triton gRPC (live)          Triton RPC (historical)   Dune (free) │
│  Dragon's Mouth              getSignaturesForAddress    SQL export  │
│  streams account updates     + getTransaction            CSV/JSON   │
│  per-program filtered        per-call credit cost        pre-indexed│
│                                                                     │
│  Old Faithful CAR files      ($0 data, bandwidth cost)              │
│  files.old-faithful.net      ~500GB per epoch (~2 days all Solana)  │
│  faithful-cli rpc locally    your data is ~0.01-0.05% of an epoch  │
└──────────┬──────────────────────┬────────────────────┬──────────────┘
           │                      │                    │
           ▼                      ▼                    ▼
┌─────────────────────────────────────────────────────────────────────┐
│                     INDEXER PIPELINE                                │
│                                                                     │
│  backfill binary ──→ tx_parser ──→ processors ──→ ClickHouse       │
│                                     (4 venues)     (6 tables)       │
│  Scans blocks/txs        Decodes        Enriches      Batch writes  │
│  Filters by program ID   instructions   Jito tips     NDJSON/HTTP   │
│  Handles v0 ALTs         (top-level     Flash loans   ≥10k rows or  │
│                           + CPI inner)  Jupiter swaps  1s flush      │
│                                         Error parsing               │
└──────────┬──────────────────────────────────────────────────────────┘
           │
           ▼
┌─────────────────────────────────────────────────────────────────────┐
│                     ANALYTICS LAYER                                 │
│                                                                     │
│  ClickHouse (local Docker, port 8123)                               │
│  ├── liquidations              (successful events)                  │
│  ├── failed_liquidation_attempts (with error codes)                 │
│  ├── obligations_snapshots     (borrower state at liquidation)      │
│  ├── reserves_snapshots        (reserve config at liquidation)      │
│  ├── tx_metadata               (fees, CU, Jito tips, signers)      │
│  ├── _indexer_progress         (backfill cursor per venue)          │
│  └── 4 materialized views      (daily volume, top liquidators, etc) │
│                                                                     │
│  analysis/sanity.sql           (10 validation queries)              │
│  scripts/query-examples.sql    (interactive exploration)            │
│  Replay/simulation harness     (would-this-have-been-profitable?)   │
└──────────┬──────────────────────────────────────────────────────────┘
           │
           ▼
┌─────────────────────────────────────────────────────────────────────┐
│                     EXECUTION BOT                                   │
│                                                                     │
│  gRPC stream ──→ detect unhealthy ──→ EV filter ──→ Jito bundle    │
│  (Kamino, Jupiter Lend)   position      min size      submission    │
│                                         min bonus                   │
│  Transaction layout:                    loss cap                    │
│    [ATA setup]                          ($50/day)                   │
│    flash_borrow (Kamino/Jupiter)                                    │
│    liquidate (target protocol)                    ┌─→ Supabase      │
│    [Jupiter swap if needed]                       │   (audit trail) │
│    flash_repay                                    │                 │
│                                          log ─────┤                 │
│                                                   └─→ ClickHouse   │
│                                                       (analytics)  │
└─────────────────────────────────────────────────────────────────────┘
```

## Data Source Economics

Historical data is needed for the indexer. Every path has a cost:

| Source | Cost Model | Best For |
|---|---|---|
| **Triton RPC** (getSignaturesForAddress + getTransaction) | Per-query + bandwidth against prepaid balance | Targeted lookups: specific signatures, small slot ranges |
| **Triton gRPC** (Dragon's Mouth) | Subscription + usage-based | Live streaming. Historical replay: check with Triton support |
| **Dune Analytics** | Free tier or paid | Validation + signature extraction. Pre-indexed, SQL. Export tx signatures, then fetch raw txs via RPC |

**Key insight:** One Solana epoch (~500GB) contains ~100M transactions. Kamino liquidation data for an entire month is ~287MB (14,355 txs × 20KB). Your useful data is 0.01-0.05% of an epoch. Downloading 500GB to extract 287MB is not practical.

**Recommended approach:**
1. Use Dune (free) to validate counts against Kamino published reports
2. Use Dune to extract liquidation transaction signatures
3. Fetch only those specific transactions via Triton RPC `getTransaction` (~$7 per month of data)
4. Phase 2 live: Triton gRPC subscription (already have this)

## Workspace Structure

```
liquidation-bot/
├── Cargo.toml                      # Workspace root (13 crates)
│
├── src/                            # Original liquidation bot
│   ├── main.rs                     # Event loop: gRPC → detect → execute
│   ├── config/mod.rs               # AppConfig from config.toml + env vars
│   ├── grpc/mod.rs                 # Yellowstone gRPC subscription
│   ├── decoder/mod.rs              # Kamino Obligation discriminator
│   ├── db/mod.rs                   # Supabase audit trail (PostgREST)
│   ├── obligation/
│   │   ├── health.rs               # LTV calculation from raw bytes
│   │   └── positions.rs            # Deposit/borrow position parsing
│   ├── liquidator/
│   │   ├── mod.rs                  # Kamino-native liquidation executor
│   │   ├── executor.rs             # Cross-protocol executor
│   │   ├── flash_loan.rs           # Atomic flash loan tx builder
│   │   ├── instructions.rs         # klend instruction builders
│   │   ├── profitability.rs        # Profit estimation
│   │   └── reserve.rs              # Reserve account parsing
│   └── protocols/
│       ├── mod.rs                  # LendingProtocol trait, ProtocolKind enum
│       ├── kamino.rs               # Kamino adapter
│       ├── jupiter_lend.rs         # Jupiter Lend adapter
│       ├── jupiter_lend_instructions.rs
│       ├── save.rs                 # Save adapter
│       ├── save_instructions.rs
│       ├── marginfi.rs             # MarginFi adapter
│       ├── marginfi_bank.rs        # Bank account parsing
│       └── marginfi_instructions.rs
│
├── decoders/                       # Indexer decoders (10 crates)
│   ├── klend/                      # Kamino: v1+v2 liquidation, flash borrow/repay
│   ├── jupiter-lend-vaults/        # Jupiter Lend: liquidate (full args)
│   ├── jupiter-lend-{liquidity,lending,oracle,flashloan,reward}/  # CPI stubs
│   ├── marginfi-v2/                # MarginFi: liquidate, start/end flashloan
│   └── save/                       # Save: hand-written Borsh tags 12,17,19,20
│                                   #        + Obligation/Reserve account decoders
│
├── crates/                         # Indexer pipeline (3 crates)
│   ├── indexer-core/
│   │   ├── events.rs               # Canonical event types (match ClickHouse schema)
│   │   ├── enrichment.rs           # Jito tips (8 accounts), flash loans (4 protocols),
│   │   │                           # Jupiter swaps, ComputeBudget, error parsing
│   │   ├── writer.rs               # ClickHouse batch writer (NDJSON over HTTP)
│   │   └── progress.rs             # Per-venue backfill progress tracking
│   ├── processors/
│   │   ├── common.rs               # Shared: enrichment, ALT merging, CPI scanning
│   │   ├── kamino.rs               # instruction → LiquidationEvent
│   │   ├── jupiter_lend.rs         # instruction → LiquidationEvent (liquidatee=NULL)
│   │   ├── marginfi.rs             # instruction → LiquidationEvent
│   │   └── save.rs                 # instruction → LiquidationEvent
│   └── backfill/
│       ├── main.rs                 # Entry: spawns writer actor + block fetcher
│       ├── config.rs               # Env-based config (RPC, ClickHouse, slot range)
│       ├── block_fetcher.rs        # Slot-by-slot with skip/error handling
│       └── tx_parser.rs            # RPC JSON → TxContext (v0 ALT support)
│
├── idls/                           # Pinned IDL JSON files (8 total)
├── schema/                         # ClickHouse DDL + data model docs
├── research/                       # Per-venue research (Phase 0)
├── tests/fixtures/                 # 12 mainnet transaction fixtures
├── analysis/sanity.sql             # 10 ClickHouse validation queries
├── scripts/                        # local-test.sh, clickhouse-shell.sh
│
├── STRATEGY.md                     # Execution plan (3 phases, exit criteria)
├── STATUS.md                       # Current status + findings
├── LOCAL_TESTING.md                # How to run ClickHouse + tests locally
└── docker-compose.yml              # ClickHouse local setup
```

## Protocol Coverage

### Lending (liquidation bot targets)

| Protocol | Program ID | Decoder | Processor | Bot | Permissionless? |
|---|---|---|---|---|---|
| **Kamino Lend** | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` | ✅ v1+v2 | ✅ top+CPI | ✅ full pipeline | Yes |
| **Jupiter Lend** | `jupr81YtYssSyPt8jbnGuiWon5f6x9TcDEFxYe3Bdzi` | ✅ full args | ✅ top+CPI | Phase 2 shadow | Yes |
| **Save (Solend)** | `So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo` | ✅ Borsh tags | ✅ top+CPI | Phase 2 evaluate | Yes |
| **MarginFi v2** | `MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA` | ✅ Anchor disc | ✅ top+CPI | Phase 2 evaluate | Yes |
| **Loopscale** | Unknown | ❌ | ❌ | Not yet | Unclear |

### Perps (keeper targets)

| Protocol | Status | Liquidation Access | Action |
|---|---|---|---|
| **Drift** | Down (April 2026 exploit, recovery in progress) | Open keeper system (0.75-3% reward) | Prepare now, deploy at relaunch |
| **Jupiter Perps** | Active, ~$220M/day volume | Whitelisted keepers only | Not accessible |
| **Pacifica** | Active, #1 by volume ($600M-1.5B/day) | Likely internal (off-chain matching) | Not accessible |
| **Phoenix Perps** | Private beta | Unknown | Too early |
| **DefiTuna** | Active, ~$3.6M TVL | Protocol is the liquidator | Not accessible |
| **Flash Trade** | Active, smaller | Unknown | Investigate |

### Other

| Protocol | Type | Assessment |
|---|---|---|
| **Project 0** | Prime broker (cross-margin) | Reduces liquidations. Not a liquidation venue. |

## On-Chain Account Layouts

All deserialization is **raw byte offset reads** — no CPI crates. This avoids solana-sdk version conflicts. Offsets validated against live mainnet data.

### Kamino Obligation (3344 bytes)

```
+0     discriminator (8B, sha256("account:Obligation")[..8])
+32    lending_market (Pubkey)
+64    owner (Pubkey)
+96    deposits[8] (8 × 136B = 1088B)
+1192  deposited_value_sf (u128)          ← total collateral USD
+1208  borrows[5] (5 × 200B = 1000B)
+2208  borrow_factor_adjusted_debt_sf     ← total debt USD
+2256  unhealthy_borrow_value_sf          ← liquidation threshold
```

All `_sf` fields: u128, 2^60 fixed-point scaling.

### Kamino Reserve (8624 bytes)

```
+128   liquidity.mint (Pubkey)
+160   liquidity.supply_vault (Pubkey)
+192   liquidity.fee_vault (Pubkey)
+224   liquidity.available_amount (u64)
+248   liquidity.market_price_sf (u128)
+2560  collateral.mint (Pubkey)
+4873  config.liquidation_threshold_pct (u8)
+4874  config.min_liquidation_bonus_bps (u16)
+4876  config.max_liquidation_bonus_bps (u16)
```

### Jupiter Lend Position (71 bytes)

```
+0     discriminator (8B, [0xaa, 0xbc, 0x8f, 0xe4, 0x7a, 0x40, 0xf7, 0xd0])
+8     vault_id (u16)
+14    position_mint (Pubkey)
+46    is_supply_only (u8)
+47    tick (i32)              ← debt/collateral ratio as 1.0015^tick
+55    supply_amount (u64)
+63    dust_debt_amount (u64)
```

### MarginFi Account (2312 bytes)

```
+0     discriminator (8B)
+8     group (Pubkey)
+40    authority (Pubkey)      ← the liquidatee
+72    balances[16] (16 × 136B)
         +0  active (u8)
         +1  bank_pk (Pubkey)
         +40 asset_shares (i128, i80F48)
         +56 liability_shares (i128, i80F48)
```

### Save Obligation (variable size, no Anchor discriminator)

```
+0     version (u8)
+10    lending_market (Pubkey)
+42    owner (Pubkey)
+74    deposited_value (u128, WAD-scaled = 10^18)
+90    borrowed_value (u128, WAD-scaled)
+122   unhealthy_borrow_value (u128, WAD-scaled)
+138   super_unhealthy_borrow_value (u128, mainnet extension)
+155   deposits_len (u8)
+156   borrows_len (u8)
+157   data_flat start
```

## Transaction Layouts

### Kamino Flash Loan Liquidation (standard)

```
ix[0]  RefreshReserve (repay reserve + oracle)
ix[1]  RefreshReserve (withdraw reserve + oracle)
ix[2]  RefreshObligation
ix[3]  FlashBorrowReserveLiquidity
ix[4]  LiquidateObligationAndRedeemReserveCollateral(V2)
ix[5]  [optional: Jupiter swap if collateral ≠ debt]
ix[6]  FlashRepayReserveLiquidity
```

### Real-world pattern: CPI through wrapper programs

Real liquidators invoke lending programs via CPI, not directly:

```
ix[0]  ComputeBudget::SetComputeUnitLimit
ix[1]  ComputeBudget::SetComputeUnitPrice
ix[2]  WrapperProgram::execute
         ├── inner: Save::RefreshReserve (tag 3)
         ├── inner: Save::RefreshReserve (tag 3)
         ├── inner: Save::RefreshObligation (tag 7)
         └── inner: Save::LiquidateObligation (tag 12)  ← the actual liquidation
```

**This is why the processors scan both top-level AND inner instructions (CPI).** Only scanning top-level misses most real-world liquidation events.

## Testing

### Test Coverage

| Suite | Count | What |
|---|---|---|
| Decoder unit tests | ~50 | Discriminators, arg parsing, account arrangement |
| Fixture round-trip tests | 4 | Real mainnet txs decoded through full pipeline |
| Validation test | 1 | Every field checked for null/invalid across all venues |
| ClickHouse integration | 2 | Write fixtures to DB, query back |
| Indexer core tests | 8 | Jito tips, flash loans, error parsing, progress tracking |
| Original bot tests | ~40 | Health calc, position parsing, profitability, instructions |
| Live validation | 3 | Byte offsets verified against mainnet accounts |
| Cross-validation | 1 | Rust health calc vs TypeScript SDK |
| Surfpool | 3 | In-process BPF execution |

**Total: 122+ tests, 0 failures.**

### Running Tests

```bash
# Full workspace
cargo test --workspace

# Decoder fixture validation (strict field checking)
cargo test -p backfill --test validation -- --nocapture

# ClickHouse integration (requires Docker)
docker compose up -d
./scripts/local-test.sh --schema
CLICKHOUSE_URL=http://localhost:8123 CLICKHOUSE_DATABASE=liquidation_indexer \
CLICKHOUSE_USER=default CLICKHOUSE_PASSWORD=dev \
cargo test -p backfill --test local_integration -- --nocapture
```

## Environment Variables

| Variable | Used By | Purpose |
|---|---|---|
| `SOLANA_RPC_URL` | Bot + backfill | Triton RPC for account fetches, tx submission, historical queries |
| `YELLOWSTONE_GRPC_ENDPOINT` | Bot | gRPC streaming for real-time detection |
| `YELLOWSTONE_GRPC_TOKEN` | Bot | Auth token for gRPC |
| `CLICKHOUSE_URL` | Indexer | ClickHouse HTTP endpoint (default: http://localhost:8123) |
| `CLICKHOUSE_DATABASE` | Indexer | Database name (default: liquidation_indexer) |
| `CLICKHOUSE_USER` | Indexer | ClickHouse user (default: default) |
| `CLICKHOUSE_PASSWORD` | Indexer | ClickHouse password |
| `BACKFILL_START_SLOT` | Backfill | Start slot for historical processing |
| `BACKFILL_END_SLOT` | Backfill | End slot (optional, defaults to current) |
| `SUPABASE_URL` | Bot | Supabase audit trail (optional) |
| `SUPABASE_SERVICE_ROLE_KEY` | Bot | Supabase auth (optional) |
| `LIQUIDATOR_KEYPAIR_PATH` | Bot | Wallet keypair for liquidation execution |

## Known Limitations

1. **`liquidatee` field is NULL** for all venues — requires reading obligation account data at liquidation slot (enrichment step not yet built)
2. **USD price enrichment not implemented** — all `_usd` and `_price` columns are NULL, needed for the replay/simulation harness
3. **`withdraw_amount` is 0** — requires computing pre/post token balance deltas
4. **No real MarginFi or Jupiter Lend liquidation fixtures** — events are extremely rare in recent mainnet; synthetic fixtures used for pipeline validation
5. **Cross-protocol flash loan wrapping is scaffolded but incomplete** in the bot — Kamino reserve lookup by mint is a TODO
6. **No Jito bundle support in the bot** — uses standard RPC submission; Phase 2 adds Jito
7. **No gRPC reconnection** in the bot — dropped connection = blind bot
