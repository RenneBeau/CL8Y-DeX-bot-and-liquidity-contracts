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
code zero atomically marks all still-unconfirmed events for the reconciled
pair/order identities through the verified execution height, including fills
indexed after freeze. Higher events remain pending. It then rebuilds aggregates,
confirms the batch, and advances `confirmed:<vault>`. Orders are refreshed after success to
capture changed or terminal state. New opposite orders appear only after a
separate owner `allocate` call.

A query timeout keeps the tx hash and is polled on the next keeper pass. A
broadcast call that fails or is interrupted is marked `unknown`; it is never
automatically rebroadcast because the node may have accepted it. This is an
intentional fail-closed state.

Swap/discovery JSON trackers use checksummed atomic replacement, three rotating
backups, and timestamped corruption quarantine. `diagnose`, reason-specific
`resolve`, and verified restore operations hold the process lock, query live
chain/account/vault state, create backups, and append audit records. SQLite
diagnosis checks integrity and foreign keys; its restore path verifies backup
integrity, stored identity, unresolved transactions, and live state before
atomic replacement. Unknown broadcasts are never eligible for clear or replay.

Startup takes a nonblocking OS `flock` beside the SQLite database or JSON state
file and holds its descriptor open. A second process using the same state exits
without querying, signing, or broadcasting. State is bound to its schema,
chain, vault set, resolved signer address/key identity, and protocol kind.
Identity changes are refused. Existing nonempty version-3 SQLite databases and
legacy JSON files without this identity are not guessed or deleted: preserve a
backup, verify all unresolved transactions, and perform an explicitly reviewed
offline migration before starting this version.

## Runbook

1. Alert if `status` reports `unknown_broadcasts`, nonzero pending work that does
   not clear, or unhealthy batch states. Compare `scanned_height` with chain tip
   externally; `status` does not currently calculate lag or event age.
2. For a `timeout`, leave the service running. It continues querying the same hash and accepts eventual inclusion.
3. Run `grid-operator diagnose [--batch-id ID]` for SQLite integrity, foreign-key,
   unresolved transaction, current vault/order, and account-sequence evidence.
   It writes an `operator_audit` record. An `unknown` broadcast is never cleared
   or rebroadcast from a not-found result.
4. After correcting a recorded deterministic intervention, run
   `grid-operator clear-intervention --batch-id ID --reason check_failed|deliver_failed|page_reverted`.
   The command holds the process lock, checks the exact recorded reason, queries
   transaction/vault/orders/account state, creates an online timestamped backup,
   and audits the reason-specific clear. Ambiguous evidence is refused.
5. `grid-operator restore --backup PATH` is explicit disaster recovery. It
   verifies backup integrity and identity plus live account/vault/order state,
   refuses backups containing unknown broadcasts, preserves the current file,
   restores atomically, and records an audit event.
6. On `ReorgError`, stop the service and investigate the RPC/archive provider. Finalized history is not automatically deleted or rewritten.
7. Back up the database with SQLite's online backup API or `sqlite3 .backup`; do not copy only the main file while WAL writes are active.
8. Configure a chain-appropriate nonempty `GRID_FEES`; the service does not infer
   production fee policy. Monitor database/WAL disk growth and archive or prune
   only through a reviewed offline procedure.

The included `grid-operator.service` creates `/var/lib/grid-operator` through
systemd `StateDirectory`, including a service-specific Terra home. Interactive
operation may use the `os` keyring. Production service signing uses
`GRID_SIGNER_COMMAND_JSON`, a JSON argv array executed without a shell. The
signer receives strict JSON on stdin: `{"version":1,"action":"address","chain_id":"..."}`
and returns only `{"address":"..."}`, or receives action `sign` with signer and
base64 `unsigned_tx` and returns only base64 `signed_tx`. It should read its key
from systemd's `CREDENTIALS_DIRECTORY/signer-key`; secrets must never be returned,
logged, placed in argv, or put in environment files. The service does not use
the insecure `test` backend. Install `/etc/grid-operator.env` before startup.
Provisioning and independently testing the host external signer and its systemd
credential is a deployment prerequisite; repository configuration cannot supply
production key material.

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
