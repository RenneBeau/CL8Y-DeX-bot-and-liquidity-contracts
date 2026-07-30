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

## Replace The Keeper

```json
{
  "update_keeper": {
    "keeper": "<NEW_KEEPER_ADDRESS>"
  }
}
```

The new keeper becomes active immediately.

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

After `rebalance_status` reports `should_rebalance: true`, the keeper executes:

```json
{
  "rebalance": {
    "params": {
      "offer_token": "<TOKEN_A_OR_B>",
      "amount": "<OFFER_AMOUNT>",
      "min_return": "<MINIMUM_OUTPUT>",
      "max_spread": "0.05",
      "deadline": 1800000000
    }
  }
}
```

See [`examples/keeper/README.md`](../examples/keeper/README.md) for key creation,
gas funding, vault registration, dry runs, signing, service operation, and key
rotation.
