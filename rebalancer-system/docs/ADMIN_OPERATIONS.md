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
It must be between 1 and 86,400 seconds, and the pair must already have enough
history for the requested window. A successful update also resets the reference
price to the validated new-window TWAP:

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

The liquidity contract admin can adjust the minimum initial deposit only before
the first LP mint. The value must exceed the 1,000 permanently locked share
units:

```json
{
  "update_config": {
    "minimum_initial_deposit": "250000"
  }
}
```

Omitted fields retain their current values. Once supply is nonzero, updates are
rejected because this bootstrap-only setting can no longer affect deposits.

## Transfer Vault Administration

```json
{
  "transfer_admin": {
    "admin": "<NEW_ADMIN_MULTISIG>"
  }
}
```

`transfer_admin` only proposes. The current admin remains active until the exact
candidate executes `{"accept_admin":{}}`. Before acceptance, the current admin may
execute `{"cancel_admin_transfer":{}}` or replace the proposal. Apply the same
two-step process independently to bot-vault, bot-liquidity, and swap-proxy. This
does not transfer the chain-level Wasm migration admin.

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

## Proxy Authority

The proxy is a pure router and holds no tokens, so there is nothing to withdraw
from it. Transfer proxy administration:

```json
{
  "transfer_admin": {
    "admin": "<NEW_PROXY_ADMIN_MULTISIG>"
  }
}
```

The proposed proxy admin must execute `{"accept_admin":{}}`; the current proxy
admin may execute `{"cancel_admin_transfer":{}}` first.

## Emergency Pause

The vault admin pauses new deposits, swaps and keeper maintenance with:

```json
{"pause":{}}
```

Pro-rata transfer settlement remains available. Single-token withdrawals that
require a swap fail without burning shares. `revoke_liquidity_contract` also pauses
the vault. After verifying and rebinding the approved controller, resume with:

```json
{"resume":{}}
```

Follow [`RELEASE_READINESS.md`](RELEASE_READINESS.md) before resuming.

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
