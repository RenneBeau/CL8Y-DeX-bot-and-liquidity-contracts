# Trust-Minimized Grid Protocol

## Architecture

`contracts/grid-manager` is a non-custodial factory and registry. It instantiates
`contracts/grid-vault` and records `(vault_id, owner, vault_address)`. It has no
CW20 receive entry point, balance ledger, order map, or asset-withdrawal message.

Each grid vault has one designated owner and permits one bot, `bot_id: 1`. The
vault address is the CL8Y maker and holds only that owner's liquid assets. CL8Y
cancellation and expired-order claims always return assets to the recorded maker,
so another vault, the manager, admin, keeper, and indexer cannot redirect them.

## Threat Model

The design protects against:

- A keeper inventing fill output, replaying reconciliation, disappearing, or
  submitting another vault's order ID.
- Indexer loss, incomplete historical fill events, and incorrect event amounts.
- One owner becoming insolvent or attempting to withdraw another owner's assets.
- Stale internal remaining values during owner recovery.
- A CW20 `Send` callback claiming an amount inconsistent with the vault's queried
  liquid balance.

The design does not protect against:

- A malicious CW20 that lies consistently in smart queries or violates execution
  semantics. Production manager configuration must be paired with a reviewed-token
  allowlist or equivalent governance policy.
- CL8Y pair governance pausing cancellation and claims, changing the wire
  interface, or maliciously violating its documented custody behavior.
- Pair migration. The vault pins the pair code ID at bot creation and rejects
  reconcile/cancel/allocate/deposit interactions after a code change, but the
  admin must re-pin the replacement code ID only after independently verifying
  the new implementation.
- Temporary inability to distinguish a missing order from an unrelated pair query
  error. Emergency processing is owner-only and should be retried after pair/RPC
  health is restored.

- Exact event-by-event opposite-rung recreation. The current pair does not retain
  cumulative maker output, and per-fill rounding prevents exact reconstruction
  from aggregate escrow reduction.

At first bot creation, a previously unconfigured vault enables its token policy
and atomically allowlists only the two assets returned by the factory-verified
pair. The admin may add operational tokens such as CL8Y later. Once enabled, an
empty allowlist denies every token; removing the final entry never reopens the
policy.

## Roles And Trust

- `owner`: controls deposits, allocation, cancellation, exit, and recipients.
- `keeper`: optional reimbursed operator. It has no exclusive reconciliation
  authority and supplies no accounting amounts.
- `admin`: configures the factory/vault fleet and pause state. It cannot withdraw
  vault CW20 balances.
- `indexer`: optional availability and discovery service. Its data is advisory.
- `CL8Y pair`: trusted to enforce maker ownership and transfer correct fills and
  refunds. This is the unavoidable external custody dependency.
- `CW20 assets`: required to have standard, exact-transfer, non-rebasing behavior.

## Initialization

The manager stores the vault code ID, CL8Y factory, operator settings, safety
limits, and `order_timeout_seconds`. `create_vault` instantiates a vault with the
caller as its designated owner and Wasm migration admin. The owner then creates the sole bot and prepays
its native gas reserve.

The vault admits only a factory-registered CL8Y CW20/CW20 pair with distinct
assets, equal decimals, nonzero reserves, compatible batch limits, valid price
bounds, and at least one bid and ask rung. At bot creation it pins the pair's
`contract_info.code_id`; every later pair interaction re-queries the code ID and
rejects a mismatch.

## Lifecycle

### Deposit

Only the designated bot owner may use the CW20 `deposit` hook. During the callback,
the vault queries its own CW20 balance. The observed liquid balance must equal the
previous free balance plus the callback amount; otherwise the transaction rejects
the token behavior. Accepted assets are divided over the applicable initial rungs.

### Order Creation

The vault reserves free balance before sending CW20 assets to the pair. Every
order has `expires_at = current_time + order_timeout_seconds`. In the reply, the
vault extracts order IDs and queries each active row to verify owner, side, price,
and post-maker-fee remaining escrow. A malformed reply reverts atomically.

### Partial Fill And Reconciliation

Anyone may call:

```json
{"reconcile":{"bot_id":1,"order_ids":[77,78]}}
```

The caller cannot provide input, output, fill count, pair, recipient, or price.
For active orders, the vault verifies immutable metadata and updates remaining
escrow monotonically. It then queries both vault CW20 balances and credits only
positive differences over recorded free balances. Thus a keeper cannot create a
withdrawable claim unsupported by assets at this vault address.

Verified proceeds remain free. The owner can call `allocate` to distribute free
assets over configured empty-side capacity. This intentionally replaces exact
fill-to-opposite-rung behavior, which cannot be proven through the current pair
query interface.

### Terminal And Parked Orders

If the active query is absent, the vault queries `expired_limit_refund`. A valid
owned refund is credited and claimed atomically. If neither active nor parked
state exists, the local record is terminal and removed; any maker output is still
credited only from the vault's physical balance. Historical fill events are not
required.

### Cancellation

Normal cancellation is owner-only and bounded. If escrow changed since the last
reconciliation, cancellation returns `UnsettledOrder`; any caller can reconcile
the current state first. Pair cancellation sends refunds only to the vault.

### Withdrawal

