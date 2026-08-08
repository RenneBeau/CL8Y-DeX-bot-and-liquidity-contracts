# Verification Evidence

Last updated: 2026-08-08

This file distinguishes local execution from GitHub Actions evidence. A workflow
definition is not proof that it ran, and source tests are not a security audit.

## Current Revision

- Repository baseline: `995f4d4` (`fee(fee-system): pin canonical CL8Y/treasury
  behind the mainnet feature`) — committed and pushed to `origin/main`
- Earlier verified baseline: `b1e943de8da888330d6c0c825b8d702ad03e8d48`
- Rust toolchain: `1.81.0`
- CL8Y fixture revision: `fad801117fe54420d7529da04e485d67d511ef2c`

All corrections below are committed. No uncommitted working-tree changes
represent them.

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

## Per-LP Fee-Tier Rework (2026-08-06)

Following the audit, the fee charge was reworked from "vault contract address"
to **per-LP**: every holder is taxed at their **own** CL8Y tier, so higher-tier
holders lose strictly less LP.

- `market-grid` `grid-vault-swap` `charge_fee` now iterates all holders
  (`SHARES.range`), computes each holder's slice of the fill value
  (`value × shares / total`), resolves that holder's `EffectiveFee`, burns the
  resulting LP from that holder's own shares, and credits the sum to the
  collector — total shares constant (no dilution). The event carries
  `fee_holders`, a weighted-average `fee_bps`, and single-holder `fee_source`/
  `tier`. The previous behaviour resolved the fee against the vault contract
  address, which holds no CL8Y and therefore charged everyone the full base fee.
- New test `each_lp_is_taxed_at_their_own_tier` (`grid_vault_swap_integration.rs`):
  two holders at different mock tiers (200 vs 1800 bps) are both charged; the
  higher-tier holder loses strictly less LP; collector = sum of losses; total
  shares conserved. (Also fixed the tiered mock's discriminator — the address's
  first byte is always `c`, so last-byte parity is used.)
- `rebalancer` `bot-vault` `charge_fee` is **poly (pool)**: it enumerates every
  LP holder of the pooled `bot-liquidity` token (`TokenInfo` + paginated
  `AllAccounts` + `Balance`), attributes each holder's slice of the fill value
  at their OWN CL8Y tier, and accrues the sum to the collector's value claim
  (`FEE_SHARES`). New unit tests cover two holders at different rates (weighted
  fee + conservation), a single holder (exact tier, `source = "live"`), and
  non-blocking registry-failure / zero-rate skips.
- Gates: `cargo test --workspace --all-targets` + `cargo clippy --workspace
  --all-targets -- -D warnings` clean on `market-grid-system` (21 tests) and
  `rebalancer-system` (36 tests).
- Deployment: `docs/DEPLOY_FEE_SYSTEM.md` records the production mainnet CL8Y
  and CMM-treasury addresses (both validated 32-byte bech32) with exact
  fee-registry / fee-collector instantiation JSON; `test-area/deploy-system.sh`
  gains `CL8Y_FEE_REGISTRY` / `CL8Y_FEE_TREASURY` / `CL8Y_FEE_COLLECTOR` env
  overrides (dummy fallback on a local chain) and wires `update_fee_config`
  onto the bot-vault.

## Uniform Single-User Fee-Only Mint Rework (2026-08-07)

Following the CMM decision, the fee mechanic is **unified across all venues into one
model (B, fee-only mint, single user)**: whichever venue, a fill crediting `V`
token-0 value resolves the **single operating user's** effective fee
(`fee = V × fee_bps / 10 000`) and **mints ONLY `fee` LP to the fee-collector** — it
does **not** mint LP to the user. The user's already-deposited position simply
appreciates via NAV (the fill's `V` assets land in the pool holdings; supply grows
by `fee`). No pooled enumeration, no burn.

Changes per contract (see `FEE_TIER_PROTOCOL.md` §5/7/8/9):

- `bot-liquidity` adds `ExecuteMsg::MintTo { recipient, amount }` (vault-gated,
  routes through the existing internal CW20 `Mint`).
- `bot-vault` (rebalancer) `charge_fee` mints only the fee LP to the collector
  (`config.admin` is billed at their own tier). `FEE_SHARES`/`TOTAL_FEE_SHARES`,
  `execute_redeem_fee_shares`, and the `RedeemShares` variant are removed; `Shares`
  now returns the collector's real external `bot-liquidity` LP balance.
