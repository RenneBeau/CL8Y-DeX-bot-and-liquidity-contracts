# Protocol Fee-Tier Behavior

Limit-grid implements this protocol for PoC coverage only. It is abandoned as a
production venue and remains in the repository and release artifacts solely for
research and reproducibility.

Status: current implementation and specification for limit-grid, market-grid,
rebalancer, `fee-registry`, and `fee-collector`.

## Canonical Mainnet Inputs

- CL8Y (`cl8y-cb`, 18 decimals):
  `terra16wtml2q66g82fdkx66tap0qjkahqwp4lwq3ngtygacg5q0kzycgqvhpax3`
- CMM treasury:
  `terra16j5u6ey7a84g40sr3gd94nzg5w5fm45046k9s2347qhfpwm5fr6sem3lr2`
- Undiscounted user rate (user-facing **tier 0**): **180 bps (1.8%)**

The `mainnet` fee-registry artifact pins CL8Y and treasury. The `mainnet`
fee-collector artifact pins treasury. Every vault `mainnet` build requires
nonempty compile-time `CL8Y_CANONICAL_FEE_REGISTRY` and
`CL8Y_CANONICAL_FEE_COLLECTOR`; market-grid and rebalancer also require
`CL8Y_CANONICAL_SWAP_PROXY`. Limit-grid has no proxy. Missing registry input was
explicitly verified to fail compilation. Vault configuration cannot substitute
another registry, collector, or applicable proxy in a mainnet artifact.

## Canonical Ladder

The registry seeds the CL8Y discount ladder below. **User-facing tier 0** means
no CL8Y holder tier is eligible and pays the undiscounted 180 bps rate. The
current query response encodes this as `tier_id: null` (`None`), not numeric ID
`0`.

Separately, storage IDs `0` and `255` are reserved governance-only entries. ID
`0` currently carries a 100% discount and ID `255` carries no discount, but the
current implementation has no address-assignment mechanism for either. These
internal reserved IDs must not be confused with user-facing tier 0.

```text
effective_fee_bps = floor(base_fee_bps * (10000 - discount_bps) / 10000)
```

| Tier | Minimum CL8Y | Discount | Effective fee at base 180 |
|---|---:|---:|---:|
| User tier 0 (`tier_id: null`) | below 1 | 0 bps | 180 bps |
| 1 | 1 | 250 bps | 175 bps |
| 2 | 5 | 1,000 bps | 162 bps |
| 3 | 20 | 2,000 bps | 144 bps |
| 4 | 75 | 3,500 bps | 117 bps |
| 5 | 200 | 5,000 bps | 90 bps |
| 6 | 500 | 6,000 bps | 72 bps |
| 7 | 1,500 | 7,500 bps | 45 bps |
| 8 | 3,500 | 8,500 bps | 27 bps |
| 9 | 7,500 | 9,500 bps | 9 bps |

A balance below 1 CL8Y is user-facing tier 0, receives no discount, and pays
180 bps. All calculations use integer floor division, which favors the payer by
discarding fractions.

## Fee Subject

Each fee-enabled execution queries one address:

| Venue | Fee-triggering execution | Registry `trader` |
|---|---|---|
| limit-grid | reconciled fill proceeds | `bot.owner` |
| market-grid | completed rebalance swap | `config.admin` |
| rebalancer | completed rebalance swap | `config.admin` |

Public depositors and other LP holders are not queried or individually tiered.
The subject is an operating identity, not each economic LP beneficiary.

## NAV-Priced LP Mint

For token-0-normalized executed value `V`, effective rate `bps`, post-settlement
asset value `A`, and total LP supply `S` before the collector mint, each venue
computes:

```text
F = floor(V * min(effective_fee_bps, 10000) / 10000)
x = floor(F * S / (A - F))
```

`F` is the economic fee value and `x` is the NAV-priced LP amount minted only to
the configured collector. No user LP is minted or burned by this fee step. Zero
fee/value/supply results mint nothing; an invalid `F >= A` condition is rejected
or skipped by the venue's guarded fee path. Because `x` is floored, the
collector's immediate post-mint claim `floor(x * A / (S + x))` cannot exceed
`F`. Events named `fee_shares` report `x`, not `F`.

Token normalization is venue-specific:

