# Grid Operator

This service is a durable indexer and single-key keeper for dedicated grid vault
instances. Every address in `GRID_VAULTS` is queried as bot `1`; its pair and
orders are discovered from the contract rather than inferred from order IDs.
Pair-local identity is always `(pair_address, order_id)`.

It uses only Python's standard library and `terrad`. SQLite runs in WAL mode with
`FULL` synchronous commits. The database retains blocks, exact raw fill events,
vault/order mappings, rebuilt aggregates, immutable batch snapshots, signed tx
attempts, and scan/confirmation cursors.

## Install And Run

Python 3.10+, an archive Tendermint RPC, and a compatible `terrad` are required.

```sh
python3 -m venv venv
venv/bin/pip install ./services/grid-operator
set -a; . services/grid-operator/.env; set +a
venv/bin/grid-operator migrate
venv/bin/grid-operator status
venv/bin/grid-operator index
venv/bin/grid-operator keep
venv/bin/grid-operator run
```

`index` fetches both `/block` and `/block_results`, starts at
`GRID_DEPLOYMENT_HEIGHT`, and only scans through `latest-finality_depth`. The
same `GRID_FINALITY_DEPTH` also gates transaction confirmation: a code-zero
reconcile remains pending and is re-queried until that many later blocks exist.
The indexer revalidates the last observed finalized hash on every pass and refuses to
continue on a hash or parent-link mismatch. Successful transaction hashes are
SHA-256 hashes of the decoded Tendermint transaction bytes.

## Transaction Safety

One bounded batch is frozen per vault. The exact contributing raw event IDs are
stored before signing. The signed transaction and then a `broadcasting` marker
are committed before broadcast. A successful CheckTx only changes the attempt
to `broadcast`; the service then polls `query tx` for DeliverTx. Only DeliverTx
code zero atomically marks events reconciled, rebuilds aggregates, confirms the
batch, and advances `confirmed:<vault>`. Orders are refreshed after success so
opposite orders created by reconciliation can be indexed.

A query timeout keeps the tx hash and is polled on the next keeper pass. A
broadcast call that fails or is interrupted is marked `unknown`; it is never
automatically rebroadcast because the node may have accepted it. This is an
intentional fail-closed state.

## Runbook

1. Alert if `status` reports `unknown_broadcasts`, finalized scan lag, old pending events, or repeated transaction errors.
2. For a `timeout`, leave the service running. It continues querying the same hash and accepts eventual inclusion.
3. For `unknown`, inspect `tx_attempts.signed_tx`, the keeper account sequence, and chain transactions around `created_at`. If included, record its hash and set the attempt/batch to `broadcast` in one reviewed SQLite transaction; the normal poller will confirm it. If definitely not accepted, set the batch back to `ready` in a reviewed transaction. Never do either based only on an RPC "not found" response.
4. On `ReorgError`, stop the service and investigate the RPC/archive provider. Finalized history is not automatically deleted or rewritten.
5. Back up the database with SQLite's online backup API or `sqlite3 .backup`; do not copy only the main file while WAL writes are active.
6. After restoring a backup, run `migrate`, `status`, then `index`. Existing event uniqueness and batch snapshots make replay idempotent.
7. CheckTx and DeliverTx failures do not move checkpoints. Correct fees, gas, sequence, contract state, or event data, then run `keep` again.

The included `grid-operator.service` is a hardened systemd example. Adjust paths,
user, environment, keyring access, and fee policy for the host.

## Tests

```sh
python3 -m unittest discover -s services/grid-operator/tests -v
```
