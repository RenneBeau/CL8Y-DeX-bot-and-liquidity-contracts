# Pair Upgrade Option — Owner Inventory Fix (for review)

This directory archives an **optional, upstream-side upgrade** to the CL8Y pair
that would let a limit-order vault reconcile open orders **without treating a
query miss as a terminal state**. It is kept here so the upstream maintainers
(`PlasticDigits/cl8y-dex-terraclassic`) can review it before deciding whether to
merge. The current bot does **not** depend on it and the project is built
against the pair exactly as shipped.

## Why it is optional

- The **market grid** (`market-grid-system`, swap-only) needs none of this — it
  only holds CW20 balances and calls classic `Swap` on the shipped pair API.
- The **limit grid** (this workspace) is **reference-only and not deployable**.
  It could use this inventory API for on-chain reconciliation, but the shipped
  pair does not provide it. This grid is intentionally left non-deployable rather
  than fork or modify PlasticDigits' code.
- Therefore this proposal is an **upstream option, not a dependency** of any
  deployable artifact in this repository.

## Contents

- `pair-owner-inventory-api.patch` — the full protocol diff (pair contract,
  dex-common, tests) vs. upstream `main`.
- What it adds: versioned typed order status; prospective terminal tombstones;
  a unified owner/order custody index; bounded keyset pagination with generation
  snapshots; capability negotiation; a permissionless, bounded, resumable
  backfill migration.

## Status

- Proposed upstream as `PlasticDigits/cl8y-dex-terraclassic` **PR #1**
  `feat(pair): add typed owner order inventory API` (branch
  `fix/grid-owner-inventory`, HEAD `c1f669b`).
- **Closed unmerged** by the author; the project instead pins the pair API as
  shipped. This PR was kept open only as a record of the idea.
- An independent code review of the patch found: the custody invariant is
  maintained on all terminal fill/park/cancel/claim paths, owners query
  fail-closed until the backfill migration completes, and no
  fund-loss/griefing vulnerability was identified. Minor non-blocking notes:
  unbounded tombstone growth and `Claimed`/`Dust` reported under a
  `Cancelled` status.

## Reviewers

Maintainers: review the patch and/or PR #1. The bot project treats this solely
as an opt-in upstream improvement; no release or contract in this repository
binds to them.