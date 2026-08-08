# Protocol Architecture

The protocol separates fee access, asset custody, and user ownership into
three contracts. There is one shared swap proxy, one vault per strategy, and one
bot-liquidity CW20 contract per vault. Multiple approved vaults may use the same
pair route.

```text
User
  | deposit / withdraw
  v
Bot Liquidity CW20 (one per bot)
  | transfers and commands
  v
Bot Vault (one per pair)
  | CW20 send
  v
Shared Swap Proxy
  | discounted CW20 send
  v
CL8Y Pair
  | output sent directly to vault
  v
Bot Vault
```

## Asset Boundaries

- The vault stores and accounts for user token A and token B.
- The proxy routes vault swaps and does not hold CL8Y. Production requires a
  deployed, pinned, DEX-whitelisted proxy for zero DEX fees; that is not yet a
  confirmed mainnet fact.
- The liquidity contract manages deposits, withdrawals, and bot LP shares.
- Every pair has a distinct vault and a distinct fungible bot LP token.
- CL8Y pools provide swap execution and market pricing for each bot.
- Vaults store the canonical factory and approved pair runtime code ID. Factory
  registration is checked at creation, and code ID is rechecked before swaps.

## Deposit Settlement

1. A user grants the bot-liquidity contract CW20 allowances.
2. The user submits exact maximum token amounts, a deadline, `min_shares`, and
   an optional swap plan.
3. CW20 `transfer_from` messages move funds directly to the assigned vault.
4. If needed, the vault routes only the deposited offer amount through the
   shared proxy.
5. A reply queries settled vault balances and mints established-vault shares
   from the smaller proportional contribution across both assets.
6. The transaction reverts atomically if allocation, deadline, or minimum-share
   checks fail.

## Withdrawal Settlement

Claims are calculated before burning:

```text
claim_i = floor(vault_balance_i * shares / total_supply_before_burn)
```

Pro-rata withdrawals burn shares and transfer both claims at the vault's
current token ratio. Single-token withdrawals use an escrow pattern: shares are
held (not burned) while the unwanted claim is swapped, then burned on swap
success. The payout is the wanted claim plus the actual vault balance increase
produced by the swap. If the swap fails, the pending operation is cleared and
the owner keeps their shares. The withdrawing user receives the execution
result and bears its swap cost.

## Share Accounting

Token assets must have equal decimals. NAV is denominated in token 0:

```text
NAV = token0_balance + token1_balance / token1_per_token0_price
```

For an established vault, each asset implies a share amount and the smaller is
minted:

```text
shares_0 = floor(added_token0 * total_supply / pre_token0)
shares_1 = floor(added_token1 * total_supply / pre_token1)
minted_shares = min(shares_0, shares_1)
```

The first mint permanently locks 1,000 smallest share units. Any assets donated
before the first deposit also receive permanently locked shares. Later direct
donations are included in pre-deposit balances and benefit existing shareholders,
not the next depositor.

## Rebalancing

The vault records a reference token1-per-token0 TWAP. Once movement reaches 5%
by default, the contract captures one TWAP and derives the correcting side,
amount, trade cap, minimum return, and maximum spread. The reply uses that same
TWAP and exact balance deltas. Strict partial improvement commits while keeping
the old reference; reaching tolerance advances the reference to the captured
TWAP.

A 30-300 second CL8Y TWAP is a reasonable range to benchmark. Shorter windows
react faster but cost less to manipulate; longer windows are safer but can miss
profitable movement. Economically significant pools still require liquidity
checks and independent oracle validation.

## CL8Y DEX Fees

The intended production proxy must be whitelisted so routed swaps pay no DEX
fee. It is not deployed or pinned yet, so zero DEX fee is a prerequisite rather
than established mainnet state. The separate protocol fee resolves
`config.admin`, not each LP holder. It first computes economic value
`F = floor(V*bps/10000)`, then mints NAV-priced collector shares
`x = floor(F*S/(A-F))` against post-settlement value `A` and pre-mint supply
`S`. Flooring keeps the collector's immediate claim no greater than `F`. See
`../../docs/FEE_TIER_PROTOCOL.md`.

The vault caches the last successful effective bps/tier for `config.admin`.
Registry unavailability uses that exact result (`vault_cached`) or 180 bps
(`lowest`) without history. The registry never prices from historical CL8Y
holding when its live token query fails.

The provenance schema is 0.2.0. Existing 0.1.x vaults and bot-liquidity contracts
must be redeployed. A 0.1.x proxy with routes must be replaced and its routes
re-registered; only empty compatible proxy state may migrate.
