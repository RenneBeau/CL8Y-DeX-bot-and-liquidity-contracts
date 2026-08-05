# Grid Operator & Protocol (Shared)

This directory is the shared **operator service and documentation** harness for
the two grid ecosystems. It contains no Rust contract crates; those live in two
separate Cargo workspaces at the repository root:

- [`market-grid-system`](../market-grid-system/README.md) — the **deployable**
  standard swap grid.
- [`limit-grid-system`](../limit-grid-system/README.md) — the limit-order grid,
  which reconciles against the shipped pair via its own cancel ledger and the
  "unknown means fully executed" classification.

Contents:

- `services/grid-operator` — discovery and transaction automation.
  `indexer.py`/`keeper.py` drive the reference limit-order grid, and
  `swap_keeper.py` drives the standard swap grid.
- `docs/` — protocol, operations, and indexer documentation.
- `IMPLEMENTATION.md` — implementation status and production-readiness notes.

The operator never receives CW20 deposits. It is optional automation on top of
the permissionless, fail-closed contracts.

Guides:

- [Implementation status](IMPLEMENTATION.md)
- [Protocol and threat model](docs/GRID_MANAGER_PROTOCOL.md)
- [Operations](docs/GRID_OPERATIONS.md)
- [Optional indexer](docs/GRID_INDEXER.md)

Run the shared operator tests:

```sh
python3 -m unittest discover -s grid-operator-system/services/grid-operator/tests -p 'test_*.py'
```

This remains pre-production code. Real mainnet-equivalent CL8Y adversarial
validation, an external security review, and staged limited-value rollout remain
required.