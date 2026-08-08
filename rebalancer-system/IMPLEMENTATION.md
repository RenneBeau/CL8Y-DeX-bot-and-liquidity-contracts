# Rebalancer Implementation Guide

## 1. Build The Shared Types

Start in `packages/bot-types`. Define cross-contract wire types such as vault
queries/execution, swap parameters, proxy hooks, and withdrawal modes. Each
contract retains its own complete instantiate/execute/query schema. Keep pair
addresses, keeper authority, and token denominations explicit.

## 2. Implement The Swap Proxy

Implement `contracts/swap-proxy` as the only trading entry point. The admin
registers each vault with its CL8Y pair; multiple vaults may route through the
same pair. On a swap, verify the caller and pair,
forward the offer token to the pair, and direct output back to that same vault.
This isolates user inventory. Zero DEX fee requires the deployed production
proxy to be pinned and separately whitelisted; that mainnet state is not yet
established.

## 3. Implement The Vault

Implement `contracts/bot-vault` around one token pair. Store liquidity-contract
and keeper authorities separately. Accept settled deposits only from the
liquidity contract. Permit rebalancing only when the configured TWAP movement
threshold and timing rules pass, then trade through the proxy. Require factory
pair lookup and an approved runtime pair code ID at creation, and recheck that
code before rebalance/proxy execution.

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
5. Pin the deployed production swap-proxy and verify its CL8Y DEX whitelist so
   routed swaps pay zero DEX fees. Treat this as an unfulfilled deployment
   prerequisite until verified on mainnet.
6. Benchmark a short nonzero TWAP window, such as 30-300 seconds, and
   independently validate the pool's liquidity and oracle safety.
7. For 0.2.0, redeploy 0.1.x vault/liquidity instances. Replace a routed 0.1.x
   proxy and re-register routes; migrate only empty compatible proxy state.

Detailed messages and commands are in [the deployment guide](docs/DEPLOYMENT.md)
and [protocol specifications](docs/protocols/).

## 6. Verify End To End

Run unit tests and strict Clippy first, then build optimized Wasm and use the
LocalTerra workflow. Verify deposits, share transfers, threshold enforcement,
keeper rebalances, every withdrawal mode, and multi-vault isolation before any
economic deployment.

## 7. Protocol Fee Shares

After a completed rebalance, normalize executed value to token 0, compute
`F = floor(V*bps/10000)`, and mint
`x = floor(F*S/(A-F))` collector shares against post-settlement NAV `A` and
pre-mint supply `S`. This prices the fee economically at current NAV; flooring
ensures the collector's immediate claim cannot exceed `F`.
