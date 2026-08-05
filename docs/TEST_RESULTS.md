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
| `cargo +1.81.0 test --locked --workspace --all-targets` in `limit-grid-system` | PASS: 50 tests (4 manager, 29 vault unit, 17 integration incl. fee rate boundaries) | Grid limit-order contracts (reference) |
| `cargo +1.81.0 test --locked --workspace --all-targets` in `market-grid-system` | PASS: 20 tests (10 vault unit, 10 swap integration incl. fee mint + redeem + non-blocking) | Grid swap contract (deployable) |
| `cargo +1.81.0 test --locked --workspace --all-targets` in `rebalancer-system` | PASS: 35 Rust tests (11 liquidity, 20 vault incl. fee charge math/redeem, 4 proxy) | Rebalancer contracts/packages |
| `cargo +1.81.0 test --locked --workspace --all-targets` in `fee-system` | PASS: 22 tests (10 fee-registry, 8 fee-ladder audit, 4 fee-collector) | Protocol fee-system (new workspace) |
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

## On-Chain Fee E2E (LocalTerra, 2026-08-05)

Full end-to-end proof of the protocol fee path on a real signed chain (chain ID
`localterra`, network node at height ~529500). `test-area/fee-e2e-test.sh`
deploys a dummy CL8Y fee token (cw20-base, code id 1, 200 CL8Y minted to a
holder that maps to tier 5), a dummy treasury, the fee-registry, the
fee-collector, and two grid-vaults wired to both, then runs two bot lifecycles
through the deployed exchange pair.

Latest run artifacts (`test-area/.fee-e2e-artifacts`):

- code ids: registry 46, collector 47, grid-vault 48 (raw `wasm32-unknown-unknown` release builds)
- dummy CL8Y: `terra18hu00pwvd8kq0cgzk03l2nmp8rr0h5gp6tektz6qazwfapqsl4cqgcvy9h`
- dummy treasury: `terra16xm26vq25wnvrr7aptun3jlk0ghgm82gh5w4gp`
- fee-registry: `terra1mpys68uwajmcjn6lwctan39v5sf4yk3agxp8pns02kqfnym3racsunwpqy`
- fee-collector: `terra1f9tjculwafs6qvrfaxxc9n3z29feel2vwelhj5h4xrvmtzvug76q67ykx2`
- grid-vault 1 (tier-5 owner): `terra126qhftp95nceprux24252tn7klxpmfvpv9sftqsjmjqylrp34fwq7ndhem`
- grid-vault 2 (zero-CL8Y owner): `terra1qpyj2nc9kzrp09k69l8wlrvzdarc42zs9q824mjuldvleu3dsqds3936nm`

Scenarios verified on chain:

| Scenario | Result |
|---|---|
| Tier-5 holder `EffectiveFee` | 90 bps, `Live` |
| Zero-CL8Y address `EffectiveFee` | 180 bps (full base fee, never under-fee) |
| Lifecycle 1: deposit, allocate, 6 fills + reconciles | fee_shares minted each reconcile: 24527, 12263, 6131, 3065, 1532, 766; collector LP grows 24527 → 48284 |
| Fee source tracked | `fee_tier=5`, `fee_bps=90`, `fee_source=Live` on every fill |
| Wind-down: cancel + keeper `Collect` | treasury receives EMBER +11659 and CORAL +34479; collector redeems all LP, vault shares 0 |
| Lifecycle 2 (second bot, zero-CL8Y owner) | first reconcile: `changed=1`, `fee_bps=180`, `fee_source=Live`, `fee_shares=9810` |

The invariant "a zero-CL8Y actor is always charged the full base fee" is
demonstrated both at query time and in the reconcile mint on chain. This run
exposed and fixed a real schema bug (the vault's local
`FeeRegistryEffectiveFeeResponse` was missing the `holding` field, so
`cw_serde` rejected the registry response and the fee was silently skipped).
Unit and Clippy gates are clean for both workspaces after that fix.

## On-Chain Per-Execution Fee E2E: market-grid + rebalancer (LocalTerra, 2026-08-05)

Second on-chain proof on the same signed chain (chain ID `localterra`, height
~535000). `test-area/fee-e2e-multi.sh` reuses the already-live fee-registry /
fee-collector / dummy treasury from the run above (same base fee 180 bps), and
deploys a fresh market-grid vault (`grid-vault-swap`) plus a fresh swap-proxy +
bot-vault (`rebalancer-system`). It proves that **market-grid and rebalancer
charge a protocol fee against the executed swap value** (never a percentage of
total holdings), are **non-blocking** (fee emitted in the settle reply after the
swap commits; a registry failure only skips the fee), and route the accrued
claim through the same fee-collector → dummy treasury path.

Latest run artifacts (`test-area/.fee-e2e-multi-artifacts`):

- code ids: market-grid vault 64, bot-vault 65, swap-proxy 66 (raw release wasms)
- market-grid vault: `terra1zlrmhd23gwr2mhw6ncw9cmkggyht5mlph62s953wjvpc3jvg59cq2a20kp`
- swap-proxy: `terra1lqum4f08cf9cs7jc7482cu4wr96u2tmleqd4lvkhsg2m9vrsc2yqdq6sxt`
- bot-vault: `terra17gw2jwn4fp0c0gkwp2u6nh4rzyz6qjxsnmu4lky2c9a3x97qpzjsygvegs`

