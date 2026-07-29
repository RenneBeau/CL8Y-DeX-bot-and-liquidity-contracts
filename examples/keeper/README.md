# Keeper Example

`keeper.py` is a small reference keeper for one bot vault. It uses only the
Python standard library and `terrad`.

The keeper:

1. Queries the vault's configured pair and swap proxy.
2. Checks the vault's on-chain 5% price-movement trigger.
3. Compares vault token balances with the current ordered pool ratio.
4. Selects the excess token and calculates an improving trade amount.
5. Caps each trade to a configurable percentage of that token balance.
6. Queries CL8Y hybrid simulation using the proxy as the discounted trader.
7. Builds a keeper-signed vault `rebalance` transaction with a minimum return,
   maximum spread, and deadline.

Dry-run is the default. It prints the transaction without signing:

```sh
python3 examples/keeper/keeper.py \
  --vault terra1... \
  --lcd http://127.0.0.1:1317 \
  --once
```

To submit through a local `terrad` keyring:

```sh
python3 examples/keeper/keeper.py \
  --vault terra1... \
  --lcd http://127.0.0.1:1317 \
  --rpc http://127.0.0.1:26657 \
  --chain-id localterra \
  --key test1 \
  --broadcast
```

For Terra Classic mainnet, set `--chain-id columbus-5`, production LCD/RPC
URLs, a secure keeper keyring backend, and reviewed gas prices. Run the keeper
under a process supervisor and monitor failed transactions, RPC health, vault
allocation, and keeper-wallet LUNC balance.

The example intentionally cannot transfer vault assets. The vault independently
enforces the trigger, token list, deadline, spread, and post-trade allocation
improvement, so a bad keeper proposal reverts.

Run its calculation tests with:

```sh
make test-keeper
```
