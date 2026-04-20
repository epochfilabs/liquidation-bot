# Liquidation Bot Technical Audit

**Date:** 2026-04-10
**Scope:** Solana cross-venue liquidation bot targeting Kamino Lend, Jupiter Lend, Save (Solend), and MarginFi
**Codebase:** `epochfilabs/liquidation-bot`

---

## 1. Position Monitoring & Health Factor Calculation

**[CRITICAL] Kamino health calculation ignores eMode / elevation groups.**
The obligation struct has an `elevation_group` field at byte 2277, which changes the LTV thresholds (up to 95% for LST-SOL pairs). The bot uses the global `unhealthy_borrow_value_sf` which is pre-computed on-chain and does account for elevation groups — so the liquidation *trigger* is correct. However, the profitability estimate uses reserve-level `liquidation_threshold_pct` which does NOT reflect elevation group overrides. This could cause the bot to skip profitable eMode liquidations or miscalculate profit.
**Fix:** Read `elevation_group` from obligation, cross-reference with reserve elevation group configs for accurate bonus/threshold.

**[CRITICAL] Jupiter Lend `is_liquidatable` always returns `false`.**
`src/protocols/jupiter_lend.rs:170` — the health evaluation computes an approximate LTV but hardcodes `is_liquidatable: false` with a comment "requires VaultState comparison." This means no Jupiter Lend positions will ever be liquidated in production.
**Fix:** Fetch `VaultState.topmost_tick` on-chain and compare against the position's tick.

**[CRITICAL] MarginFi `is_liquidatable` always returns `false`.**
`src/protocols/marginfi.rs:117` — health is computed from share ratios without Bank account lookups. The code explicitly sets `is_liquidatable: false` with comment "Never auto-trigger from shares alone."
**Fix:** Fetch Bank accounts for each active balance, apply maintenance weights and oracle prices, compute real health.

**[WARNING] Health evaluation happens only on account update, not on oracle price change.**
`src/main.rs:82-85` — health is evaluated when a position account is updated via gRPC. But positions become liquidatable when oracle prices change, not when the obligation account itself changes. The bot should also subscribe to oracle account updates (Pyth/Switchboard) and re-evaluate affected positions.
**Fix:** Maintain an in-memory obligation cache; subscribe to oracle accounts; on oracle update, re-evaluate all obligations that reference the changed reserve.

**[WARNING] New position creation events not explicitly handled.**
The gRPC subscription filters by program owner, so new accounts created by any protocol will be picked up. However, the bot only evaluates health when accounts are updated — newly created obligations with immediate borrows could be missed until the next update.
**Fix:** This is likely acceptable since new positions start healthy, but worth monitoring.

**[INFO] Single unified monitoring loop — good architecture.**
All protocols share the same gRPC stream and main loop. No redundancy or gaps in coverage.

---

## 2. Liquidation Execution Logic

**[CRITICAL] Cross-protocol flash loan path is incomplete.**
`src/liquidator/executor.rs:117-120` — contains a TODO: "Find matching Kamino reserve for flash loan, build full tx." For Jupiter, Save, and MarginFi, the bot currently submits the liquidation instruction without the flash loan wrapper, meaning the liquidator must already hold the repay token. This defeats the purpose of flash loan liquidation.
**Fix:** Implement `find_kamino_reserve_by_mint()` to locate a Kamino reserve matching the target protocol's debt token, then wrap: `kamino_flash_borrow` → `target_liquidate` → `kamino_flash_repay`.

**[CRITICAL] No `RefreshReserve` / `RefreshObligation` instructions in the Kamino liquidation tx.**
`src/liquidator/flash_loan.rs` builds only 3 instructions (flash_borrow + liquidate + flash_repay). As proven by the Surfpool integration test, Kamino requires `RefreshReserve` (for both repay and withdraw reserves) and `RefreshObligation` before the liquidation instruction. Without these, every Kamino liquidation will fail on-chain with error 6051 (IncorrectInstructionInPosition).
**Fix:** Add refresh instructions to `build_tx()` before the liquidation instruction. Also handle `RefreshObligationFarmsForReserve` when the obligation uses farms.

