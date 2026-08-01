# Grid Implementation Status

## Implemented

- Separate non-custodial manager and dedicated vault Wasm contracts.
- One designated owner and one bot per vault address.
- Factory registration of vault addresses by owner.
- Pair/factory validation and pair-owned maker verification.
- Pair code-ID pinning at bot creation and validation before every mutating pair
  interaction, with an admin re-pin message after verified migrations.
- Read-only liquid-plus-escrow `solvency` query that reports drift and
  verification warnings without blocking execution.
- Permissionless order-ID reconciliation with no reported amounts.
- Proceeds credited from queried per-vault CW20 balance deltas.
- Exact observed-balance check during CW20 deposit callbacks.
- Pair-level expiry on every newly placed order.
- Irreversible owner exit and physical-balance emergency withdrawal.
- Bounded reconciliation, cancellation, and expired-refund claims.
- Reply-confirmed cancellation and parked-refund claims with per-page rollback.
- Fail-closed pair-token admission plus admin allowlist and quarantine controls.
- `cw-multi-test` lifecycle, malicious-token, pause, concurrent-fill, parked
  refund, isolation, exact-accounting, and randomized conservation coverage.
- Manager-created vault deployment and a durable production grid operator.

## Required Before Production

1. Complete adversarial testing against the production CL8Y pair/runtime,
   including chain upgrades, fee policy, archive-provider failure, and load.
2. Complete an independent external audit.
3. Perform a staged testnet and limited-value rollout with monitored limits.

The complete lifecycle, threat model, invariants, migration plan, and external
pair limitations are in [the protocol](docs/GRID_MANAGER_PROTOCOL.md).
