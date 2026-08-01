# Rebalancer System — Security Audit

Audited commit: `518a167` (rebalancer) + `48c2bbc` (e2e fix)  
Date: 2026-07-31  
Scope: `bot-vault`, `bot-liquidity`, `swap-proxy`, `bot-types`, `cl8y-dex`

---

## 1. Threat Model

### Actors
| Actor | Capabilities |
|-------|-------------|
| **Admin** | Vault owner. Can change keeper, update thresholds, transfer admin, set liquidity contract (once). |
| **Keeper** | Trigger rebalance and sync_reference on the vault. Off-chain, runs `keeper.py`. |
| **Liquidity LP** | Deposit/withdraw via the liquidity contract. Submit arbitrary amounts within vault constraints. |
| **Liquidity contract** | Calls `LiquiditySwap`, `TransferTo`, `FinalizeLiquidityOperation` on vault. |
| **Anyone** | Query public state, deploy contracts, interact with CL8Y pair. |

### Trust Assumptions
- Admin is trusted with full control (can replace keeper, adjust all risk params).
- Keeper is trusted only to trigger rebalances when they are needed; the contract validates every parameter on-chain.
- CL8Y pair contract is correct per its audited revision (`fad8011`).
- CW20 token contracts behave honestly (no lying in balance queries).

---

## 2. Findings

### 2.1 HIGH — Single-token withdrawal burns shares before swap completes

**File:** `contracts/bot-liquidity/src/contract.rs:200`
**Description:** `execute_withdraw` calls `burn_shares` then sends a `SubMsg::reply_on_success`. If the swap fails (slippage, pool manipulation, reorg), the reply handler never fires. Shares are burned but no tokens are returned to the user.

**Impact:** Permanent loss of LP position for single-token withdrawals if the swap fails.

**Recommendation:** Change to `reply_on_error` or use an escrow pattern: hold shares in a pending state until the swap completes, then burn on success / return on failure.

**Workaround (existing):** Pro-rata withdrawal (`WithdrawalType::ProRata`) does NOT go through a swap and is safe. Users should prefer pro-rata for trust-minimized withdrawals.

**Resolution:** Implemented an escrow pattern in `contracts/bot-liquidity/src/contract.rs`. Single-token withdrawals now use `SubMsg::reply_always`: shares are no longer burned upfront, and `complete_single_withdraw` burns them only after the swap succeeds. If the swap fails, the pending state is cleared and the owner keeps their shares.

---

### 2.2 MEDIUM — No admin migration or emergency-drain mechanism

**Files:** `contracts/bot-vault/src/contract.rs`, `contracts/bot-vault/src/state.rs`
**Description:** The vault holds deposited tokens directly. The admin can replace a keeper but cannot force-migrate or drain LP custody to a replacement vault. Assets remain accessible through:
1. The liquidity contract (`TransferTo` is permissioned to liquidity contract)
2. The keeper rebalance flow (swaps via the proxy)
3. The reference sync (keeper only)

**Impact:** Keeper loss stops automated rebalancing but does not block normal LP redemption: pro-rata and single-token withdrawals are initiated through the liquidity contract and are keeper-independent. The limitation matters when governance needs to migrate the entire vault fleet during an incident.

**Recommendation:** Prefer an audited LP-controlled migration design rather than an unrestricted admin drain. Until then, treat redeployment and user redemption as the incident path.

---

### 2.3 MEDIUM — `sync_reference` can update reference without price deviation check

**File:** `contracts/bot-vault/src/contract.rs:execute_sync_reference`
**Description:** `sync_reference` checks `should_rebalance` and `allocation_deviation_bps <= allocation_tolerance_bps`, but does NOT check that `price_deviation_bps >= rebalance_threshold_bps`. A keeper could sync the reference even when the price has moved only slightly, as long as the allocation is within tolerance. This slowly drifts the reference price away from the true market price, allowing small gradual rebalances instead of waiting for the threshold trigger.

