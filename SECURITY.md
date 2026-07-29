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
