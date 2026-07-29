# Verification Report

Date: 2026-07-29

Environment:

- Terra Classic LocalTerra, chain ID `localterra`
- CL8Y DEX revision `fad801117fe54420d7529da04e485d67d511ef2c`
- `cosmwasm/workspace-optimizer:0.16.1`
- EMBER/CORAL minimal test pool
- Protocol fee: disabled
- Vault price source: spot mode for deterministic local testing

## Rust Verification

Commands:

```sh
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Result: PASS

- Bot liquidity unit tests: 2 passed
- Bot vault unit tests: 3 passed
- Swap proxy unit tests: 2 passed
- Documentation tests: passed
- Strict Clippy: passed with no warnings

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

## Extended Soak

Command:

```sh
SOAK_ROUNDS=25 make local-all
```

Result: PASS

- 25 of 25 alternating inventory-rebalance rounds passed.
- Duration of latest run: 99 seconds.
- Every round crossed the configured price threshold.
- Every vault rebalance spent exactly its declared offer amount.
- Bot LP total supply remained unchanged in every round.
- The reference price updated only after the post-swap allocation check.
- Neither protocol contract acquired CL8Y DEX LP tokens.

For a longer local run:

```sh
SOAK_ROUNDS=100 make local-soak
```

## Scope

These results demonstrate functional behavior against the pinned CL8Y code and
LocalTerra VM. They are not a substitute for an independent security audit,
mainnet TWAP validation, economic simulation, or formal verification.
