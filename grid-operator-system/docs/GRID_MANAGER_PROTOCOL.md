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
- An order whose *both* active and parked queries fail at once. The vault returns
  `OrderStatusUnverifiable` and leaves the row and counter intact; emergency
  processing is owner-only and should be retried after pair/RPC health is
  restored. A missing order combined with a healthy parked query is not
  ambiguous: the vault classifies it using its own cancel ledger (see
  *Cancelled Orders*).

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
limits, `order_timeout_seconds`, and optional paired `fee_registry` /
`fee_collector`. Partial fee configuration is rejected and `mainnet` requires
both. `create_vault` propagates the current pair and instantiates a vault with the
caller as its designated owner and Wasm migration admin. The owner then creates the sole bot and prepays
its native gas reserve.

Manager configuration is a creation-time template. Updating the manager keeper,
vault code ID, or fee addresses affects only subsequently created vaults;
existing vaults retain independent admin, keeper, fee, and pinned-code state.
An old fee-disabled vault requires migration or redeployment because the vault
has no post-instantiation fee update.

Fee resolution is keyed to `bot.owner`. The vault stores the last successful
effective bps/tier locally. If the registry is unavailable, it charges that
exact result with source `vault_cached`, or 180 bps/source `lowest` without
history. If the registry is reachable but its live CL8Y token query fails, the
registry returns full base/`Lowest`; historical holdings are not pricing input.

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

### Terminal, Parked, And Cancelled Orders

If the active query fails, the vault queries `expired_limit_refund`:

- A zero-remaining active response is a positive completion proof and may retire
  the row.

- A valid owned refund is claimed through a reply-confirmed page and the row is
  retired (`parked`).
- A valid `Err + None` refund leaves the order absent from the pair with no
  parked refund. The vault then checks its own cancel ledger
  (`CANCELLED_ORDERS`):
  - A recorded cancel means the vault already refunded the escrow and the row is
    retired (`cancelled`).
  - No recorded cancel means the order could only have left the pair through
    execution, so it is retired as fully executed; the fill proceeds were
    already credited to the vault balance by balance synchronization.
- A failed parked query (both queries errored) is indeterminate, not terminal:
  the transaction returns `OrderStatusUnverifiable`, preserves the local row and
  counter, and must be retried. The current pair API cannot otherwise distinguish
  genuine absence from contract/query/schema failure when *no* query succeeds.

Because pair cancellations can only be initiated by this vault, its local cancel
ledger is authoritative: an order that is absent from the pair, unclaimed, and
never cancelled was necessarily fully executed. This lets the vault reconcile
against the shipped CL8Y DEX pair without any owner-inventory or typed-status
pair
extension.

An owner can submit `recover_order` with a known pair order ID and rung. The vault
adopts it only after positively verifying active ownership, side, price and
remaining amount, or positively verifying an owned parked refund. It cannot be
used to declare an order terminal, and it rejects an order already recorded as
cancelled. This recovers known IDs from placement records, but does not prove
that the owner supplied a complete inventory.

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
without deleting rows. `emergency_withdraw` then burns only the owner's shares
and transfers only the owner's pro-rata liquid balances. Collector shares,
backing assets, and remaining total shares are preserved; once active orders are
zero, the collector can redeem while the vault remains in Exit. Expired orders
may require a CL8Y cleanup
walk before their refund row becomes claimable. Pair pause can delay this flow but
cannot redirect the funds.

### Cancelled-Order Ledger

Cancellation is owner-only and bounded. On a confirmed cancel page, the vault
persists a `CancelledOrder` record — rung, side, price, refunded remaining, and
cancel height — keyed by `(bot_id, order_id)` before removing the local row. The
records are exposed through the paginated `cancelled_orders` query and are never
reused: `recover_order` rejects an order already in the ledger.

The ledger exists to make the *absent-order* classification sound. Since only
this vault can cancel its orders on the pair, any tracked order that is absent
from the pair, holds no parked refund, and is not in the ledger was necessarily
fully executed. Reconciliation therefore needs no pair-side inventory index or
typed status query and works against the pair exactly as shipped.

