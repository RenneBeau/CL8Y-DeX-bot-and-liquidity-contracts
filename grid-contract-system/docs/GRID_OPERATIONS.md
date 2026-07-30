# Grid Operations

## Deployment

1. Build and upload both `cl8y_grid_vault.wasm` and
   `cl8y_grid_manager.wasm`.
2. Instantiate the manager with the uploaded vault code ID, CL8Y factory, reviewed
   operator addresses, bounded limits, and a nonzero `order_timeout_seconds`.
3. Users call `{"create_vault":{"label":null}}`; query the returned vault ID or
   manager owner index for the dedicated address.
4. The designated owner calls `create_bot` on that vault and attaches at least
   `minimum_gas_reserve + keeper_reward` in the configured gas denom.
5. Register each vault address independently for its CL8Y fee tier.

Manager example:

```json
{
  "admin":"<MULTISIG>",
  "keeper":"<OPTIONAL_KEEPER>",
  "dex_factory":"<CL8Y_FACTORY>",
  "vault_code_id":123,
  "gas_denom":"uluna",
  "keeper_reward":"30000000",
  "minimum_gas_reserve":"30000000",
  "order_timeout_seconds":604800,
  "max_grid_count":20,
  "max_orders_per_reconcile":20,
  "max_active_orders_per_vault":100
}
```

## Maintenance

Reconciliation is permissionless:

```json
{"reconcile":{"bot_id":1,"order_ids":[77,78]}}
```

No off-chain amounts are accepted. After reconciliation, the owner may call
`{"allocate":{"bot_id":1}}` to place verified free proceeds.

## Recovery

If the keeper or indexer disappears, query the vault's `orders`, submit their IDs
to `reconcile`, then use bounded `cancel_all`. Historical events are unnecessary.

For stale state or incident response, the owner uses:

```json
{"enter_exit":{"bot_id":1}}
{"emergency_cancel":{"bot_id":1}}
{"emergency_withdraw":{"bot_id":1,"recipient":null}}
```

Repeat `emergency_cancel` until no tracked orders remain. Orders carry pair-level
expiry, but CL8Y may require a matching/cleanup walk before an expired row is
parked and claimable. If the pair is paused or queries fail, retain the vault and
retry after pair health is restored.

## Monitoring

- Vault creation and fee-tier registration.
- Oldest active order versus configured timeout.
- Pair pause, blacklist, and code migration state.
- Vault gas credit and active-order count.
- CW20 balance-delta mismatch errors.
- Reconciliation/cancellation query errors and unresolved exits.
- Manager/vault admin and code-ID changes.

Do not deploy arbitrary CW20 assets. Exact-transfer, non-rebasing behavior and
honest balance queries are mandatory until an explicit token allowlist is added.
