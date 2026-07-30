# Grid Keeper And Admin Operations

## Deployment

Build and verify the isolated workspace:

```sh
cargo test --manifest-path grid-contract-system/Cargo.toml
cargo clippy --manifest-path grid-contract-system/Cargo.toml --all-targets -- -D warnings
cargo build --manifest-path grid-contract-system/Cargo.toml \
  --target wasm32-unknown-unknown --release
```

For Terra Classic-compatible optimized Wasm:

```sh
docker run --rm \
  -v "$PWD/grid-contract-system:/code" \
  cosmwasm/workspace-optimizer:0.16.1
```

Instantiate:

```json
{
  "admin": "<ADMIN_MULTISIG>",
  "keeper": "<DEDICATED_GRID_KEEPER>",
  "factory": "<CL8Y_FACTORY>",
  "gas_denom": "uluna",
  "keeper_reward": "30000000",
  "minimum_gas_reserve": "30000000",
  "max_grid_count": 20,
  "max_orders_per_reconcile": 20,
  "max_active_orders_per_bot": 100
}
```

Use a keeper key dedicated to grid reconciliation. Do not reuse a rebalance
vault keeper unless that operational coupling is intentional.

## Shared CL8Y Tier

The grid manager, not the keeper and not each user, owns all limit orders.
Transfer the tier's required CL8Y balance to the manager and have fee-registry
governance register the manager:

```json
{
  "register_wallet": {
    "wallet": "<GRID_MANAGER>",
    "tier_id": 5
  }
}
```

Verify the discount for trader and sender equal to the manager address.

## Create A Bot

Attach at least `minimum_gas_reserve + keeper_reward` in the configured native
denom:

```json
{
  "create_bot": {
    "pair": "<FACTORY_REGISTERED_PAIR>",
    "lower_price": "0.8",
    "upper_price": "1.2",
    "grid_count": 5
  }
}
```

Prices are token1 per token0. The current pool price must be strictly inside the
bounds.

## Deposit Assets

Encode this hook:

```json
{"deposit":{"bot_id":1}}
```

Then send either pair CW20 to the manager:

```json
{
  "send": {
    "contract": "<GRID_MANAGER>",
    "amount": "10000000",
    "msg": "<BASE64_DEPOSIT_HOOK>"
  }
}
```

Token0 deposits allocate across asks. Token1 deposits allocate across bids.

## Fund And Recover Gas

Add LUNC credit by attaching funds to:

```json
{"fund_gas":{"bot_id":1}}
```

Recover permitted credit:

```json
{
  "withdraw_gas": {
    "bot_id": 1,
    "amount": "30000000",
    "recipient": null
  }
}
```

Funded or active bots retain the configured reserve plus one keeper reward.

## Keeper Reconciliation

The indexer supplies one aggregate per changed order. The keeper sends:

```json
{
  "reconcile": {
    "bot_id": 1,
    "reports": [{
      "pair": "<CL8Y_PAIR>",
      "order_id": 77,
      "input_amount": "100",
      "output_amount": "200",
      "fill_count": 3
    }]
  }
}
```

The keeper loop serves every bot and pair with one wallet. Broadcast serially to
avoid account-sequence conflicts. Simulate first, wait for confirmation, and
advance the indexer checkpoint only after contract success.

One successful transaction receives one fixed keeper reward regardless of its
number of reports. Tune limits and reward from measured chain gas.

## Residual Allocation

If integer remainder or an active-order cap leaves free balances, the owner may
execute:

```json
{"allocate":{"bot_id":1}}
```

The contract, rather than the caller, chooses sides, prices, and amounts.

## Exit

Reconcile all indexed fills first. Then call bounded cancellation repeatedly:

```json
{"cancel_all":{"bot_id":1}}
```

Continue until `remaining_orders=0`. If cancellation returns `UnsettledOrder`,
the indexer and keeper must reconcile intervening fills before retrying.

Burn internal shares and withdraw free assets:

```json
{
  "withdraw": {
    "bot_id": 1,
    "shares": "<OWNER_SHARES>",
    "recipient": null
  }
}
```

## Keeper Rotation

The admin rotates the one global grid keeper:

```json
{"update_keeper":{"keeper":"<NEW_GRID_KEEPER>"}}
```

This changes reconciliation authority for every bot on every pair immediately.
It does not alter rebalance-vault keepers.

## Monitoring

Monitor:

- Grid keeper LUNC balance and account sequence
- Indexer finalized and confirmed checkpoints
- Oldest unreconciled fill
- Per-bot gas credit
- Per-bot active-order count
- Deferred allocation attributes
- Failed escrow/report validation
- Manager CL8Y balance and fee tier
- Pair pause and blacklist state

## Incident Response

If the indexer is unavailable, stop reconciliation and preserve checkpoints. Do
not guess fill output. If the keeper is compromised, rotate it globally before
resuming. Rebuild indexer aggregates from archived raw events, compare them with
current pair escrow, then reconcile in finalized order.

If an owner wants to exit during an incident, all changed orders still require
trusted indexed reconciliation before cancellation can safely proceed.

## Verification

```sh
make local-setup
make local-grid
make local-e2e
```

The grid suite uses one dedicated keeper for four bots across two unchanged
standard CL8Y pairs, performs fills on both pairs, rejects an unauthorized
keeper, verifies sibling isolation, and withdraws every bot.
