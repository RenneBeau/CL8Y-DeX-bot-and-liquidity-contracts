# Shared Swap Proxy Protocol

Source: `contracts/swap-proxy`

## Purpose

The proxy gives many isolated bot vaults access to one whitelisted CL8Y fee
registration. It connects every approved vault to its assigned pair and returns
each trade's output to the requesting vault. The proxy never holds tokens:
protocol fees are charged by the vaults directly and CL8Y fee tiers are resolved
by the fee-registry, so no CL8Y balance needs to live on the proxy.

## Registration

The proxy admin registers a vault and pair. Registration queries both contracts
and verifies:

- The pair reports the requested pair address.
- Both pair assets are distinct CW20 tokens.
- The vault reports the same pair, proxy, and ordered token addresses.
- No other vault is registered for that pair.

Stored routing is keyed by vault. A registered route contains only its pair and
two permitted offer tokens.

## Swap Flow

The vault sends an offer CW20 to the proxy using `Cw20ReceiveMsg`. The proxy
validates the real CW20 contract (`info.sender`), embedded vault sender, pair,
amount, deadline, and maximum spread. It forwards the exact received amount to
the pair with output fixed to the originating vault.

The registered route fixes the recipient to the originating vault and grants
zero-fee access exclusively to registered vault contracts.

## Administrative Authority

The admin may register or remove vault routes and transfer proxy administration.
There is no arbitrary-message execution, no token balance, and no method to
withdraw vault asset tokens.

## Invariants

- One registered vault per pair.
- Only a route's two tokens can be offered.
- Swap output always returns to the route's vault.
- The exact received amount is forwarded in the same transaction.
- User accounting and bot LP supply remain in each bot's liquidity contract.

## Trust Assumptions

- Proxy administration must be controlled by a multisig.
- The swap-proxy itself must be whitelisted on the CL8Y DEX so the swaps it
  routes pay no DEX fee.
- Registered CL8Y pairs must match the reviewed deployed pair implementation.
