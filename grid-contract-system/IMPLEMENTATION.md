# Grid Implementation Status

## Implemented

- Separate non-custodial manager and dedicated vault Wasm contracts.
- One designated owner and one bot per vault address.
- Factory registration of vault addresses by owner.
- Pair/factory validation and pair-owned maker verification.
- Permissionless order-ID reconciliation with no reported amounts.
- Proceeds credited from queried per-vault CW20 balance deltas.
- Exact observed-balance check during CW20 deposit callbacks.
- Pair-level expiry on every newly placed order.
- Irreversible owner exit and physical-balance emergency withdrawal.
- Bounded reconciliation, cancellation, and expired-refund claims.

## Required Before Production

1. Replace optimistic cancellation/claim accounting with dedicated reply state so
   every state transition is confirmed independently.
2. Pin and validate supported CL8Y pair code/interface revisions.
3. Add an on-chain reviewed-token allowlist and operational quarantine process.
4. Add `cw-multi-test` and real CL8Y integration tests for concurrent fills,
   parking, pair pause, query failures, and malicious CW20s.
5. Add property tests for liquid-plus-escrow conservation per vault.
6. Update deployment scripts and operator payloads for manager-created vaults.
7. Complete external audit and staged testnet/limited-value rollout.

The complete lifecycle, threat model, invariants, migration plan, and external
pair limitations are in [the protocol](docs/GRID_MANAGER_PROTOCOL.md).
