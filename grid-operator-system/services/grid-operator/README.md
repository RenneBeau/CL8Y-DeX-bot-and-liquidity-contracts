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
cp services/grid-operator/.env.example services/grid-operator/.env
# Edit .env for the target chain, vaults, archive RPC, keyring, and fees.
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
batch, and advances `confirmed:<vault>`. Orders are refreshed after success to
capture changed or terminal state. New opposite orders appear only after a
separate owner `allocate` call.

A query timeout keeps the tx hash and is polled on the next keeper pass. A
broadcast call that fails or is interrupted is marked `unknown`; it is never
automatically rebroadcast because the node may have accepted it. This is an
intentional fail-closed state.

## Runbook

1. Alert if `status` reports `unknown_broadcasts`, nonzero pending work that does
   not clear, or unhealthy batch states. Compare `scanned_height` with chain tip
   externally; `status` does not currently calculate lag or event age.
2. For a `timeout`, leave the service running. It continues querying the same hash and accepts eventual inclusion.
3. For `unknown`, inspect `tx_attempts.signed_tx`, the keeper account sequence, and chain transactions around `created_at`. If included, record its hash and set the attempt/batch to `broadcast` in one reviewed SQLite transaction; the normal poller will confirm it. If definitely not accepted, set the batch back to `ready` in a reviewed transaction. Never do either based only on an RPC "not found" response.
4. On `ReorgError`, stop the service and investigate the RPC/archive provider. Finalized history is not automatically deleted or rewritten.
5. Back up the database with SQLite's online backup API or `sqlite3 .backup`; do not copy only the main file while WAL writes are active.
6. After restoring a backup, run `migrate`, `status`, then `index`. Existing event uniqueness and batch snapshots make replay idempotent.
7. CheckTx and DeliverTx failures do not move checkpoints. Correct fees, gas, sequence, contract state, or event data, then run `keep` again.
8. Configure a chain-appropriate nonempty `GRID_FEES`; the service does not infer
   production fee policy. Monitor database/WAL disk growth and archive or prune
   only through a reviewed offline procedure.

The included `grid-operator.service` creates `/var/lib/grid-operator` through
systemd `StateDirectory`. Adjust paths, user, environment, keyring access, and
fee policy for the host. Install the reviewed environment file as
`/etc/grid-operator.env` before starting the service.

## Swap-Only Vault Keeper

The limit-order flow above applies only to `cl8y-grid-vault` instances. The
swap-only design (`cl8y-grid-vault-swap`) has no limit orders: it holds CW20
balances and re-balances toward the current grid cell by paying a pair Swap
taker. Rebalance is fully permissionless, so any keeper may submit it.

`grid-operator keep-swap` polls `{"grid_status": {}}` on the configured vault
(`--vault` or `GRID_SWAP_VAULTS`, one swap vault per process) and, when the
vault reports `should_rebalance` with no swap already pending, submits
`{"rebalance": {"deadline": <now + GRID_SWAP_DEADLINE_SECONDS>}}`.

```sh
GRID_SWAP_VAULTS=<vault-address> \
GRID_SWAP_RPC_URL=http://127.0.0.1:26657 \
GRID_SWAP_CHAIN_ID=localterra \
GRID_SWAP_KEY_NAME=grid-keeper \
venv/bin/grid-operator keep-swap
```

It is a dry-run by default; pass `--broadcast` to sign and submit. State lives
in a fail-closed JSON tracker (`GRID_SWAP_STATE_FILE`): an unresolved broadcast
is never automatically rebroadcast, and a deterministic DeliverTx failure
suppresses the identical plan until the vault state changes. It self-funds its
own gas via the configured `GRID_SWAP_FEES`.

## Discovery Keeper (keep-discover)

`grid-operator keep-discover` removes the one-vault-per-process limitation:
a single process scans the chain for both kinds of CL8Y vault and keeps every
vault it finds. Each newly deployed vault is therefore kept automatically, with
no per-vault configuration.

Two kinds are discovered and kept, each matching a configured code id:

| kind       | contract               | code id env              | query / execute                         | auth                      |
|------------|------------------------|--------------------------|-----------------------------------------|---------------------------|
| `grid`     | `grid-vault-swap`      | `GRID_SWAP_CODE_ID`      | `{"grid_status": {}}` → `{"rebalance"}` | permissionless            |
| `rebalance`| `bot-vault`            | `GRID_REBALANCE_CODE_ID` | `{"rebalance_plan": {}}` → `{"rebalance"}` or `{"sync_reference"}` | keeper-restricted (`config.keeper`) |

- Discovery is event-based: the block indexer records any instantiate event
  whose `code_id` matches a configured kind into the `discovered_vaults` table.
  No on-chain registry or external vault list is required.
- The keep loop then behaves like the single-vault keepers for each enabled
  discovered vault, using the protocol for its kind. A rebalance vault with a
  pure reference-price drift (no `offer_token`) is handled with
  `{"sync_reference": {}}` instead of a swap.
- Signing is serial across all vaults — the single keeper key never signs
  concurrently. Each vault owns a fail-closed tracker file under
  `GRID_SWAP_STATE_DIR` (`<vault>.json`), preserving the same no-rebroadcast
  guarantees as the single-vault keepers.
- Grid rebalances remain fully permissionless; a grid vault with no running
  keeper simply waits for any relayer. Rebalance vaults are keeper-restricted,
  so the discovery keeper key must be the address each `bot-vault` authorizes
  in its `config.keeper`.

```sh
GRID_RPC_URL=http://127.0.0.1:26657 \
GRID_CHAIN_ID=localterra \
GRID_DEPLOYMENT_HEIGHT=100 \
GRID_SWAP_CODE_ID=123 \
GRID_REBALANCE_CODE_ID=456 \
GRID_DB_PATH=/var/lib/grid-operator/operator.sqlite3 \
venv/bin/grid-operator keep-discover --broadcast
```

## Tests

```sh
python3 -m unittest discover -s services/grid-operator/tests -v
```
