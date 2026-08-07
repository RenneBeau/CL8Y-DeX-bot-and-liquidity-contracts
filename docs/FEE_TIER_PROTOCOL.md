# Protocol Fee-Tier & Treasury Design

Cross-system design (limit-grid, market-grid, rebalancer). **Design only — no code.**
Status: for review.

## 0. Deploy constants (Terra Classic)

- `cl8y` — CL8Y (`cl8y-cb`) CW20 token:
  `terra16wtml2q66g82fdkx66tap0qjkahqwp4lwq3ngtygacg5q0kzycgqvhpax3`
- `treasury` — CMM treasury:
  `terra16j5u6ey7a84g40sr3gd94nzg5w5fm45046k9s2347qhfpwm5fr6sem3lr2`

These are the `cl8y` and `treasury` addresses the `fee-registry` and
`fee-collector` are configured with (see §8).

## 1. Rationale

The CL8Y systems are being **whitelisted**, which makes their DEX swaps **0-fee**.
Currently revenue is implicit: the DEX charges a swap/limit fee and the proxy
holds CL8Y to qualify for a cheaper tier. Once whitelisted there is no DEX fee
and therefore no implicit revenue.

The systems must collect their own protocol tax, **differentiated by each user's
CL8Y (`cl8y-cb`) holding**, charged **per fill**, denominated in **vault LP**,
realized by a **fee-collector**, and forwarded to the **CMM treasury**.

## 2. Two fee planes

| Plane | Who collects | Basis | Destination | Status |
|---|---|---|---|---|
| DEX (infrastructure) | CL8Y DEX on the proxy/vault | CL8Y held by the proxy | DEX fees | becomes 0 for whitelisted systems |
| Protocol (user tax) | each vault on its users | **CL8Y held by the user** | fee-collector → CMM treasury | **this design** |

The DEX plane and the protocol plane are independent. The proxy no longer needs
to *hold* CL8Y for its own tier; the user's CL8Y balance now drives the protocol
tax.

## 3. CL8Y fee-tier ladder (canonical)

The canonical CL8Y DEX `fee-discount` registry defines a **discount** ladder
(`docs/reference/fee-discount-tiers.md`, aligned on
`STANDARD_PRODUCTION_TIERS`). CL8Y uses **18 decimals** (`1 CL8Y = 10^18`).
Higher tiers mean more CL8Y held → bigger discount → lower fee.

| Tier ID | CL8Y held (min) | `min_cl8y_balance` (wei) | Discount (bps) | Discount % |
|---|---|---|---|---|
| 0 | 0 (gov-assigned, market makers) | 0 | 10000 | 100% |
| 1 | 1 | 1,000,000,000,000,000,000 | 250 | 2.5% |
| 2 | 5 | 5,000,000,000,000,000,000 | 1000 | 10% |
| 3 | 20 | 20,000,000,000,000,000,000 | 2000 | 20% |
| 4 | 75 | 75,000,000,000,000,000,000 | 3500 | 35% |
| 5 | 200 | 200,000,000,000,000,000,000 | 5000 | 50% |
| 6 | 500 | 500,000,000,000,000,000,000 | 6000 | 60% |
| 7 | 1,500 | 1,500,000,000,000,000,000,000 | 7500 | 75% |
| 8 | 3,500 | 3,500,000,000,000,000,000,000 | 8500 | 85% |
| 9 | 7,500 | 7,500,000,000,000,000,000,000 | 9500 | 95% |
| 255 | 0 (gov-assigned, blacklist) | 0 | 0 | 0% |

### From discount to protocol fee

We reuse this ladder as-is and resolve a user's discount live (highest tier whose
`min_cl8y_balance ≤` the user's current CL8Y balance; no eligible tier ⇒ 0
discount ⇒ full base fee). This mirrors the DEX's own invariants:

- **I4 (effective fee):** `fee_bps × (10000 − discount_bps) / 10000` with integer
  division.
