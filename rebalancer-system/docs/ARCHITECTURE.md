# Protocol Architecture

The protocol separates fee access, asset custody, and user ownership into
three contracts. There is one shared swap proxy, one vault per trading pair,
and one bot-liquidity CW20 contract per vault.

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
- The proxy holds CL8Y for its fee tier and routes vault swaps.
- The liquidity contract manages deposits, withdrawals, and bot LP shares.
- Every pair has a distinct vault and a distinct fungible bot LP token.
- CL8Y pools provide swap execution and market pricing for each bot.

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

Pro-rata withdrawals transfer both claims at the vault's current token ratio.
Single-token withdrawals swap exactly the unwanted claim and pay the wanted
claim plus the actual vault balance increase produced by the swap. The
withdrawing user receives the execution result and bears its swap cost.

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

## CL8Y Discount

The shared proxy holds CL8Y and is the effective trader seen by CL8Y pairs.
Fee-registry governance registers the proxy by executing:

```json
{
  "register_wallet": {
    "wallet": "<SWAP_PROXY>",
    "tier_id": 5
  }
}
```

Standard tiers continue checking the proxy's CL8Y balance during discount
queries. Pair-side discount values may remain cached for up to 300 seconds.
