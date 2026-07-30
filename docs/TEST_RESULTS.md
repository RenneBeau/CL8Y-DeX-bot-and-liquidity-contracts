# Verification Report

Date: 2026-07-30

Environment:

- Terra Classic LocalTerra, chain ID `localterra`
- CL8Y DEX revision `fad801117fe54420d7529da04e485d67d511ef2c`
- `cosmwasm/workspace-optimizer:0.16.1`
- Standard `wallet` seed with EMBER/CORAL and LUNC-C/EMBER grid pairs
- Protocol fee: disabled
- Vault price source: spot mode for deterministic local testing
- Standard unmodified CL8Y limit-order pair

## Rust Verification

Commands:

```sh
cargo fmt --manifest-path rebalancer-system/Cargo.toml --all -- --check
cargo test --manifest-path rebalancer-system/Cargo.toml
cargo clippy --manifest-path rebalancer-system/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path grid-contract-system/Cargo.toml --all -- --check
cargo test --manifest-path grid-contract-system/Cargo.toml
cargo clippy --manifest-path grid-contract-system/Cargo.toml --all-targets -- -D warnings
```

Result: PASS

- Bot liquidity unit tests: 2 passed
- Bot vault unit tests: 3 passed
- Swap proxy unit tests: 2 passed
- Documentation tests: passed
- Strict Clippy: passed with no warnings
- Grid manager unit tests: 6 passed

## Optimized Wasm

Command:

```sh
./test-area/deploy-system.sh
```

Result: PASS

Latest verified checksums:

```text
730ac20674a19bdee45b2ee559c6fa08d13f5d306e1823b4a0bfa16459c8d7ad  cl8y_bot_liquidity.wasm
4456a87a38edf2573373aab53dc73074d9674604d6cabc8eb16963459a027f0d  cl8y_bot_vault.wasm
1b822e77d3c268886187c6cea72700ea8276d818e964704f730534f1a4fe2dd4  cl8y_swap_proxy.wasm
3eecb5b7ed9d21b1a1abb526924c492cf59c1909b7e23d8cab57dc5049840622  cl8y_grid_manager.wasm
```

## Signed LocalTerra E2E

Command:

```sh
make local-e2e
```

Result: PASS

Verified scenarios:

1. Proxy, vault, liquidity-controller, pair, and fee-tier configuration.
2. Unregistered proxy caller and unauthorized vault-transfer rejection.
3. First proportional deposit and permanent initial share lock.
4. Donation-safe second-user share pricing with a separate signed wallet.
5. Single-token deposit with atomic proxy swap and settled share mint.
6. Pro-rata withdrawal at the vault's current A/B ratio.
7. Proportional single-token withdrawal using only the user's unwanted claim.
8. Exact 5% price trigger, wrong-direction rollback, and successful inventory
   rebalance without changing bot LP supply.
9. Zero DEX LP balances in vault and liquidity contracts; unchanged proxy CL8Y.
10. Zero deposit and unauthorized CL8Y-withdrawal failure paths.
11. Four independently owned grid bots, two on EMBER/CORAL and two on
    LUNC-C/EMBER, with isolated balances, shares, orders, and gas credits.
12. Automatic sell-A and sell-B allocation using each side's rung count.
13. Real partial CL8Y ask fill, indexed-event reconciliation validated against
    pair escrow, and an opposite bid containing only the filled portion.
14. The same dedicated grid keeper reconciled fills on both pairs while an
    unauthorized wallet was rejected.
15. Unchanged sibling-bot state after each pair's fill and reconciliation.
16. Active-order withdrawal rejection, bounded cancellation, settlement, and
    complete withdrawal of all four bots, confirming pooled solvency.

See [Grid Indexer Protocol](GRID_INDEXER.md) for indexed event aggregation and
checkpoint requirements.

## Extended Soak

Command:

```sh
SOAK_ROUNDS=25 make local-all
```

Result: PASS

- 25 of 25 alternating inventory-rebalance rounds passed.
- Duration of latest run: 97 seconds.
- Every round crossed the configured price threshold.
- Every vault rebalance spent exactly its declared offer amount.
- Bot LP total supply remained unchanged in every round.
- The reference price updated only after the post-swap allocation check.
- Neither protocol contract acquired CL8Y DEX LP tokens.
- The same `local-all` run passed the four-bot, two-pair grid suite before soak.

For a longer local run:

```sh
SOAK_ROUNDS=100 make local-soak
```

## Scope

These results demonstrate functional behavior against the pinned CL8Y code and
LocalTerra VM. They are not a substitute for an independent security audit,
mainnet TWAP validation, economic simulation, or formal verification.
