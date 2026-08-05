# Verification Evidence

Last updated: 2026-08-05

This file distinguishes local execution from GitHub Actions evidence. A workflow
definition is not proof that it ran, and source tests are not a security audit.

## Current Revision

- Repository baseline: `b1e943de8da888330d6c0c825b8d702ad03e8d48`
- Reported security corrections: uncommitted working-tree changes after that baseline
- Rust toolchain: `1.81.0`
- CL8Y fixture revision: `fad801117fe54420d7529da04e485d67d511ef2c`

No GitHub Actions run can yet represent the uncommitted working tree. After these
changes are committed, all workflows must be rerun for that exact commit.

## Local Results

The following commands were executed locally while implementing the current
working-tree corrections:

| Command | Result | Scope |
|---|---|---|
| `cargo +1.81.0 test --locked --workspace --all-targets` in `limit-grid-system` | PASS: 48 tests (4 manager, 29 vault unit, 15 integration) | Grid limit-order contracts (reference) |
| `cargo +1.81.0 test --locked --workspace --all-targets` in `market-grid-system` | PASS: 17 tests (10 vault unit, 7 swap integration) | Grid swap contract (deployable) |
| `cargo +1.81.0 test --locked --workspace --all-targets` in `rebalancer-system` | PASS: 30 Rust tests (11 liquidity, 15 vault, 4 proxy) | Rebalancer contracts/packages |
| `cargo +1.81.0 test --locked --workspace --all-targets` in `fee-system` | PASS: 10 tests (fee-registry) | Protocol fee-registry (new workspace) |
| `python3 -m unittest -v test_keeper.py` | PASS: 50 tests | Rebalancer keeper |
| `python3 -m unittest discover -s grid-operator-system/services/grid-operator/tests -p 'test_*.py'` | PASS: 24 tests | Grid operator |
| `make local-e2e` | PASS: 10 rebalancer and 10 grid scenarios | Signed LocalTerra, uncommitted tree |

These are local results, not GitHub Actions results. Both strict Clippy workspaces,
formatting, release Wasm builds, optimized E2E Wasm builds, and shell syntax also
passed locally. Soak, schema generation (no generator is present), and complete
funded migration scenarios were not run.

The successful `make local-e2e` run used the healthy local Docker Compose stack,
chain ID `localterra`, pinned CL8Y revision above, Rust `1.81.0`, and optimizer
image pinned by `Makefile`. Optimized hashes from that run were:

- `cl8y_bot_liquidity.wasm`: `e5b1a511e6cc1aa0e539fdf44df14453bfb3d613aeaa72d9fe9c245eed7c4b42`
- `cl8y_bot_vault.wasm`: `d15d79b6004cf84938e2ddc62ca1c0febd010feda8b992137c02ad8e89315e57`
- `cl8y_swap_proxy.wasm`: `732f146ed78269b50d2f013855a9d734b0ea355d19b351d90b3b39a5f4409fc2`
- `cl8y_grid_manager.wasm`: `a276a70b25151567e0b3c4cb8df72e0a0607d9680a50bafceee0ffcf65c2b533`
- `cl8y_grid_vault.wasm`: `c6687ddbb2eaf56f909a54757639c6f4670b61f18d66c357443e3cb2294bc0f4`

## Existing CI Evidence

The baseline commit had retained source-quality, dependency-security, and
reproducible-build runs. Those runs do not validate the current working tree:

- Source quality: `actions/runs/30697337166`
- Dependency security: `actions/runs/30697337170`
- Reproducible Wasm: `actions/runs/30697337164`

No retained `LocalTerra E2E` GitHub Actions run was found for the baseline or the
current changes. Therefore no E2E PASS is claimed here. `.github/workflows/e2e.yml`
now runs both signed suites through `make local-e2e`, preserves command exit codes,
records the tested SHA, publishes logs, and writes a success-only job summary.

## E2E Scope And Limits

The scripts contain rebalancer and grid scenarios, but scenario presence is not
execution evidence. A future evidence entry must include the exact commit, run
URL/ID, date, runner image, tool versions, scenario counts, skipped scenarios,
artifact name and digest, and known limitations.

LocalTerra requires Docker with Compose, Git/network access to the pinned CL8Y
revision, `make`, `jq`, Python 3, sufficient disk space, and up to 90 minutes.
Signed local scenarios use a deterministic fixture and do not establish mainnet
profitability, oracle robustness, or production safety.

## Interpretation

Passing tests demonstrate only the behavior covered by those tests. Independent
review, external audit, migration rehearsal, and staged deployment evidence
remain required before economic deployment.
