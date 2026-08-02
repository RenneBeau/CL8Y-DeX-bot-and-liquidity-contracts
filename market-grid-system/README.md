# Standard Swap Grid

This Cargo workspace contains `contracts/grid-vault-swap`, the **deployable**
swap-only grid vault.

The vault holds CW20 balances, reads the pool price over a TWAP window, and
executes classic `Swap` calls when the price crosses a grid level. It uses the
exact CL8Y pair API as shipped (`Pool`, `Observe`, `Swap` via the CW20 hook) and
requires no pair modification, fork, or upstream merge.

Share accounting mints against the vault's current net-asset-value, not a fixed
basis, so deposits/withdrawals cannot extract value from existing holders.

The permissionless rebalance keeper for this vault lives in the shared
[`grid-operator-system`](../grid-operator-system/README.md)
`services/grid-operator` (`swap_keeper.py`).

Build and test:

```sh
cargo test --manifest-path market-grid-system/Cargo.toml
cargo clippy --manifest-path market-grid-system/Cargo.toml --all-targets -- -D warnings
```

The related reference-only limit-order grid is in
[`limit-grid-system`](../limit-grid-system/README.md) and must not be deployed.