# CL8Y DeX Bot And Liquidity Contracts

This repository contains three trading venues, one shared protocol-fee system,
and one off-chain grid operator:

- [`rebalancer-system`](rebalancer-system/README.md): isolated two-token
  rebalancer vaults, transferable `bot-liquidity` LP, and a shared swap router.
- [`market-grid-system`](market-grid-system/README.md): swap-based market grid
  (`grid-vault-swap`) using the shipped CL8Y pair API.
- [`limit-grid-system`](limit-grid-system/README.md): abandoned production venue
  retained as a PoC artifact (`grid-vault` and `grid-manager`) using the shipped
  CL8Y pair API directly.
- [`fee-system`](fee-system): shared `fee-registry` and `fee-collector`.
- [`grid-operator-system`](grid-operator-system/README.md): off-chain discovery,
  reconciliation, and operations tooling for the grid venues. It is not a Cargo
  workspace.

The four Cargo workspaces are `rebalancer-system`, `market-grid-system`,
`limit-grid-system`, and `fee-system`.

## Fee Behavior

The canonical undiscounted rate is **180 bps (1.8%)**. In user-facing terms,
this is **tier 0**: no CL8Y holder tier is eligible. The current registry response
represents that case as `tier_id: null`, not numeric ID `0`. A fee-enabled fill
or rebalance resolves one operating address through the shared CL8Y ladder:

- limit-grid: `bot.owner`
- market-grid: `config.admin`
- rebalancer: `config.admin`

Other LP holders are not individually tiered. Deposits and withdrawals have no
direct protocol fee. For executed token-0-normalized value `V`, rate `bps`,
post-settlement vault asset value `A`, and pre-mint LP supply `S`, fee-enabled
fills and rebalances compute:

```text
F = floor(V * bps / 10000)
x = floor(F * S / (A - F))
```

Only `x` NAV-priced LP shares are minted to the collector. Flooring guarantees
the collector's immediate post-mint claim is no greater than economic fee `F`.
This applies to limit-grid, market-grid, and rebalancer.

See [`docs/FEE_TIER_PROTOCOL.md`](docs/FEE_TIER_PROTOCOL.md) for current behavior.

## Production Status

**Production deployment is blocked.** Mainnet builds now fail when any required
compile-time `CL8Y_CANONICAL_FEE_REGISTRY`, `CL8Y_CANONICAL_FEE_COLLECTOR`, or
`CL8Y_CANONICAL_SWAP_PROXY` value is missing or empty. Limit-grid embeds the
registry and collector but has no proxy. Missing registry input was explicitly
verified to fail compilation. CI/release/reproducible definitions cover all four
workspaces, mainnet-feature artifacts, and manifests; limit-grid artifacts are
labelled PoC and are not production candidates. Production
registry, collector, and proxy addresses are still unavailable, no approved
values have been supplied, and the shared proxy is not yet deployed or
independently verified/whitelisted. Market-grid and rebalancer pair provenance
is now factory/code-ID bound. Market `grid-vault-swap` 0.1.x and rebalancer
`bot-vault` 0.1.x must be redeployed. A routed `swap-proxy` 0.1.x must be
replaced by a fresh 0.2.0 proxy and its routes re-registered; only empty compatible
proxy state may migrate. `bot-liquidity` 0.1.x cannot derive a trusted admin and
must not migrate. Limit `grid-vault` 0.1.0 to 0.1.1 remains supported. Existing
fee-disabled limit vaults require an approved migration or redeployment.

Do not deploy economic assets until the blockers in
[`docs/DEPLOY_FEE_SYSTEM.md`](docs/DEPLOY_FEE_SYSTEM.md) and
[`RELEASE.md`](RELEASE.md) are closed and independently verified. All contracts
remain unaudited.

## Verification

```sh
make test
make clippy
make local-e2e
```

The default `local-e2e` path runs without protocol-fee configuration. A separate
`make local-fee-e2e` target and canonical fee E2E workflow now exist, but that
full LocalTerra fee suite was not run in this working tree. Source verification
passed, including 63 rebalancer keeper tests, 71 grid operator tests, and the
70% Python branch-coverage gate; this is not independent audit evidence. See
[`docs/TEST_RESULTS.md`](docs/TEST_RESULTS.md) for SHA-scoped evidence and
[`test-area/README.md`](test-area/README.md) for exact suite scope.

## Documentation Sources

- [`docs/FEE_TIER_PROTOCOL.md`](docs/FEE_TIER_PROTOCOL.md): current fee behavior
  and specification.
- [`docs/DEPLOY_FEE_SYSTEM.md`](docs/DEPLOY_FEE_SYSTEM.md): production fee
  deployment runbook; currently **BLOCKED**.
- [`docs/TEST_RESULTS.md`](docs/TEST_RESULTS.md): SHA-scoped test evidence and
  explicitly labelled historical results.
- [`docs/FULL_REPOSITORY_AUDIT_2026-08-08.md`](docs/FULL_REPOSITORY_AUDIT_2026-08-08.md):
  current internal static-audit findings, severity, and required remediation;
  not an independent audit.
- [`RELEASE.md`](RELEASE.md): release process and current operational gates.
- [`rebalancer-system/docs/AUDIT.md`](rebalancer-system/docs/AUDIT.md): historical
  internal review, not current production-readiness evidence.
- [`SECURITY.md`](SECURITY.md): security policy.