Normal withdrawal requires no active orders and burns the sole owner's internal
shares against free balances. The manager cannot invoke this path and each vault
can transfer only tokens held at its own address.

### Emergency Exit

The owner can irreversibly enter exit mode, disabling deposits and placements.
Bounded `emergency_cancel` pages use current pair state rather than indexed fill
history or stale recorded remaining values. They cancel active rows, claim parked
refunds, and retire terminal rows. `emergency_withdraw` then queries and transfers
the vault's actual liquid CW20 balances. Expired orders may require a CL8Y cleanup
walk before their refund row becomes claimable. Pair pause can delay this flow but
cannot redirect the funds.

### Pair Code Pinning

The vault stores the pair's code ID at bot creation. Deposit, allocate,
reconcile, cancel, and emergency-cancel re-query `contract_info` and abort with
`PairCodeMismatch` if the deployed code ID no longer matches. After a verified,
governance-approved pair migration, the admin re-pins with:

```json
{"update_pair_code":{"bot_id":1,"code_id":<NEW_CODE_ID>}}
```

Until re-pinning, the owner's funds remain recoverable via the normal recovery
paths, but pair interactions are intentionally disabled.

### Solvency Monitoring

`solvency` is a read-only query that cross-checks the accounting invariant per
token:

```json
{"solvency":{"bot_id":1}}
```

```text
expected = free_balance + sum(tracked order.remaining for that token)
actual   = queried vault CW20 balance + sum(on-chain pair escrow for that token)
```

Each tracked order's escrow is re-queried on the pair; unverifiable or foreign
escrow is reported as a warning string rather than failing the query. In-flight
fills move value from escrow to the vault balance one-for-one, so the totals
conserve. A nonzero difference between `expected` and `actual` signals drift in
tracked state or custody and should be investigated. The query never blocks
execution.

## State

Manager state:

- `Config`: admin handoff, keeper, CL8Y factory, vault code ID, gas settings,
  order timeout, and bounded-work limits.
- `PendingVault`: reply ID, vault ID, and owner.
- `Vault`: owner and instantiated address.
- Owner-to-vault secondary index.

Vault state:

- `Config`: designated owner, roles, CL8Y factory, timeout, and limits.
- `VaultMode`: `Active`, `Paused`, or irreversible `Exit`.
- `Bot`: pair and pinned pair code ID, two assets, grid parameters, free
  balances, gas credit, shares, active-order count, and pair batch limit.
- `Rung`: price and initial side.
- `GridOrder`: pair-local ID key, rung, side, price, and last remaining escrow.
- `PlacementPlan`: reply-scoped expected rungs and gross amounts.

## Security Invariants

1. The manager never holds user CW20 funds or owns CL8Y orders.
2. One vault address is associated with one designated owner and at most one bot.
3. A vault message never references another vault's state or custody.
4. Pair order ownership must equal the executing vault address.
5. Keeper/indexer values never increase accounting balances.
6. Free balances increase only from verified deposit deltas, queried liquid
   balances, or atomic pair refunds.
7. Recorded active-order remaining is monotonic.
8. New orders are funded only from that vault's free balance and have a timeout.
9. Exit is irreversible and keeps the owner recovery path available while normal
   maintenance is disabled.
10. Repeated reconciliation cannot credit the same physical balance increase twice.
11. Pair interactions require the deployed pair code ID to match the pinned ID.

## Migration From Pooled Custody

There is no safe in-place state migration from the pooled manager. The old
contract owns orders and refunds under its address; changing namespaces or code
does not change pair ownership.

Migration must:

1. Pause pooled deposits and new placement.
2. Reconcile where possible, cancel every active order, and claim every parked
   refund under the old manager address.
3. Withdraw each user's settled assets from the old contract using audited
   accounting and an independently verified custody snapshot.
4. Deploy the new manager and vault code, create one vault per owner/bot, and
   deposit directly into those addresses.
5. Register each vault independently for any CL8Y fee tier.
6. Keep the old contract recoverable until every old pair-owned order is resolved.

## Failure And Recovery

- Keeper unavailable: owner or any third party reconciles; owner cancels/exits.
- Indexer database lost: rediscover tracked IDs from vault queries; no fill amounts
  or historical records are needed for accounting or exit.
- Partial fill before cancellation: permissionless reconciliation updates current
  escrow and observed proceeds, then cancellation can retry.
- Pair paused: retain order records and retry after governance restores pair
  operations.
- CW20 delta mismatch: reject the operation and quarantine/remove the token from
  future supported deployments.
- Vault-specific accounting failure: only that vault is affected; no other vault's
  address, orders, or balances can subsidize it.

## Implementation Priority

1. Address-level custody split and non-custodial factory.
2. Remove keeper-reported amounts and make reconciliation permissionless.
3. Enforce observed-balance deposits and timed orders.
4. Harden cancellation/claim replies and pair-query error classification.
5. Add reviewed-token admission policy.
6. Add multi-contract integration/property tests against the real CL8Y pair.
7. Update the operator to discovery-only operation with durable monitoring.

Items 1 through 3 and item 5's pair code/interface pinning are implemented in
this workspace. Items 4, 6, and 7, plus reviewed-token admission and property
tests for the liquid-plus-escrow invariant, remain required before a production
security review; this code is not declared mainnet ready.