- limit-grid converts credited token 1 using the bot reference price and adds
  credited token 0;
- market-grid and rebalancer normalize the executed ask amount to token 0 using
  the captured execution price path.

Deposits and withdrawals have no direct protocol fee. A fee-enabled fill or
rebalance can nevertheless dilute existing LP through the collector mint.

## Resolution And Failure Behavior

Registry `EffectiveFee { trader }` is live-first:

1. Query the trader's current CL8Y CW20 balance.
2. If successful, use the highest eligible non-governance tier. Source is
   `Live`.
3. If the live CL8Y token query fails, return the full configurable base rate
   (180 bps in production), `tier_id: null`, and source `Lowest`.

Registry `RefreshHolding` history is observability only. It is never used to
price `EffectiveFee`, so a stale token holding cannot retain a discount.

Each vault separately caches the last successful complete effective result
(`fee_bps` and `tier_id`) keyed by its fee subject: market-grid/rebalancer
`config.admin`, limit-grid `bot.owner`. If the registry contract is unreachable,
the vault charges that exact local result with source `vault_cached`. With no
local history it charges 180 bps, `tier_id: null`, source `lowest`. Registry
outage therefore never bypasses the protocol fee.

The locally cached tier intentionally remains valid for the duration of a
registry outage. This is the explicit availability/revenue policy, not an
accidental registry holding cache: it may differ from the subject's current
unknown tier until a successful registry response refreshes it. By contrast, a
reachable registry whose live CL8Y token query fails grants no discount and
returns `Lowest` immediately.

## Collector Behavior

`Collect { vault, bot_id }` is keeper-only. It:

1. queries the collector's current shares from the target vault;
2. rejects zero entitlement;
3. records the observed amount in `VAULT_SHARES`;
4. queries vault config for `liquidity_contract`;
5. instructs the vault or `bot-liquidity` contract to pay the configured
   treasury directly.

For grids, the collector sends `RedeemShares { bot_id, recipient: treasury }` to
the vault. For rebalancer, it sends `Withdraw` to `bot-liquidity` with the
treasury as recipient and zero minimum assets.

In limit-grid Exit, owner emergency withdrawal leaves collector shares and their
pro-rata backing intact. Once active orders are zero, collector redemption is
permitted while the vault remains in Exit.

There is no anti-dust threshold. `Collect` does not consult the collector's
`registry` field. `VAULT_SHARES` is cumulative historical bookkeeping; it does
not control current entitlement or redemption. Cumulative updates use checked
addition and fail rather than wrapping on overflow.

## DEX Fee And Proxy Scope

A DEX-whitelisted shared swap proxy, producing zero DEX fee for routed swaps, is
an intended production prerequisite. It is not a confirmed current-mainnet fact:
the proxy has not been deployed or pinned. Rebalancer requires the proxy;
market-grid can route through an optional proxy. Limit-grid has no proxy field
and places/cancels orders directly against the CL8Y pair.

Protocol fee and DEX fee are separate. The intended zero DEX fee does not remove
the protocol collector mint.

## Current Production Gate

Production deployment is **BLOCKED** because approved registry/collector/proxy
addresses are not yet available, so required mainnet build environment values
cannot be approved; and the proxy is not deployed or independently
verified/whitelisted. The release
definitions now cover all four workspaces, mainnet artifacts, and manifests,
but complete current-SHA reproducible and canonical fee E2E evidence is not
recorded in this working tree. Market-grid and bot-vault 0.1.x require
redeployment; routed swap-proxy 0.1.x and bot-liquidity 0.1.x must not be
migrated. Only empty compatible proxy state, limit grid-vault 0.1.0 to 0.1.1,
and the tested fee-system paths are migration candidates.

The rebalancer ladder is covered in `cw-multi-test` by a full settlement flow
using actual protocol contracts and stateful pair/factory models for no-holder
and tiers 1 through 9. This proves the in-process contract path through
NAV-priced collector mint and pro-rata withdrawal. It is not LocalTerra or
on-chain execution; exact-candidate canonical LocalTerra fee E2E remains release
evidence.

Use [`DEPLOY_FEE_SYSTEM.md`](DEPLOY_FEE_SYSTEM.md) only after its unblock
conditions are satisfied.
