# Rebalancer Release Readiness

This is an operational gate, not a claim of security. Every item must identify
the exact release commit and retained evidence before funds are accepted.

Current status: **BLOCKED**. Approved real registry/collector/proxy addresses do
not exist, canonical fee E2E is unrun, and the `0.2.0` redeployment described
below has not been executed.

## Required Roles

| Role | Minimum policy | Scope |
|---|---|---|
| Internal contract admin | 2-of-3 multisig | Pause/resume, keeper, risk configuration and role proposals |
| Wasm migration admin | 2-of-3, separate signers preferred | Code migration only |
| Keeper | Dedicated low-balance operational key | Rebalance and reference synchronization |
| Release approver | Reviewer other than the author | Commit, artifacts and deployment manifest |

Do not deploy with a personal wallet as internal or Wasm admin. Record each
multisig address, threshold, signers, hardware-key custody and replacement process
in the private deployment record. Never commit signer identities or secrets here.

## Pre-Deployment Gate

- Exact commit has successful source, dependency, reproducible-Wasm and LocalTerra
  E2E GitHub Actions checks.
- Optimized Wasm hashes match retained CI artifacts.
- An independent reviewer approved the complete diff from the prior release.
- The contract-specific 0.2.0 cutover was rehearsed: bot-vault and bot-liquidity
  0.1.x were redeployed, any routed 0.1.x proxy was replaced, and routes were
  re-registered. Only an empty compatible proxy may use migration.
- Internal and Wasm admin addresses were independently verified by two operators.
- `liquidity_code_id`, factory, pair, `pair_code_id`, proxy, fee registry,
  collector, assets, decimals, and keeper match the signed deployment manifest.
- Vault `paused` is queried and understood before funding.
- Monitoring and incident contacts are active.

## Canary Limits

The first deployment must define these values in its approved manifest:

| Limit | Required value |
|---|---|
| Maximum aggregate vault value | `<APPROVED_CANARY_TVL>` |
| Maximum single depositor value | `<APPROVED_USER_CAP>` |
| Maximum trade basis points | Lower than or equal to the contract hard bound |
| Maximum pool-depth share | Lower than or equal to the contract hard bound |
| Canary observation period | `<MINIMUM_BLOCKS_OR_DAYS>` |
| Required successful rebalance/withdraw cycles | `<APPROVED_COUNT>` |

These caps are operational because direct CW20 donations cannot be prevented.
Monitoring must pause when queried balances exceed the approved aggregate cap.
Do not increase limits before the observation period completes without unresolved
alerts and a second multisig approval records the decision.

## Emergency Controls

Pause the vault:

```json
{"pause":{}}
```

Pause blocks deposits, liquidity swaps, keeper rebalances and reference
synchronization. Exact authorized transfers remain available so an existing
pro-rata exit is not converted into a fund lock. Single-token withdrawals requiring
a swap fail safely and retain shares.

Revoke a compromised liquidity controller:

```json
{"revoke_liquidity_contract":{}}
```

Revocation also pauses the vault. Resume requires a configured controller whose
runtime code ID equals the approved `liquidity_code_id`:

```json
{"resume":{}}
```

Before resume, verify code ID, configuration, balances, pending authorization,
pair state and latest finalized transactions.

## Monitoring And Stop Conditions

Alert and pause on:

- vault balance above the approved canary cap;
- liquidity code ID mismatch or missing reciprocal binding;
- transaction status unresolved beyond the keeper timeout;
- repeated CheckTx or DeliverTx failure for an unchanged plan;
- spot/TWAP, spread, pool-depth or allocation rejection;
- unexpected admin, keeper, route or Wasm-admin change;
- withdrawal failure, zero-share result or pending state outside a transaction;
- dependency or workflow failure on the deployed commit.

Retain transaction hashes, block heights, queried configuration, balances and
alert acknowledgements. Logs without commit and chain ID are not release evidence.

## `0.2.0` Redeployment Order

1. Pause the old vault and stop keeper broadcasting.
2. Confirm no pending rebalance or liquidity settlement exists.
3. Withdraw or otherwise complete the separately reviewed 0.1.x vault and
   liquidity wind-down; do not migrate either state into 0.2.0.
4. Deploy the reviewed `0.2.0` proxy/vault/liquidity artifact set with canonical
   registry/collector/proxy values, factory, and approved `pair_code_id`.
5. Bind bot-liquidity, verify reciprocal vault/assets/runtime code IDs, and
   register the new vault route on the proxy.
6. Execute low-value deposit, pro-rata withdrawal, single-token withdrawal and
   keeper rebalance checks.
7. Resume only after two operators compare state with the deployment manifest.

If the old proxy has any routes, do not migrate it: deploy a fresh 0.2.0 proxy.
Only frozen-fixture-tested empty compatible proxy state may migrate.

Abort rather than clear an unexpected pending state. Internal admin transfer and
chain Wasm-admin transfer are separate procedures.

## Current Limitations

- No external audit is bundled with the current working tree.
- Operational caps require monitoring and governance enforcement.
- CL8Y pair behavior, chain finality and standard CW20 semantics remain external
  trust assumptions.
- Passing this checklist does not imply zero vulnerabilities.
