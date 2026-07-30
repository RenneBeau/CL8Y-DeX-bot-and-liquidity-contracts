# Grid System Implementation Guide

## 1. Model Bot-Scoped State

Key balances, shares, gas credit, order records, and configuration by `bot_id`.
The manager owns all CL8Y orders, so every execute path must resolve an order
back to both its bot and pair before changing state.

## 2. Validate Standard Pairs

At bot creation, query the configured CL8Y factory and accept only registered
CW20/CW20 pairs with distinct assets and equal decimals. Require nonzero pool
reserves and a current price strictly inside the requested bounds.

## 3. Allocate The Initial Grid

Use arithmetic spacing between the lower and upper prices. Divide token0 across
ask rungs and token1 across bid rungs. Keep integer remainders in the bot's free
balance. Record each returned pair-local order ID with its side, price, and
remaining input.

## 4. Reconcile Indexed Fills

Run one archive-capable indexer for all supported pairs. Aggregate exact
`limit_order_fill` events by order into `input_amount`, `output_amount`, and
`fill_count`. Every report also includes its pair address. One dedicated keeper
submits those reports. The contract verifies
keeper authority, order ownership, side, remaining escrow, consumed input, and
CL8Y aggregate rounding bounds before crediting output and placing the opposite
order. If a single opposite placement fails or is skipped, reconciliation still
commits and its output returns to the bot's free balance for later allocation.

This trust boundary is required because unchanged standard pairs remove
completed orders and contracts cannot query historical transaction events.

## 5. Fund Operations And Exits

Require separate prepaid LUNC gas credit for each bot. Reimburse the keeper only
after useful work, subject to the configured cap and emergency reserve. Process
reconciliation and cancellation in bounded pages. Do not allow withdrawal until
all indexed fills are reconciled and active orders are cancelled.

## 6. Verify Isolation

Test multiple bots on multiple pairs with one indexer and keeper. Confirm that an
unauthorized reporter is rejected, each fill changes only its owning bot, gas
credits remain separate, partial fills preserve remaining orders, and every bot
can cancel and withdraw independently.

See [the protocol](docs/GRID_MANAGER_PROTOCOL.md), [indexer guide](docs/GRID_INDEXER.md),
and [operations guide](docs/GRID_OPERATIONS.md) for complete interfaces and
deployment requirements.
