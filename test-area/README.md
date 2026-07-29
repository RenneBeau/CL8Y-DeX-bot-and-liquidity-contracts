# LocalTerra Integration Area

The harness pins CL8Y DEX revision
`fad801117fe54420d7529da04e485d67d511ef2c`, starts its official LocalTerra
image, deploys the minimal EMBER/CORAL DEX, and then deploys the shared proxy,
one isolated bot vault, and its CW20 bot-liquidity contract.

The managed CL8Y checkout is under `test-area/.cache/` and is not committed.
Contracts are built with `cosmwasm/workspace-optimizer:0.16.1` for Terra Classic
VM compatibility. LCD/RPC use `1317`/`26657`; optional gRPC is remapped to
`29090`/`29091`.

```sh
make local-setup  # start LocalTerra and deploy DEX plus clean bot system
make local-test   # run tests against the current deployment
make local-e2e    # redeploy bot contracts, then run signed E2E scenarios
make local-soak   # redeploy and run 25 inventory-rebalance rounds
make local-all    # one deploy followed by E2E and soak suites
make local-stop   # stop services while retaining state
make local-reset  # delete local chain and database state
```

Use `SOAK_ROUNDS=100 make local-soak` for an extended run.

The E2E suite covers proxy and vault authorization, the governance-assigned
CL8Y discount, absence of DEX LP custody, first and subsequent share mints,
donation-safe pricing, proportional balanced withdrawals, single-token deposit
and withdrawal settlement, the 5% trigger, wrong-direction rollback, unchanged
share supply during rebalances, and zero protocol-fee accounting.

The soak suite alternates price shocks and vault inventory rebalances. Every
round verifies exact offer-token spending, unchanged bot LP supply, and the
reply-based reference-price update.

The pinned upstream minimal deploy may fail in its optional indexer bootstrap
after all DEX contracts are available. The setup accepts that post-deploy error
only when the generated address file exists and the factory confirms the pool.
