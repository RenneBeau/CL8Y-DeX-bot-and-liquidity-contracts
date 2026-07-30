# CL8Y Portfolio Rebalancer

This Cargo workspace contains the production-oriented rebalancing system. Each
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
token donations are included in pre-deposit NAV. Deposits, withdrawals, and
rebalances currently use a zero protocol charge.

## Price And Fee Tier

Vaults support CL8Y arithmetic TWAP through `twap_window_seconds`. LocalTerra
uses `0` for spot-price testing; an economic deployment should normally use at
least 1,800 seconds and independent oracle validation.

The proxy cannot self-register with CL8Y. Fee-registry governance must register
the deployed proxy and the proxy must hold the selected tier's minimum CL8Y
balance.

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
