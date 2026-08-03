# Standard Swap Grid

This Cargo workspace contains `contracts/grid-vault-swap`, the **deployable**
swap-only grid vault.

The vault holds CW20 balances, reads the pool price over a TWAP window, and
executes classic `Swap` calls when the price crosses a grid level. It uses the
exact CL8Y pair API as shipped (`Pool`, `Observe`, `Swap` via the CW20 hook) and
requires no pair modification, fork, or upstream merge.

Share accounting mints against the vault's current net-asset-value, not a fixed
basis, so deposits/withdrawals cannot extract value from existing holders.

## How the grid works

The grid is a classic **grid of price levels**, defined by four values configured
at instantiation/update:

| Parameter      | Meaning                                                        | Code ref            |
|----------------|----------------------------------------------------------------|---------------------|
| `lower_price`  | Lower boundary of the grid                                     | `instantiate`        |
| `upper_price`  | Upper boundary of the grid                                     | `instantiate`        |
| `grid_count`   | Number of grid cells (1..=500)                                 | `MAX_GRID_COUNT`     |
| spacing        | Equal interval `(upper_price - lower_price) / grid_count`      | `grid_cell`          |

The current price (a TWAP over `twap_window_seconds`) is mapped onto one of the
`grid_count` cells, `0..=grid_count` (`grid_cell` in `contract.rs`). The target
holdings are the **linear inventory curve** across the cells:

```
target_token1 = total_value * (cell / grid_count)
```

So the vault rebalances toward:

| Price                    | cell        | token0 : token1 target |
|--------------------------|--------------------|-------------------------|
| **`lower_price`**       | 0      | **100 : 0** (all token0 / base) |
| **midprice**   | `grid_count / 2` | **50 : 50** |
| **`upper_price`**       | `grid_count` | **0 : 100** (all token1 / quote) |

Every level crossed trades an equal, linear inventory slice of
`1 / grid_count` of the pool value toward that level's target weight. It is a
**swap-only grid (position / rebalancing)**: it does not place or cancel resting
limit orders — it executes a `Swap` when the price crosses a level
(`execute_rebalance`) to realign the allocation.

Because `grid_cell` floors the cell index
(`lower_price` maps to cell `0`, `upper_price` to cell `grid_count`, and an
exact midprice floors to midpoint), positioning at a boundary asymptotically
favors the lower cells. This is intentional and bounded, but worth a reviewer
note.

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