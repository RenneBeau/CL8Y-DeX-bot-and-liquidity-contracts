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
pair. The allowlist governs pair admission at bot creation; removing an entry
does not disable an existing bot. Runtime deposits and allocation remain limited
to the bot's pair assets, and quarantine is the runtime disable control.

## Roles And Trust

- `owner`: controls deposits, allocation, cancellation, exit, and recipients.
- `keeper`: optional reimbursed operator. It has no exclusive reconciliation
  authority and supplies no accounting amounts.
- `admin`: configures the manager template and each vault's administrative
  controls. It cannot withdraw vault CW20 balances.
- `indexer`: optional availability and discovery service. Its data is advisory.
- `CL8Y pair`: trusted to enforce maker ownership and transfer correct fills and
  refunds. This is the unavoidable external custody dependency.
- `CW20 assets`: required to have standard, exact-transfer, non-rebasing behavior.

## Initialization

The manager stores the vault code ID, CL8Y factory, operator settings, safety
limits, and `order_timeout_seconds`. `create_vault` instantiates a vault with the
caller as its designated owner and Wasm migration admin. The owner then creates the sole bot and prepays
its native gas reserve.

Manager configuration is a creation-time template. Updating the manager keeper
or vault code ID affects only subsequently created vaults; existing vaults retain
independent admin, keeper, and pinned-code state and must be updated separately.

The vault admits only a factory-registered CL8Y CW20/CW20 pair with distinct
assets, equal decimals, nonzero reserves, compatible batch limits, valid price
bounds, and at least one bid and ask rung. At bot creation it pins the pair's
`contract_info.code_id`; every later mutating pair interaction re-queries the
code ID and rejects a mismatch. Monitoring should compare the reported pinned
ID independently before trusting read-only pair-derived diagnostics.

## Lifecycle

### Deposit

Only the designated bot owner may use the CW20 `deposit` hook. During the callback,
the vault queries its own CW20 balance. The observed liquid balance must equal the
previous free balance plus the callback amount; otherwise the transaction rejects
the token behavior. Accepted assets are divided over the applicable initial rungs.

Unsolicited pair-token transfers are not deposits and mint no shares. The owner
can explicitly credit physical excess with `{"sync_balances":{"bot_id":1}}`;
this changes free accounting only and places no orders.

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

If the active query fails, the vault queries `expired_limit_refund`. A valid owned
refund is claimed through a reply-confirmed page. `Err + None` is indeterminate,
not terminal: the transaction returns `OrderStatusUnverifiable`, preserves the
local row and counter, and must be retried. A zero-remaining active response is a
positive completion proof and may retire the row. The current pair API cannot
otherwise distinguish genuine absence from contract/query/schema failure.

An owner can submit `recover_order` with a known pair order ID and rung. The vault
adopts it only after positively verifying active ownership, side, price and
remaining amount, or positively verifying an owned parked refund. It cannot be
used to declare an order terminal. This recovers known IDs from placement records,
but does not prove that the owner supplied a complete inventory.

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
history or stale recorded remaining values. They cancel active rows and claim
positively verified parked refunds. Any ambiguous query aborts the complete call
without deleting rows. `emergency_withdraw` then queries and transfers
the vault's actual liquid CW20 balances. Expired orders may require a CL8Y cleanup
walk before their refund row becomes claimable. Pair pause can delay this flow but
cannot redirect the funds.

### Legacy Inventory Reconciliation

Vaults migrated from a release that could forget pair orders remain locked behind
`inventory_reconciliation_required`. The owner advances the bounded state machine
with `{"continue_inventory_reconciliation":{"limit":<1..100>}}`. New bot creation,
deposits, and allocation are disabled for the entire reconciliation; normal and
emergency withdrawals remain disabled. Administrative configuration and pair-code
pinning remain available, but there is no administrative unlock.

Before taking a snapshot, the vault verifies the pair's runtime code ID and
requires pair protocol schema v1 with `typed_order_status`, `owner_inventory`, and
`owner_index_backfill`, a ready owner index, and the protocol page cap. It persists
the pair-issued generation and order-ID high-water. The scanning phase walks that
snapshot with a contract-owned exclusive cursor. Every validated active or parked
row is copied into a migration-only recovery map; it is observable custody evidence
and is never assigned a strategy rung or credited to accounting. Invalid, duplicate,
out-of-order, or replayed pages and query failures leave the prior cursor and map
unchanged.

After the complete scan, draining repeatedly queries page one under the same
snapshot. Active IDs are atomically cancelled first; when no active row is present,
parked IDs are atomically claimed. Each batch is capped by the caller limit, owner
inventory limit, and pair `LimitOrderConfig`. Failed pair replies keep the snapshot,
recovery records, local rows, and lock and expose the error in `config` for retry.

Only a complete empty pair page is terminal proof. The vault then removes recovery
records and stale local rows in separate bounded phases, verifies that this contract
contains exactly bot 1, sets `active_orders` to zero from the empty map, and replaces
both free balances with actual vault CW20 balances. It unlocks only if the pair code
and protocol generation still match and no pair/local page is pending. Pair custody,
not a client cursor or the vulnerable local order map, is canonical during this flow.

