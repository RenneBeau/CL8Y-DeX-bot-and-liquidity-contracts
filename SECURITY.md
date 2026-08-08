# Security

This repository contains unaudited financial smart-contract software. Economic
deployment requires an independent audit and a mainnet oracle/TWAP review.

Please report vulnerabilities through
[GitHub's private vulnerability reporting](https://github.com/RenneBeau/CL8Y-DeX-bot-and-liquidity-contracts/security/advisories/new)
and reserve public issues for non-sensitive reports.

Key deployment requirements:

- Use multisig control for proxy and vault administration.
- Use separate, constrained keeper keys.
- Configure and benchmark a nonzero TWAP window appropriate to the strategy;
  shorter reaction windows require stronger liquidity and manipulation checks.
- Verify the canonical fee-registry, fee-collector, and swap-proxy addresses.
- Require factory lookup and the approved runtime pair code ID for every
  market-grid/rebalancer vault, and recheck pair code before execution.
- Test every new pair and token implementation independently.
- Do not use fee-on-transfer or rebasing CW20 tokens.
- Treat the experimental grid operator as optional automation, not an accounting
  authority; reconciliation accepts order IDs only and derives custody on-chain.
- Retain archive-capable CL8Y fill history if the current operator must rebuild
  its automated queue after database loss; owner recovery itself is independent
  of fill history.
- Use a dedicated grid keeper key separate from rebalance keeper keys.
- Support for unrelated grid depositors requires live NAV share accounting.
- Independently audit the experimental grid manager before economic deployment.
- Redeploy market-grid and bot-vault 0.1.x. Replace any routed swap-proxy 0.1.x
  with a fresh 0.2.0 proxy and re-register routes; migrate only empty compatible
  proxy state. Never migrate bot-liquidity 0.1.x because no trusted admin can be
  derived. Limit grid-vault 0.1.0 to 0.1.1 remains supported.

Dependency policy allows `RUSTSEC-2024-0344` only for
`curve25519-dalek` 3.2.0 in all four current lockfiles, transitively through
CosmWasm 1.5.11/ed25519-zebra 3.1.0, and only until 2027-02-01 UTC. Validation
fails on expiry, disappearance, package/version drift, or an extra vulnerability;
there is no global `cargo audit --ignore`. `curve25519-dalek` is absent from the
`wasm32-unknown-unknown` contract graph. The `RUSTSEC-2024-0388` derivative 2.2.0
unmaintained warning is informational, not a vulnerability exception.
