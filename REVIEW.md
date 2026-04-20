# Repository Review: Liquidation Bot

## Overview

A Solana DeFi liquidation bot that monitors positions across 4 lending protocols (Kamino, Save/Solend, MarginFi, Jupiter Lend) via Yellowstone gRPC streaming, detects underwater positions, and executes flash-loan-based liquidations. Audit trail is persisted to Supabase.

## Architecture

Clean modular design with a well-defined `LendingProtocol` trait. The key architectural decision (raw byte deserialization instead of protocol crate dependencies) is sound given the solana-sdk version conflicts between protocols. The code is well-documented with struct layout comments and validated offsets.

## Strengths

1. **Solid test coverage** — 37 tests including unit tests, cross-validation against the TS SDK, live mainnet validation, and surfpool integration tests
2. **Protocol trait abstraction** — Clean extensibility for adding new protocols
3. **Careful on-chain math** — Uses integer comparisons for liquidation checks (matching on-chain logic) rather than floating-point
4. **Graceful degradation** — Supabase is optional, errors during DB writes don't block liquidation execution
5. **Good documentation** — Struct layouts, offsets, and design rationale are well-commented

## Issues and Risks

### High Priority

1. **`min_receive_amount = 0`** (`src/liquidator/flash_loan.rs:121`)
   This accepts any collateral amount, making the bot vulnerable to sandwich attacks. Should compute a minimum based on expected collateral value minus slippage tolerance.

2. **Cross-protocol flash loans not wired up** (`src/liquidator/executor.rs:117-119`)
   The `execute_cross_protocol` path submits liquidation instructions *without* a flash loan, requiring the liquidator wallet to already hold tokens. The TODO at line 117 is load-bearing.

3. **MarginFi health evaluation always returns `is_liquidatable: false`** (`src/protocols/marginfi.rs:144`)
   Without Bank lookups, MarginFi positions will never trigger liquidation. This protocol is essentially non-functional for detection.

4. **Jupiter Lend detection always returns `is_liquidatable: false`** (`src/protocols/jupiter_lend.rs:179`)
   Same issue: requires VaultState comparison that isn't implemented.

5. **Blocking RPC calls** (`src/liquidator/mod.rs:38-63`, `src/liquidator/flash_loan.rs:57-101`)
   Uses synchronous `RpcClient` inside an async context. `rpc.get_account()` is blocking and will stall the Tokio runtime. Should use `solana_client::nonblocking::rpc_client::RpcClient`.

### Medium Priority

6. **No reconnection logic** (`src/grpc/mod.rs:28`)
   If the gRPC stream drops, the subscription task terminates and the error is logged but the bot becomes deaf. Needs retry/reconnect with backoff.

7. **Duplicate position types**
   `obligation::positions::ObligationPositions` vs `protocols::Positions`, `obligation::health::HealthResult` vs `protocols::HealthResult`. The Kamino protocol adapter converts between them, but `src/liquidator/mod.rs` still uses the old types directly.

8. **Kamino-native re-fetches obligation** (`src/liquidator/executor.rs:190-198`)
   `execute_kamino_native` re-fetches the obligation from RPC even though the data was already available via the gRPC stream. This adds latency in a time-sensitive path.

9. **Save close factor bug** (`src/liquidator/executor.rs:352`)
   `repay_amount.min(repay_amount / 2)` is always `repay_amount / 2`. The intent is to cap at 50%, but the `.min()` is redundant/misleading. Should be `repay_amount / 2` directly or `repay_amount * close_factor / 100`.

10. **MarginFi oracle accounts missing** (`src/liquidator/executor.rs:419-425`)
    The remaining accounts passed to the liquidate instruction use bank accounts as placeholders instead of actual oracle accounts. This would fail on-chain.

11. **Hardcoded SOL price** (`src/liquidator/profitability.rs:82`)
    `$150/SOL` is hardcoded for the min-profit threshold conversion. Should use a live price or configure in config.

### Low Priority

12. **49 compiler warnings** — Mostly unused imports/functions. Clean these up.
13. **`HealthResult` `deposited_value_usd` / `borrowed_value_usd`** in MarginFi returns share values, not USD — confusing for downstream consumers.
14. **Config validation** — No validation that `rpc_url` or `grpc_url` are non-empty before attempting connections.

## Summary

The Kamino liquidation path is the most complete and production-ready. Save has structural support but the executor path has a bug. MarginFi and Jupiter Lend have detection + instruction building scaffolding but are non-functional for automated liquidation due to the `is_liquidatable: false` returns and missing oracle/Bank resolution. The most critical fixes before going live are switching to non-blocking RPC and adding a minimum receive amount to prevent MEV extraction.
