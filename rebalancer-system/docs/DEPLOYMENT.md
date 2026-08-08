# Deployment And Pair Setup

This guide explains how to upload the contracts, initialize the shared swap
proxy, connect a CL8Y pair, deploy a vault and bot LP token for that pair, and
change the bot settings later.

## Upgrade Policy

The contracts expose guarded `migrate` entry points that verify the cw2 contract
identity and preserve existing storage. Rebalancer vault migration deliberately
revokes the legacy liquidity binding because older releases did not verify it;
the admin must bind the deployed bot-liquidity contract again after migration.
Pending admin proposals are cleared. Internal admin migration does not transfer
the chain-level Wasm admin. Migrated vaults start paused and require a verified
liquidity rebind before `resume`. Rehearse the exact source version and funded-state
fixture before any production upgrade; the current local migration tests are not
a substitute for a chain rehearsal.

Release gates, multisig separation, canary caps, pause commands and monitoring
stop conditions are defined in [`RELEASE_READINESS.md`](RELEASE_READINESS.md).

## Contract Instances

Upload each Wasm code once:

- `cl8y_swap_proxy.wasm`: one shared instance for the protocol.
- `cl8y_bot_vault.wasm`: one new instance for every CL8Y pair bot.
- `cl8y_bot_liquidity.wasm`: one new instance for every vault.

For ten bot pairs, the normal deployment is one proxy, ten vaults, and ten bot
LP contracts.

## Build And Upload

Run the build and upload commands from the `rebalancer-system` directory.

Use the optimizer version tested by this repository:

```sh
docker run --rm \
  -v "$PWD:/code" \
  --mount type=volume,source=cl8y_bot_target_cache,target=/code/target \
  --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
  cosmwasm/workspace-optimizer:0.16.1@sha256:b9c92b2900b7ebaab3499203615c1b8589592bc557355ed3432e48851ffde69e
```

The Wasm files are written to `artifacts/`. Upload each one with `terrad` and
record its code ID:

```sh
terrad tx wasm store artifacts/cl8y_swap_proxy.wasm $TX_FLAGS
terrad tx wasm store artifacts/cl8y_bot_vault.wasm $TX_FLAGS
terrad tx wasm store artifacts/cl8y_bot_liquidity.wasm $TX_FLAGS
```

Example flags:

```sh
TX_FLAGS="--from <DEPLOYER_KEY> --keyring-backend <BACKEND> \
  --chain-id <CHAIN_ID> --node <RPC_URL> --gas auto \
  --gas-adjustment 1.4 --gas-prices <GAS_PRICE> --broadcast-mode sync -y"
```

Wait for every transaction to be included before using its code ID or address.

## Create A CL8Y Pair

Skip this section when the desired pair already exists.

Execute the CL8Y factory with the two CW20 addresses in their intended order:

```json
{
  "create_pair": {
    "asset_infos": [
      { "token": { "contract_addr": "<TOKEN_A>" } },
      { "token": { "contract_addr": "<TOKEN_B>" } }
    ]
  }
}
```

The factory can require a native `uluna` pair-creation fee. Query its `config`
and attach the exact `pair_creation_fee_uluna` amount. Initialize the new CL8Y
pool reserves through the normal CL8Y pool setup process, then record its pair
address. The vault reads token order directly from this pair.

## Initialize The Shared Proxy

Instantiate one proxy for the protocol:

```json
{
  "admin": "<PROXY_ADMIN_MULTISIG>"
}
```

```sh
terrad tx wasm instantiate <PROXY_CODE_ID> '<JSON_ABOVE>' \
  --label cl8y-shared-swap-proxy \
  --admin <PROXY_ADMIN_MULTISIG> $TX_FLAGS
```

Record `PROXY_ADDRESS`. The proxy never holds tokens: Vaults route their pool
swaps through it, and because the proxy is whitelisted on the CL8Y DEX those
swaps pay no DEX fee. Protocol fees are pushed directly to the fee-collector by
the vault itself.

## Deploy A Bot For A Pair

Repeat these steps for every new pair.

### 1. Instantiate The Vault

```json
{
  "admin": "<VAULT_ADMIN_MULTISIG>",
  "keeper": "<KEEPER_WALLET>",
  "proxy": "<PROXY_ADDRESS>",
  "pair": "<PAIR_ADDRESS>",
  "twap_window_seconds": 60,
  "rebalance_threshold_bps": 500,
  "allocation_tolerance_bps": 500,
  "max_trade_bps": 2500,
  "max_execution_deviation_bps": 500,
  "quote_slippage_bps": 200,
  "max_spot_twap_deviation_bps": 500,
  "max_trade_pool_bps": 1000,
  "max_spread": "0.05"
}
```

