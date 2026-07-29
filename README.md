# CL8Y DeX Bot And Liquidity Contracts

This project lets users pool two CW20 tokens in an automated trading bot. Each
bot keeps its token A and token B portfolio in an isolated vault, rebalances the
portfolio when the market moves, and gives depositors transferable bot LP
tokens representing their share of the vault. A shared swap proxy holds CL8Y so
all approved bot vaults can benefit from the proxy's assigned CL8Y fee tier.

## Contracts

### Shared swap proxy

`contracts/swap-proxy` connects approved bot vaults to their CL8Y trading pools.
It holds the CL8Y balance used for one governance-assigned fee tier and shares
that discount across every registered vault. Swap output returns directly to
the vault that requested the trade.

### Bot vault

`contracts/bot-vault` is the portfolio account for one bot and one token pair.
It stores the users' combined token A and token B inventory. Its liquidity
contract handles deposits and withdrawals, while its keeper rebalances the
portfolio after a configurable pool-price movement. The default trigger is 5%.

### Bot liquidity token

`contracts/bot-liquidity` manages users and issues the transferable CW20 bot LP
token for one vault. It sends deposits into the vault, mints shares after the
deposit settles, burns shares on withdrawal, and supports withdrawals as both
tokens, token A only, or token B only.

The first mint permanently locks 1,000 smallest share units. Direct token
donations are included in pre-deposit NAV and cannot be captured by the next
depositor. Deposits, withdrawals, and rebalances currently use a zero protocol
charge.

## Price Source

Vaults support CL8Y's arithmetic TWAP through `twap_window_seconds`. LocalTerra
uses `0` for immediate spot-price testing. An economic deployment should use a
nonzero window, normally at least 1,800 seconds, and should not rely on a
low-liquidity pool as its only oracle.

## CL8Y Tier

The shared proxy cannot self-register. CL8Y fee-registry governance must call
`register_wallet` for the proxy. Standard tiers still require the proxy to hold
the tier's minimum CL8Y balance.

## Verification

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
make local-setup
make local-e2e
SOAK_ROUNDS=100 make local-soak
```

See `test-area/README.md` for the LocalTerra lifecycle.

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Shared swap proxy protocol](docs/protocols/SWAP_PROXY.md)
- [Bot vault protocol](docs/protocols/BOT_VAULT.md)
- [Bot liquidity token protocol](docs/protocols/BOT_LIQUIDITY.md)
- [Verification report](docs/TEST_RESULTS.md)
- [Security notice](SECURITY.md)
