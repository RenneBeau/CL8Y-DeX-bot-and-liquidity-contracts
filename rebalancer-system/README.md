# CL8Y Portfolio Rebalancer

This Cargo workspace contains the portfolio rebalancing system. Each
bot keeps one CW20 token pair in an isolated vault and gives depositors
transferable CW20 LP shares. A shared swap proxy holds CL8Y so approved vaults
can use the proxy's governance-assigned fee tier.

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

Vaults support CL8Y arithmetic TWAP through `twap_window_seconds`. LocalTerra
uses `0` for spot-price testing. A short-term strategy can start by testing a
30-300 second window so meaningful swings trigger quickly while averaging
multiple blocks. The final value must be chosen from pair liquidity, block
time, volatility, and manipulation-cost measurements.

CL8Y fee-registry governance registers the deployed proxy, and the proxy holds
the selected tier's minimum CL8Y balance.

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
