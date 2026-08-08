# Standard Swap Grid

This Cargo workspace contains `contracts/grid-vault-swap`, the swap-only grid
vault. It is pre-production: production deployment is blocked pending approved
collector/proxy build values, deployed-address verification, canonical fee E2E,
and the remaining audit gates documented in
[`docs/DEPLOY_FEE_SYSTEM.md`](../docs/DEPLOY_FEE_SYSTEM.md).

The vault holds CW20 balances, reads the pool price over a TWAP window, and
executes classic `Swap` calls when the price crosses a grid level. It uses the
exact CL8Y pair API as shipped (`Pool`, `Observe`, `Swap` via the CW20 hook) and
requires no pair modification, fork, or upstream merge.

Version `0.2.0` requires `factory` and `pair_code_id`. Instantiation verifies the
factory's pair lookup and the pair's runtime code ID, and rebalance/proxy swap
rechecks the code ID. Existing 0.1.x vaults require redeployment and their proxy
routes must be re-registered. Migration of that incompatible state must not be
attempted.

Share accounting mints against the vault's current net-asset-value, not a fixed
basis, so deposits/withdrawals cannot extract value from existing holders.
Deposits and withdrawals have no direct protocol fee, and deposits are
admin-only. A fee-enabled rebalance converts economic fee value `F` to
NAV-priced collector LP as `floor(F*S/(A-F))`, where `A` is post-settlement
asset value and `S` is pre-mint supply. Flooring keeps the collector's immediate
claim at or below `F`. The fee subject is `config.admin`; accepting an admin
transfer also migrates that admin's shares to the new admin.
The last successful effective fee is cached for `config.admin`. If the registry
is unreachable, the vault charges that exact bps/tier with source
`vault_cached`; without history it charges 180 bps/source `lowest`. A reachable
registry with a failed CL8Y token query returns full base/`Lowest`, so stale
token holdings never preserve a discount.

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

At a zero target, deviation is now zero only for zero actual holdings and 10,000
bps otherwise, so lower-bound portfolios produce a corrective plan rather than
an error. Planned offers use the full value difference rather than an extra
division by two; ideal-target tests cover both directions. Migration is an
exported entry point that checks CW2 contract identity and requires an older
semantic version.

Pause blocks deposits and rebalance initiation but permits pro-rata owner and
collector withdrawals. A pending swap/rebalance still blocks withdrawal.

The permissionless rebalance keeper for this vault lives in the shared
[`grid-operator-system`](../grid-operator-system/README.md)
`services/grid-operator` (`swap_keeper.py`).

Build and test:

```sh
cargo test --manifest-path market-grid-system/Cargo.toml
cargo clippy --manifest-path market-grid-system/Cargo.toml --all-targets -- -D warnings
```

The related limit-order grid is in
[`limit-grid-system`](../limit-grid-system/README.md).
