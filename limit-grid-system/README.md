# Limit-Order Grid

This Cargo workspace contains `contracts/grid-manager` (non-custodial vault
factory/registry) and `contracts/grid-vault` (one-owner, one-bot CL8Y
limit-order custody).

## Autonomy from the pair

`grid-vault` reconciles against the shipped CL8Y DEX pair using only the shipped
query interface (`LimitOrder`, `ExpiredLimitRefund`). It requires no typed order
status, no owner-inventory API, no pair fork, and no upstream PR.

- Pair cancellations can only be initiated by the vault, so the vault records
  each confirmed cancel locally (`CANCELLED_ORDERS`).
- An order that is absent from the pair, holds no parked refund, and was never
  cancelled could only have left the book through execution: it is classified as
  fully executed and retired on the next reconcile. Its fill proceeds were
  already credited to the vault balance.
- An order whose active *and* parked queries both fail is `OrderStatusUnverifiable`
  and is retained for retry.

The cancelled ledger is exposed through the paginated `cancelled_orders` query and
is never reused (`recover_order` rejects a cancelled order).

## Status

Pre-production, unaudited reference code, but no longer gated on a pair upgrade.
See [`grid-operator-system/docs/GRID_MANAGER_PROTOCOL.md`](../grid-operator-system/docs/GRID_MANAGER_PROTOCOL.md)
for the full protocol and threat model.

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