Migrating from a release that predates the ledger is safe for this vault: every
current order is reconciled directly against live pair state (active, parked, or
executed) on the next `reconcile`, and past cancels are detected the same way.
No pre-deployment scan or recovery state is required; there is no lock.

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
remaining amount are validated before custody is counted. A missing order with no
parked refund is reported as `cancelled_orders` if the vault's cancel ledger holds
it, otherwise as `executed_orders`; the proceeds of an executed order are expected
to already sit in the vault's liquid balance. The response reports
`active_escrow_orders`, `parked_refund_orders`, `executed_orders`,
`cancelled_orders`, and `unverifiable_orders`; an order whose *both* queries fail,
or invalid or unqueryable escrow, produces a warning rather than being counted as
terminal.
Fills consume one token's escrow and credit the opposite token's liquid balance.
This query is therefore a custody/reconciliation diagnostic, not a same-token
profit invariant: equality is expected after complete reconciliation, while a
difference can represent an unreconciled fill, terminal row, invalid escrow, or
actual custody drift. Warnings and escrow-state categories distinguish these
cases. The query never blocks execution.

## State

Manager state:

- `Config`: admin handoff, keeper, CL8Y factory, vault code ID, gas settings,
  order timeout, bounded-work limits, fee registry, and fee collector.
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
- `CancelledOrder`: pair-local ID key, rung, side, price, refunded remaining, and
  cancel height; written on a confirmed cancel page and queryable, never reused.

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
12. No local order is removed solely because both active and parked queries failed
    at once; the row survives as `OrderStatusUnverifiable`.
13. A local order is removed without a fill only after a confirmed cancel or a
    recorded cancel in the vault's own ledger, or when it is absent from the pair
    with no parked refund (fully executed).
14. Exit withdrawal preserves `sum(SHARES) == total_shares`; owner recovery
    cannot transfer assets backing collector shares.
15. A fee-registry outage cannot bypass the fee; it selects the explicit
    vault-local cached/base outage policy.

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
5. Limit-grid does not use a swap-proxy. Wire `fee_registry` and `fee_collector`
   at vault instantiation; the vault interacts directly with the CL8Y pair.
6. Keep the old contract recoverable until every old pair-owned order is resolved.

Upgrade order is mandatory:

1. Deploy the swap-only `grid-vault-swap` design against the shipped CL8Y DEX
   pair; no
   pair upgrade is needed.
2. Migrate any funded vault to the swap-only vault by deploying a fresh contract
   and depositing the withdrawn balances; there is no in-place custody migration.
3. Rehearse with funded historical-state fixtures before economic deployment.

Once a vault is live, do not roll the pair back or repin a different
implementation while orders are outstanding. Restore the exact verified pair
implementation and resume. There is no snapshot, protocol generation, or scan
phase to keep in sync; the vault reconciles directly against current pair state,
so no rollback-ordering constraint applies beyond the ordinary pinned-code check.

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

### Limit-Order Vault (pre-production)

`contracts/grid-vault` is the limit-order grid vault. It reconciles directly
against the shipped CL8Y DEX pair using exactly the shipped query interface
(`LimitOrder`, `ExpiredLimitRefund`). Order cancellations are initiated only by
the vault, which records them locally; an order that is absent from the pair, has
no parked refund, and was never cancelled is classified as fully executed and its
proceeds are assumed already credited to the vault balance. This requires no pair
modification, no fork, and no upstream PR, and it has no reconciliation gate or
locked state.

### Swap-Only Vault (alternative design)

`contracts/grid-vault-swap` is an alternative grid design. It holds its CW20
balances directly in the vault address, reads the pool price, and executes
classic pair `Swap` calls through the CW20 receive hook when the price crosses a
grid level. It uses exactly the shipped CL8Y DEX pair API (`Pool`, `Observe`,
`Swap`) and
requires no upstream merge and no fork.

Production readiness still requires adversarial validation against the production
CL8Y runtime, an independent external audit, and staged testnet/limited-value
rollout.

Production is additionally blocked by approved canonical build inputs,
deployment verification, external pair semantics, unrun canonical fee E2E, and
independent review. H-05 is the sole partial repository audit finding. The
manager propagates fee configuration only to future vaults; old fee-disabled
vaults still need migration or redeployment. See
`../../docs/DEPLOY_FEE_SYSTEM.md` and `../../RELEASE.md`.
