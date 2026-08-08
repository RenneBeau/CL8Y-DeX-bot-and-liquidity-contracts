# CL8Y Portfolio Rebalancer

This Cargo workspace contains the portfolio rebalancing system. Each
bot keeps one CW20 token pair in an isolated vault and gives depositors
transferable CW20 LP shares. A shared swap proxy routes approved vaults' swaps
through their CL8Y pairs; the proxy is whitelisted on the CL8Y DEX, so the
swaps it routes pay no DEX fees.

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
oracle price from diluting incumbents. Deposits, withdrawals, and
rebalances currently use a zero protocol charge.

## Price And Fee Tier

Vaults require a nonzero CL8Y arithmetic TWAP window. One captured TWAP drives
the trigger, inventory direction, amount, execution floor, and reply check;
spot reserves never drive keeper safety. A short-term strategy can start by
testing a 30-300 second window. The final value must be chosen from pair
liquidity, block time, volatility, and manipulation-cost measurements.

The deployed swap-proxy is whitelisted on the CL8Y DEX, so the swaps it routes
pay no DEX fees. The protocol fee (fee-registry + fee-collector) is
separate and resolves each LP holder's CL8Y tier at fill time (see
`docs/FEE_TIER_PROTOCOL.md` §2); the proxy does not need to hold CL8Y for a fee
tier.

The contracts are unaudited. Do not deploy them with economic assets before an
independent contract and oracle review.

## Verify

From the repository root:

```sh
cargo test --manifest-path rebalancer-system/Cargo.toml
cargo clippy --manifest-path rebalancer-system/Cargo.toml --all-targets -- -D warnings
make optimize
make local-e2e
```

## Documentation

- [Implementation guide](IMPLEMENTATION.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Deployment and new-pair setup](docs/DEPLOYMENT.md)
- [Admin and keeper operations](docs/ADMIN_OPERATIONS.md)
- [Swap proxy protocol](docs/protocols/SWAP_PROXY.md)
- [Bot vault protocol](docs/protocols/BOT_VAULT.md)
- [Bot liquidity protocol](docs/protocols/BOT_LIQUIDITY.md)
- [Keeper example](examples/keeper/README.md)
