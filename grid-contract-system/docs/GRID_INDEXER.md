# Optional Grid Indexer

The indexer is no longer an accounting authority. Contracts never accept its
input amount, output amount, fill count, price, or recipient.

Its optional duties are:

- Discover vaults from manager `register_grid_vault` events.
- Discover order IDs from vault placement events and `orders` queries.
- Detect changes to active remaining escrow and submit bounded permissionless
  `reconcile` calls containing only order IDs.
- Detect parked/expired orders and prompt claims or owner exit.
- Monitor pair pause, vault gas credit, order age, and transaction finality.

Payload:

```json
{"reconcile":{"bot_id":1,"order_ids":[77,78]}}
```

Durable persistence, reorg handling, finality checkpoints, and event deduplication
remain operationally useful, but database loss cannot create an accounting loss.
The service can rebuild current work from manager/vault state and active pair
queries. Historical fill events are optional analytics only.

The configured keeper receives reimbursement only for a useful reconciliation.
Any other address can perform the same state transition without reimbursement,
so keeper key loss does not block settlement or recovery.
