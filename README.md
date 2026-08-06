# CL8Y DeX Bot And Liquidity Contracts

This repository contains two independent CL8Y trading systems. Each system has
its own Cargo workspace, contracts, protocol documentation, implementation
guide, and operating instructions.

## Systems

### Rebalancer

[`rebalancer-system`](rebalancer-system/README.md) is the portfolio rebalancing
system. It provides isolated two-token vaults, transferable bot LP tokens, and a
shared swap proxy through which approved vaults route their swaps. The proxy and
vault contracts are whitelisted on the CL8Y DEX, so the DEX charges them no
fees.

### Grid

The grid work is split into two independent Cargo workspaces plus a shared
operator/docs harness (`rebalancer-system` is the third, non-grid, system):

- [`market-grid-system`](market-grid-system/README.md) — the **deployable**
  standard swap grid (`grid-vault-swap`). The vault holds CW20 balances, reads
  the pool price, and executes classic `Swap` calls as the price crosses a grid
  level. Uses the exact CL8Y pair API as shipped; no fork or upstream merge.
- [`limit-grid-system`](limit-grid-system/README.md) — the limit-order grid
  (`grid-vault` + `grid-manager`). The vault reconciles against the shipped CL8Y
  DEX pair
  by tracking its own cancels and treating an order that vanished without a
  cancel as fully executed, so it needs no pair extension and no fork.
- [`grid-operator-system`](grid-operator-system/README.md) — the shared
  `grid-operator` worker and the grid protocol/ops documentation used by both
  designs.

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
- [Market grid (deployable swap)](market-grid-system/README.md)
- [Limit grid (reference only)](limit-grid-system/README.md)
- [Grid operator and protocol](grid-operator-system/README.md)
- [Security policy](SECURITY.md)
