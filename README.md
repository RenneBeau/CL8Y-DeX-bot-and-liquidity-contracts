# CL8Y DeX Bot And Liquidity Contracts

This repository contains two independent CL8Y trading systems. Each system has
its own Cargo workspace, contracts, protocol documentation, implementation
guide, and operating instructions.

## Systems

### Rebalancer

[`rebalancer-system`](rebalancer-system/README.md) is the portfolio rebalancing
system. It provides isolated two-token vaults, transferable bot LP tokens, and a
shared swap proxy through which approved vaults can use one governance-assigned
CL8Y fee tier.

### Grid

[`grid-contract-system`](grid-contract-system/README.md) is the experimental
one-owner, one-bot-per-vault limit-order grid system. It runs on standard CL8Y
pairs. Reconciliation is permissionless and derives amounts on-chain; the
indexer/operator is optional automation, and only its configured keeper address
is eligible for useful-reconciliation reimbursement.

## Verification

```sh
make test
make clippy
make optimize
make local-all
```

See [the verification report](docs/TEST_RESULTS.md) for the tested scenarios and
[the LocalTerra test guide](test-area/README.md) for the complete local workflow.
All contracts are unaudited and require an independent security review before
an economic deployment. The grid system additionally remains experimental.

## Repository Guides

- [Rebalancer overview](rebalancer-system/README.md)
- [Rebalancer implementation](rebalancer-system/IMPLEMENTATION.md)
- [Grid overview](grid-contract-system/README.md)
- [Grid implementation](grid-contract-system/IMPLEMENTATION.md)
- [Security policy](SECURITY.md)
