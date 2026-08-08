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

**Abandoned as a production venue.** This workspace is retained, tested, and
published only as a PoC artifact. It must not be deployed with economic funds.
The unresolved external pair terminal-state and direct-maker fee semantics are
accepted as out of production scope, not technically validated.

Fee addresses are instantiate-only. The manager now stores and
propagates both addresses to newly created vaults, rejects partial
configuration, and requires both on `mainnet`; manager updates do not alter
existing vaults, which require migration or redeployment if fee-disabled.
Limit-grid has no swap proxy and interacts directly with the pair. Its fee
subject is `bot.owner`.

Frozen released-state fixtures cover the supported grid-vault 0.1.0 to 0.1.1
migration. That narrow path does not make unrelated fee-disabled or incompatible
state safe to migrate; use only an explicitly reviewed migration or redeploy.

The vault caches the last successful effective bps/tier for `bot.owner`.
Registry unavailability charges that exact result with source `vault_cached`, or
180 bps/source `lowest` when no local history exists, so an outage cannot bypass
the fee. A reachable registry whose CL8Y token query fails grants no discount;
registry holding history is observability only.

Fee value is converted to LP shares at current NAV using
`x = floor(F*S/(A-F))`; flooring ensures the collector's immediate claim does
not exceed `F`. In Exit, owner emergency withdrawal burns only owner shares and
transfers only the owner's pro-rata assets, preserving collector backing and
remaining total shares. After active orders reach zero, the collector may redeem
its preserved shares while the vault remains in Exit.
See [`grid-operator-system/docs/GRID_MANAGER_PROTOCOL.md`](../grid-operator-system/docs/GRID_MANAGER_PROTOCOL.md)
for the full protocol and threat model.

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
