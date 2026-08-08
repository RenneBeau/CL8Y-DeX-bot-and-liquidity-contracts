# Shared Swap Proxy Protocol

Source: `contracts/swap-proxy`

## Purpose

The proxy gives many isolated bot vaults one route that can be whitelisted for
zero DEX fee. It connects every approved vault to its assigned pair and returns
each trade's output to the requesting vault. The proxy never holds tokens:
protocol fees are charged by the vaults directly and CL8Y fee tiers are resolved
by the fee-registry, so no CL8Y balance needs to live on the proxy.

## Registration

The proxy admin registers a vault and pair. Registration queries both contracts
and verifies:

- The pair reports the requested pair address.
- Both pair assets are distinct CW20 tokens.
- The vault reports the same pair, proxy, and ordered token addresses.
- The vault reports a factory registration and nonzero pinned runtime pair code
  ID; the proxy independently verifies both.
- The vault route is not already registered. Other approved vaults may use the
  same pair.

Stored routing is keyed by vault. A registered route contains its pair, pinned
pair code ID, and two permitted offer tokens.

## Swap Flow

The vault sends an offer CW20 to the proxy using `Cw20ReceiveMsg`. The proxy
validates the real CW20 contract (`info.sender`), embedded vault sender, pair,
amount, deadline, and maximum spread. It forwards the exact received amount to
the pair with output fixed to the originating vault. It first rejects a runtime
pair code ID that differs from the registered route.

The registered route fixes the recipient to the originating vault. Zero DEX fee
depends on separate CL8Y DEX whitelisting of the deployed proxy; routing alone
does not prove or grant that status.

## Administrative Authority

The admin may register or remove vault routes and transfer proxy administration.
There is no arbitrary-message execution, no token balance, and no method to
withdraw vault asset tokens.

## Invariants

- Each vault has at most one registered route; multiple vaults may share a pair.
- Only a route's two tokens can be offered.
- Swap output always returns to the route's vault.
- The exact received amount is forwarded in the same transaction.
- User accounting and bot LP supply remain in each bot's liquidity contract.

## Trust Assumptions

- Proxy administration must be controlled by a multisig.
- The production proxy must be deployed, compile-time pinned by fee-aware vaults,
  and whitelisted on the CL8Y DEX. None of those mainnet conditions is currently
  established in this repository.
- Registered CL8Y pairs must match the reviewed deployed pair implementation.
- A 0.1.x proxy containing any routes rejects migration: deploy a fresh 0.2.0
  proxy and register the redeployed vault routes. Only empty compatible 0.1.x
  proxy state may migrate.

Limit-grid is outside this protocol: it has no proxy and interacts directly with
the pair.