**[WARNING] Liquidation bonus calculated as midpoint, not actual on-chain value.**
`src/liquidator/profitability.rs:59-63` — uses `(min_bonus + min_bonus) / 2` which is just `min_bonus`. The actual bonus scales linearly between min and max based on how far past the threshold the LTV is. This consistently underestimates profit on deeply underwater positions.
**Fix:** Calculate `actual_bonus = min_bonus + (max_bonus - min_bonus) * (ltv - liquidation_ltv) / (1.0 - liquidation_ltv)` clamped to [min, max].

**[WARNING] Save close factor hardcoded to 50%.**
`src/liquidator/executor.rs:358` — `repay_amount = repay_amount.min(repay_amount / 2)` which always halves the amount. Save's close factor comes from on-chain config and can be different per market.
**Fix:** Fetch `LendingMarket.liquidation_max_debt_close_factor_pct` from the Save lending market account.

**[WARNING] `min_receive_amount` hardcoded to 0 (no slippage protection).**
`src/liquidator/flash_loan.rs:120` — accepts any collateral amount. A sandwich attack could manipulate oracle prices between tx submission and execution.
**Fix:** Calculate expected collateral from `repay_amount * (1 + bonus) / collateral_price` and set `min_receive_amount` to 95-99% of that.

**[INFO] Partial liquidation model handled correctly for Kamino.**
The close factor from `LendingMarket` is respected. However, the bot does not re-evaluate and re-liquidate the same position — it relies on the next gRPC update to trigger again.

---

## 3. Flash Loan Integration

**[CRITICAL] No atomic flash loan wrapping for cross-protocol liquidations.**
As noted above in section 2. For Kamino-native liquidations, the atomic lifecycle is correct: `flash_borrow → liquidate → flash_repay` in a single tx. For other protocols, the flash loan wrapper is missing.

**[WARNING] Flash loan fee defaulted to 0.3% when not set on-chain.**
`src/liquidator/profitability.rs:69` — falls back to 0.003 if `flash_loan_fee_sf == 0`. The zero check is wrong: a fee of 0 could mean genuinely free flash loans (as on Jupiter Lend). Should check if the fee field is explicitly configured vs. uninitialized.
**Fix:** Only use default if the reserve config indicates no oracle/fee setup (check other config fields).

**[INFO] Flash loan repayment failure is atomic.**
The entire Solana transaction reverts if any instruction fails, so flash loan repayment failure correctly unwinds all state. This is inherent to Solana's transaction model.

---

## 4. Jito Bundle Construction

**[CRITICAL] No Jito bundle support implemented.**
The bot uses standard `rpc.send_and_confirm_transaction()` (`src/liquidator/mod.rs:157`). No Jito bundles, no tip transactions, no bundle auction participation. Against any competing liquidation bot using Jito, this bot will lose every race.
**Fix:** Integrate `jito-sdk` or the Jito JSON-RPC bundle endpoint. Build bundles with a dynamic tip based on expected profit (e.g., 50% of expected profit as tip). Use `sendBundle` instead of `sendTransaction`.

**[INFO]** The `.env` file has a `NOZOMI_RPC_URL` from the staging environment — Nozomi is a competing MEV solution. Consider evaluating both Jito and Nozomi for tx submission.

---

## 5. Oracle & Price Feed Handling