- `fee-collector` is liquidity-aware and stateless per vault (one collector serves
  every bot): it branches on the vault's `Config.liquidity_contract` — set
  (rebalancer) → the collector withdraws its external LP via `bot-liquidity`
  `Withdraw` (pro-rata); unset (grids) → sends `RedeemShares` to the vault.
- `market-grid` (`grid-vault-swap`) `charge_fee`: fee-only mint (`config.admin`) on
  its internal `SHARES` ledger. Rebalance swaps route through the shared
  `swap-proxy` (single provider, whitelisted = 0 DEX fee) when configured.
- `limit-grid` (`grid-vault`) `charge_fee`: fee-only mint (`bot.owner`) — no user
  LP is minted on a reconcile.

Tests updated/added to cover the fee-only mint
(`charge_fee_mints_only_fee_lp_to_collector`,
`rebalance_mints_fee_lp_to_collector_and_can_redistribute_to_treasury`,
`single_user_is_billed_at_the_admin_tier`, `protocol_fee_mints_collector_lp_and_redeems`,
`tier9_rebalance_through_zero_fee_proxy_collector_gets_exact_9bps`).

Gates (all green, `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` +
`cargo test --all-targets --locked`):

| Workspace | Unit | Integration | Total |
|---|---|---|---|
| `rebalancer-system` | 30 | 4 | 34 |
| `fee-system` | 18 | 4 | 22 |
| `market-grid-system` | 10 | 13 | 23 |
| `limit-grid-system` | 29 | 21 | 50 |

129 tests passed across the four workspaces; Clippy was strict (`-D warnings`) with
zero findings everywhere.

### On-chain fee E2E (LocalTerra, 2026-08-07, single shared collector + treasury)

All venues are wired to **one** `fee-registry`, **one** `fee-collector` and **one**
`fee-treasury`; the market-grid and rebalancer share **one** `swap-proxy`.

- **limit-grid** (`fee-e2e-test.sh`): tier-5 holder → 90 bps (live); zero-CL8Y owner
  → 180 bps. Collector LP grew per fill; `collect` → treasury EMBER +8283, CORAL
  +22635; collector shares → 0.
- **market-grid** (`fee-e2e-multi.sh`): rebalance routed through the shared proxy;
  fee 90 bps tier-5 live; `fee_shares=17,361,905` LP minted to the collector
  (matches); `collect` → treasury EMBER +13,865,450 CORAL +1,910,513; collector
  shares → 0.
- **1000-EMBER market-grid** (`fee-e2e-market-1000.sh`): seed = 1000 EMBER;
  rebalance swapped ~195.8 EMBER (0.90% of ~195,822,444 raw); `fee_shares=1,762,402`
  → collector LP; `collect` → treasury EMBER +1,407,441 CORAL +190,197; collector
  shares → 0. Confirms the fee-only mint on a 1000/grid pool.
- **rebalancer** (`fee-e2e-multi.sh`): route registered on the same proxy, but the
  leg currently reports no fee because the E2E does not provision `bot-liquidity`
  (`config.liquidity_contract` unset); per §7 the collector then realizes via the
  internal vault path only once that LP pool is set. This is a script-wiring gap,
  not a contract defect.

## Mainnet Address Lock — `mainnet` feature (2026-08-08)

Commit `d06b5d8` baseline was the fee-only-mint single-user rework. Commit
`995f4d4` adds a **compile-time production lock** to the fee system:

- All four workspaces now pass `make test` (exit 0) and `make clippy` (exit 0).
- The `fee-system` workspace is folded into `make test` + `make clippy`
  (`cargo clippy --features mainnet --all-targets -- -D warnings`).
- A new `fee-wasm` Makefile target produces the mainnet artifact with
  `MAINNET=1` (`cargo build --release --target wasm32-unknown-unknown
  --features mainnet`); the feature is **off by default** so local
  test-a-net / E2E dummy-address deployments are unaffected.

Lock semantics (feature-gated):
- `fee-registry`: `CANONICAL_CL8Y` / `CANONICAL_TREASURY` pinned at compile
  time; `instantiate` REJECTS any non-canonical `cl8y`/`treasury`;
  `update_config` refuses (hard error) to re-point them. Governance /
  `fee_collector` / `base_fee_bps` stay mutable.
- `fee-collector`: `CANONICAL_TREASURY` pinned; `instantiate` REJECTS any other
  value and `update_config` refuses to re-point the `Collect` payout target.
  `governance` / `registry` / `keeper` stay mutable.

