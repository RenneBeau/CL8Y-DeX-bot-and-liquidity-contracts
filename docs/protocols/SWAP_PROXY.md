# Shared Swap Proxy Protocol

Source: `contracts/swap-proxy`

## Purpose

The proxy allows many isolated bot vaults to share one CL8Y fee registration.
It holds the protocol's CL8Y balance and forwards approved vault swaps to the
vault's immutable CL8Y pair.

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

The caller cannot select an arbitrary recipient or route. An unregistered
contract or wallet cannot use the proxy's discount.

## Administrative Authority

The admin may register or remove vault routes, transfer proxy administration,
and withdraw only the configured CL8Y token. There is no arbitrary-message
execution or method to withdraw vault asset tokens.

## Invariants

- One registered vault per pair.
- Only a route's two tokens can be offered.
- Swap output always returns to the route's vault.
- The exact received amount is forwarded in the same transaction.
- User accounting and bot LP supply never exist in this contract.

## Trust Assumptions

- Proxy administration must be controlled by a multisig.
- CL8Y registry governance must assign the desired tier.
- The proxy must retain the CL8Y balance required by that tier.
- Registered CL8Y pairs must match the reviewed deployed pair implementation.
