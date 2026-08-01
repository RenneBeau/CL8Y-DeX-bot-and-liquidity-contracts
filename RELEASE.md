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

Grid releases that include legacy inventory reconciliation additionally require
the owner-inventory pair API from
[PlasticDigits/cl8y-dex-terraclassic PR #1](https://github.com/PlasticDigits/cl8y-dex-terraclassic/pull/1).
The reviewed development reference is fork commit
`c1f669b06c98936005b665cf56d5540a33a49edd`; it is not a production dependency
pin until the upstream PR is merged and the release records the merged revision.
Rehearse with funded historical-state fixtures: upgrade pair first, finish owner
index backfill, migrate and re-pin each vault, drain/rescan, clean local rows, and
verify exact CW20 balance synchronization. Do not claim deployability without the
merged pin and successful funded rehearsal.

Monitor reconciliation phase, snapshot generation/high-water, scan cursor,
recovered-record count, pending action, last error, pair pages, local cleanup
count, and the withdrawal gate. After a
snapshot is captured, pair rollback or owner-index generation change invalidates
the proof. Roll forward to the verified pair generation instead; rollback is only
an option before any affected vault starts reconciliation. If a vault code rollback
does occur, all saved reconciliation phases, including `Complete`, are treated as
untrusted and the next supported migration must lock and repeat the pair proof.

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
