# Fee-System Production Deployment

Limit-grid is retained in builds as a PoC only and is outside the production
deployment scope. Any limit-grid instructions below are for local research and
reproducibility, not authorization to deploy economic funds.

Status: **BLOCKED**. This is the canonical production runbook, but it must not be
executed for an economic deployment until every unblock condition below is met.

## Production Constants

| Role | Terra Classic mainnet address | Current compile-time status |
|---|---|---|
| CL8Y CW20 | `terra16wtml2q66g82fdkx66tap0qjkahqwp4lwq3ngtygacg5q0kzycgqvhpax3` | fee-registry `mainnet` pin active |
| CMM treasury | `terra16j5u6ey7a84g40sr3gd94nzg5w5fm45046k9s2347qhfpwm5fr6sem3lr2` | registry and collector `mainnet` pins active |
| fee-registry | not assigned | required via `CL8Y_CANONICAL_FEE_REGISTRY` for every vault mainnet build |
| fee-collector | not assigned | required via `CL8Y_CANONICAL_FEE_COLLECTOR` for mainnet builds |
| shared swap-proxy | not assigned | required via `CL8Y_CANONICAL_SWAP_PROXY` for mainnet builds; limit vault does not use it |

The collector's `registry` field is mutable and is not used by `Collect`.

## Unblock Conditions

Address derivation requires a staged release: proxy and collector addresses do
not exist until their verified bootstrap artifacts are uploaded and
instantiated. Do not upload any current release artifact or fund any resulting
contract. Use this order after the operational gates are approved:

1. Obtain accountable approval for the production collector/proxy deployment
   plan and eventual canonical addresses.
2. Double-build, compare hashes, run mainnet-lock tests, and retain SHA-scoped
   checksums for the proxy and fee-system bootstrap artifacts.
3. Deploy and independently verify the canonical swap-proxy, then complete and
   verify its CL8Y DEX whitelist/zero-DEX-fee configuration.
4. Complete the registry/collector bootstrap below and identify the canonical
   collector address.
5. Supply the approved addresses as `CL8Y_CANONICAL_FEE_REGISTRY`,
   `CL8Y_CANONICAL_FEE_COLLECTOR`, and `CL8Y_CANONICAL_SWAP_PROXY` when building
   final mainnet artifacts. Missing or empty required values fail compilation;
   limit-grid embeds registry and collector but has no proxy. Missing registry
   input has been explicitly verified to fail.
6. Verify registry/collector reciprocity operationally.
7. Produce a second, final reproducible build of all fee-aware vault artifacts
   with `mainnet` enabled; verify their locks reject absent or alternate values.
8. Complete independent audit, mainnet-equivalent rehearsal, multisig setup,
   staged limits, monitoring, and rollback review.

The bootstrap contracts must remain unfunded and no vault may be deployed until
the final pinned artifact set passes step 7. If any bootstrap input changes,
restart verification and record the new artifact/address relationship.

Current release/reproducible definitions cover all four workspaces, default and
mainnet artifact sets, and manifests. `.github/release-policy.json` is the
authoritative inventory, classifies limit-grid as artifact-only PoC, and requires
all production package versions to match an exact stable release tag. Current
production packages, including fee-registry and fee-collector, are `0.2.0`.
The definitions still require approved environment
values and current-SHA execution evidence. Merely passing `make test` or `make
clippy` does not prove that the complete released Wasm set was reproduced.

## Registry/Collector Bootstrap

Registry and collector addresses are circular: registry instantiate requires a
valid collector address, while collector instantiate requires the registry
address. Use this exact sequence. All transactions must be confirmed before the
next step.

### 1. Instantiate Registry With A Temporary Valid Address

Use a governance-controlled valid bech32 address as
`TEMPORARY_PLACEHOLDER_COLLECTOR`. It is temporary configuration, not the
production collector.

```sh
terrad tx wasm instantiate <FEE_REGISTRY_CODE_ID> \
  '{
    "governance":"<GOVERNANCE_ADMIN>",
    "cl8y":"terra16wtml2q66g82fdkx66tap0qjkahqwp4lwq3ngtygacg5q0kzycgqvhpax3",
    "treasury":"terra16j5u6ey7a84g40sr3gd94nzg5w5fm45046k9s2347qhfpwm5fr6sem3lr2",
    "fee_collector":"<TEMPORARY_PLACEHOLDER_COLLECTOR>",
    "base_fee_bps":180
  }' \
  --label cl8y-fee-registry \
  --admin '<GOVERNANCE_ADMIN>' $TX_FLAGS
```

