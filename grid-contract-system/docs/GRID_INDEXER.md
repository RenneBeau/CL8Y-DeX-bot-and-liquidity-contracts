# Optional Grid Indexer

The indexer is no longer an accounting authority. Contracts never accept its
input amount, output amount, fill count, price, or recipient.

The included operator currently:

- Registers vaults listed in `GRID_VAULTS` and refreshes their current order IDs.
- Scans archive Tendermint blocks for authenticated pair fill events.
- Freezes bounded, durable order-ID batches and submits permissionless
  `reconcile` calls through one fail-closed signing key.
- Persists reorg/finality checkpoints, signed attempts, and confirmation cursors.

Manager-event discovery, parked-order prompting, solvency/pause/gas/order-age
alerts, and pair-code migration alerts are operator responsibilities not yet
automated by this service.

Payload:

```json
{"reconcile":{"bot_id":1,"order_ids":[77,78]}}
```

Database loss cannot create an on-chain accounting loss or block owner recovery.
However, this service currently builds reconciliation batches from stored fill
events, so rebuilding its automated queue requires an archive rescan from
`GRID_DEPLOYMENT_HEIGHT`; current vault/order discovery alone does not recreate
past batches.

The configured keeper receives reimbursement only for a useful reconciliation.
Any other address can perform the same state transition without reimbursement,
so keeper key loss does not block settlement or recovery.
