# Trust-Minimized Grid Contract System

This independent Cargo workspace contains:

- `contracts/grid-manager`: non-custodial vault factory and registry.
- `contracts/grid-vault`: one-owner, one-bot custody and CL8Y order contract.
- `services/grid-operator`: optional discovery and transaction automation.

The manager never receives CW20 deposits. Each vault address owns only its own
funds and CL8Y orders. Reconciliation is permissionless and accepts order IDs
only; credited proceeds come from queried vault balances, not keeper/indexer
amounts. Every newly placed order has a configured timeout, and the owner retains
an indexer-independent exit path. The vault pins its CL8Y pair code ID at bot
creation, validates it before every pair interaction, and exposes a read-only
`solvency` query that cross-checks free balances plus pair escrow per token.

The current CL8Y pair does not retain cumulative maker output or typed terminal
history. Exact event-by-event opposite-order recreation is therefore intentionally
not part of the trust-minimized flow. See the [protocol and threat model](docs/GRID_MANAGER_PROTOCOL.md).

Additional guides:

- [Implementation status](IMPLEMENTATION.md)
- [Operations](docs/GRID_OPERATIONS.md)
- [Optional indexer](docs/GRID_INDEXER.md)

Build and test:

```sh
cargo test --manifest-path grid-contract-system/Cargo.toml
cargo clippy --manifest-path grid-contract-system/Cargo.toml --all-targets -- -D warnings
```

This remains pre-production code pending CL8Y integration/property testing,
token allowlisting, and an external security review.