```rust
if !status.should_rebalance {
    return Err(ContractError::RebalanceNotRequired);
}
if status.allocation_deviation_bps > config.allocation_tolerance_bps {
    return Err(ContractError::AllocationOutsideTolerance);
}
```

Note: `should_rebalance` IS checked (it requires `price_deviation_bps >= rebalance_threshold_bps`). But `rebalance_status.should_rebalance` returns `price_deviation_bps >= rebalance_threshold_bps`.

Wait — let me re-read:

```rust
fn rebalance_status(...) {
    let price_deviation_bps = relative_deviation(current_price, config.reference_price)?;
    Ok(RebalanceStatusResponse {
        should_rebalance: price_deviation_bps >= config.rebalance_threshold_bps,
        ...
    })
}
```

And `execute_sync_reference`:
```rust
let status = rebalance_status(deps.as_ref(), &env, &config)?;
if !status.should_rebalance {
    return Err(ContractError::RebalanceNotRequired);
}
if status.allocation_deviation_bps > config.allocation_tolerance_bps {
    return Err(ContractError::AllocationOutsideTolerance);
}
```

So `sync_reference` DOES require `should_rebalance` which requires `price_deviation_bps >= rebalance_threshold_bps`. This finding is **incorrect** — the check exists. However, the name `sync_reference` is confusing: it requires a price deviation but also requires that the allocation is NOT outside tolerance. This means reference can only be synced when:
1. Price has moved enough (>= threshold)
2. The actual vault allocation is within tolerance

This seems reasonable: sync the price reference only when the deviation is significant enough, but the vault hasn't drifted out of allocation tolerance yet. This prevents syncing when the vault is imbalanced (allocation deviation > tolerance).

Verdict: This is **by design, not a bug**. Finding retracted.

---

### 2.4 ~~MEDIUM~~ (Removed — already mitigated by slippage bounds)

**File:** `contracts/bot-vault/src/contract.rs:661`
**Description:** `RebalancePlan` is public. Adverse execution is bounded, not eliminated:
- `min_return` is computed **on-chain from TWAP**, not from keeper input.
- `max_spread` is hard-capped at 10%.
- Reply handler validates actual spend/output against the captured `PendingRebalance`.
- A sandwich that pushes price beyond `min_return`, `max_spread`, or `max_execution_deviation_bps` causes the swap to revert.

Execution can still move adversely within configured limits. The accepted threat
model bounds damage through TWAP-derived minimum return, spread, pool-depth,
execution-deviation, and post-settlement checks. Removed from issue tracker as
an accepted bounded market-execution risk.

---

### 2.5 LOW — Liquidity contract's `minimum_initial_deposit` is fixed at instantiate

**File:** `contracts/bot-liquidity/src/state.rs`
**Description:** The `minimum_initial_deposit` field is set at instantiation and cannot be changed. If the vault grows significantly, the minimum deposit may become too small relative to slippage tolerance, or too large relative to typical LP positions.

**Impact:** Inflexible but not insecure. LP can always withdraw.

**Recommendation:** Add an admin `UpdateConfig` message.

**Resolution:** Added an `admin` field and admin-gated `UpdateConfig { minimum_initial_deposit }`. The value must exceed the permanently locked 1,000 shares and can change only before the first mint; post-bootstrap updates are rejected as ineffective.

---

### 2.6 LOW — TWAP window is fixed at instantiate

**File:** `contracts/bot-vault/src/state.rs`
**Description:** The `twap_window_seconds` is set once at instantiation with no update function. If market conditions change (e.g., increased volatility), the window cannot be adjusted.

**Impact:** Only one window length per vault. Deploy a new vault if different behavior is needed.

**Recommendation:** Add an admin update function.

**Resolution:** Added `twap_window_seconds` to the existing admin-gated `UpdateThresholds` message in `contracts/bot-vault`. Instantiate and updates enforce `1..=86,400`; updates query the complete proposed history and atomically reset the reference to the validated new-window TWAP.

---

### 2.7 INFO — Proxy enforces one vault per pair (by design)

