# Bot Vault Protocol

Source: `contracts/bot-vault`

## Purpose

Each bot vault stores one bot's pooled token inventory. It recognizes exactly
two ordered CW20 assets and one CL8Y pair, executes the bot's portfolio trades,
and sends proportional assets to users through its liquidity contract.

## Roles

- `admin`: configures the liquidity controller once, updates keeper, thresholds,
  and the TWAP observation window, and transfers administration.
- `liquidity_contract`: the only caller allowed to perform user-flow swaps or
  transfer underlying assets to withdrawal recipients.
- `keeper`: may perform threshold-gated inventory rebalances or synchronize a
  reference when allocation is already within tolerance.
- `proxy`: the only swap route used by the vault.

## Initialization

The vault queries pair metadata and both CW20 token records. It rejects native
assets, duplicate assets, mismatched token decimals, a zero TWAP window, and
invalid threshold or risk-control values. The liquidity-controller address can be assigned
only once.

## Configuration Updates

The admin can update `rebalance_threshold_bps`, `allocation_tolerance_bps`,
`max_trade_bps`, `max_execution_deviation_bps`, `quote_slippage_bps`,
`max_spread`, and `twap_window_seconds` at any time through `UpdateThresholds`;
omitted fields retain their current values. The TWAP window must remain
nonzero. This lets a vault adapt its observation window to changed volatility
without redeploying.

## User Operations

The assigned liquidity contract may request a swap with a fixed offer token,
amount, minimum return, maximum spread, and deadline. It may transfer only the
two configured assets. There is no generic token withdrawal method.

## Rebalancing

Price is token1 per token0. The vault derives it from CL8Y `price_a` cumulative
observations, whose orientation was verified at pinned revision `fad8011` as
reserve token1 divided by reserve token0. A single captured TWAP controls the
trigger, equal-value allocation direction and amount, execution-price floor,
and post-swap allocation check. The amount is additionally capped by
`max_trade_bps`. The minimum return is the greater of the TWAP floor using
`max_execution_deviation_bps` and the CL8Y simulation floor using
`quote_slippage_bps`; `max_spread` is also read from on-chain configuration.

The pending record contains the captured TWAP, pre-swap balances and deviation,
and complete offer. Reply requires exact offer spending, at least the planned
minimum output, and either tolerance or strict improvement. Partial improvement
outside tolerance commits but keeps the old reference. Reaching tolerance
updates the reference to the captured TWAP.

## Invariants

- One immutable pair and two ordered assets per vault.
- User withdrawals can be initiated only by the assigned liquidity contract.
- Keeper swap output returns directly to the vault.
- Bot LP minting, burning, and transfers remain under the liquidity contract.
- Failed swaps or post-swap checks roll back all nested messages atomically.
- Foreign CW20 donations are ignored because only configured assets are queried.

## Trust Assumptions

- Admin and keeper keys must be separately controlled.
- Each vault reads price from its single registered CL8Y pair.
- The keeper supplies only a deadline; all economic parameters are on-chain.

Deployment and configuration examples are in
[`docs/DEPLOYMENT.md`](../DEPLOYMENT.md) and
[`docs/ADMIN_OPERATIONS.md`](../ADMIN_OPERATIONS.md).
