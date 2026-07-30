# Experimental Grid Contract System

This directory is an independent Cargo workspace for the proposed CL8Y grid
system. It is intentionally separate from the production rebalance contracts
while its execution costs, onboarding flow, and multi-bot accounting are being
evaluated.

Canonical documentation:

- [Implementation guide](IMPLEMENTATION.md)
- [Grid manager protocol](docs/GRID_MANAGER_PROTOCOL.md)
- [Grid keeper and admin operations](docs/GRID_OPERATIONS.md)
- [Grid indexer protocol](docs/GRID_INDEXER.md)
- [Verification report](../docs/TEST_RESULTS.md)

`contracts/grid-manager` is a multi-tenant prototype. Users create independent
bots with their own bounds and rung count. Every bot has isolated token
balances, internal LP shares, CL8Y order records, and prepaid LUNC gas credit.
The manager address owns the CL8Y orders, allowing all bots to use one
governance-assigned fee tier.

The contract calculates prices, allocations, and opposite orders. One trusted
grid keeper relays exact `limit_order_fill` events supplied by a trusted chain
indexer. The contract verifies each report against the standard pair's current
escrow, order metadata, and CL8Y rounding arithmetic before crediting output. A
keeper receives a capped reimbursement only after a valid reconciliation.
Users cancel a bot's active orders before burning internal bot LP shares for a
pro-rata withdrawal. Other bots' balances and shares are never included.
Cancellation processes a bounded page at a time; owners repeat it until the
reported `remaining_orders` reaches zero.
CW20 deposits are allocated automatically: token A is divided by the number of
sell-A rungs and token B by the number of sell-B rungs. Integer remainders stay
free and can be allocated later.

The manager accepts only standard pairs registered by its configured CL8Y
factory and requires matching CW20 decimals. It uses arithmetic price spacing.
The trusted indexer is required because standard pairs remove completed orders
and contracts cannot query historical transaction events themselves.

The grid manager has one dedicated keeper address. It is independent from each
rebalance vault's keeper and can reconcile every `bot_id` managed by the grid
contract. The trusted indexer is an off-chain data source rather than another
transaction signer: it streams exact fill events to the grid keeper, which
submits one constant-size aggregate report per changed order and pays gas before
reimbursement. The report includes consumed escrow, exact maker output, and fill
count; the contract checks it against the current order and aggregate rounding
bounds.

Every bot prepays a separate LUNC gas credit. The keeper pays transaction fees
and receives a fixed reimbursement only after useful reconciliation. Owners can
recover excess gas credit, while active bots must retain the emergency reserve.
The onboarding and up-front funding tradeoff remains tracked in GitHub issue #1.

Run its tests independently:

```sh
cargo test --manifest-path grid-contract-system/Cargo.toml
cargo clippy --manifest-path grid-contract-system/Cargo.toml --all-targets -- -D warnings
make local-grid
```
