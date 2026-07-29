# Bot Vault Protocol

Source: `contracts/bot-vault`

## Purpose

Each bot vault stores one bot's pooled token inventory. It recognizes exactly
two ordered CW20 assets and one CL8Y pair, executes the bot's portfolio trades,
and sends proportional assets to users through its liquidity contract.

## Roles

- `admin`: configures the liquidity controller once, updates keeper and
  thresholds, and transfers administration.
- `liquidity_contract`: the only caller allowed to perform user-flow swaps or
  transfer underlying assets to withdrawal recipients.
- `keeper`: may perform threshold-gated inventory rebalances or synchronize a
  reference when allocation is already within tolerance.
- `proxy`: the only swap route used by the vault.

## Initialization

The vault queries pair metadata and both CW20 token records. It rejects native
assets, duplicate assets, mismatched token decimals, empty pool reserves, and
invalid threshold values. The liquidity-controller address can be assigned
only once.

## User Operations

The assigned liquidity contract may request a swap with a fixed offer token,
amount, minimum return, maximum spread, and deadline. It may transfer only the
two configured assets. There is no generic token withdrawal method.

## Rebalancing

Price is token1 per token0. With a nonzero TWAP window, the vault computes price
from CL8Y cumulative observations. A keeper swap is permitted only when price
has moved at least `rebalance_threshold_bps` from the stored reference.

After execution, the reply compares vault holdings with the current ordered
pool reserve ratio. The transaction reverts unless allocation improved or is
inside `allocation_tolerance_bps`. Only then is the reference price updated.

## Invariants

- One immutable pair and two ordered assets per vault.
- User withdrawals can be initiated only by the assigned liquidity contract.
- Keepers cannot transfer assets to themselves.
- Rebalances cannot mint, burn, or transfer bot LP shares.
- Failed swaps or post-swap checks roll back all nested messages atomically.
- Foreign CW20 donations are ignored because only configured assets are queried.

## Trust Assumptions

- Admin and keeper keys must be separately controlled.
- Mainnet deployments require a manipulation-resistant price configuration.
- Keeper-provided swap parameters remain bounded by deadline, spread, token,
  pair, and post-allocation checks.
