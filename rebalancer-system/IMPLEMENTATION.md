# Rebalancer Implementation Guide

## 1. Build The Shared Types

Start in `packages/bot-types`. Define the instantiate, execute, query, and reply
messages shared by the three contracts. Keep pair addresses, keeper authority,
and token denominations explicit so every vault can be validated independently.

## 2. Implement The Swap Proxy

Implement `contracts/swap-proxy` as the only trading entry point. The admin
registers each vault with its CL8Y pair. On a swap, verify the caller and pair,
forward the offer token to the pair, and direct output back to that same vault.
This isolates user inventory while sharing the proxy's CL8Y fee tier.

## 3. Implement The Vault

Implement `contracts/bot-vault` around one token pair. Store liquidity-contract
and keeper authorities separately. Accept settled deposits only from the
liquidity contract. Permit rebalancing only when the configured TWAP movement
threshold and timing rules pass, then trade through the proxy.

## 4. Implement Share Accounting

Implement `contracts/bot-liquidity` with NAV-based minting and pro-rata burning.
Include pre-existing balances in NAV, lock minimum liquidity on the first mint,
and settle asynchronous CW20 transfers through replies before minting. For a
single-token withdrawal, route the unwanted side through the vault and proxy.

## 5. Configure A Deployment

1. Deploy the proxy, vault, and liquidity contracts.
2. Register the vault and its CL8Y pair in the proxy.
3. Connect the vault and liquidity contract authorities.
4. Assign a dedicated keeper to the vault.
5. Have CL8Y governance register the proxy for the intended fee tier.
6. Fund the proxy with the tier's required CL8Y balance.
7. Use a nonzero TWAP window and independently validate the pool's oracle safety.

Detailed messages and commands are in [the deployment guide](docs/DEPLOYMENT.md)
and [protocol specifications](docs/protocols/).

## 6. Verify End To End

Run unit tests and strict Clippy first, then build optimized Wasm and use the
LocalTerra workflow. Verify deposits, share transfers, threshold enforcement,
keeper rebalances, every withdrawal mode, and multi-vault isolation before any
economic deployment.
