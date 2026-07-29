# Bot Liquidity Token Protocol

Source: `contracts/bot-liquidity`

## Purpose

This contract is both the user-flow controller and the transferable CW20 bot LP
token for one vault. Its shares represent proportional ownership of that
vault's token A and token B inventory.

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
messages settle. It measures actual vault balance changes, checks final
allocation, calculates deposit NAV from one pre-operation price snapshot, and
then mints shares. No shares are minted before settlement.

## Withdrawals

Claims use vault balances and total supply before shares are burned. Balanced
withdrawals transfer both claims. Token-0-only and token-1-only withdrawals
require a swap spending exactly the user's unwanted-token claim. The final
payout uses the actual wanted-token vault balance increase and enforces the
user's minimum output.

## Inflation And Donation Protection

- The initial depositor must satisfy `minimum_initial_deposit`.
- At least 1,000 smallest share units are permanently locked.
- Pre-first-deposit donations receive permanently locked shares.
- Existing vault donations are included in pre-deposit NAV.
- Deposits that round to zero shares revert.
- User-provided `min_shares` protects against adverse settlement.

## Invariants

- At most one pending deposit or single-token withdrawal per transaction.
- Share minting occurs only after settled balance and allocation checks.
- Withdrawal claims are proportional to pre-burn supply.
- A single-token withdrawal swaps only that owner's proportional claim.
- The current test implementation mints and redeems shares without an
  additional protocol charge.

## Supported Assets

The first release requires the pair's two CW20 assets to use equal decimals.
Fee-on-transfer and rebasing tokens are not supported.
