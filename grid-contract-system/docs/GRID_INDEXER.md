# Grid Indexer Protocol

## Purpose

The trusted grid indexer archives exact maker-fill history for completed and
active standard CL8Y orders. It watches every supported pair and delivers
bounded aggregate reports to the one trusted grid keeper.

The indexer serves as the keeper's authenticated off-chain event source. The
contract authenticates the keeper address, while operators authenticate the
indexer-to-keeper channel.

## Event Selection

Consume finalized pair Wasm events with:

```text
action = limit_order_fill
maker = grid manager address
```

Map each event by the globally unique tuple `(pair_address, order_id)` because
each pair maintains its own order-ID sequence.

Canonical amount mapping:

| Side | Report input | Report output |
|---|---|---|
| Ask | `token0_amount` | `token1_amount` |
| Bid | `token1_amount` | `token0_amount` |

Do not reverse price arithmetic to derive output. CL8Y floors each fill, making
the inverse ambiguous.

## Durable Identity

Recommended event primary key:

```text
(chain_id, pair_address, tx_hash, event_index, order_id)
```

Persist:

- Manager address and chain ID
- Pair address and token order
- Pair/order to bot/rung mapping
- Last finalized scanned height
- Last successfully reconciled event per order
- Aggregate input, output, and fill count after that checkpoint
- Terminal, parked, cancelled, and claimed observations

New order mappings come from manager placement transactions and the manager's
`orders` query. Opposite orders created during reconciliation must be indexed
before they can later fill.

## Aggregation

Aggregate every unreconciled event for one order into constant-size values:

```text
input_amount  = sum(mapped event input)
output_amount = sum(mapped event output)
fill_count    = number of mapped fill events
```

Delivery envelope:

```json
{
  "chain_id": "columbus-5",
  "manager": "<GRID_MANAGER>",
  "pair": "<CL8Y_PAIR>",
  "bot_id": 42,
  "through_height": 12345678,
  "reports": [{
    "pair": "<CL8Y_PAIR>",
    "order_id": 77,
    "input_amount": "100",
    "output_amount": "200",
    "fill_count": 3
  }]
}
```

Only `bot_id` and `reports` are included in the on-chain message. Each report's
`pair` must equal the bot's configured pair.

## Keeper Delivery

The keeper should:

1. Read finalized reports from the indexer.
2. Group each on-chain call by one `bot_id`.
3. Bound each call by `max_orders_per_reconcile`.
4. Simulate against current manager and pair state.
5. Broadcast serially from the single keeper wallet.
6. Wait for successful inclusion.
7. Atomically advance indexer checkpoints only after success.
8. Re-read finalized events and retry if pair escrow changed before execution.

The same loop scans all admitted pairs and serves every bot with one keeper key
and one logical indexer. Operators may scale that indexer internally for
throughput.

## Terminal Orders

When an active order query disappears:

- Query `expired_limit_refund`.
- If a refund exists, include all fills before parking. If there were no fills,
  submit zero amounts and `fill_count: 0` so the manager can claim the refund.
- If no refund exists, include every uncheckpointed fill through full completion.

Archive-capable event retention is required. Losing full-order fill history can
make exact reconciliation and owner cancellation impossible.

## Finality And Recovery

- Define a chain-specific confirmation/finality policy.
- Backfill from the last durable height after any disconnect.
- Deduplicate using the durable primary key.
- Keep checkpoints separate for scanned, delivered, and confirmed-on-chain.
- Never advance confirmed state for a failed or reverted reconciliation.
- Preserve raw events long enough to rebuild all aggregates independently.

## Monitoring

Alert on:

- Finalized event lag
- Oldest unreconciled fill age
- Pair/order mappings missing a bot ID
- Aggregate input differing from manager escrow delta
- Keeper transaction failure or sequence mismatch
- Missing archived fills for completed orders
- Bot gas credit below the next reimbursement requirement
- Active-order cap causing deferred allocation

## Security

Use authenticated transport between indexer and keeper, least-privilege service
accounts, durable backups, and independent reconciliation of raw events against
pair state. The contract validates escrow and rounding bounds, but exact output
and fill count within those bounds ultimately depend on this trusted event
history. Because physical token custody is pooled, incorrect trusted reports can
affect global manager solvency.
