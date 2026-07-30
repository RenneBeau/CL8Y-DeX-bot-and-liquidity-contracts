# Grid Manager Protocol

## Status And Purpose

`grid-contract-system/contracts/grid-manager` is an experimental multi-user
limit-order grid manager for standard CL8Y DEX pairs. One contract can hold many
independent bots across many factory-registered pairs while the manager address
owns every CL8Y order and receives one governance-assigned CL8Y fee tier.

The manager trades through standard CL8Y pairs using their existing interfaces.
A trusted off-chain indexer archives exact fill events, including completed-order
maker output, and one trusted grid keeper relays bounded reports on-chain.

## Roles

- `admin` rotates the one global grid keeper.
- `keeper` is the only address authorized to submit indexed fill reports.
- `indexer` is the authenticated off-chain source of exact fill history.
- `bot owner` creates, funds, allocates, cancels, and withdraws one bot.
- CL8Y fee-registry governance registers the manager address for a tier.

The grid keeper is independent from every rebalance-vault keeper. One grid
keeper serves every grid `bot_id` and every admitted pair.

## Initialization

```json
{
  "admin": "<ADMIN_MULTISIG>",
  "keeper": "<GRID_KEEPER>",
  "factory": "<CL8Y_FACTORY>",
  "gas_denom": "uluna",
  "keeper_reward": "30000000",
  "minimum_gas_reserve": "30000000",
  "max_grid_count": 20,
  "max_orders_per_reconcile": 20,
  "max_active_orders_per_bot": 100
}
```

`max_active_orders_per_bot` must be at least `max_grid_count`. A bot must be
created with at least `minimum_gas_reserve + keeper_reward` in `gas_denom`.

## Pair Admission

`create_bot` accepts a pair only when all checks pass:

- The configured CL8Y factory returns the same pair for its two assets.
- Both assets are distinct CW20 contracts.
- Both CW20s use the same decimals.
- Both assets are reviewed exact-transfer CW20 implementations; fee-on-transfer,
  rebasing, and tokens that can fabricate receive amounts are unsupported.
- Pool assets match pair assets in the same token0/token1 order.
- Both pool reserves are nonzero.
- `grid_count` is within the manager and pair batch-rung limits.
- The current token1-per-token0 price lies strictly inside the bounds.
- Every generated rung is within the standard pair's price bounds and distinct.
- At least one initial bid and one initial ask rung exist.

The current prototype uses arithmetic price spacing including both bounds.
Prices below the creation reference become bids, prices above it become asks,
and an exactly equal rung starts neutral.

## Bot Isolation

Every bot has a unique `bot_id` and separate state for:

- Owner
- Pair and token addresses
- Bounds, reference price, and rungs
- Free token balances
- Internal bot LP shares
- LUNC gas credit
- Active CL8Y order records

Physical CW20 custody is shared by the manager address. Logical isolation
therefore relies on correct indexed fill attribution and the trusted keeper.
The contract validates reports against standard pair state and arithmetic before
changing a bot ledger.

## Deposits And Shares

Only the bot owner may deposit. Deposits use CW20 `Send` with:

```json
{"deposit":{"bot_id":1}}
```

Token0 is automatically divided by the number of sell-token0 ask rungs. Token1
is divided by the number of sell-token1 bid rungs. Integer remainder stays free.

Internal shares are bot-specific and non-transferable:

```text
token0 shares = token0 deposit
token1 shares = floor(token1 deposit / creation reference price)
```

This owner-only prototype uses the immutable creation reference price. Support
for unrelated depositors requires complete free-plus-escrow NAV accounting.

## Order Placement

The manager sends token0 to the pair for asks and token1 for bids using standard
`place_limit_order_batch` hooks. Gross free balance is reserved before the
submessage. Its reply extracts each `limit_order_placed` ID and queries the
standard `limit_order` endpoint to record actual post-maker-fee escrow.

Order slots are reserved when placement is scheduled. If the active-order cap
would be exceeded, output remains free and the response includes
`allocation_deferred=active_order_limit`.

## Indexed Reconciliation

The trusted indexer aggregates all new fill events for one order since its last
successful checkpoint:

```json
{
  "order_id": 77,
  "input_amount": "100",
  "output_amount": "200",
  "fill_count": 3
}
```

Amount mapping follows CL8Y pair token order:

