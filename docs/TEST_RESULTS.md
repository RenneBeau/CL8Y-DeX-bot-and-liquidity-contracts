# Verification Report

Last synchronized: 2026-08-01

This report describes the current tree. GitHub Actions artifacts are the
authoritative per-commit record for logs, optimized Wasm checksums, SBOMs, and
release evidence; static checksums are intentionally not duplicated here.

## Environment

- Rust `1.81.0` from `rust-toolchain.toml`
- CL8Y DEX revision `fad801117fe54420d7529da04e485d67d511ef2c`
- Immutable optimizer image from the root `Makefile`
- Terra Classic LocalTerra, chain ID `localterra`
- Standard unmodified CL8Y limit-order pair
- One-second TWAP for deterministic LocalTerra tests
- Protocol fee disabled in the local fixture

## Source Verification

```sh
make test
make clippy
```

Result: PASS

- Rebalancer Rust: 20 tests (7 liquidity, 11 vault, 2 proxy)
- Grid Rust: 38 tests (4 manager/limits, 23 vault unit, 11 integration)
- Rebalancer keeper Python: 48 tests
- Grid operator Python: 24 tests
- Strict Clippy and formatting: PASS

Grid integration coverage includes manager-created dedicated vaults, complete
deposit/fill/reconcile/cancel/withdraw lifecycles, concurrent fills, parked
refunds, pair pause, generic query failures, malicious CW20 balance behavior,
unsolicited-balance synchronization, token admission/quarantine, reply rollback,
cross-vault isolation, exact fixture-output accounting, and randomized custody
conservation.

The exact fixture-output test verifies two independent vaults. One vault remains
unchanged while the other is filled; after configured ask/bid fixture fills and
integer rounding, the resulting base-token increases are exactly 300 and 187.
These are deterministic accounting fixtures, not guarantees of market profit or
production CL8Y fee/execution behavior.

## Dependency Security

```sh
make security
```

Result: PASS with parser-compatible scanner revisions pinned in
`.github/scripts/install-security-tools.sh` and `.github/workflows/security.yml`.
The narrow `RUSTSEC-2024-0344` host-only exception and its removal condition are
documented in `SECURITY.md` and `deny.toml`.

## Reproducible Wasm

```sh
make reproducible
```

The workflow builds both workspaces twice with the immutable optimizer image and
compares every output digest. Per-commit checksums and evidence are uploaded by
the Reproducible Wasm workflow and regenerated for signed releases.

## Signed LocalTerra E2E

```sh
make local-setup
make local-e2e
```

Result: PASS, 10 rebalancer scenarios and 10 grid scenarios.

The rebalancer suite verifies authorization boundaries, governance fee tier,
first/subsequent share minting, donation-safe pricing, proportional and
single-token settlement, TWAP-triggered bounded rebalancing through the
production keeper, unchanged LP supply, no DEX LP custody, and failure limits.

The grid suite deploys four dedicated one-bot vaults across two pairs. It
verifies pair-token policy bootstrap, per-side allocation, real CL8Y fills,
production operator indexing/signing/SQLite restart without replay, exact
chain-observed fill proceeds, permissionless third-party reconciliation without
keeper reimbursement, cross-vault state isolation, bounded cancellation,
withdrawal, and per-vault solvency. Reconciliation does not automatically create
opposite orders; the owner places new free proceeds with a separate `allocate`.

Signed E2E uses zero confirmation depth for speed. Nonzero confirmation depth,
shallow transaction disappearance/reappearance, ambiguous broadcasts, forged
emitters, reverted pages, and retry/backoff policy are covered by the production
keeper/operator unit suites.

## Extended Soak

```sh
SOAK_ROUNDS=25 make local-soak
```

The soak suite alternates inventory pressure, requires each rebalance to spend
its declared offer amount, preserves LP supply, updates reference price only
after validated improvement, and confirms no protocol contract acquires DEX LP
tokens.

## Scope

These checks demonstrate deterministic accounting and functional behavior
against the pinned local CL8Y code. They do not establish production economic
profit. Production readiness still requires independent security review,
mainnet-equivalent TWAP/liquidity analysis, adversarial CL8Y runtime testing, and
staged limited-value deployment.
