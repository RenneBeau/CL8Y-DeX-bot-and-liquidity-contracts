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

The E2E suite covers proxy and vault authorization, the whitelisted-proxy zero
DEX fee, absence of DEX LP custody, first and subsequent share mints,
donation-safe pricing, pro-rata withdrawals at the vault ratio, single-token
deposit and withdrawal settlement, the 5% trigger, wrong-direction rollback,
unchanged share supply during rebalances, and zero protocol-fee accounting.

The soak suite alternates price shocks and vault inventory rebalances. Every
round verifies exact offer-token spending, unchanged bot LP supply, and the
reply-based reference-price update.

The pinned upstream minimal deploy may fail in its optional indexer bootstrap
after all DEX contracts are available. The setup accepts that post-deploy error
only when the generated address file exists and the factory confirms the pool.
