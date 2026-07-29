# CL8Y Inventory Bot Contracts

This workspace contains a clean three-contract architecture for token inventory
bots using CL8Y DEX as a swap venue. The bot never provides DEX liquidity and
never holds CL8Y DEX LP tokens.

## Contracts

### Shared swap proxy

`contracts/swap-proxy` holds CL8Y for one governance-assigned fee tier. It
supports multiple registered vault/pair routes, accepts swaps only from the
vault assigned to that pair, forwards the exact received CW20 amount to CL8Y,
and sends output directly back to that vault.

### Bot vault

`contracts/bot-vault` is instantiated once per bot/pair. It holds only the
pair's token A and token B for accounting purposes. Its assigned liquidity
contract controls user deposits and withdrawals; its keeper can perform a
constrained inventory rebalance after a configurable pool-price movement. The
default trigger is 5%.

### Bot liquidity token

`contracts/bot-liquidity` is instantiated once per vault and implements that
bot's transferable CW20 ownership token. Deposits are pulled directly into the
assigned vault. Shares are minted only in a reply after transfers and any swap
settle. Withdrawals burn shares against proportional vault balances and support
balanced, token-A-only, or token-B-only output.

The first mint permanently locks 1,000 smallest share units. Direct token
donations are included in pre-deposit NAV and cannot be captured by the next
depositor. This version charges no protocol fee.

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