New feature-gated suite `tests/mainnet_lock.rs` in both contracts (rejecting
fake addresses at instantiate, refusing re-pointing in `update_config`, and
proving governance/base-fee still update). The dummy-address suites are now
`#![cfg(not(feature = "mainnet"))]` so each configuration compiles its own
coherent set. Gate runs:

| Command | Result |
|---|---|
| `cargo test --locked --manifest-path fee-system/Cargo.toml --all-targets` (default features) | PASS: 22 tests (8 audit, 10 integration, 4 collector) |
| `cargo test --locked --manifest-path fee-system/Cargo.toml --all-targets --features mainnet` | PASS: 9 tests (5 registry + 4 collector lock) |
| `cargo clippy --locked --manifest-path fee-system/Cargo.toml --all-targets --features mainnet -- -D warnings` | PASS |
| wasm build featureless vs `--features mainnet` | both build; sha256 differs (lock compiled in) |

On-chain LocalTerra remained green after the change:
`test-area/fee-e2e-test.sh` **PASSED** (tier-5 → 90 bps, zero-CL8Y → 180 bps,
6 fills, collector → dummy treasury), and
`test-area/fee-e2e-multi.sh` completed both the market-grid and rebalancer
legs (treasury received EMBER/CORAL and LUNC-C/EMBER respectively; collector
shares → 0 on both). The second `fee-e2e-multi.sh` re-run fails for state
non-idempotency (the `bot-liquidity` pool was already provisioned by the first
run → the re-bootstrap deposit exceeds the allocation tolerance), not a
contract defect.

Caveats from the broader evidence file still stand: the lock protects only
against non-canonical `cl8y`/`treasury`; the deployed geometry is the
compile-time pinned one only when the artifact was built with `MAINNET=1`; the
address values themselves (CL8Y token, CMM treasury) are the same ones recorded
in `docs/DEPLOY_FEE_SYSTEM.md` §1 and now compiled verbatim into the contracts.

