# Release Engineering

## Current Status

**Production release is blocked operationally, not by the former workspace or
feature omissions.** The repository has four Cargo workspaces:
`rebalancer-system`, `market-grid-system`, `limit-grid-system`, and `fee-system`.
Current CI, security, Wasm, reproducible-build, and release definitions cover all
four, produce default and `mainnet` artifact sets, and emit per-workspace
manifests. Release checks include tag ancestry and the dedicated canonical fee
E2E job.

`.github/release-policy.json` is the authoritative inventory and classifies each
package/artifact as production or PoC. Limit-grid is artifact-only PoC. The
machine validator checks Cargo membership and artifact flags and derives
workspace/artifact counts from policy instead of duplicated workflow constants.

Limit-grid remains in the artifact set as an abandoned **PoC** for reproducibility
and research. Its artifacts are not approved production candidates and must not
be deployed with economic funds.

`mainnet` compilation requires nonempty `CL8Y_CANONICAL_FEE_REGISTRY`,
`CL8Y_CANONICAL_FEE_COLLECTOR`, and `CL8Y_CANONICAL_SWAP_PROXY` values for the
vaults that use them. Limit-grid requires registry and collector but has no
proxy. Missing registry input was explicitly verified to fail compilation.
Approved production values are not yet available, so no production artifact set
can be approved or deployed. See
[`docs/DEPLOY_FEE_SYSTEM.md`](docs/DEPLOY_FEE_SYSTEM.md) for the complete gate.
Do not publish or deploy a production candidate until approved addresses,
deployment/pinning decisions, E2E evidence, and the remaining audit gates are
verified.

## Intended Release Properties

Releases are triggered only by annotated, signed, exact stable
`vMAJOR.MINOR.PATCH` tags whose cryptographic signature GitHub verifies.
Prereleases and numeric components with leading zeros are rejected. Every
production package version must equal the tag; all current production packages,
including fee-registry and fee-collector, are `0.2.0`. The workflow checks
required jobs for the tagged SHA, performs
double builds, compares Wasm digests, creates an SPDX JSON SBOM and GitHub
Sigstore provenance, and publishes `SHA256SUMS`.

The current working-tree workflow is intended to:

1. Build all four Cargo workspaces.
2. Publish `cl8y_fee_registry.wasm` and `cl8y_fee_collector.wasm` with the
   `mainnet` feature.
3. Build every fee-aware vault with the `mainnet` feature, with limit-grid
   explicitly classified as PoC-only.
4. Ensure optimizer/reproducible builds use those features rather than default
   workspace features.
5. Double-build and compare every production Wasm.
6. Run and retain mainnet-lock tests and artifact checks for the exact tag SHA.
7. Attest and checksum the complete artifact set.
8. Include fee-system source-quality and dependency-policy checks in required
   release evidence.
9. Record factory address and runtime pair code ID for every market-grid and
   rebalancer deployment and re-register every `0.2.0` proxy route.

The definitions implement these properties, but workflow presence is not run
evidence. Release-policy fixtures cover valid, version mismatch, malformed,
prerelease, expired, wrong-package, wrong-version, and extra-advisory cases.

`RUSTSEC-2024-0344` is scoped only to `curve25519-dalek` 3.2.0 in all four
current lockfiles, transitively introduced through CosmWasm
1.5.11/ed25519-zebra 3.1.0, and expires 2027-02-01 UTC. Policy fails on expiry,
exception disappearance, package/version drift, or any extra vulnerability; no
global `cargo audit --ignore` remains. The `RUSTSEC-2024-0388` derivative 2.2.0
unmaintained notice is informational and is not a vulnerability exception.

## Pinned Inputs

- Rust is pinned by `rust-toolchain.toml` to `1.81.0`, including `rustfmt`,
  `clippy`, and `wasm32-unknown-unknown`.
- GitHub Actions are referenced by immutable commit SHA.
- Wasm optimization uses
  `cosmwasm/workspace-optimizer:0.16.1@sha256:b9c92b2900b7ebaab3499203615c1b8589592bc557355ed3432e48851ffde69e`.
- Security-tool revisions are pinned in
  `.github/scripts/install-security-tools.sh` and `security.yml`.

Update a tool tag and digest together only after verifying the registry
manifest.

## Candidate Validation

The existing local commands remain useful source checks:

```sh
make ci
make security
make reproducible
```

`make test` and `make clippy` passed in this working tree across all four
workspaces and their configured `mainnet` gates. The reproducible target now
double-builds default and mainnet artifact sets for all four workspaces and emits
manifests, but the primary validation did not run that target. Selected double
builds reported elsewhere are not a claim that the complete release set was
reproduced here.

Release-policy fixture tests, release inventory validation, and live RustSec
policy validation passed. `cargo deny` was reported passing by the implementation
agent. Full release/reproducible Docker artifacts and a candidate tag workflow
were not executed.

Canonical fee-disabled and fee-enabled LocalTerra workflows are
scheduled/manually dispatchable and must pass on the exact candidate SHA. The
dedicated fee target/workflow was added but was not run locally in this working
tree, so it is not current evidence.

Market-grid `grid-vault-swap`, rebalancer `bot-vault`, `swap-proxy`, and
`bot-liquidity`, fee-registry, and fee-collector are version `0.2.0`. Pair
provenance fields are required.
Market-grid and bot-vault 0.1.x require redeployment. A swap-proxy 0.1.x with any
routes rejects migration and requires a fresh 0.2.0 proxy plus route
re-registration; only empty compatible proxy state may migrate. Bot-liquidity
0.1.x rejects migration because no trusted admin can be derived. Limit
grid-vault 0.1.0 to 0.1.1 is supported, and fee-system initial fixtures remain
queryable after migration. No 0.2.0 redeployment or artifact rollout has been
executed.

## Publishing After Unblock

1. Supply approved registry, collector, and proxy environment values and verify the pipeline
   builds all four workspaces and all expected mainnet-feature Wasms on the
   candidate SHA.
2. Run canonical E2E and an explicit fee-enabled deployment rehearsal on that
   SHA; retain logs, hashes, feature flags, and code IDs.
3. Confirm compiled production values match approved deployments and lock tests
   reject missing or alternate registry/collector/proxy addresses.
4. Execute the contract-specific `0.2.0` redeployment plan, re-register routes,
   and retain factory/pair/code-ID evidence. Never migrate incompatible 0.1.x
   state; only the explicitly supported empty-proxy, limit 0.1.0-to-0.1.1, and
   tested fee-system paths may use migration.
5. Create and push an annotated signed tag matching the release package version.
6. Review the complete `Signed release` evidence before publishing/deploying.
7. Verify `SHA256SUMS` and provenance with `gh attestation verify`.

Repository environment and branch protections are live GitHub settings. Verify
them independently for each release; this tree cannot prove their current state.