**[CRITICAL] No oracle staleness checking.**
The bot reads `market_price_sf` from reserve accounts but never checks when the price was last updated. Stale prices will cause:
- Incorrect health evaluation (detecting positions as liquidatable when they're not, or missing positions that are)
- Failed liquidations on-chain (klend checks oracle freshness during RefreshReserve)

**Fix:** Read `reserve.liquidity.market_price_last_updated_ts` (offset 256, u64) and compare against current time. Skip positions with prices older than 30 seconds.

**[WARNING] Bot doesn't consume oracle prices directly.**
Health evaluation uses pre-computed `deposited_value_sf` and `borrow_factor_adjusted_debt_value_sf` from the obligation account. These values are stale — they were computed the last time someone called `RefreshObligation` on-chain. The bot should calculate fresh values from oracle prices.
**Fix:** For maximum accuracy, subscribe to Pyth oracle accounts and recalculate obligation health using current prices. This is what top liquidation bots do.

**[WARNING] Pyth oracle pubkeys extracted from reserve at compile-time offsets.**
The oracle pubkey extraction at offset 5216 (from Surfpool test) is correct for the current klend version, but if Kamino adds new config fields, the offset will shift.
**Fix:** Validate by checking that the oracle account's owner is the Pyth program.

---

## 6. RPC & Network Configuration

**[WARNING] No gRPC reconnection logic.**
`src/grpc/mod.rs:27-29` — the gRPC subscription runs in a `tokio::spawn`. If the connection drops, the error is logged and the task terminates. The main loop will continue receiving from the mpsc channel but get no new updates — the bot goes blind.
**Fix:** Add reconnection loop with exponential backoff in `run_subscription()`.

**[WARNING] No `simulateTransaction` before submission.**
The bot builds and submits the tx without simulation. This means failed transactions cost SOL in fees and waste Jito tip.
**Fix:** Call `rpc.simulate_transaction(&tx)` before `send_and_confirm_transaction()`. Only submit if simulation succeeds.

**[INFO] Using dedicated Triton RPC — good. No public endpoints in the hot path.**

**[INFO] gRPC streaming via Yellowstone — correct choice for production. WebSocket would be a risk.**

**[INFO] Commitment level is `confirmed` everywhere — appropriate for the liquidation use case (balance of speed vs. finality).**

---

## 7. Error Handling & Resilience

**[WARNING] No duplicate liquidation protection.**
If the same obligation triggers two gRPC updates in quick succession, the bot could submit two liquidation transactions. The second will fail on-chain (position no longer liquidatable), wasting fees.
**Fix:** Maintain a `HashSet<Pubkey>` of recently-submitted positions with a TTL (e.g., 30 seconds). Skip positions already in-flight.

**[WARNING] No circuit breaker.**
If the bot encounters a systematic error (e.g., wrong program ID, all txs failing), it will keep submitting and burning fees indefinitely.
**Fix:** Track failure rate over a rolling window. If >10 consecutive failures, pause for 60 seconds and alert.

**[WARNING] No wallet balance check before liquidation.**
The bot doesn't verify it has SOL for transaction fees before attempting a liquidation.
**Fix:** Check `rpc.get_balance(&liquidator_pubkey)` > minimum threshold before building tx.

**[INFO] Program errors are logged with context — the error messages from klend are Anchor-style with error codes and messages, which flow through to the logs. Good.**

**[INFO] Health evaluation errors are silently skipped (`Err(_) => continue` in `main.rs:84`). This is appropriate — malformed accounts shouldn't crash the bot — but consider logging at debug level.**

---

## 8. Capital & Profit Accounting

**[WARNING] SOL/USD price hardcoded to $150.**
`src/liquidator/profitability.rs:86` — used to convert `min_profit_lamports` to USD. If SOL price changes significantly, the profitability threshold becomes meaningless.
**Fix:** Fetch SOL price from the SOL reserve's `market_price_sf` field, or from a Pyth oracle.

**[WARNING] No post-liquidation accounting.**
`src/liquidator/mod.rs:174` — `actual_profit_usd` is set to `profit.net_profit_usd` (the estimate), not the real on-chain result. The TODO comment acknowledges this.
**Fix:** After confirmation, fetch the tx metadata to get actual token balance changes and compute real profit.

**[WARNING] No swap slippage estimation.**
When collateral token != repay token, the bot needs to swap. There is no Jupiter aggregator integration for swap quotes. The profitability estimate assumes collateral is received at oracle price, which ignores DEX slippage.
**Fix:** Integrate Jupiter swap API to get real-time quotes for the collateral→repay conversion.

---

## 9. Code Quality & Security

**[WARNING] Secrets in `.env` but also in `config.toml`.**
`config.toml` has `grpc_token = "YOUR_TOKEN_HERE"` as a placeholder, but if someone fills it in and commits, it leaks. The `.env` approach is correct; `config.toml` should not have secret fields.
**Fix:** Remove `grpc_token` from `config.toml`. Only load secrets from env vars.

**[INFO] No hardcoded private keys anywhere — good. Keypair loaded from file path in config.**

**[INFO] Protocol abstraction is clean.** The `LendingProtocol` trait makes adding new venues straightforward. Each venue is a separate module with its own instruction builders.

**[INFO] Input validation on account data is present but could be stronger.** Bounds checks exist for all byte offset reads. No panics from out-of-bounds — all use `.try_into()` with error propagation.

**[INFO] No race conditions identified.** Single-threaded main loop with async tasks. Database writes are fire-and-forget. No shared mutable state across tasks.

---

## Priority Fix List

| Priority | Issue | File | Effort |
|---|---|---|---|
| P0 | Add RefreshReserve/Obligation ixs to Kamino tx | `flash_loan.rs` | Medium |
| P0 | Implement Jito bundle submission | New module | High |
| P0 | Complete cross-protocol flash loan wrapping | `executor.rs` | Medium |
| P0 | Fix Jupiter/MarginFi `is_liquidatable` (always false) | `jupiter_lend.rs`, `marginfi.rs` | Medium |
| P1 | Add gRPC reconnection logic | `grpc/mod.rs` | Low |
| P1 | Oracle staleness checking | `health.rs`, new module | Medium |
| P1 | Subscribe to oracle price updates | `grpc/mod.rs`, `main.rs` | Medium |
| P1 | Add simulateTransaction before submission | `mod.rs` | Low |
| P1 | Duplicate liquidation protection | `main.rs` | Low |
| P1 | Dynamic SOL price for profitability | `profitability.rs` | Low |
| P2 | Actual bonus calculation (not midpoint) | `profitability.rs` | Low |
| P2 | min_receive_amount slippage protection | `flash_loan.rs` | Low |
| P2 | Post-liquidation actual profit tracking | `mod.rs` | Medium |
| P2 | Circuit breaker for repeated failures | `main.rs` | Low |
| P2 | Wallet balance pre-check | `mod.rs` | Low |

---

## Test Coverage Summary

| Suite | Count | What it validates |
|---|---|---|
| Unit tests (lib) | 33 | Health calc, position parsing, instruction layouts, profitability, discriminators |
| Cross-validate | 1 | TS SDK fixture comparison |
| Live mainnet | 3 | Byte offsets against 97K obligations, 55 reserves, lending market |
| Integration (in-memory) | 3 | Forge underwater accounts + detection + tx building against mainnet |
| Surfpool (in-process SVM) | 3 | BPF execution: flash borrow executed, liquidation instruction reached |
| **Total** | **76+** | |

---

## Validated Byte Offsets

### Kamino Obligation (3344 bytes)
| Field | Offset | Validated Against |
|---|---|---|
| lending_market | 32 | 97K live accounts |
| owner | 64 | 97K live accounts |
| deposits (8 x 136B) | 96 | 97K live accounts |
| deposited_value_sf | 1192 | 97K live accounts |
| borrows (5 x 200B) | 1208 | 97K live accounts |
| bf_adjusted_debt_sf | 2208 | 97K live accounts |
| unhealthy_borrow_sf | 2256 | 97K live accounts |

### Kamino Reserve (8624 bytes)
| Field | Offset | Validated Against |
|---|---|---|
| liquidity_mint | 128 | 55 live reserves |
| liquidity_supply_vault | 160 | 55 live reserves |
| liquidity_fee_vault | 192 | 55 live reserves |
| collateral_mint | 2560 | 55 live reserves |
| liquidation_threshold_pct | 4873 | 55 live reserves |
| min/max_liquidation_bonus_bps | 4874/4876 | 55 live reserves |
| pyth_oracle | 5216 | Surfpool BPF test |

### Jupiter Lend Position (71 bytes)
| Field | Offset | Validated Against |
|---|---|---|
| vault_id | 8 | 43K live positions |
| tick | 47 | 43K live positions |
| supply_amount | 55 | 43K live positions |
| dust_debt_amount | 63 | 43K live positions |

### MarginFi Account (2312 bytes)
| Field | Offset | Validated Against |
|---|---|---|
| group | 8 | Live discriminator check |
| authority | 40 | Live discriminator check |
| balances (16 x 136B) | 72 | Live discriminator check |
