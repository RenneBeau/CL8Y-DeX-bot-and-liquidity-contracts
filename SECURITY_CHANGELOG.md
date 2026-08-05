# Security Change Log

This is an internal traceability record, not an audit or proof of security. Commit
timestamps show publication order only; short intervals do not prove whether a
review occurred. No pull-request review evidence or signed commit was identified
for the entries below.

| Commit | Time (UTC+02) | Author | Change and surface | Tests/evidence at commit | Residual risk |
|---|---|---|---|---|---|
| `a8fd57a` | 2026-08-01 11:12:46 | rennebeau | Grid reconciliation/operator and bot-vault rebalance hardening; 16 files, contracts, operator, deployment and docs | Operator and vault tests changed; later baseline CI exists, no independent review identified | Cross-system change was broad; ambiguous grid terminal classification remained |
| `e829c3e` | 2026-08-01 12:09:03 | rennebeau | Persist ambiguous rebalancer broadcast state; keeper implementation/docs/tests | 51 keeper-test lines added; later source CI exists | Operator intervention remains required when no hash was durably recorded |
| `9daa295` | 2026-08-01 12:15:46 | rennebeau | Confirmation depth in both keepers; 8 files | Keeper and operator tests added; later source CI exists | Chain-specific finality assumptions remain operational |
| `cc8b486` | 2026-08-01 12:20:48 | rennebeau | Bot-liquidity bootstrap minimum; contract/errors/docs | Contract tests added later; later source CI exists | Economic minimum remains deployment-specific |
| `8806ee6` | 2026-08-01 12:43:55 | rennebeau | Grid parked-escrow solvency reporting; contract/message/docs | Unit coverage added; later source CI exists | Diagnostic inherited ambiguous active-query classification, corrected in current worktree |
| Uncommitted issue #46 work | 2026-08-01 | OpenCode/user worktree | Bounded pair-first legacy vault inventory drain, empty rescan proof, stale-local cleanup and exact balance synchronization | Local invariant tests and release checks must pass before commit | Depends on unmerged upstream PR #1; superseded by the cancel-ledger redesign below |
| Uncommitted limit-grid autonomy work | 2026-08-05 | OpenCode/user worktree | Drop the pair inventory-reconciliation dependency: the vault records its own cancels (`CANCELLED_ORDERS`) and classifies any order that vanished without a recorded cancel as fully executed; solvency split into `executed_orders`/`cancelled_orders`; migration no longer scans the pair | `cargo test --workspace` green (4 manager + 29 vault unit + 12 integration); `cargo clippy --all-targets -- -D warnings` clean | The "unknown means fully executed" default trusts that the shipped pair only ever loses an order to a fill or a vault-initiated cancel; an order missing with a healthy parked query is classified, not rejected; no independent review |
| Uncommitted fee-system + protocol fee | 2026-08-05 | OpenCode/user worktree | New `fee-system` workspace (fee-registry + fee-collector) and grid-vault protocol fee per fill: canonical tier ladder seeded at instantiate (`ladder_version`), `RefreshHolding` persistence (user-directed: persist via message so it is always readable back), collector-only `RedeemShares`, value-as-token-0 fee minted as LP to the collector (dilutes holders) | `cargo test --workspace` green (limit-grid 48 = 4 manager + 29 unit + 15 integration incl. fee tests; fee-system 10); `cargo clippy --all-targets -- -D warnings` clean | Fees depends on a live fee-registry read and the reference-price conversion assumption mirrored from `deposit_shares`; no independent review |
| Uncommitted audit fixes (live-first + non-blocking fee) | 2026-08-05 | OpenCode/user worktree | (1) Registry `EffectiveFee` is now live-first (current CL8Y balance) with the persisted `RefreshHolding` value only as a fallback on a transient read failure and the lowest tier (full base fee) otherwise, so a holder is never under-fee; (2) vault `charge_fee` made non-blocking — a registry query failure skips the fee (`fee_skipped` attribute) instead of reverting the reconcile; (3) defensive `fee_bps` cap at 10 000 | `cargo test --workspace` green (limit-grid 48, fee-system 10); `cargo clippy --all-targets -- -D warnings` clean on both | The non-blocking skip means a holder may go un-fee on a registry incident (operational loss, chosen over reverting traders' fills); persisted-only fallback is gone in favor of live reads; no independent review |

The commits were close in time and interacted with shared custody, keeper, and
documentation surfaces. Observable evidence supports active remediation and later
automated source checks, but not independent review, signed authorship, or a
stabilization period. The current uncommitted corrections must receive their own
commit-specific CI, migration rehearsal, and review evidence before this table can
record them as CI-verified.

For the limit grid, the pair inventory-reconciliation flow (issue #46, upstream
PR #1) has been removed in favor of the vault-local cancel ledger. The vault no
longer depends on pair-side owner-inventory queries, snapshot generations, or a
withdrawal lock; it reconciles directly against current pair state. Operators must
still monitor `OrderStatusUnverifiable` rows (both active and parked queries
failed) and retry after pair/RPC health is restored.