A fully filled order can disappear between scanning and draining. The vault does
not query or interpret pair tombstones during migration and does not claim
vault-side knowledge of why a row disappeared. The subsequent complete empty
owner-inventory proof is what safely permits stale recovery and local rows to be
retired; actual CW20 balances are then synchronized exactly.

Migration validates the stored `cw2` contract name and semantic version and accepts
only an older supported source. Repeating migration at the same or a newer version
fails before changing the lock. For every supported migration from vulnerable
`0.1.0` with a bot or history, rollback storage is never authoritative: even a
stored `Complete` phase is discarded, the Boolean gate is set, and the pair proof
starts again from a fresh snapshot. Stale recovery rows are tagged with an older
epoch, may be overwritten idempotently during adoption, and are removed by the same
bounded recovery cleanup without affecting the new scan's count. A pre-bot vault
stays unlocked only when reconciliation/recovery, bot, order, pending-page and
placement state are absent and `next_bot_id` proves no bot was previously allocated.

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

Each tracked order is first queried as active escrow, then as an expired parked
refund. Owner, order ID, side, immutable price (when active), and nonincreasing
remaining amount are validated before custody is counted. The response reports
`active_escrow_orders`, `parked_refund_orders`, `terminal_orders`, and
`unverifiable_orders`; `Err + None`, invalid, or unqueryable escrow produces a warning
rather than being counted as terminal.
Fills consume one token's escrow and credit the opposite token's liquid balance.
This query is therefore a custody/reconciliation diagnostic, not a same-token
profit invariant: equality is expected after complete reconciliation, while a
difference can represent an unreconciled fill, terminal row, invalid escrow, or
actual custody drift. Warnings and escrow-state categories distinguish these
cases. The query never blocks execution.

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
- `InventoryReconciliation`: phase, pair snapshot/code pin, exclusive scan cursor,
  recovered-row count, pending drain action, bounded-work counters, and last error.
- `RecoveredInventoryRow`: migration-only validated pair custody metadata keyed by
  order ID; it is cleaned before stale strategy orders and never affects shares.

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
12. No local order is removed solely because an active or parked query failed.

## Migration From Pooled Custody

There is no safe in-place state migration from the pooled manager. The old
contract owns orders and refunds under its address; changing namespaces or code
does not change pair ownership.

Migration must:

1. Pause pooled deposits and new placement.
2. Reconcile where possible, cancel every active order, and claim every parked
   refund under the old manager address.
3. Withdraw each user's settled assets from the old contract using independently
   reviewed accounting and a verified custody snapshot.
4. Deploy the new manager and vault code, create one vault per owner/bot, and
   deposit directly into those addresses.
5. Register each vault independently for any CL8Y fee tier.
6. Keep the old contract recoverable until every old pair-owned order is resolved.

Upgrade order is mandatory:

1. Deploy the swap-only `grid-vault-swap` design against the shipped pair; no
   pair upgrade is needed.
2. Migrate any funded vault to the swap-only vault by deploying a fresh contract
   and depositing the withdrawn balances; there is no in-place custody migration.
3. Rehearse with funded historical-state fixtures before economic deployment.

Once a vault has captured a snapshot, do not roll the pair back, change its owner
index generation, or repin a different implementation. Restore the exact verified
pair implementation and generation and resume. A failed deployment is rolled
forward; rollback is permitted only before any affected vault captures a snapshot.
If code is nevertheless rolled back and migrated forward again, no saved phase,
including `Complete`, can replace a new scan and empty canonical pair proof.

## Failure And Recovery

- Keeper unavailable: owner or any third party reconciles; owner cancels/exits.
- Indexer database lost: on-chain accounting and owner exit need no fill history,
  but the included operator must archive-rescan from deployment height to rebuild
  its automated event queue.
- Partial fill before cancellation: permissionless reconciliation updates current
  escrow and observed proceeds, then cancellation can retry.
- Pair paused: retain order records and retry after governance restores pair
  operations.
- CW20 delta mismatch: reject the operation and quarantine/remove the token from
  future supported deployments.
- Vault-specific accounting failure: only that vault is affected; no other vault's
  address, orders, or balances can subsidize it.

## Production Status

### Limit-Order Vault (reference only, not deployable)

`contracts/grid-vault` is retained for reference. It is **not deployable** and
must not be funded. Its legacy inventory-reconciliation flow depends on pair
queries that do not exist in the shipped CL8Y pair: typed order status
(`typed_order_status`), owner inventory (`owner_inventory`), and owner-index
backfill (`owner_index_backfill`). No pair modification, fork, or upstream PR is
permitted for the grid system, so those queries will never be available. Any
deployment of this vault against the shipped pair stays permanently locked behind
`inventory_reconciliation_required` and cannot prove custody completeness. The
design remains documented only as an analysis artifact.

### Swap-Only Vault (deployable design)

`contracts/grid-vault-swap` is the deployable grid design. It holds its CW20
balances directly in the vault address, reads the pool price, and executes
classic pair `Swap` calls through the CW20 receive hook when the price crosses a
grid level. It uses exactly the shipped pair API (`Pool`, `Observe`, `Swap`) and
requires no upstream merge, no fork, and no reconciliation state.

Production readiness still requires adversarial validation against the production
CL8Y runtime, an independent external audit, and staged testnet/limited-value
rollout.
