# CL8Y Portfolio Rebalancer

This Cargo workspace contains the portfolio rebalancing system. Each
bot keeps one CW20 token pair in an isolated vault and gives depositors
transferable CW20 LP shares. A shared swap proxy routes approved vaults' swaps
through their CL8Y pairs. Production requires that proxy to be deployed, pinned,
and whitelisted on the CL8Y DEX so routed swaps pay no DEX fee. That mainnet
state is not yet established.

## Components

- `contracts/swap-proxy`: validates approved vaults and routes swaps through
  their configured CL8Y pairs.
- `contracts/bot-vault`: holds one bot's inventory and allows its dedicated
  keeper to rebalance after the configured price movement.
- `contracts/bot-liquidity`: accepts deposits, mints transferable shares, burns
  shares, and supports proportional or single-token withdrawals.
- `examples/keeper`: reference off-chain keeper and service configuration.

The first liquidity mint permanently locks 1,000 smallest share units. Direct
token donations are included in pre-deposit balances. Established-vault shares
use the minimum proportional contribution across both assets, preventing an
oracle price from diluting incumbents. Deposits and withdrawals have no direct
protocol fee. When fee configuration is present, a completed rebalance converts
economic fee value `F` into NAV-priced protocol-fee LP using
`x = floor(F*S/(A-F))`. Flooring ensures the collector's immediate post-mint
claim is no greater than `F`.

## Price And Fee Tier

Vaults require a nonzero CL8Y arithmetic TWAP window. One captured TWAP drives
the trigger, inventory direction, amount, execution floor, and reply check;
spot reserves never drive keeper safety. A short-term strategy can start by
testing a 30-300 second window. The final value must be chosen from pair
liquidity, block time, volatility, and manipulation-cost measurements.

The intended production swap-proxy must be whitelisted on the CL8Y DEX so routed
swaps pay no DEX fee; it is not yet deployed or pinned. The protocol fee is
separate and resolves only `config.admin` at rebalance time. Individual LP
holders are not tiered. See `../docs/FEE_TIER_PROTOCOL.md`.

Versions `bot-vault` 0.2.0, `swap-proxy` 0.2.0, and `bot-liquidity` 0.2.0 are the
current rebalancer set. Bot-vault requires `factory` and `pair_code_id`, verifies factory pair
registration and runtime code at vault creation, and rechecks code before
rebalance/proxy swap. Existing 0.1.x vault and liquidity instances require
redeployment. A 0.1.x proxy with routes must be replaced and routes re-registered;
only empty compatible proxy state may migrate. The vault caches the last
successful effective fee for `config.admin`; registry outage uses that exact
bps/tier (`vault_cached`) or 180 bps (`lowest`) without local history. Registry
token-query failure itself grants no discount.

The contracts are unaudited and production deployment is blocked pending
approved registry/collector/proxy compile-time values, deployed proxy provenance
and whitelisting, canonical fee E2E, the `0.2.0` redeployment, and independent
review. H-05 is the sole partial repository audit finding. Mainnet compilation
fails if any required environment value is missing or empty. See
`../docs/DEPLOY_FEE_SYSTEM.md` and `../RELEASE.md`.

## Verify

From the repository root:

```sh
cargo test --manifest-path rebalancer-system/Cargo.toml
cargo clippy --manifest-path rebalancer-system/Cargo.toml --all-targets -- -D warnings
make optimize
make local-e2e
```

`real_registry_ladder` provides genuine `cw-multi-test` full settlement with
actual CW20 CL8Y, fee-registry, bot-vault, bot-liquidity, and swap-proxy
contracts plus stateful pair/factory models. It covers no-holder and tiers 1
through 9 through NAV-priced collector mint and pro-rata withdrawal. This is
not LocalTerra/on-chain E2E; canonical exact-candidate LocalTerra evidence is
still required for release.

## Documentation

- [Implementation guide](IMPLEMENTATION.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Deployment and new-pair setup](docs/DEPLOYMENT.md)
- [Admin and keeper operations](docs/ADMIN_OPERATIONS.md)
- [Swap proxy protocol](docs/protocols/SWAP_PROXY.md)
- [Bot vault protocol](docs/protocols/BOT_VAULT.md)
- [Bot liquidity protocol](docs/protocols/BOT_LIQUIDITY.md)
- [Keeper example](examples/keeper/README.md)