- **I5 (insufficient balance):** a holder whose on-chain balance is below the
  tier minimum pays `discount_bps = 0` ⇒ full base fee.
- **I10 (registry query failure):** the DEX fail-closes to the full pair fee.
  Our protocol is *more lenient* by design: it falls back to the saved holding
  (§6) instead of the full fee.

**Base fee:** the CL8Y mainnet deploy sets `default_fee_bps = 180` (1.8%) on the
factory. We use the same figure as `base_fee_bps` unless CMM chooses otherwise:

```
effective_fee_bps = base_fee_bps × (10000 − discount_bps) / 10000   (integer div)
```

| User CL8Y held | Tier | Discount | Effective fee (base 180) |
|---|---|---|---|
| 0 (or first-ever query KO) | lowest | 0% | 180 bps (1.8%) |
| 1 | 1 | 2.5% | 175 bps |
| 5 | 2 | 10% | 162 bps |
| 200 | 5 | 50% | 90 bps |
| 7,500 | 9 | 95% | 9 bps |

Source: `RenneBeau/cl8y-dex-terraclassic`
[`docs/reference/fee-discount-tiers.md`](https://github.com/RenneBeau/cl8y-dex-terraclassic/blob/main/docs/reference/fee-discount-tiers.md)
and the mainnet deploy trace (`default_fee_bps=180`).

## 4. Architecture

A small shared, per-network deployable core referenced by every vault:

```
                 fee-registry                fee-collector
   (governance)  cl8y addr, ladder,     (governance) registry,
                 base_fee_bps,            treasury, keeper
                 treasury ref                |
┌────────────┐  GetEffectiveFee(trader)        │ collect (keeper only)
│ grid/market│               │                 │  redeem collected LP
│ /rebal vault├──────────────┘   mints LP ──────►  forward assets
│            │  on each fill                     │  → CMM treasury
└────────────┘                                   ▼
```

- **`fee-registry`** — source of truth: `cl8y` (cl8y-cb) token address, the tier
  ladder, `base_fee_bps`, treasury address. Governance-updatable without a code
  release. Vaults query `GetEffectiveFee { trader }` at fill time.
- **`fee-collector`** — accumulates the LP it is credited in each vault, redeems
  it, and forwards proceeds to the CMM treasury. `collect` is **keeper-only**.
- **Vaults** — on each fill, determine the fee, mint LP to the fee-collector
  (see §5), and book the change.

## 5. Charging the fee (per fill, single user, mint)

The three venues share **one** mechanic (model A, single user per bot/vault). On a
fill that credits `V` of token-0-normalized value, the vault:

1. resolves the **single operating user's** effective fee via the fee-registry;
2. `fee = V × effective_fee_bps / 10 000`;
3. **mints** `V − fee` of LP to the user and `fee` of LP to the fee-collector.

LP therefore **grows by `V`** with each fill — the user keeps their value net of
the fee and the collector receives the fee, both as fresh LP (correct dilution,
no burn). No other holder is minted or burned.

- **Limit-grid:** the single user is the bot owner (`bot.owner`); LP is the grid's
  internal `SHARES (bot_id, addr)` ledger.
- **Market-grid:** the single user is the vault operator (`config.admin`); LP is
  the vault's internal `SHARES` ledger (token-0 normalized).
- **Rebalancer:** the single user is the vault operator (`config.admin`); LP is the
  pooled external `bot-liquidity` CW20 token, minted through bot-liquidity
  `MintTo` (vault-gated).

### Design rationale: single user everywhere

Every CL8Y venue is modelled as **one operator per bot/vault** (limit-grid: bot
`owner`; market-grid and rebalancer: `admin`). The protocol tax is therefore a
single tier resolution per fill — cheap, uniform, and identical across all three
systems. Fees are never a negative incentive: the effective fee is capped at
`10 000` bps (100%) so `fee ≤ V`, and the collector only ever receives the LP it was
credited.

Mechanics (v1): on a fill landing value `V` in the vault, `fee = V ×
effective_fee_bps / 10 000`. The vault mints `V − fee` to the user and `fee` to the
fee-collector, growing supply by `V` (correct dilution). A transient fee-registry
failure is non-blocking: the fill completes and the fee is skipped, never a revert.

### Rebalancer optionality (pooled → single-user)

An earlier design treated the rebalancer as a **pooled, gas-minimal** venue that
taxed every LP holder of the shared `bot-liquidity` token at their own tier
(uncapped enumeration). This is **superseded**: the rebalancer now uses the same
single-user model as the grids (`config.admin`), so there is no enumeration and no
pooled-tier breakout. A rebalance execution serves one operator; the venue's
gas-profile is uniform with the grids.

## 6. Tier resolution — live-first, saved holding as fallback

The fee-registry resolves a user's tier by comparing their CL8Y holding against
the **historised** CL8Y DEX tier table. Because a CosmWasm query is read-only it
cannot persist, so persistence is a separate, permissionless `RefreshHolding`
message: it reads the CL8Y balance live and stores the last-known-good holding
per user, so a transient query failure never stalls the fee.

Resolution on each fill (`GetEffectiveFee { trader }`):
1. The registry queries the CL8Y balance of `trader` **live**.
2. On **success** (never a keeper-submitted amount — the registry is the sole
   reader of the CL8Y balance): compute the discount from the historised ladder
   (highest tier with `min_cl8y_balance ≤ amount`). Source = `Live`.
3. On **failure** (single read miss): fall back to the saved `holding[trader]`
   (`RefreshHolding`) and compute the discount from it. Source = `Cached` —
   possibly stale, never a hard error.
4. If **no saved holding exists and the live query failed**: fall back to the
   **lowest tier** (a holder is never under-fee), i.e. an effective fee of
   `base_fee_bps` with no discount. Source = `Lowest`.

The vault never submits an amount; the registry is the sole reader of the CL8Y
balance, so keeper/indexer input cannot change a tier. Comparing to the saved
holding also removes any dependence on periodic registration state.

- **Historised table:** the tier ladder is versioned on-chain (see §8) so a
  tier change by governance is auditable over time and a saved holding maps to a
  consistent ladder version.
- **Gameability (accepted for v1):** a user can top up CL8Y for the block of a
  fill. The saved-holding fallback *reduces* gaming (a top-up is only honoured
  while it actually persists on-chain), and a snapshot/epoch capture can be
  layered on later — the registry already models `registration_epoch`.
- **Cost:** one live token query per fill plus one registry read; kept
  non-blocking (see §9 `UnverifiableOrder`).

## 7. Fee-collector (keeper-only realization)

`fee-collector` is venue-agnostic but liquidity-aware: for a target vault/bot it
reads the collector's entitlement via `VaultQueryMsg::Shares`, books it, and
realizes it (keeper-only trigger). What realization yields depends on the vault's
`Config.liquidity_contract`:

- **Grids (limit-grid, market-grid)** — no `liquidity_contract`: sends
  `VaultExecuteMsg::RedeemShares` to the vault, which burns the collector's LP in
  its internal `SHARES` ledger and pays the underlying assets.
- **Rebalancer** — `liquidity_contract` set: the collector owns the fee as
  external `bot-liquidity` LP (minted via `MintTo`), so the collector calls
  `bot-liquidity` `Withdraw` (pro-rata) to route the underlying assets through.

`collect` (keeper only) then routes the proceeds to the CMM treasury.

- **Trigger:** keeper-only (per decision).
- **Gas:** funded from the collector (or the protocol gas reserve); never from
  user funds.
- **Anti-dust:** a minimum realized value threshold before a transfer fires;
  below it the LP accrues in the collector.

## 8. Draft state & messages

`fee-registry`
- State: `Config { governance, cl8y, treasury, fee_collector, base_fee_bps }`,
  `Tiers: Map<u8, Tier{ min_cl8y_balance, discount_bps, governance_only }>`,
  `Holdings: Map<Addr, HoldingSnapshot{ amount, at_height }>`, and a
  monotonically increasing `ladder_version` so the historised ladder (and the
  saved holding it is compared against) is auditable over time.
- `ExecuteMsg`: `AddTier`, `UpdateTier`, `RemoveTier`, `UpdateConfig` (all gov),
  `RefreshHolding { trader }` (permissionless — persists the live CL8Y balance).
- `QueryMsg`: `EffectiveFee { trader } → { fee_bps, discount_bps, tier_id,
  holding, source (Live|Cached|Lowest) }`, `Holding { trader }`, `Tiers`,
  `Tier { tier_id }`, `Config`.

`fee-collector`
- State: `Config { governance, registry, keeper, treasury }`,
  `VaultShares: Map<(Addr, u64), Uint128>`.
- `ExecuteMsg`: `Collect { vault, bot_id }` (keeper), `UpdateConfig` (gov).
- `QueryMsg`: `Shares { vault, bot_id }`, `Config`.

Vaults (grid / market-grid / rebalancer)
- Add to `InstantiateMsg`/config: `fee_registry`, `fee_collector`.
- On fill, per §5: resolve the **single user's** tier; mint `V − fee` LP to the
  user and `fee` LP to the fee-collector. Grids mint their internal `SHARES`
  ledger; the rebalancer mints through the pooled `bot-liquidity` `MintTo`.
- Event attrs: `fee_tier`, `fee_bps`, `fee_shares`.

## 9. Security invariants

1. The user keeps exactly `V − fee` and the collector receives exactly `fee`
   (both as fresh LP), so the pool grows by `V` per fill — the supply is never
   diluted by more than the credited value and never burned. Priced on the traded
   token.
2. A user's fee tier is resolved from their own balance, never from keeper input.
3. The collector withdraws only the LP it was credited; it cannot touch user free
   balances or escrow beyond its share.
4. `collect` is keeper-only; the treasury is a fixed governance address.
5. Cross-token valuation: v1 prices the collector's LP against the traded token
   only (the token that grew on the fill). Review required where a fill's escrow
   and free balances are large relative to the other token; a future version may
   use a CL8Y/USTC-based valuation.
6. Fee computation must be monotonic-safe and never underflow the fill output
   (skip the fee if `fee = 0` or dust; cap `fee_bps ≤ 10 000`).
7. Only the fee-registry reads the CL8Y balance. A saved `Holdings` value is
   written only from a *successful* registry query — never from keeper/sender
   input — and is used only as a fallback when a live query fails.
8. A fill whose live query and saved holding are both absent falls back to the
   **lowest tier** (effective fee = `base_fee_bps`, no discount), never an
   under-fee. The saved holding is refreshed by `RefreshHolding`; the fee is
   never under-fee because a live read that succeeds always wins over a saved
   value.

## 10. Rollout

1. Deploy the shared core (`fee-registry` + `fee-collector`).
2. Integrate the three venues on the **uniform single-user mint** model: limit-grid
   (bot `owner`), market-grid (vault `admin` internal `SHARES`), rebalancer
   (vault `admin` + external `bot-liquidity` LP via `MintTo`). One mechanic, one
   tier resolution per fill across all systems.

## 11. Open items (confirm before implementation)

- `base_fee_bps`: proposed **180** (the CL8Y mainnet `default_fee_bps`); confirm
  whether one global value or per-strategy/per-pair.
- `fee-collector`: whether to add a timelock on treasury transfers.
- v1 pricing (per-token) acceptance vs. requiring a cross-token valuation.
- Whether maker grid fees use the same ladder as taker swap fees.