Scenarios verified on chain:

| Scenario | Result |
|---|---|
| Market-grid: single-token deposit (EMBER only, no CL8Y) | `grid_status` allocation_deviation=10000, `should_rebalance=true` |
| Market-grid rebalance (keeperless, executed swap EMBER→CORAL) | reply: `fee_bps=180`, `fee_source=live`, `fee_shares=34609105` |
| Market-grid collector LP minted to fee-collector | `shares{bot_id:0,address:collector}=34609105` (== fee_shares, LP dilution path) |
| Market-grid `collect` → dummy treasury | EMBER +27591791, CORAL +5392338; collector shares → 0 |
| Rebalancer: single-token funding (EMBER only) | `rebalance_status` allocation_deviation=10000, `should_rebalance=true` |
| Rebalancer keeper `Rebalance` through swap-proxy | reply: `fee_bps=180`, `fee_source=live`, `fee_shares=81626937` |
| Rebalancer `FEE_SHARES` accrued to collector | `shares{bot_id:0,address:collector}=81626937` (== fee_shares) |
| Rebalancer `collect` → dummy treasury (pro-rata of both balances) | EMBER +42722804, CORAL +31363604; collector fee-shares → 0 |

The zero-CL8Y market-grid vault and bot-vault are both charged the full 180 bps
`live` base fee. Note the source string is `live` here (raw registry string)
versus `Live` in the limit-grid vault (its enum `Display`); both denote the same
live `EffectiveFee` tier. Gate: since the market-grid grid-vault is charged in
the swap settle reply after the executed trade, the treasury claim is a genuine
LP/fee-shares dilution of the vault rather than a fixed withdrawal fee.

Unit and Clippy gates for `market-grid-system` (10 unit + 10 integration, incl.
`rebalance_mints_fee_lp_to_collector_and_can_redistribute_to_treasury` and
`rebalance_is_non_blocking_when_fee_registry_is_unreachable`) and
`rebalancer-system` (20 bot-vault tests incl. fee admin gating, collector-only
redeem, no-fee-without-config, charge math, and non-blocking registry skip) are
clean after these additions.

## Fee-System Audit Pass (2026-08-05)

A fresh-eyes audit of the whole protocol-fee path (fee-registry + fee-collector +
grid-vault + grid-vault-swap + bot-vault). New evidence:

- `fee-registry` `tests/audit_tiers.rs` (8 tests): every holder tier (1..9) at its
  exact CL8Y boundary, one raw wei below each boundary, a 31-point balance matrix
  cross-checked against an independent reference resolution (never over-charged),
  base-fee edge cases (0, 1, 180, 1800, 5000, 10000 bps), a governance-added 100%
  discount tier driving the fee to zero, reserved-tier add/update rejection, and
  ladder-version bumping.
- `limit-grid` `grid_vault_integration.rs` (2 new tests): a 100% (10_000 bps) rate
  credits the entire collected value to the collector; 0/1 bps rates round to zero
  and never dilute holders.
- `market-grid` `grid_vault_swap_integration.rs` (1 new test): re-pointing the
  vault at a valid-but-empty registry address makes rebalance complete with a
  `fee_skipped` attribute and no fee minted (non-blocking proof at vault level).
- `bot-vault` `contract.rs` unit tests (2 new): `charge_fee` applies the
  exact `value * fee_bps / 10_000` math and accrues to `FEE_SHARES`/`TOTAL_FEE_SHARES`;
  a registry query failure skips non-blockingly and a zero rate skips.

Audit conclusions (all gates green: `cargo test --workspace --all-targets` +
`cargo clippy --workspace --all-targets -- -D warnings` in all four workspaces):

1. Pricing is monotone and never under-fees: `EffectiveFee` is live-first,
   cached-fallback-on-failure, otherwise the full base fee; `resolve_discount`
   picks the highest met non-governance tier; `effective_fee` is an exact
   floor `base * (10000 - discount) / 10000`.
2. Governance tiers 0/255 are reserved and can never auto-apply to a holder
   balance (verified by add + update tests). Note: there is currently no
   mechanism to *assign* a specific address to tier 0/255, so "governance-assigned
   market makers" are not yet reachable through `EffectiveFee`; this is
   conservative (they simply keep paying the base fee) and is future work.
3. Vault charge math (`value * fee_bps / 10_000`, capped at 10_000 bps, rounded
   down) is identical across grid-vault / grid-vault-swap / bot-vault and is
   exercised at 0, 1, 200/1800/10000 bps. Rounding always favors the protocol
   slightly (floor of the discounted rate).
4. Non-blocking invariant holds at every layer: unreachable registry → `fee_skipped`
   attribute, trade commits, no fee minted, no revert.
5. Redeem is strictly collector-only, refuses zero/insufficient shares, and burns
   the claim after paying a pro-rata slice of the vault balances.

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
