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
- Verify CL8Y pair code and fee-registry addresses before registration.
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

Dependency scanning ignores `RUSTSEC-2024-0344` only for the pinned CosmWasm
1.5 host dependency. `curve25519-dalek` is absent from the
`wasm32-unknown-unknown` contract graph, and the host path verifies public
signatures without handling secret scalars. Reassess and remove this exception
when upgrading the CosmWasm dependency family.