- Ask: input is `token0_amount`; output is `token1_amount`.
- Bid: input is `token1_amount`; output is `token0_amount`.

One `reconcile` transaction contains reports for one bot only:

```json
{
  "reconcile": {
    "bot_id": 1,
    "reports": [{
      "pair": "<CL8Y_PAIR>",
      "order_id": 77,
      "input_amount": "100",
      "output_amount": "200",
      "fill_count": 3
    }]
  }
}
```

The contract verifies:

- Caller equals the configured keeper.
- Report count is nonzero and bounded.
- Every report names the bot's configured pair.
- Every order ID is unique and belongs to the specified bot.
- Pair-reported owner, side, price, and remaining escrow match recorded state.
- Reported input equals the exact escrow decrease.
- Amounts are positive for nonzero fill counts.
- Aggregate output lies within CL8Y per-fill floor-rounding bounds.
- Zero reports are accepted only for terminal parked orders with no fills.

For `n` indexed fills, summing individual floors differs from flooring the
aggregate by less than `n` smallest units. The manager verifies:

```text
indexed_floor <= aggregate_floor
aggregate_floor - indexed_floor < fill_count
```

The indexer's exact output and fill count remain trusted because several valid per-fill
decompositions can share the same aggregate escrow decrease.

All bots share the manager's physical CW20 custody. A compromised keeper or
indexer can over-credit within the accepted aggregate rounding envelope and
therefore threaten other bots' solvency. Bot isolation is logical, not a
cryptographic replacement for trustworthy event history.

## Opposite Orders

After a valid partial fill:

- The unfilled original order remains active.
- Exact indexed output is credited to that bot.
- Ask output creates a bid one rung lower.
- Bid output creates an ask one rung higher.
- Only the filled output amount funds the opposite order.
- If the pair rejects or skips a single opposite placement, settlement remains
  committed and the output returns to the bot's free balance.

Multiple indexed fills are aggregated into one opposite placement, keeping the
on-chain report and transaction size bounded.

## Full And Parked Orders

If `limit_order` no longer exists, the manager queries
`expired_limit_refund`:

- A refund row means the order is parked. The manager credits the refund,
  submits `claim_expired_limit_orders`, and removes its local order.
- No refund row means the indexed order is fully filled. A report covering the
  entire remaining escrow is required.

Indexer archival history is mandatory because a completed standard order no
longer exposes its maker output.

## Cancellation And Withdrawal

The owner calls `cancel_all` repeatedly. Each call processes at most
`max_orders_per_reconcile` records and chunks pair messages by that pair's batch
limit. Cancellation succeeds only when pair remaining escrow equals the last
reconciled amount. Any intervening fill produces `UnsettledOrder`; the keeper
must reconcile indexed events first.

Withdrawal requires zero active orders. Shares burn against free balances:

```text
claim_i = floor(free_balance_i * burned_shares / total_shares)
```

## Gas Accounting

Each bot prepays an isolated native gas credit. The keeper signs and initially
pays reconciliation gas. A successful useful reconciliation reimburses one
fixed `keeper_reward` from that bot. No-op or invalid reports receive nothing.

An active or funded bot retains `minimum_gas_reserve + keeper_reward` when the
owner withdraws gas. The keeper funds the gas cost of failed transactions.

## Execute Messages

- `create_bot`: create one owner-controlled strategy.
- `receive/deposit`: credit and automatically allocate one CW20 asset.
- `fund_gas`: add native keeper credit to any bot.
- `withdraw_gas`: owner recovery subject to reserve rules.
- `allocate`: place residual free balances across configured sides.
- `reconcile`: keeper-only indexed fill settlement and opposite placement.
- `cancel_all`: bounded owner cancellation after reconciliation.
- `withdraw`: burn internal shares and transfer free assets.
- `update_keeper`: admin rotation of the global grid keeper.

## Queries

- `config`: global roles and safety limits.
- `bot`: pair, balances, shares, gas, and active-order count.
- `rungs`: bot prices and initial sides.
- `orders`: tracked CL8Y IDs and remaining escrow.
- `shares`: one address's internal shares for one bot.

## Trust And Scope

One keeper and one indexer are operationally sufficient for all bots across all
compatible pairs. Throughput is bounded by keeper signing rate, chain block
capacity, and configured reports per transaction. They are global availability
and trust dependencies. The system currently supports factory-registered
CW20/CW20 pairs with equal decimals and the tested standard CL8Y event schema.
