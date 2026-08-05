# Release Engineering

Releases are generated only from annotated `v*` tags whose cryptographic
signature GitHub verifies. Lightweight or unsigned tags fail before a build.
The release workflow builds both Cargo workspaces twice, compares every Wasm
digest, generates an SPDX JSON SBOM, creates a GitHub Sigstore provenance
attestation, and publishes all evidence with `SHA256SUMS`.

## Pinned Inputs

- Rust is pinned by `rust-toolchain.toml` to `1.81.0`, including `rustfmt`,
  `clippy`, and `wasm32-unknown-unknown`.
- Actions are referenced by immutable commit SHA. The adjacent version comment
  is for Dependabot and human review only.
- Wasm optimization uses
  `cosmwasm/workspace-optimizer:0.16.1@sha256:b9c92b2900b7ebaab3499203615c1b8589592bc557355ed3432e48851ffde69e`.
  Update the tag and digest together in `Makefile`, the workflows, and the
  LocalTerra deploy helper after verifying the registry manifest.
- Security-tool source revisions are pinned in
  `.github/scripts/install-security-tools.sh` and `security.yml`. They use the
  runner's current stable compiler so they can parse current advisory formats;
  project compilation remains pinned to Rust 1.81.0.

## Candidate Validation

Run the local counterparts of the required source, security, and Wasm checks
before tagging:

```sh
make ci
make security
make reproducible
```

`make security` installs the pinned scanner revisions if needed and requires
network access. `make reproducible` requires Docker and writes optimized Wasm
and build evidence to `artifacts/release/`.

LocalTerra E2E is scheduled and manually dispatchable because it clones a
pinned upstream revision, starts privileged external services, and can be
network-sensitive. Its logs are retained as workflow artifacts. A successful
E2E run must pass on the exact release-candidate commit, but it is intentionally
not triggered by every pull request.

The grid system deploys strictly against the pinned CL8Y pair contract as
shipped; no pair modification, fork, or upstream dependency is permitted. The
two vault designs are split into two independent Cargo workspaces:
`limit-grid-system` and `market-grid-system`:

- `limit-grid-system` (`grid-vault`/`grid-manager`, the limit-order maker design)
  reconciles against the shipped pair using only its shipped queries. The vault
  records its own cancels locally and classifies any order that vanished without
  a recorded cancel as fully executed, so it needs no pair queries beyond
  `LimitOrder` and `ExpiredLimitRefund`, and no fork or upstream PR.
- `market-grid-system` (`grid-vault-swap`) is the swap-only design. It
  holds CW20 balances in the vault, reads the pool price, and executes classic
  `Swap` calls when the price crosses a grid level. No limit orders, no pair
  custody, no reconciliation state. It uses the exact pair API as shipped
  (`Pool`, `Observe`, `Swap` via CW20 hook) and requires no upstream merge.

The shared `grid-operator` and protocol documentation remain under
`grid-operator-system`.

## Publishing

1. Manually dispatch LocalTerra E2E for the exact candidate commit and confirm
   all required workflows pass on that same SHA.
2. Create and push an annotated signed tag, for example
   `git tag -s v0.1.0 -m 'v0.1.0' && git push origin v0.1.0`.
3. Review the `Signed release` run and its retained evidence.
4. Verify downloaded assets with `sha256sum -c SHA256SUMS` from the directory
   containing the release tree, and verify provenance with `gh attestation
   verify <artifact> --repo RenneBeau/CL8Y-DeX-bot-and-liquidity-contracts`.

GitHub environment protection and branch protection are repository settings
and cannot be represented fully in this tree. The repository currently requires
source, security, and Wasm checks on `main`, and owner approval through the
`release` environment. Verify those live settings before each release.