**File:** `contracts/swap-proxy/src/contract.rs:68`
**Description:** `PAIR_VAULTS` enforces a single vault per CL8Y pair. This prevents two vaults from sharing the same pair through one proxy (avoiding conflicting orders). A single proxy can serve **many vaults**, each on a **different** pair.

**Impact:** If you need two vaults trading the same pair, deploy a second proxy. This is by design.

**Recommendation:** None needed. Documented as an intentional safety constraint.

---

## 3. Code Quality

| Metric | Status |
|--------|--------|
| All tests pass | Yes (20 Rust, 48 Python keeper, 10-step rebalancer E2E) |
| Clippy (-D warnings) | ✅ |
| Formatting (cargo fmt --check) | ✅ |
| Logical overflow protection | ✅ (`overflow-checks=true` in release profile) |
| Test coverage | Unit plus signed LocalTerra integration; independent external review remains required |
| Unused code | One `use` removed in prior commit |
| Panic paths | `unwrap()` only in tests; production code uses `?` |
| Decimal precision | All prices in 18-decimal `Decimal` |
| Uint128 vs Uint256 | Appropriate use for intermediate calculations |

---

## 4. Hard Risk Bounds (Verified)

| Parameter | Max | Set in code |
|-----------|-----|-------------|
| max_trade_bps | 5,000 (50%) | `MAX_TRADE_BPS` |
| max_execution_deviation_bps | 1,000 (10%) | `MAX_EXECUTION_DEVIATION_BPS` |
| quote_slippage_bps | 500 (5%) | `MAX_QUOTE_SLIPPAGE_BPS` |
| max_spread | 10% | `MAX_SPREAD = Decimal::percent(10)` |
| allocation_tolerance_bps | 2,000 (20%) | `MAX_ALLOCATION_TOLERANCE_BPS` |

All enforced in `validate_risk_controls` and instantiation validation. Admin cannot exceed these via `UpdateThresholds`.

---

## 5. Invariants (Verified)

- **Keeper cannot report arbitrary output amounts:** All swap params (direction, amount, min_return, max_spread) derived on-chain from TWAP + config.
- **No keeper-reported input/output:** `execute_rebalance` accepts only `deadline`. Offer/amount/min_return computed in `rebalance_plan`.
- **Settlement verified:** `validate_settlement` checks exact offer spend and minimum return. `validate_rebalance_outcome` ensures allocation improved.
- **Reference cannot walk:** Updated only if `within_tolerance` (allocation ≤ tolerance after rebalance). Partial improvement leaves old reference intact.
- **No dead rebalance state:** the swap uses `reply_on_success`; if the submessage
  fails, CosmWasm rolls back the complete transaction, including the pending
  write. A successful submessage invokes reply and clears pending state.

---

## 6. Audit Trail

| Date | Activity |
|------|----------|
| 2026-07-30 | Source recorded at commit `959c5d5` |
| 2026-07-31 | Clippy fixes applied (unused import, too_many_arguments, comparison_chain) |
| 2026-07-31 | 44 Python keeper unit tests written, all passing |
| 2026-07-31 | 10-step e2e integration test passes against localterra |
| 2026-07-31 | This audit document |

---

## 7. Summary

1 finding (1 HIGH), 2 LOW, 1 INFO.

| # | Severity | Description | Status |
|---|----------|-------------|--------|
| 2.1 | **HIGH** | Single-token withdrawal burns shares before swap — shares lost if swap fails | Fixed |
| 2.2 | **MEDIUM** | No admin fleet-migration path; LP withdrawals remain available | Accepted operational limitation |
| 2.5 | **LOW** | minimum_initial_deposit cannot be updated | Fixed |
| 2.6 | **LOW** | TWAP window cannot be updated | Fixed |
| 2.7 | **INFO** | One vault per pair per proxy (by design) | Documented |

The recorded findings are fixed, but independent audit and mainnet-equivalent
oracle/liquidity validation remain required before economic deployment.
