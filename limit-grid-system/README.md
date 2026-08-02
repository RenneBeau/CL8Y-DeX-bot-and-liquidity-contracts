# Reference Limit-Order Grid

This Cargo workspace contains `contracts/grid-manager` (non-custodial vault
factory/registry) and `contracts/grid-vault` (one-owner, one-bot CL8Y
limit-order custody).

**This design is reference-only and NOT deployable.** It requires pair queries
that do not exist in the shipped CL8Y pair (typed order status, owner inventory,
owner-index backfill). Do not deploy or fund it. The deployable design is the
standard swap grid in [`market-grid-system`](../market-grid-system/README.md).
cargo test --manifest-path limit-grid-system/Cargo.toml
cargo clippy --manifest-path limit-grid-system/Cargo.toml --all-targets -- -D warnings
```

This remains pre-production, unaudited reference code.