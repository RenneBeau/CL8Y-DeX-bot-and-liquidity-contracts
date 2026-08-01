# Admin And Keeper Operations

## Update The Rebalance Trigger

The vault admin can change the trigger at any time by executing on the vault:

```json
{
  "update_thresholds": {
    "rebalance_threshold_bps": 750,
    "allocation_tolerance_bps": null
  }
}
```

| Value | Trigger |
|---:|---:|
| `100` | 1% |
| `500` | 5% |
| `750` | 7.5% |
| `1000` | 10% |

The accepted range is 1 through 10,000 basis points. The update applies
immediately and keeps the existing reference price.

## Update Allocation Tolerance

```json
{
  "update_thresholds": {
    "rebalance_threshold_bps": null,
    "allocation_tolerance_bps": 300
  }
}
```

Both values can also be changed in one transaction.

## Update The TWAP Window

The vault admin can change the TWAP observation window with the same message.
It must be greater than zero:

```json
{
  "update_thresholds": {
    "rebalance_threshold_bps": null,
    "allocation_tolerance_bps": null,
    "twap_window_seconds": 600
  }
}
```

## Update Risk Controls

The same message can update `max_trade_bps`,
`max_execution_deviation_bps`, `quote_slippage_bps`,
`max_spot_twap_deviation_bps`, `max_trade_pool_bps`, and `max_spread`; omitted
or null fields retain their current values. Hard maxima are respectively 5,000,
1,000, 500, 1,000, and 2,000 basis points and 10% spread. These bounds cannot be
overridden by the admin or keeper.

## Replace The Keeper

```json
{
  "update_keeper": {
    "keeper": "<NEW_KEEPER_ADDRESS>"
  }
}
```

The new keeper becomes active immediately.

## Update The Liquidity Minimum Initial Deposit

The liquidity contract admin can adjust the minimum initial deposit for the
first LP entry at any time:

```json
{
  "update_config": {
    "minimum_initial_deposit": "250000"
  }
}
```

Omitted fields retain their current values. The update applies to the next
initial deposit; existing positions are unaffected.

## Transfer Vault Administration

```json
{
  "transfer_admin": {
    "admin": "<NEW_ADMIN_MULTISIG>"
  }
}
```

## Add Or Remove A Proxy Route

Add:

```json
{
  "register_vault": {
    "vault": "<VAULT_ADDRESS>",
    "pair": "<PAIR_ADDRESS>"
  }
}
```

Remove:

```json
{
  "remove_vault": {
    "vault": "<VAULT_ADDRESS>"
  }
}
```

Removing a route pauses that vault's proxy swaps until registration is restored.

## Proxy CL8Y Management

```json
{
  "withdraw_cl8y": {
    "amount": "<RAW_CL8Y_AMOUNT>",
    "recipient": "<RECIPIENT>"
  }
}
```

The remaining proxy balance should satisfy its assigned tier.

Transfer proxy administration:

```json
{
  "transfer_admin": {
    "admin": "<NEW_PROXY_ADMIN_MULTISIG>"
  }
}
```

## Keeper Rebalance

After `rebalance_plan` reports `should_rebalance: true` and an offer, the keeper
executes only the deadline-bearing command:

```json
{
  "rebalance": {
    "deadline": 1800000000
  }
}
```

The vault captures a fresh TWAP and derives all economic parameters during
execution. If the plan has no offer because allocation is already within
tolerance, execute `{"sync_reference":{}}` instead.

See [`examples/keeper/README.md`](../examples/keeper/README.md) for key creation,
gas funding, vault registration, dry runs, signing, service operation, and key
rotation.
