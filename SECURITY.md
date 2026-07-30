# Security

This repository contains unaudited financial smart-contract software. Do not
deploy it with economic assets without an independent audit and a mainnet
oracle/TWAP review.

Please report vulnerabilities privately to the repository owner rather than
opening a public issue with exploit details.

Key deployment requirements:

- Use multisig control for proxy and vault administration.
- Use separate, constrained keeper keys.
- Configure a nonzero TWAP window for economic deployments.
- Verify CL8Y pair code and fee-registry addresses before registration.
- Test every new pair and token implementation independently.
- Do not use fee-on-transfer or rebasing CW20 tokens.
- Treat the experimental grid keeper and indexer as trusted global components.
- Retain archive-capable CL8Y fill history before operating grid bots.
- Use a dedicated grid keeper key separate from rebalance keeper keys.
- Do not expose owner-only grid shares to unrelated depositors without live NAV accounting.
- Independently audit the experimental grid manager before economic deployment.
