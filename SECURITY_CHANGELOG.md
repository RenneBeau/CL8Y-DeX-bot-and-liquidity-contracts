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
| Uncommitted issue #46 work | 2026-08-01 | OpenCode/user worktree | Bounded pair-first legacy vault inventory drain, empty rescan proof, stale-local cleanup and exact balance synchronization | Local invariant tests and release checks must pass before commit | Depends on unmerged upstream PR #1; not deployable until a merged revision is pinned and funded historical fixtures are rehearsed |

The commits were close in time and interacted with shared custody, keeper, and
documentation surfaces. Observable evidence supports active remediation and later
automated source checks, but not independent review, signed authorship, or a
stabilization period. The current uncommitted corrections must receive their own
commit-specific CI, migration rehearsal, and review evidence before this table can
record them as CI-verified.

For issue #46, pair upgrade and owner-index backfill readiness must precede vault
migration. Operators must monitor snapshot generation and pending failures and
must not roll a pair back after a vault captures its snapshot; recovery is a roll
forward to the same verified implementation and generation. There is no vault
admin bypass for the withdrawal lock.
