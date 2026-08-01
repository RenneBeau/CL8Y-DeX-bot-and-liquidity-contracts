# Internal Security Review

> This is an author-produced internal review, not an independent security audit.
> It does not establish that the contracts are secure, production-ready, free of
> vulnerabilities, or suitable for economic deployment. A qualified third-party
> audit remains required.

## Review Metadata

- Original baseline examined: rebalancer commit `518a167`
- Later E2E baseline referenced by the old notes: `48c2bbc`
- Current repository baseline: `b1e943de8da888330d6c0c825b8d702ad03e8d48`
- Current security corrections: uncommitted working-tree changes after that baseline
- Review date: 2026-08-01
- Scope: `bot-vault`, `bot-liquidity`, `swap-proxy`, shared types, keeper
- Validation type: internal source review and local tests only
- External validation: none identified for the current working tree

The previous document mixed several revisions, intermediate reasoning, retracted
claims, and later fixes. This replacement records only resolved conclusions.

## Trust Model

| Actor/component | Remaining authority or trust |
|---|---|
| Admin | Changes keepers and risk controls, proposes successor admins, binds or revokes liquidity integration |
| Keeper | Triggers typed on-chain rebalance actions; cannot supply amount or minimum return |
| Bot-liquidity | Custodial component for vault transfers; its reciprocal binding and runtime code ID are checked |
| Bot-liquidity Wasm admin | Trusted not to migrate the component maliciously; a changed code ID halts privileged calls |
| Swap proxy admin | Controls registered routes and accumulated CL8Y withdrawals |
| CL8Y pair | Trusted external execution and oracle dependency; no third-party report is bundled here |
| CW20 contracts | Expected to implement standard balance and transfer semantics |

The liquidity component remains in the trusted computing base. Code-ID pinning and
revocation reduce accidental substitution and post-binding migration risk, but do
not prove that the initially pinned implementation is correct.

## Findings

| ID | Severity | Title | Status |
|---|---|---|---|
| ISR-001 | High | Shares burned before failed single-token withdrawal | Fixed, verified locally |
| ISR-002 | High | Liquidity integration had unrestricted unpinned custody authority | Fixed, verified locally |
| ISR-003 | High | Arithmetic operators could panic on extreme external values | Fixed, verified locally |
| ISR-004 | Medium | Rebalancer admin transfer was immediate or unavailable | Fixed, verified locally |
| ISR-005 | Medium | No complete funded-fleet migration procedure | Open |
| ISR-006 | Low | Bootstrap minimum was immutable | Fixed, verified locally |
| ISR-007 | Low | TWAP window was immutable | Fixed, verified locally |
| ISR-008 | Info | One vault per pair per proxy | Accepted design constraint |

Summary: 3 High, 2 Medium, 2 Low, 1 Informational. Open: ISR-005. No item is marked
externally verified or CI-verified for the current tree.

### ISR-001: Withdrawal Share Loss

- Component/version: `bot-liquidity`, original `518a167` baseline
- Root cause: shares were burned before swap success was confirmed
- Impact/scenario: a failed swap could leave an LP without shares or payout
- Correction: pending withdrawal plus `reply_always`; burn only after settlement
- Tests: failed reply preserves shares and clears pending state
- Status: fixed and verified locally; no current CI or third-party verification
- Residual risk: correctness still depends on vault settlement and CW20 behavior

### ISR-002: Liquidity Custody Authority

- Component/version: `bot-vault` and `bot-liquidity`, through current working tree
- Root cause: address equality alone authorized arbitrary configured-token transfers
- Impact/scenario: a malicious or migrated liquidity contract could drain the vault
- Correction: reciprocal vault/asset binding, approved and pinned runtime code ID,
  explicit revocation, exact pending-operation authorization for swaps/transfers,
  transfer replies, and mutual exclusion with keeper rebalances
- Tests: unauthorized callers, runtime code-ID mismatch and revocation are covered
  locally; malicious initial candidate and operation-scoped authorization coverage
  remain incomplete
- Status: fixed and verified locally; no current CI or third-party verification
- Residual risk: the approved bot-liquidity implementation and chain migration admins
  remain trusted software/governance dependencies

### ISR-003: Arithmetic Panic Paths

- Component/version: rebalancer contracts, current working tree
- Root cause: infallible decimal multiplication, ratio construction, and intermediate
  multiplication before a cap
- Impact/scenario: extreme direct token balances or pair values could make queries and
  operations trap repeatedly
- Correction: checked `Uint256` intermediates, explicit denominator checks and fallible
  `Uint128` conversion
- Tests: normal, maximum-value, minimum-price and intermediate-overflow unit cases
- Status: fixed and verified locally; no current CI or third-party verification
- Residual risk: remaining contract arithmetic must be continuously linted and tested

### ISR-004: Administrative Handoff

- Component/version: all rebalancer contracts, current working tree
- Root cause: immediate replacement in vault/proxy and no transfer in bot-liquidity
- Impact/scenario: one valid but mistyped address permanently lost internal control
- Correction: proposal, acceptance by the candidate, cancellation, pending-admin query,
  and old-admin continuity until acceptance
- Tests: proposal replacement, wrong candidate, cancellation, acceptance and loss of
  old-admin authority are covered locally across the components
- Status: fixed and verified locally; no current CI or third-party verification
- Residual risk: internal admin and chain Wasm migration admin remain separate roles

### ISR-005: Fleet Migration

- Component/version: complete rebalancer deployment
- Root cause: no versioned funded-state migration and coordinated handoff runbook
- Impact/scenario: incident response relies on LP redemption and redeployment
- Recommendation: add guarded migrate entry points, fixtures from released state, a
  chain-admin handoff procedure, and rehearsed rollback criteria
- Status: open
- Residual risk: high operational dependency during incidents

### ISR-006 Through ISR-008

- Bootstrap minimum and TWAP-window controls were added with bounds and local tests.
- One-vault-per-pair proxy routing is an explicit isolation constraint, not a finding
  asserting safety.
- None has current external verification.

## Evidence Rules

- Local and CI results are reported separately in `docs/TEST_RESULTS.md`.
- A test count is not evidence of security or coverage completeness.
- A later fix does not retroactively change the revision originally reviewed.
- Retracted hypotheses are excluded from the findings table rather than preserved as
  conversational reasoning.
- Any future status change must identify a commit, command/test, environment, result,
  and whether validation was internal, CI-based, or independent.
