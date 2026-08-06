# Rebalancer Implementation Guide

## 1. Build The Shared Types

Start in `packages/bot-types`. Define cross-contract wire types such as vault
queries/execution, swap parameters, proxy hooks, and withdrawal modes. Each
contract retains its own complete instantiate/execute/query schema. Keep pair
addresses, keeper authority, and token denominations explicit.

## 2. Implement The Swap Proxy

Implement `contracts/swap-proxy` as the only trading entry point. The admin
registers each vault with its CL8Y pair. On a swap, verify the caller and pair,
forward the offer token to the pair, and direct output back to that same vault.
This isolates user inventory while routing swaps through the whitelisted proxy
(zero DEX fee).

## 3. Implement The Vault

Implement `contracts/bot-vault` around one token pair. Store liquidity-contract
and keeper authorities separately. Accept settled deposits only from the
liquidity contract. Permit rebalancing only when the configured TWAP movement
threshold and timing rules pass, then trade through the proxy.

## 4. Implement Share Accounting

Implement `contracts/bot-liquidity` with initial NAV valuation, minimum-ratio
minting for established vaults, and pro-rata burning. Include pre-existing
balances, lock minimum liquidity on the first mint, and settle asynchronous
CW20 transfers through replies before minting. For a
single-token withdrawal, route the unwanted side through the vault and proxy.

## 5. Configure A Deployment

1. Deploy the proxy, vault, and liquidity contracts.
2. Register the vault and its CL8Y pair in the proxy.
3. Connect the vault and liquidity contract authorities.
4. Assign a dedicated keeper to the vault.
5. Whitelist the deployed proxy and vault contracts on the CL8Y DEX so they pay
   zero DEX fees.
6. Fund the proxy with the tier's required CL8Y balance.
7. Benchmark a short nonzero TWAP window, such as 30-300 seconds, and
   independently validate the pool's liquidity and oracle safety.

Detailed messages and commands are in [the deployment guide](docs/DEPLOYMENT.md)
and [protocol specifications](docs/protocols/).

## 6. Verify End To End

Run unit tests and strict Clippy first, then build optimized Wasm and use the
LocalTerra workflow. Verify deposits, share transfers, threshold enforcement,
keeper rebalances, every withdrawal mode, and multi-vault isolation before any
economic deployment.