Record `FEE_REGISTRY_ADDRESS`. Confirm `base_fee_bps` is **180**, not 1800.
This is the undiscounted user-facing tier-0 rate (no eligible CL8Y holder tier);
the current query response represents it as `tier_id: null`. Do not confuse it
with reserved governance storage ID `0`.

### 2. Instantiate Collector Pointing To Registry

```sh
terrad tx wasm instantiate <FEE_COLLECTOR_CODE_ID> \
  '{
    "governance":"<GOVERNANCE_ADMIN>",
    "registry":"<FEE_REGISTRY_ADDRESS>",
    "keeper":"<KEEPER_ADDRESS>",
    "treasury":"terra16j5u6ey7a84g40sr3gd94nzg5w5fm45046k9s2347qhfpwm5fr6sem3lr2"
  }' \
  --label cl8y-fee-collector \
  --admin '<GOVERNANCE_ADMIN>' $TX_FLAGS
```

Record `FEE_COLLECTOR_ADDRESS`.

### 3. Replace The Placeholder In Registry

Execute on `FEE_REGISTRY_ADDRESS` as registry governance:

```json
{
  "update_config": {
    "governance": null,
    "cl8y": null,
    "treasury": null,
    "fee_collector": "<FEE_COLLECTOR_ADDRESS>",
    "base_fee_bps": null
  }
}
```

### 4. Verify Reciprocity And Canonical Values

Query both `config` endpoints and require all of the following:

- registry `cl8y` and `treasury` equal the canonical addresses above;
- registry `base_fee_bps` is 180;
- registry `fee_collector` equals `FEE_COLLECTOR_ADDRESS`;
- collector `registry` equals `FEE_REGISTRY_ADDRESS`;
- collector `treasury` equals the canonical treasury;
- governance and keeper equal approved multisig/operator addresses;
- tier queries produce 175, 162, 144, 117, 90, 72, 45, 27, and 9 bps for
  tiers 1 through 9.

Do not wire a vault during the placeholder interval.

## Venue Wiring

These interfaces are not interchangeable.

### Rebalancer `bot-vault`

The instantiate message requires `liquidity_code_id` and accepts fee addresses:

```json
{
  "admin": "<VAULT_ADMIN>",
  "keeper": "<KEEPER>",
  "proxy": "<CANONICAL_SWAP_PROXY>",
  "pair": "<PAIR>",
  "factory": "<CL8Y_FACTORY>",
  "pair_code_id": 456,
  "liquidity_code_id": 123,
  "twap_window_seconds": 60,
  "rebalance_threshold_bps": 500,
  "allocation_tolerance_bps": 500,
  "max_trade_bps": 2500,
  "max_execution_deviation_bps": 500,
  "quote_slippage_bps": 200,
  "max_spot_twap_deviation_bps": 500,
  "max_trade_pool_bps": 1000,
  "max_spread": "0.05",
  "fee_registry": "<FEE_REGISTRY_ADDRESS>",
  "fee_collector": "<FEE_COLLECTOR_ADDRESS>"
}
```

For an existing compatible 0.2.x vault, the admin executes the exact message.
Do not use this as an upgrade path for a 0.1.x bot-vault:

```json
{
  "update_fee_config": {
    "fee_registry": "<FEE_REGISTRY_ADDRESS>",
    "fee_collector": "<FEE_COLLECTOR_ADDRESS>"
  }
}
```

The vault requires factory lookup to return `pair` for its ordered assets and
requires the pair's current runtime code ID to equal `pair_code_id`. It rechecks
the code ID before privileged execution. Bind a verified `bot-liquidity`
instance whose code ID equals `liquidity_code_id`, then register the vault/pair
route on the canonical proxy.
Without `liquidity_contract`, rebalancer fee charging is skipped.

### Market-Grid `grid-vault-swap`

Set required `factory` and `pair_code_id` provenance, `fee_registry`,
`fee_collector`, and the canonical `proxy` in instantiate. The factory lookup
and runtime code ID are verified at creation and the code ID is rechecked before
rebalance/proxy swap.
For an existing compatible 0.2.x vault, the admin uses `update_config`. Do not
use this as an upgrade path for a 0.1.x market vault:

