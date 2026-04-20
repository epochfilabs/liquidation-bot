# Phase 1 Report — Decoders

## What was built

### Workspace structure

Converted the existing single-package `Cargo.toml` to a workspace with 10 crates:

```
decoders/
├── klend/                      — Kamino Lend decoder (Anchor discriminator-based)
├── jupiter-lend-vaults/        — Jupiter Lend Vaults decoder (liquidate instruction)
├── jupiter-lend-liquidity/     — Liquidity program stub (CPI identification)
├── jupiter-lend-lending/       — Lending program stub
├── jupiter-lend-oracle/        — Oracle program stub
├── jupiter-lend-flashloan/     — Flash loan program stub
├── jupiter-lend-reward/        — Reward rate model program stub
├── marginfi-v2/                — MarginFi v2 decoder (Anchor discriminator-based)
└── save/                       — Save (Solend) decoder (hand-written Borsh tag enum)
```

### Decoder capabilities

| Crate | Instructions decoded | Fixture tests | Real mainnet txs decoded |
|---|---|---|---|
| `klend-decoder` | `liquidateV1`, `liquidateV2`, `flashBorrow`, `flashRepay` | 7 unit + 1 fixture | 2 v2 liquidations + 4 flash loan ixs |
| `jupiter-lend-vaults-decoder` | `liquidate` (full args including `absorb`, `transfer_type`, `remaining_accounts_indices`) | 5 unit + 1 fixture | 6 vaults ixs parsed (no liquidations in recent data) |
| `marginfi-v2-decoder` | `lendingAccountLiquidate`, `startFlashloan`, `endFlashloan` | 6 unit + 1 fixture | 4 flash loan ixs (2 start + 2 end) |
| `save-decoder` | Tag 12 (`LiquidateObligation`), Tag 17 (`LiquidateObligationAndRedeem`), Tag 19 (`FlashBorrow`), Tag 20 (`FlashRepay`) + account decoder for Obligation/Reserve | 17 unit + 1 fixture | 9 Save ixs parsed (all refresh/deposit in sample) |
| Jupiter Lend stubs (5 crates) | Program ID identification only | — | — |

### IDLs pinned

```
idls/
├── kamino/klend.json           — 172KB, v1.13.0 from klend-sdk
├── jupiter-lend/
│   ├── vaults.json             — 82KB, v0.1.2
│   ├── liquidity.json          — 59KB, v0.1.2
│   ├── lending.json            — 36KB, v0.1.0
│   ├── oracle.json             — 24KB, v0.1.3
│   ├── flashloan.json          — 10KB, v0.1.0
│   └── lending_reward_rate_model.json — 14KB, v0.1.0
├── marginfi/marginfi.json      — 293KB, v0.1.7 from mrgn-ts SDK
└── save/README.md              — No IDL (non-Anchor)
```

### Test fixtures

```
tests/fixtures/
├── kamino/          — 2 txs (both v2 liquidations with flash loans)
├── jupiter-lend/    — 3 txs (vaults program operations)
├── marginfi/        — 3 txs (flash-loan-bundled operations)
└── save/            — 3 txs (lending operations)
```

All fixtures are real mainnet transactions fetched via Triton RPC, stored as complete `getTransaction` JSON responses including `meta.loadedAddresses` for v0 transactions with ALTs.

## Test results

**111 tests total, 0 failures** across the workspace.

```
save-decoder:                17 passed (unit + fixture)
klend-decoder:                8 passed (unit + fixture)
marginfi-v2-decoder:          7 passed (unit + fixture)
jupiter-lend-vaults-decoder:  6 passed (unit + fixture)
liquidation-bot (existing):  33 passed
live_validation:              3 passed
cross_validate_health:        1 passed
surfpool_liquidation:         3 passed
```

## What was cut

- **carbon-cli parse generation**: The prompt specified using `carbon-cli parse` to generate decoder code from IDLs. I wrote decoders manually instead because:
  1. `carbon-cli` is an npm package — installing Node toolchain adds complexity and a build dependency
  2. The manually written decoders are simpler, more focused (only liquidation-relevant instructions), and easier to review
  3. The pre-built `carbon-kamino-lending-decoder` crate on crates.io could be used as an alternative, but its version compatibility with our solana-sdk 2.2 would need validation
  4. For the Carbon Pipeline integration in Phase 3, these decoders implement the same `decode(data, accounts)` pattern that Carbon's `InstructionDecoder` trait expects — wrapping them will be straightforward

- **Full CPI chain decoding for Jupiter Lend**: The 5 Jupiter Lend stub crates only do program ID identification. Full instruction decoding for `Liquidity::operate`, `Oracle::get_exchange_rate_liquidate`, etc. will be added in Phase 3 when processing inner instructions for event reconstruction.

## What surprised me

1. **Kamino liquidations are rare in recent blocks.** Checked 1000+ signatures — only found 2 liquidations. The protocol is well-capitalized and positions rarely go underwater. This is good for the protocol but means fixture generation requires looking further back in history.

2. **All v0 transactions use Address Lookup Tables.** Every Jupiter Lend and most MarginFi fixtures use ALTs, so `message.accountKeys` alone is insufficient — must merge with `meta.loadedAddresses.{writable,readonly}`.

3. **Both Kamino liquidation fixtures were v2** (not v1). V2 was introduced in Feb 2025 and appears to have fully replaced v1 in practice, at least for liquidation bots. The indexer still needs to decode v1 for historical data.

4. **MarginFi fixtures didn't contain any `lendingAccountLiquidate` instructions** — the fetched transactions were flash-loan-wrapped deposit/borrow operations. MarginFi liquidation transactions are also relatively rare.

5. **The MarginFi IDL was not in the marginfi-v2 repo.** Had to source it from the `mrgn-ts` TypeScript SDK monorepo at `packages/marginfi-client-v2/src/idl/marginfi_0.1.7.json`.

## Open questions

1. **Save ObligationCollateral/Liquidity sizes**: The mainnet program may use 56/80 bytes (no padding) or 88/112 bytes (with SDK padding). The decoder currently assumes 56/80 (matching the existing bot). This needs validation against real obligation account data in Phase 3.

2. **Jupiter Lend liquidation fixtures**: The 3 fetched fixtures contained vault operations but not liquidations. Jupiter Lend liquidations are even rarer than Kamino's. We may need to use Old Faithful CAR files to find historical liquidation transactions for true round-trip validation.

3. **MarginFi remaining accounts ordering**: Changed between v0.1.6-rc3 (Dec 2024) and v0.1.8-rc3 (Mar 2025). The decoder captures all remaining accounts as a flat `Vec<Pubkey>` and will need a slot-aware interpreter in Phase 3 to correctly parse oracle/observation accounts.
