# Grid Operations

## Deployment

1. Build and upload both `cl8y_grid_vault.wasm` and
   `cl8y_grid_manager.wasm`.
2. Instantiate the manager with the uploaded vault code ID, CL8Y factory, reviewed
   operator addresses, bounded limits, a nonzero `order_timeout_seconds`, and
   paired fee addresses.
3. Users call `{"create_vault":{"label":null}}`; query the returned vault ID or
   manager owner index for the dedicated address.
4. The designated owner calls `create_bot` on that vault and attaches at least
   `minimum_gas_reserve + keeper_reward` in the configured gas denom.
5. Limit-grid has no swap-proxy and interacts directly with the CL8Y pair. The
   manager propagates `fee_registry` and `fee_collector` to each new vault; there
   is no later vault fee-config update message.

Manager example:

```json
{
  "admin":"<MULTISIG>",
  "keeper":"<REIMBURSED_KEEPER_ADDRESS>",
  "dex_factory":"<CL8Y_FACTORY>",
  "vault_code_id":123,
  "gas_denom":"uluna",
  "keeper_reward":"30000000",
  "minimum_gas_reserve":"30000000",
  "order_timeout_seconds":604800,
  "max_grid_count":20,
  "max_orders_per_reconcile":20,
  "max_active_orders_per_vault":100,
  "fee_registry":"<FEE_REGISTRY>",
  "fee_collector":"<FEE_COLLECTOR>"
}
```

The keeper address is required, but reconciliation remains permissionless; only
that configured address receives reimbursement. Hard ranges are
`max_grid_count: 2..=100`, `max_orders_per_reconcile: 1..=100`, and
`max_active_orders_per_vault: max_grid_count..=500`. Keeper reward and order
timeout must be nonzero.

The manager rejects a partial fee pair and requires both values in `mainnet`.
Mainnet artifacts also compile-time pin both registry and collector; missing
registry input fails compilation. Limit-grid has no proxy requirement.
Admin updates to manager fee configuration affect future vaults only. Existing
fee-disabled vaults require reviewed migration or redeployment; direct vault
instantiation requires the exact fields documented in
`../../docs/DEPLOY_FEE_SYSTEM.md`.

## Maintenance

Reconciliation is permissionless:

```json
{"reconcile":{"bot_id":1,"order_ids":[77,78]}}
```

No off-chain amounts are accepted. After reconciliation, the owner may call
`{"allocate":{"bot_id":1}}` to place verified free proceeds.

If unsolicited pair-token transfers make physical balances exceed accounting,
the owner can credit them without minting shares or placing orders:

```json
{"sync_balances":{"bot_id":1}}
```

If a CL8Y pair is migrated to new code after an approved upgrade, the vault
aborts pair interactions until the admin re-pins the verified code ID:

```json
{"update_pair_code":{"bot_id":1,"code_id":<NEW_CODE_ID>}}
```

## Recovery

If the keeper or indexer disappears, query the vault's `orders`, submit their IDs
to `reconcile`, then use bounded `cancel_all`. Historical events are unnecessary.

If the pair's code ID no longer matches the pinned ID, pair interactions are
disabled by design; re-pin only after independently verifying the replacement
pair, then continue normal recovery.

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

`emergency_withdraw` transfers only the owner's pro-rata assets and preserves
collector backing and remaining total shares. After active orders reach zero,
the fee collector may redeem its shares while the vault remains in Exit.

## Monitoring

- Vault creation and exact instantiate-time fee wiring.
- Fee source changes among registry `Live`/`Lowest` and vault
  `vault_cached`/`lowest`; alert on registry outages and prolonged local cache use.
- Oldest active order versus configured timeout.
- Pair pause, blacklist, and code migration state.
- `solvency` per vault after reconciliation: `expected` versus `actual`
  liquid-plus-escrow totals, escrow-state categories, and warnings. Before
  reconciliation, opposite-token fill conversion can legitimately create drift.
- Vault gas credit and active-order count.
- CW20 balance-delta mismatch errors.
- Reconciliation/cancellation query errors and unresolved exits.
- Manager/vault admin and code-ID changes.

Do not deploy arbitrary CW20 assets. First bot creation enables a fail-closed
policy and admits only its factory-verified pair tokens. Admin allowlist changes
govern future pair admission; quarantine is the runtime disable control for an
existing bot. Exact-transfer, non-rebasing behavior and honest balance queries
remain mandatory.
