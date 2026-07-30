# Experimental Grid Contract System

This directory is an independent Cargo workspace for the proposed CL8Y grid
system. It is intentionally separate from the production rebalance contracts
while its execution costs, onboarding flow, and multi-bot accounting are being
evaluated.

`contracts/grid-manager` is a multi-tenant prototype. Users create independent
bots with their own bounds and rung count. Every bot has isolated token
balances, internal LP shares, CL8Y order records, and prepaid LUNC gas credit.
The manager address owns the CL8Y orders, allowing all bots to use one
governance-assigned fee tier.

The contract, rather than the keeper, calculates prices, allocations, fill
amounts, and opposite orders. A keeper calls reconciliation and receives a
capped reimbursement only when tracked orders changed. An unfilled portion
remains on its original order; only the filled portion funds an opposite order.
Users cancel a bot's active orders before burning internal bot LP shares for a
pro-rata withdrawal. Other bots' balances and shares are never included.
Cancellation processes a bounded page at a time; owners repeat it until the
reported `remaining_orders` reaches zero.
CW20 deposits are allocated automatically: token A is divided by the number of
sell-A rungs and token B by the number of sell-B rungs. Integer remainders stay
free and can be allocated later.

The manager accepts only pairs registered by its configured CL8Y factory and
requires matching CW20 decimals. It uses arithmetic price spacing. Exact bot
attribution comes from the persistent cumulative maker output supplied by the
CL8Y extension in `cl8y-extension/`; inferred escrow arithmetic is not used.

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
