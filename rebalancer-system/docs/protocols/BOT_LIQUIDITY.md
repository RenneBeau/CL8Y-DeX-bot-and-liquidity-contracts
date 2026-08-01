# Bot Liquidity Token Protocol

Source: `contracts/bot-liquidity`

## Purpose

This contract is both the user-flow controller and the transferable CW20 bot LP
token for one vault. Its shares represent proportional ownership of that
vault's token A and token B inventory.

## Roles

- `admin`: may update `minimum_initial_deposit` via `UpdateConfig` only before
  the first LP mint. Omitted fields retain their current values.
- `vault`: the only contract allowed to settle deposits and single-token
  withdrawal swaps.

## CW20 Behavior

The contract delegates standard transfer, send, burn, allowance, delegated
transfer, marketing, and CW20 query behavior to `cw20-base`. Only the contract
itself is configured as minter. Each vault receives its own liquidity-token
instance, keeping every bot's shares tied to that bot's portfolio.

## Deposits

Deposits specify two maximum inputs, a deadline, minimum acceptable shares, and
an optional swap. Transfers go directly from the user to the vault. A deposit
swap can spend only the offer token and amount included in that deposit, which
prevents a depositor from rebalancing incumbent assets for personal benefit.

The contract stores one pending operation and uses a reply after all nested
messages settle. It measures actual vault balance changes and checks final
allocation. The first mint uses a price snapshot; established-vault mints use
the smaller proportional contribution across both assets. No shares are minted
before settlement.

## Withdrawals

Claims use vault balances and total supply before shares are burned. Pro-rata
withdrawals burn shares and transfer both claims at the vault's current A/B
ratio atomically. Token-0-only and token-1-only withdrawals require a swap
spending exactly the user's unwanted-token claim and use an escrow pattern:
the shares are **not** burned up front. The swap is dispatched as a
`reply_always` submessage. On success the reply burns the owner's shares and
pays out using the actual wanted-token vault balance increase, enforcing the
user's minimum output. On failure the pending operation is cleared and the
owner keeps their shares — a failed swap can never strand a position.

## Inflation And Donation Protection

- The initial depositor must satisfy `minimum_initial_deposit`, which must be
  greater than the 1,000 permanently locked share units.
- At least 1,000 smallest share units are permanently locked.
- Pre-first-deposit donations receive permanently locked shares.
- Existing vault donations are included in pre-deposit balances.
- Deposits that round to zero shares revert.
- User-provided `min_shares` protects against adverse settlement.

## Invariants

- At most one pending deposit or single-token withdrawal per transaction.
- Share minting occurs only after settled balance and allocation checks.
- Withdrawal claims are proportional to pre-burn supply.
- A single-token withdrawal swaps only that owner's proportional claim.
- Shares are burned only after a single-token swap succeeds; a failed swap
  refunds the pending operation and keeps the owner's shares.
- The current test implementation uses a zero protocol charge for share minting
  and redemption.

## Supported Assets

The first release requires the pair's two CW20 assets to use equal decimals.
Supported assets use exact-transfer, fixed-balance CW20 semantics.
