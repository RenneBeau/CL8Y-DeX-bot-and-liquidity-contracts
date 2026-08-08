# LocalTerra Integration Area

The harness pins CL8Y DEX revision
`fad801117fe54420d7529da04e485d67d511ef2c`, starts its official LocalTerra
image, deploys the minimal EMBER/CORAL DEX, and then deploys the shared proxy,
one isolated bot vault, and its CW20 bot-liquidity contract.
The harness deploys the experimental grid manager and four one-owner,
one-bot-per-vault instances against the
unchanged pinned CL8Y pairs. The upstream `wallet` seed supplies
`EMBER/CORAL`, `LUNC-C/EMBER`, and `USTC-C/CORAL`; grid E2E uses the first two.
LocalTerra uses a dedicated `gridkeeper` key; rebalance operations continue to
use the separately configured vault keeper.

The harness stores its generated CL8Y checkout under the ignored
`test-area/.cache/` directory.
Contracts are built with `cosmwasm/workspace-optimizer:0.16.1` for Terra Classic
VM compatibility. LCD/RPC use `1317`/`26657`; optional gRPC is remapped to
`29090`/`29091`.

```sh
make local-setup  # start LocalTerra and deploy DEX plus clean bot system
make local-test   # run tests against the current deployment
make local-grid   # run signed grid order and isolation scenarios
make local-e2e    # redeploy bot contracts, then run signed E2E scenarios
make local-fee-e2e # run strict fee/proxy/collector/treasury scenarios
make local-soak   # redeploy and run 25 inventory-rebalance rounds
make local-all    # one deploy followed by E2E and soak suites
make local-stop   # stop services while retaining state
make local-reset  # delete local chain and database state
```

`local-reset` retains the managed source checkout and Docker optimizer caches.
Remove those separately only when intentionally forcing a cold rebuild. Signed
E2E sets confirmation depth to zero for runtime speed; shallow-reorg and nonzero
confirmation behavior is covered by keeper/operator unit tests.

Use `SOAK_ROUNDS=100 make local-soak` for an extended run.

The canonical `make local-e2e` suite runs with no protocol fee configuration. It
covers proxy and vault authorization, the local fixture's whitelisted-proxy zero
DEX fee, absence of DEX LP custody, first and subsequent share mints,
donation-safe pricing, pro-rata withdrawals at the vault ratio, single-token
deposit and withdrawal settlement, the 5% trigger, wrong-direction rollback,
unchanged share supply during rebalances, and fee-disabled accounting.

Fee-enabled E2E has a dedicated aggregate target:

```sh
make local-fee-e2e
```

The aggregate runner invokes the venue fee scripts, and the dedicated
`Canonical fee E2E` workflow retains evidence. It remains separate from
`run-e2e.sh` and `make local-e2e`. The workflow/target's presence is not evidence
that it passed for the current SHA; it was not run locally in this working tree.
The market-1000 scenario now requires proxy routing, exact tier/rate/source,
nonzero matching collector shares, and positive treasury deltas.

`deploy-system.sh`, `fee-e2e-multi.sh`, and `fee-e2e-market-1000.sh` now query
the deployed pair's runtime code ID and pass both factory and `pair_code_id` to
the `0.2.0` market/rebalancer schemas. This script wiring was syntax/helper
validated, but the full LocalTerra suite was not run after the change.
No current result should be inferred from historical LocalTerra sections in
`docs/TEST_RESULTS.md`; the complete reproducible release artifact set also
remains unrun.

The soak suite alternates price shocks and vault inventory rebalances. Every
round verifies exact offer-token spending, unchanged bot LP supply, and the
reply-based reference-price update.

The pinned upstream minimal deploy may fail in its optional indexer bootstrap
after all DEX contracts are available. Setup accepts a post-deploy error only
after validating every required address as a live on-chain contract; a stale env
file alone is insufficient.