Remaining follow-up: once the fee-collector is live on mainnet, add its real
address (the reference registry's `fee_collector`) to the `mainnet` feature of
`fee-registry` so a deploying key cannot point the system at a personal
collector. The lock is already wired for `cl8y`/`treasury`; the collector pin is
the same pattern applied to `fee_collector` (and, optionally, a per-vault
`fee_collector` validation).

## Swap-Proxy Slim-Down (2026-08-08)

`swap-proxy` was reduced to a pure router. Its config previously carried
`cl8y_token` and `fee_registry` and exposed a `WithdrawCl8y` sweep; all three
were legacy from the pre-whitelist tier design and are removed:

- `InstantiateMsg` / `Config` / `ConfigResponse` now hold only `admin`.
- `WithdrawCl8y` (and its `execute_withdraw_cl8y`) is deleted — the proxy never
  holds tokens, so there is nothing to sweep.
- `deploy-system.sh` no longer funds the proxy with CL8Y nor registers it for a
  tier; `integration-test.sh` asserts the proxy holds **0** CL8Y and tests
  `remove_vault` as the remaining admin-only action instead of `withdraw_cl8y`.
- Docs updated (`SWAP_PROXY.md`, `DEPLOYMENT.md`, `ADMIN_OPERATIONS.md`,
  `IMPLEMENTATION.md`, `ARCHITECTURE.md`, root/rebalancer/README, grid
  operations, `FEE_TIER_PROTOCOL.md`).

Whitelisting scope is now stated exactly: **only the swap-proxy is
whitelisted** on the CL8Y DEX; vaults pay no DEX swap fee by routing their swaps
through it. No contract holds CL8Y. The protocol fee is driven by each user's
CL8Y balance via `fee-registry` at fill time and is the DEX's revenue path: the
bots are the DEX's own bots and the collected fee is forwarded to the CMM
treasury.

Validation after the change: `cargo test --locked --workspace --all-targets` in
`rebalancer-system` green (11 bot-liquidity, 19 bot-vault, 4 proxy, 2 types, 3
dex helpers), `cargo clippy --locked --manifest-path rebalancer-system/Cargo.toml
--all-targets -- -D warnings` clean, `cargo fmt --check` clean across all four
workspaces, and `bash -n` clean on every `test-area/*.sh`.

## Vault Address Lock — `mainnet` feature on the three vaults (2026-08-08)

Following the swap-proxy slim-down, the compile-time `mainnet` lock is extended
from the fee-system to the **three vaults**, so a deploying key cannot point a
vault at a fee-collector (or swap router) it controls:

- `bot-vault` (rebalancer): `src/mainnet.rs` pins `CANONICAL_FEE_COLLECTOR` and
  `CANONICAL_SWAP_PROXY`; `instantiate` validates both and
  `update_fee_config` refuses to re-point them; absent collector/proxy rejected.
- `grid-vault-swap` (market-grid): `src/mainnet.rs` pins the same two; validated
  in `instantiate` and `update_config` (proxy is optional, so the proxy assert
  takes `Option<&Addr>`).
- `grid-vault` (limit-grid): `src/mainnet.rs` pins `CANONICAL_FEE_COLLECTOR`
  only (no proxy field, and no fee update message — the collector is fixed for
  the contract's lifetime); validated in `instantiate`.

All constants are `Option<&str> = None` until the fee-collector and swap-proxy
are deployed to mainnet, so the lock is **inert while unset** (local/E2E
dummy-address deployments keep working) and **active the moment it is filled**
(rejects any non-canonical value and an absent collector). Each contract gains
`ContractError::NonCanonicalAddress { field, expected }` and inline unit tests
for the pin helper. `make test` and `make clippy` now build all three vault
workspaces with `--features mainnet` in addition to the default config.

Gate runs (feature-gated):

| Command | Result |
|---|---|
| `cargo test --features mainnet` — `rebalancer-system` | PASS: 23 bot-vault tests |
| `cargo test --features mainnet` — `market-grid-system` | PASS: 13 grid-vault-swap tests |
| `cargo test --features mainnet` — `limit-grid-system` | PASS: 17 grid-vault tests |
| `make test` (all workspaces, default + mainnet) | PASS: 245 Rust tests, exit 0 |
| `make clippy` (all workspaces, `--features mainnet`, `-D warnings`) | PASS, exit 0 |

## Real CL8Y Ladder Detection In Vault Workspaces (2026-08-08)

The earlier gap was real: the fee-registry's canonical CL8Y tier ladder was
already tested in `fee-system/contracts/fee-registry/tests/audit_tiers.rs`, but
the three vault workspaces only used local mock registries (fixed bps, byte
parity, or hard-coded tier-9). To prove the vault-facing query shape against
the DEX's actual ladder, each workspace now carries a targeted test that wires
the **real** `cl8y-fee-registry` contract to a real mintable CW20 CL8Y token
and resolves `EffectiveFee` at every canonical tier boundary and one wei above.

- `market-grid-system/contracts/grid-vault-swap/tests/grid_vault_swap_integration.rs`:
  `real_registry_detects_every_ladder_tier_for_the_operating_user`
- `limit-grid-system/contracts/grid-vault/tests/grid_vault_integration.rs`:
  `real_registry_detects_every_ladder_tier_for_the_bot_owner`
- `rebalancer-system/contracts/bot-vault/tests/real_registry_ladder.rs`:
  `real_registry_detects_every_ladder_tier_for_the_bot_admin_model`

These tests use the same `EffectiveFee` query that the vaults issue on a fill /
rebalance / reconcile path, so any future schema mismatch (`holding`, `source`,
`tier_id`, etc.) between a vault and the real fee-registry will fail in the
vault workspace rather than only in on-chain E2E.

Gate runs:

| Command | Result |
|---|---|
| `cargo test --manifest-path market-grid-system/Cargo.toml -p cl8y-grid-vault-swap real_registry_detects_every_ladder_tier_for_the_operating_user` | PASS |
| `cargo test --manifest-path limit-grid-system/Cargo.toml -p cl8y-grid-vault real_registry_detects_every_ladder_tier_for_the_bot_owner` | PASS |
| `cargo test --manifest-path rebalancer-system/Cargo.toml -p cl8y-bot-vault real_registry_detects_every_ladder_tier_for_the_bot_admin_model` | PASS |
| `cargo clippy --manifest-path market-grid-system/Cargo.toml -p cl8y-grid-vault-swap --tests -- -D warnings` | PASS |
| `cargo clippy --manifest-path limit-grid-system/Cargo.toml -p cl8y-grid-vault --tests -- -D warnings` | PASS |
| `cargo clippy --manifest-path rebalancer-system/Cargo.toml -p cl8y-bot-vault --tests -- -D warnings` | PASS |

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