```json
{
  "update_config": {
    "grid_count": null,
    "lower_price": null,
    "upper_price": null,
    "allocation_tolerance_bps": null,
    "max_trade_bps": null,
    "max_execution_deviation_bps": null,
    "quote_slippage_bps": null,
    "max_spot_twap_deviation_bps": null,
    "max_trade_pool_bps": null,
    "max_spread": null,
    "fee_registry": "<FEE_REGISTRY_ADDRESS>",
    "fee_collector": "<FEE_COLLECTOR_ADDRESS>",
    "proxy": "<CANONICAL_SWAP_PROXY>"
  }
}
```

Register the market-grid vault route on the canonical proxy before rebalancing.

Market-grid `grid-vault-swap`, rebalancer `bot-vault`, `swap-proxy`, and
`bot-liquidity`, fee-registry, and fee-collector are `0.2.0`. The cutover policy
is contract-specific:

- redeploy market-grid 0.1.x and bot-vault 0.1.x;
- replace a swap-proxy 0.1.x that contains any routes with a fresh 0.2.0 proxy,
  then re-register every route; only empty compatible proxy state may migrate;
- redeploy bot-liquidity 0.1.x because migration cannot derive a trusted admin;
- retain the supported limit grid-vault 0.1.0 to 0.1.1 migration path;
- fee-registry and fee-collector initial-state migrations retain queryable state.

Never attempt to migrate incompatible state. Rehearse the applicable supported
migration or redeployment and retain artifacts before funding.

### Limit-Grid `grid-vault`

Limit-grid has no swap proxy and no fee-config update message. Set both fee
addresses at instantiate:

```json
{
  "admin": "<VAULT_ADMIN>",
  "owner": "<BOT_OWNER>",
  "keeper": "<KEEPER>",
  "factory": "<CL8Y_FACTORY>",
  "gas_denom": "uluna",
  "keeper_reward": "30000000",
  "minimum_gas_reserve": "30000000",
  "order_timeout_seconds": 604800,
  "max_grid_count": 20,
  "max_orders_per_reconcile": 20,
  "max_active_orders_per_bot": 100,
  "fee_registry": "<FEE_REGISTRY_ADDRESS>",
  "fee_collector": "<FEE_COLLECTOR_ADDRESS>"
}
```

`grid-manager` now stores and propagates both fee fields into newly created
vaults, rejects partial configuration, and requires both fields in `mainnet`.
Manager updates affect future vaults only. Existing fee-disabled vaults cannot
be repaired by a manager update and require an approved vault migration or
redeployment.

## Post-Deployment Verification

Before any funding:

- verify uploaded checksums against the fixed four-workspace production build;
- verify all vault config fee addresses and proxy fields exactly;
- verify rebalancer `liquidity_code_id`, bound liquidity address, and reciprocal
  vault relationship;
- verify proxy route registration for rebalancer and market-grid only;
- verify market/rebalancer factory pair lookup, recorded `pair_code_id`, and
  runtime code recheck before rebalance/proxy swap;
- verify limit-grid communicates directly with its pair and has no proxy;
- execute a bounded canary at base and discounted rates;
- verify `F = floor(V*bps/10000)` and NAV conversion
  `x = floor(F*S/(A-F))`, including that the collector's immediate claim is no
  greater than `F`;
- trigger `Collect` and verify treasury receives assets directly;
- verify collector `vault_shares` is checked cumulative bookkeeping, not current LP;
- exercise registry outage after a successful fee query and require the exact
  cached bps/tier with source `vault_cached`; exercise no-history outage and
  require 180 bps/source `lowest`;
- exercise a reachable registry with a failing CL8Y token query and require the
  full configured base, no tier, and `Lowest` regardless of holding history;
- verify paused market-grid owner/collector pro-rata withdrawals and confirm a
  pending swap/rebalance still blocks withdrawal;
- record exact commit, feature flags, code IDs, addresses, hashes, transactions,
  and query output.

Local dummy-address fee scripts are development evidence only. A dedicated
`make local-fee-e2e` target and canonical workflow now exist, but the LocalTerra
fee suite was not run in this working tree. The full reproducible release
artifact set was also not run.

The rebalancer's actual-contract `cw-multi-test` ladder test covers full
settlement from rebalance through collector withdrawal for no-holder and tiers
1 through 9. It is valuable source/test coverage but does not replace this
runbook's exact-candidate LocalTerra/on-chain canary requirement.