```sh
terrad tx wasm instantiate <VAULT_CODE_ID> '<JSON_ABOVE>' \
  --label <TOKEN_A_SYMBOL>-<TOKEN_B_SYMBOL>-bot-vault \
  --admin <VAULT_ADMIN_MULTISIG> $TX_FLAGS
```

Settings:

- `rebalance_threshold_bps: 500` means a 5% price movement.
- `allocation_tolerance_bps: 500` allows a 5% ratio deviation.
- `twap_window_seconds` must be between 1 and 86,400 and requires existing CL8Y
  cumulative-observation history for the entire window.
- Hard maxima are 50% of the offered balance per trade, 10% TWAP execution
  deviation, 5% quote slippage, 10% spot/TWAP deviation, 20% of the offered-side
  pool reserve, and 10% CL8Y spread.
- `60` is a short-term starting point, not a universal safe value. Benchmark
  30-300 seconds against the pair's liquidity, block time, and volatility.

Record `VAULT_ADDRESS`.

### 2. Instantiate The Bot LP Contract

The bot LP decimals must match the pair-token decimals. The vault requires both
pair tokens to use the same decimals.

```json
{
  "admin": "<VAULT_ADMIN_MULTISIG>",
  "vault": "<VAULT_ADDRESS>",
  "name": "TOKEN A TOKEN B Bot Liquidity",
  "symbol": "ABOTLP",
  "decimals": 6,
  "minimum_initial_deposit": "100000",
  "marketing": null
}
```

`minimum_initial_deposit` is bootstrap-only, must exceed 1,000 smallest share
units, and cannot be changed after the first LP mint.

```sh
terrad tx wasm instantiate <LIQUIDITY_CODE_ID> '<JSON_ABOVE>' \
  --label <TOKEN_A_SYMBOL>-<TOKEN_B_SYMBOL>-bot-liquidity \
  --admin <VAULT_ADMIN_MULTISIG> $TX_FLAGS
```

Record `LIQUIDITY_ADDRESS`.

### 3. Connect The Liquidity Contract

The vault admin executes this on `VAULT_ADDRESS`:

```json
{
  "set_liquidity_contract": {
    "liquidity_contract": "<LIQUIDITY_ADDRESS>"
  }
}
```

The vault instantiate message must contain the governance-approved uploaded
`liquidity_code_id`. The vault rejects an account, unrelated contract, or any
candidate whose current code ID differs. It queries the candidate's
configured vault and ordered assets, records its current Wasm code ID, and checks
that code ID on every swap, transfer, and finalize call. Query vault `config` and
record both `liquidity_contract` and `liquidity_code_id`. Treat the liquidity
contract and its chain migration admin as custodial components. Use
`revoke_liquidity_contract` immediately if the binding is compromised; restoring
service requires binding a valid controller again.

### 4. Register The Vault With The Proxy

The proxy admin executes:

```json
{
  "register_vault": {
    "vault": "<VAULT_ADDRESS>",
    "pair": "<PAIR_ADDRESS>"
  }
}
```

The proxy verifies the pair, ordered token addresses, vault, and proxy before
saving the route. One vault route can be registered for each CL8Y pair.

### 5. Verify Initialization

Query these endpoints:

```text
Proxy:     {"route":{"vault":"<VAULT_ADDRESS>"}}
Proxy:     {"config":{}}
Vault:     {"config":{}}
Vault:     {"balances":{}}
Vault:     {"rebalance_status":{}}
Bot LP:    {"config":{}}
Bot LP:    {"token_info":{}}
Registry:  {"effective_fee":{"trader":"<HOLDER_ADDRESS>"}}
```

Confirm the pair and token order, keeper, liquidity address, initial zero bot
LP supply, and that the proxy config exposes only the admin. Protocol fee tiers
are resolved per LP holder through the fee-registry at fill time.

## First Deposit

The user grants the bot LP contract allowances on token A and token B, then
executes on `LIQUIDITY_ADDRESS`:

```json
{
  "deposit": {
    "amounts": ["<TOKEN_A_AMOUNT>", "<TOKEN_B_AMOUNT>"],
    "min_shares": "<MINIMUM_BOT_LP>",
    "deadline": 1800000000,
    "swap": null
  }
}
```

For a single-token deposit, set one amount to zero and include the bounded
`swap` object. Shares mint after transfers and the swap settle.

## Withdrawal At The Vault Ratio

```json
{
  "withdraw": {
    "shares": "<BOT_LP_AMOUNT>",
    "recipient": null,
    "deadline": 1800000000,
    "output": {
      "pro_rata": {
        "min_assets": ["<MIN_A>", "<MIN_B>"]
      }
    }
  }
}
```

The other output variants are `token0` and `token1`. They include a swap for
exactly the user's proportional unwanted-token claim.

## LocalTerra Example

From the repository root, the test harness automates the complete sequence:

```sh
make local-setup
```

See `test-area/deploy-system.sh`. Generated addresses are written to the ignored
`test-area/.env` file.
