# Full Repository Internal Static Audit - 2026-08-08

**Status:** Complete internal static review with uncommitted working-tree
remediation update; production decision: **BLOCKED**.

## Scope

- Inspected code baseline: repository HEAD `921859b7bcfa9e6e80ffb78672474874332dd03d` (`921859b`). Source references below are to that immutable baseline unless explicitly labelled working tree.
- Scope: all four Rust workspaces, Python keepers/operator, shell deployment and E2E tooling, GitHub workflows, release tooling, and repository documentation.
- Working-tree note: the audit fixes described as closed/partial below are
  uncommitted. They are not part of immutable baseline `921859b` and are not
  production evidence until reviewed, committed, and rerun on an exact SHA.
- Classification: author/internal static audit report, **not** an independent security audit, certification, or production approval.

## Method

- Manual static review of contract accounting, authorization, migration, external-contract trust, settlement, operator transaction lifecycle, persisted state, deployment scripts, CI/release definitions, and documented claims.
- Cross-check of the confirmed finding list against exact source paths and line ranges at `921859b`.
- Review of current uncommitted documentation/test diffs only to identify corrections made during this audit and avoid repeating superseded claims.
- Duplicate observations were merged by root cause and impact.

## Limitations

- After remediation, `make test`, `make clippy`, shell syntax validation,
  `test-area/common-test.sh`, workflow YAML parsing, and an explicit missing
  registry environment compile-failure check were reported successful on the
  uncommitted working tree. Python isolated-install, package-build, lint, type,
  and branch-coverage gates also passed. No LocalTerra fee E2E, production
  deployment, `0.2.0` redeployment, complete reproducible release-artifact run,
  commit, or push was performed as part of this update.
- This review did not independently inspect the deployed CL8Y pair/factory implementation, current mainnet code IDs, proxy whitelist, governance configuration, GitHub branch protections, runner settings, or retained artifact contents.
- Static review cannot prove absence of vulnerabilities. Economic modeling, adversarial chain testing, migration rehearsal, artifact inspection, and an independent external audit remain required.
- Exact line references can move after `921859b`; use `git show 921859b:<path>` when validating them.

## Executive Summary

No Critical finding was confirmed. Of 38 findings, 35 are closed,
documentation-corrected, or formally de-scoped in the working tree, and three
are partial. No finding remains open. H-05 remains operationally partial because approved deployed production
registry, collector, and proxy addresses do not exist. H-08 remains an evidence
gap for deeper rebalancer real-registry full-flow execution on the current
SHA. M-30 remains partial for package/tag version consistency and the
narrow/expiring treatment of the RustSec exception. M-15 and M-32 are closed as
formally accepted out of scope because limit-grid is abandoned as a production
venue and retained only as a PoC artifact; their underlying DEX semantics were
not technically validated.

The contracts also contain meaningful strengths: pending settlement state and reply checks; bounded spread, price, pool-depth, trade-size, and allocation controls; generally checked arithmetic; reciprocal liquidity binding and runtime code-ID checks; proxy route validation; two-step administration; and extensive unit/integration coverage.

### Production Decision

**BLOCKED.** Do not publish production artifacts or accept economic funds until
approved registry/collector/proxy addresses exist, proxy/pair external
semantics are verified, remaining High/Medium findings are closed or formally
accepted, canonical fee E2E runs on the exact candidate SHA, and an independent
audit plus the `0.2.0` redeployment rehearsal completes.

## Severity Table

| Severity | Count | Disposition |
|---|---:|---|
| Critical | 0 | None confirmed |
| High | 8 | 6 closed in working tree; 2 partially open |
| Medium | 24 | 23 closed/de-scoped; 1 partial in working tree |
| Low/Coverage | 6 | 6 closed/doc-corrected |
| **Total** | **38** | Production remains blocked |

Across all severities: 35 findings are closed/doc-corrected/de-scoped in the
working tree and 3 are partial. None remains open. These are working-tree dispositions,
not immutable release evidence.

- **Partial:** H-05, H-08, M-30.
- **Open:** none.

## Findings

### Critical

None confirmed.

### High

#### H-01 - Nominal fee value is minted as LP units without NAV/share conversion

- **References:** `921859b:limit-grid-system/contracts/grid-vault/src/contract.rs:1838-1874`; `921859b:market-grid-system/contracts/grid-vault-swap/src/contract.rs:646-710`; `921859b:rebalancer-system/contracts/bot-vault/src/contract.rs:1196-1269`; contrast market deposit NAV conversion at `921859b:market-grid-system/contracts/grid-vault-swap/src/contract.rs:1307-1341`.
- **Impact:** Collector economic value differs from `fill_value * fee_bps / 10_000` whenever NAV/share is not 1; holders may be over- or under-diluted relative to the advertised fee.
- **Evidence/reasoning:** Each venue computes token-0 value, names the result `fee_lp`/`fee_value`, and adds or mints that number directly as share units. No division by current NAV/share is performed.
- **Remediation:** Define the intended economic fee, compute shares as fee value divided by pre-fee NAV/share with explicit rounding, and add conservation/redemption tests across NAV below, equal to, and above 1.
- **Working-tree resolution:** All three venues now compute economic fee
  `F = floor(V*bps/10000)` and NAV-priced shares
  `x = floor(F*S/(A-F))`. Flooring ensures the collector's immediate claim is
  no greater than `F`; varied-NAV regressions cover the conversion.
- **Status:** **closed in working tree**.

#### H-02 - Limit-grid emergency exit can strand collector shares after transferring all assets

- **References:** `921859b:limit-grid-system/contracts/grid-vault/src/contract.rs:952-1008` (collector redemption); `:1218-1271` (emergency transfer of complete vault token balances, removal only of owner shares, and `total_shares = 0`); `:1860-1870` (collector shares minted).
- **Impact:** A bot owner can receive assets backing collector LP while collector ledger entries remain. Subsequent collector redemption cannot be solvent and can divide by zero or fail. This is a direct protocol-fee theft/insolvency risk once collector shares exist.
- **Evidence/reasoning:** Emergency withdrawal transfers each contract-wide CW20 balance to the owner-selected recipient, removes only `(bot_id, bot.owner)`, zeros total shares, and does not clear or redeem `(bot_id, fee_collector)`.
- **Remediation:** Make exit pro-rata across all shareholders or require collector redemption first; scope transferred balances to the bot; enforce `sum(SHARES) == total_shares`; add fee-bearing emergency-exit tests.
- **Working-tree resolution:** Exit withdrawal now burns/transfers only the
  owner's pro-rata position, preserves collector backing and total shares, and
  permits collector redemption in Exit after active orders reach zero.
- **Status:** **closed in working tree**.

#### H-03 - Market-grid lower boundary makes status/rebalance error with token 1 held

- **References:** `921859b:market-grid-system/contracts/grid-vault-swap/src/contract.rs:981-1009` (cell 0 and target weight 0); `:1012-1032` (zero target); `:1088-1094` (`actual > 0`, `expected == 0` returns `empty reference`); call sites `:826-857` and `:877-903`.
- **Impact:** At or below `lower_price`, any nonzero token-1 inventory can make `grid_status` and rebalance planning fail instead of planning a move toward the 0% token-1 target.
- **Evidence/reasoning:** Cell 0 produces a zero target; `allocation_deviation` passes nonzero actual and zero expected to `ratio_deviation`, which returns an error rather than 10,000 bps.
- **Remediation:** Define zero-target deviation as 0 when actual is zero and 10,000 when actual is nonzero; test both lower and upper boundary portfolios and successful corrective trades.
- **Working-tree resolution:** Zero-target deviation is defined at the lower
  boundary and corrective-plan regressions cover ideal target behavior.
- **Status:** **closed in working tree**.

#### H-04 - Production release and required CI omit fee-system/mainnet artifact guarantees

- **References:** `921859b:.github/workflows/release.yml:37-69`; `921859b:.github/workflows/wasm.yml:20-28`; `921859b:.github/workflows/ci.yml:15-34`; `921859b:.github/workflows/security.yml:31-40`; `921859b:.github/scripts/reproducible-build.sh:28-44`; `921859b:Makefile:29-36,43-61`.
- **Impact:** A signed release can omit both fee-system Wasms and publish default-feature vault Wasms without production locks. Fee-system source/security checks are not release-required.
- **Evidence/reasoning:** Release/reproducible workflows list only three workspaces; the optimizer receives no feature selection; source and security matrices omit `fee-system`; the standalone fee target is not integrated into signed reproducible release evidence.
- **Remediation:** Build, double-build, checksum, attest, and inspect all production Wasms with explicit `mainnet` features; add fee-system source/security jobs and make them required for release.
- **Working-tree resolution:** CI/security/release/reproducible definitions now
  cover all four workspaces, default/mainnet artifact sets, fee-system, required
  checks, and manifests. Complete reproducible execution was not part of the
  primary validation reported here.
- **Status:** **closed in working tree**; exact-SHA workflow evidence remains a
  release requirement.

#### H-05 - Canonical collector/proxy locks are inert and registry collector is mutable

- **References:** `921859b:limit-grid-system/contracts/grid-vault/src/mainnet.rs:5-16`; `921859b:market-grid-system/contracts/grid-vault-swap/src/mainnet.rs:15-18`; `921859b:rebalancer-system/contracts/bot-vault/src/mainnet.rs:15-18`; `921859b:fee-system/contracts/fee-registry/src/contract.rs:40-56,310-347`; `921859b:fee-system/contracts/fee-registry/tests/mainnet_lock.rs:197-210`.
- **Impact:** Current production deployment is blocked. Unset vault constants accept arbitrary collector/proxy values, and registry governance can repoint `fee_collector`, defeating a complete canonical topology guarantee.
- **Evidence/reasoning:** Vault constants are `None`; helper behavior deliberately accepts any/absent address when unset. Registry mainnet checks pin CL8Y and treasury, not collector, and a mainnet test exercises collector update.
- **Remediation:** Deploy and independently verify canonical collector/proxy, populate constants, pin registry collector (or document and approve a deliberately mutable trust model), and test artifact-level rejection of absent/alternate addresses.
- **Working-tree resolution:** Mainnet vault compilation now pins all applicable
  topology inputs: `CL8Y_CANONICAL_FEE_REGISTRY`,
  `CL8Y_CANONICAL_FEE_COLLECTOR`, and `CL8Y_CANONICAL_SWAP_PROXY`. Limit-grid
  uses registry and collector and has no proxy. Missing/empty input fails the
  build; missing registry was explicitly verified to fail compilation.
- **Status:** **partially closed**. Code pinning is closed, but approved real
  registry/collector/proxy addresses do not exist and canonical proxy
  deployment/verification/whitelisting remains open.

#### H-06 - Grid manager creates permanently fee-disabled limit vaults

- **References:** `921859b:limit-grid-system/contracts/grid-manager/src/msg.rs:5-17,35-47`; `921859b:limit-grid-system/contracts/grid-manager/src/contract.rs:103-125`; limit vault fee config at `921859b:limit-grid-system/contracts/grid-vault/src/msg.rs:5-20`; execute surface at `:41-112` has no fee update; charging requires both fields at `contract.rs:1817-1819`.
- **Impact:** Manager-created limit vaults cannot charge protocol fees, and the instantiated vault cannot add fee addresses later.
- **Evidence/reasoning:** Manager config/template omits `fee_registry` and `fee_collector`; limit-vault fees are instantiate-only.
- **Remediation:** Add pinned fee addresses to manager state/template and guarded updates for future creations, or add a safe vault fee-config migration/update path; test manager-created fee-enabled vaults end to end.
- **Working-tree resolution:** The manager stores and propagates both fee
  addresses, rejects partial configuration, and requires both on `mainnet`.
  Updates affect future vaults only; old fee-disabled vaults require migration
  or redeployment.
- **Status:** **closed in working tree** for new vault creation.

#### H-07 - Canonical base-fee inconsistency (180 vs 1800 bps)

- **References:** baseline inconsistency at `921859b:market-grid-system/contracts/grid-vault-swap/tests/grid_vault_swap_integration.rs:355-377,1520-1579`; canonical 180 evidence at `921859b:test-area/fee-e2e-test.sh:147-219` and fee docs; working-tree corrections in `market-grid-system/contracts/grid-vault-swap/tests/grid_vault_swap_integration.rs` and `docs/TEST_RESULTS.md:23-32` set/describe 180 bps.
- **Impact:** Historical tests could validate a 10x fee and normalize incorrect configuration. The present risk is release/config drift, not a confirmed current production deployment.
- **Evidence/reasoning:** Baseline real-registry market tests seed 1,800 bps while tier-9 comments elsewhere derive 9 bps from 180. Current uncommitted corrections standardize 180.
- **Remediation:** Commit the correction, define one machine-readable canonical
  undiscounted rate, assert it in deployment/E2E, and reject manifests that
  differ without explicit governance approval. Document that user-facing tier 0
  is currently encoded as `tier_id: None`; reserved governance storage ID `0`
  is a separate 100%-discount entry and must not share the same operational name.
- **Working-tree resolution:** Code/tests/docs consistently use 180 bps for the
  undiscounted user-facing tier 0, encoded as `tier_id: None`; reserved storage
  ID `0` remains separate.
- **Status:** **closed in working tree**; candidate configuration still requires
  exact-SHA verification.

#### H-08 - Fee-ladder coverage was overstated across venues

- **References:** `921859b:market-grid-system/contracts/grid-vault-swap/tests/grid_vault_swap_integration.rs:1613-1647,1649-1698`; `921859b:limit-grid-system/contracts/grid-vault/tests/grid_vault_integration.rs:2280-2390`; `921859b:rebalancer-system/contracts/bot-vault/tests/real_registry_ladder.rs:38-125`; `921859b:rebalancer-system/contracts/bot-vault/src/contract.rs:2097-2139`; corrected scope at working-tree `docs/TEST_RESULTS.md:23-32,432-492`.
- **Impact:** Release decisions could rely on tests that do not exercise the claimed integration path.
- **Evidence/reasoning:** Market-grid has a real-registry full rebalance ladder. Limit-grid and rebalancer ladder tests query the real registry directly; rebalancer charge-rate coverage uses mocked registry rates. This is a coverage gap, not evidence that fee resolution is wrong.
- **Remediation:** Add a real-registry full rebalancer swap/settlement test; keep
  documentation precise about direct-query versus full-flow evidence. Limit-grid
  coverage is PoC-only and no longer a production release criterion.
- **Status:** **partially closed**. Documentation is corrected; deeper
  real-registry rebalancer swap/settlement full-flow coverage remains open.

### Medium

#### M-09 - Market-grid migration has neither entry point nor version checks

- **References:** `921859b:market-grid-system/contracts/grid-vault-swap/src/contract.rs:1368-1370`; compare versioned migrations in `921859b:fee-system/contracts/fee-registry/src/contract.rs:489-493`.
- **Impact:** Migration export/wiring may be unreliable and arbitrary source versions can migrate without compatibility enforcement.
- **Evidence/reasoning:** `migrate` lacks `#[entry_point]`, ignores storage/message, and does not call `ensure_from_older_version` or update CW2.
- **Remediation:** Add the entry point and strict contract-name/from-version checks; migrate state explicitly and test accepted/rejected versions.
- **Working-tree resolution:** Migration is exported as an entry point, validates
  CW2 contract identity, requires a strictly older semantic version, and updates
  CW2 metadata; accepted/rejected source cases are tested.
- **Status:** **closed in working tree**.

#### M-10 - Market-grid planned offer likely under-trades the target

- **References:** `921859b:market-grid-system/contracts/grid-vault-swap/src/contract.rs:1035-1070`, especially `:1050-1058`; contrast equal-value rebalancer formula at `921859b:rebalancer-system/contracts/bot-vault/src/contract.rs:878-905`.
- **Impact:** Rebalances can stop materially short of the configured cell target, causing repeated transactions, excess fees/gas, or failure to reach tolerance.
- **Evidence/reasoning:** The function computes the difference between current and target token-1 value and then divides it by 2. For a fixed target fraction, trading the difference already moves value from one side to the other; the additional `/2` appears unjustified. Because pair price/target conventions were not independently proven here, this is a **high-confidence likely bug**, not labelled a confirmed invariant violation.
- **Remediation:** Derive the equation formally for both directions including execution loss, remove or justify `/2`, and add exact post-trade target tests across cells.
- **Working-tree resolution:** The extra division by two was removed and ideal
  target tests cover both trade directions and grid cells.
- **Status:** **closed in working tree**.

#### M-11 - Swap proxy permits only one vault per pair

- **References:** `921859b:rebalancer-system/contracts/swap-proxy/src/contract.rs:81-126`; state index `921859b:rebalancer-system/contracts/swap-proxy/src/state.rs:18`.
- **Impact:** Multi-strategy or replacement vaults for one pair cannot coexist on one canonical proxy; migration/incident response may require route removal downtime.
- **Evidence/reasoning:** `PAIR_VAULTS.has(pair)` rejects any second vault regardless of strategy or owner.
- **Remediation:** Confirm this isolation constraint operationally or key routes by pair plus vault/strategy with explicit policy and tests.
- **Working-tree resolution:** Routes are keyed only by vault; multiple vaults
  can register and route independently through one pair, with migration/tests
  for the former pair index.
- **Status:** **closed in working tree**.

#### M-12 - Public market-grid deposits conflict with admin-tier fee attribution

- **References:** public depositor credit at `921859b:market-grid-system/contracts/grid-vault-swap/src/contract.rs:220-262`; fee subject `config.admin` at `:646-680`.
- **Impact:** If pooled public deposits are intended, LPs receive the admin's CL8Y tier rather than their own economic fee treatment; admin changes can alter everyone’s fee.
- **Evidence/reasoning:** Any CW20 sender can deposit and own shares, while every rebalance bills one unrelated address.
- **Remediation:** Enforce single-owner deposits or define pooled fee policy and disclose/administer it explicitly; add multi-depositor tests.
- **Working-tree resolution:** Deposits are admin-only. Accepting an admin
  transfer migrates the old admin's shares to the new admin so ownership and fee
  identity remain aligned.
- **Status:** **closed in working tree**.

#### M-13 - Market-grid/rebalancer pair authenticity is weaker than limit-grid

- **References:** market validation `921859b:market-grid-system/contracts/grid-vault-swap/src/contract.rs:46-65`; rebalancer `921859b:rebalancer-system/contracts/bot-vault/src/contract.rs:77-99`; proxy `921859b:rebalancer-system/contracts/swap-proxy/src/contract.rs:91-116`; limit-grid factory/code checks `921859b:limit-grid-system/contracts/grid-vault/src/contract.rs:271-333,1348-1353`.
- **Impact:** A malicious lookalike contract can self-report pair metadata and become a trusted execution/oracle dependency.
- **Evidence/reasoning:** Market/rebalancer validate self-reported address/assets but do not prove factory registration or pin runtime pair code ID as limit-grid does.
- **Remediation:** Require canonical factory lookup and approved runtime code ID at instantiate and before privileged execution.
- **Working-tree resolution:** Market-grid and rebalancer vaults now require a
  factory address and nonzero `pair_code_id`, verify the factory pair lookup and
  current runtime code at instantiate, and recheck runtime code before
  rebalance/proxy execution. Proxy registration records and enforces that
  provenance. Affected vault schemas are `0.2.0`; market-grid and bot-vault
  0.1.x require redeployment and route re-registration rather than migration.
- **Status:** **closed in working tree**. The unexecuted `0.2.0` redeployment is
  an explicit release/migration risk, not evidence of production completion.

#### M-14 - Fee failures fail open and cached holdings can misprice fees

- **References:** vault skip paths `921859b:limit-grid-system/contracts/grid-vault/src/contract.rs:1820-1830`; `921859b:market-grid-system/contracts/grid-vault-swap/src/contract.rs:668-674`; `921859b:rebalancer-system/contracts/bot-vault/src/contract.rs:1226-1234`; registry fallback `921859b:fee-system/contracts/fee-registry/src/contract.rs:447-481`.
- **Impact:** Registry/schema outages cause revenue loss; stale saved holdings can undercharge or overcharge relative to an unavailable current balance.
- **Evidence/reasoning:** Vault-level query errors mint no fee. Registry live-read errors use an unbounded-age snapshot or full base fee.
- **Remediation:** Set an explicit failure policy, snapshot age bound, monitoring/SLO, and recoverable fee debt or conservative cap if economically approved.
- **Working-tree resolution:** The approved outage policy is implemented in all
  vaults. Each caches the last successful effective bps/tier by fee subject
  (market/rebalancer `config.admin`; limit `bot.owner`). Registry unreachable
  charges that exact result with source `vault_cached`; no local history charges
  180 bps with no tier and source `lowest`. A reachable registry whose live CL8Y
  token query fails returns its full configurable base rate (180 in production),
  no tier, and `Lowest`; registry holding history is observability only and never
  pricing. Fee bypass and stale-holding discounts are therefore closed.
- **Status:** **closed in working tree**. The intentionally persistent local
  tier during registry outage is an explicit availability/revenue tradeoff, not
  an accidental unbounded registry holding cache.

#### M-15 - Disappeared limit orders are assumed fully executed

- **References:** `921859b:limit-grid-system/contracts/grid-vault/src/contract.rs:640-650`; tests codify behavior at `921859b:limit-grid-system/contracts/grid-vault/tests/grid_vault_integration.rs:1400-1440,1559-1560`.
- **Impact:** If a pair can drop an order for any reason other than fill or vault-recorded cancel, accounting treats it as execution and may misattribute balance changes.
- **Evidence/reasoning:** Absence from active and parked queries plus no local cancel record is classified as fully executed. Safety depends on external pair semantics.
- **Remediation:** Verify and pin those semantics against production pair code; otherwise require positive terminal evidence or a reconciliation quarantine.
- **Disposition:** limit-grid is abandoned as a production venue and retained
  only as a PoC artifact. This external semantic assumption was not validated.
- **Status:** **closed by formal production de-scoping**.

This finding is unrelated to stale CL8Y balance fallback. That fallback was
removed and is resolved under M-14; M-15 remains the external pair terminal-state
assumption described above.

#### M-16 - Fee collector cumulative ledger uses unchecked u128 addition and ambiguous naming

- **References:** `921859b:fee-system/contracts/fee-collector/src/contract.rs:144-162,263-270`; state type at `921859b:fee-system/contracts/fee-collector/src/state.rs:14-17`.
- **Impact:** Extreme cumulative collections can overflow/trap; `VaultShares` appears current but is historical cumulative bookkeeping, creating monitoring/integration ambiguity.
- **Evidence/reasoning:** `existing.unwrap_or_default() + shares.shares.u128()` is unchecked and is never decremented after redemption.
- **Remediation:** Use checked addition or `Uint128`; rename/query as cumulative collected shares, or store current entitlement with defined lifecycle.
- **Working-tree resolution:** Cumulative share accounting now uses checked
  addition and returns an overflow error rather than wrapping/trapping. Current
  documentation explicitly identifies the ledger as cumulative observability,
  not current entitlement.
- **Status:** **closed in working tree**.

#### M-17 - Swap keeper clears ambiguous broadcast state and can retry

- **References:** fail-closed claim `921859b:grid-operator-system/services/grid-operator/grid_operator/swap_keeper.py:10-13`; ambiguous network path `:204-228`; retry eligibility `:174-197`.
- **Impact:** A node may accept a transaction and then drop the response; clearing `broadcasting` permits automatic rebroadcast, violating the stated fail-closed guarantee and risking duplicate action/signing.
- **Evidence/reasoning:** `RpcError` after `terrad.broadcast` clears pending/broadcasting state and raises; the next loop can submit the same plan again.
- **Remediation:** Preserve an unknown-broadcast state before network contact; recover hash from signed bytes/account sequence or require operator resolution before retry.
- **Working-tree resolution:** Pre-send durable broadcasting state is retained
  on transport or malformed-response ambiguity and is never automatically
  rebroadcast; restart regressions cover the accepted-but-client-error window.
- **Status:** **closed in working tree**.

#### M-18 - No single-process signer/state ownership lock

- **References:** SQLite open/transactions `921859b:grid-operator-system/services/grid-operator/grid_operator/db.py:74-110`; serial loop only within one process `keeper.py:235-247`; JSON trackers `swap_keeper.py:49-96` and `921859b:rebalancer-system/examples/keeper/keeper.py:38-80`.
- **Impact:** Two service instances sharing a signer can race sequences and independently broadcast despite per-process serialization.
- **Evidence/reasoning:** No OS file lock, database lease, PID ownership, or signer-wide mutex is acquired at startup.
- **Remediation:** Add an exclusive process lease tied to signer/chain/state path, with startup refusal and stale-lock recovery procedure.
- **Working-tree resolution:** Keepers acquire a nonblocking OS `flock` beside
  their database/state file before querying, signing, or broadcasting; a second
  process refuses startup.
- **Status:** **closed in working tree**.

#### M-19 - Persisted transaction state is not bound to chain, vault, and signer

- **References:** rebalancer JSON fields `921859b:rebalancer-system/examples/keeper/keeper.py:38-75`; swap JSON fields `921859b:grid-operator-system/services/grid-operator/grid_operator/swap_keeper.py:49-91`; transaction attempts schema `db.py:40-64`.
- **Impact:** Reusing a state file/database with another chain, vault, or key can suppress valid work, poll an unrelated hash, or mis-handle pending state.
- **Evidence/reasoning:** Identity appears in plan fingerprints but not as validated tracker metadata; swap state stores pending vault but not chain/signer; DB attempts do not bind signer/chain.
- **Remediation:** Persist schema version, chain ID, genesis hash, vault set, signer address, and operator mode; reject mismatches before any query/signing.
- **Working-tree resolution:** JSON and SQLite state validate schema, chain,
  vault/vault set, resolved signer/key identity, and protocol kind before use.
- **Status:** **closed in working tree**.

#### M-20 - Missing transaction response fields default to success

- **References:** rebalancer sync/final parsing `921859b:rebalancer-system/examples/keeper/keeper.py:188-205,208-239`; swap parsing `921859b:grid-operator-system/services/grid-operator/grid_operator/swap_keeper.py:109-122,125-159`; limit keeper `keeper.py:166-180,190-215`.
- **Impact:** Malformed or changed node envelopes can be accepted as code 0; a partial query response can confirm and clear durable state.
- **Evidence/reasoning:** `.get("code", 0)` and nested-envelope fallbacks treat absence as success; schema/type/required-height/hash checks are incomplete.
- **Remediation:** Strictly validate one supported response schema, require code/hash/height and requested-hash match, and treat missing fields as unknown/fail-closed.
- **Working-tree resolution:** Broadcast and finality parsing now requires
  explicit code, hash, and positive height with requested-hash matching; malformed
  envelopes remain unresolved instead of clearing state.
- **Status:** **closed in working tree**.

#### M-21 - Late fills can cross frozen reconciliation batches

- **References:** batch freeze selects current events at `921859b:grid-operator-system/services/grid-operator/grid_operator/keeper.py:60-99`; reconcile message contains only order IDs at `:101-109`; confirmation marks only frozen event IDs at `:223-233`; indexer can append later fills at `indexer.py:163-200`.
- **Impact:** A fill arriving after freeze but before on-chain reconcile can be economically included on chain while its event remains pending locally, causing a later duplicate/stale reconciliation attempt.
- **Evidence/reasoning:** The contract reconciles current pair state, not the frozen event totals/height; database confirmation closes only `batch_events` captured earlier.
- **Remediation:** Freeze by final on-chain order state/height, prevent fills crossing an active order batch, or reconcile/mark all events through confirmed execution height after state verification.
- **Working-tree resolution:** Successful reconciliation now atomically consumes
  every still-unconfirmed fill for the verified pair/order identities through
  the transaction's verified execution height, including fills indexed after
  the batch freeze. Events at later heights remain pending. Race regressions
  cover exact-once consumption across freeze, indexing, and confirmation.
- **Status:** **closed in working tree**.

#### M-22 - Operational errors can become permanent suppression/intervention

- **References:** three-failure intervention `921859b:grid-operator-system/services/grid-operator/grid_operator/keeper.py:41-58`; deterministic-plan suppression `swap_keeper.py:194-197,212-219`; rebalancer suppression `921859b:rebalancer-system/examples/keeper/keeper.py:266-273,310-332`.
- **Impact:** Misclassified transient/node/config errors can permanently stop a vault until manual state surgery, delaying reconciliation and risk control.
- **Evidence/reasoning:** String-marker classification is narrow; unchanged fingerprints remain suppressed; three failures enter intervention with no safe automated recovery workflow.
- **Remediation:** Use typed error classes/codes, reason-specific retry policy, alerts, and audited operator commands that revalidate chain state before clearing suppression.
- **Working-tree resolution:** Transaction and intervention failures now use
  typed result codes. Diagnose, resolve, and clear-intervention operations hold
  the process lock, verify reason-specific chain/transaction/vault/order/account
  evidence, create backups, and append audit records. Ambiguous evidence is
  refused. Unknown broadcast state remains fail-closed and is never
  automatically cleared or rebroadcast.
- **Status:** **closed in working tree**.

#### M-23 - Rebalancer LCD failures can prevent RPC fallback

- **References:** `921859b:rebalancer-system/examples/keeper/keeper.py:208-239`, especially `:221-223`.
- **Impact:** Any LCD HTTP error other than 404 aborts transaction resolution even when Tendermint RPC could provide the result.
- **Evidence/reasoning:** Non-404 errors are re-raised before the RPC query path.
- **Remediation:** Fall back on transport/5xx/schema failures, distinguish authoritative rejection from endpoint failure, and test both endpoints failing independently.
- **Working-tree resolution:** LCD transport, 5xx, and schema failures fall
  through to strict Tendermint RPC parsing, with independent endpoint tests.
- **Status:** **closed in working tree**.

#### M-24 - Timing environment values permit zero/invalid operation

- **References:** `921859b:grid-operator-system/services/grid-operator/grid_operator/config.py:6-10,45-51`; swap validation omits `poll_seconds` at `swap_keeper.py:250-269`; rebalancer validation omits `poll_seconds` at `921859b:rebalancer-system/examples/keeper/keeper.py:351-365`.
- **Impact:** Zero loop/poll/finality/deployment values can busy-loop, disable intended finality, or create unsafe operational behavior.
- **Evidence/reasoning:** Shared integer parser rejects negatives only; per-keeper validation does not cover all timing fields.
- **Remediation:** Define strict positive/minimum/maximum bounds for each duration, finality, deployment height, and batch size; reject non-finite gas values.
- **Working-tree resolution:** Main-loop, transaction polling/timeout,
  deployment/finality, and non-finite timing inputs now receive explicit
  validation with negative tests.
- **Status:** **closed in working tree**.

#### M-25 - No safe corrupted/unresolved-state recovery mechanism

- **References:** unguarded JSON load `921859b:rebalancer-system/examples/keeper/keeper.py:46-53` and `swap_keeper.py:60-68`; unresolved states `keeper.py:127-139`; database migration `db.py:83-100`.
- **Impact:** Truncated JSON or an unknown SQLite batch can halt service or require direct file/SQL edits that may trigger replay or lost suppression.
- **Evidence/reasoning:** There is no quarantine, backup restore, signed-state reconstruction, or audited resolve command that verifies chain/account sequence first.
- **Remediation:** Add checksum/versioned state, backup/quarantine, read-only diagnosis, and explicit resolve/abandon commands with chain verification and audit log.
- **Working-tree resolution:** JSON trackers use checksummed, versioned atomic
  replacement, three rotating backups, corruption quarantine, diagnosis/audit,
  reason-specific chain-verified resolution, and explicit verified restore.
  SQLite adds integrity and foreign-key diagnosis, operator audit records,
  online backups, and explicit identity/chain-verified restore. Unknown
  broadcasts are never treated as safe to abandon or rebroadcast.
- **Status:** **closed in working tree**.

#### M-26 - systemd example and OS keyring assumptions may conflict

- **References:** default keyring `921859b:grid-operator-system/services/grid-operator/grid_operator/config.py:25-29`; service isolation `grid-operator-system/services/grid-operator/grid-operator.service:8-21`.
- **Impact:** An unattended service user with `ProtectHome=true` may not access or unlock an OS desktop keyring; operators may weaken hardening or switch to insecure key storage ad hoc.
- **Evidence/reasoning:** The example provides no credential-agent/keyring provisioning model compatible with the service sandbox.
- **Remediation:** Document and test a production signer integration (hardware/remote signer or service-compatible credential store) without reducing sandbox controls.
- **Working-tree resolution:** The rebalancer systemd unit uses the service's
  Terra home and a `file` keyring unlocked only through a systemd
  `LoadCredential` password sent on stdin. The grid service uses a strict JSON
  external-signer argv protocol without a shell and expects signer material from
  systemd credentials. The `test` keyring backend is prohibited in production.
  Host credential and signer provisioning remains a deployment prerequisite,
  not an unresolved code/documentation finding.
- **Status:** **closed in working tree**.

#### M-27 - Test-area broadcast helper does not validate sync JSON code

- **References:** `921859b:test-area/common.sh:57-88`; callers commonly extract `.txhash`, e.g. `921859b:test-area/deploy-system.sh:42,124,132`.
- **Impact:** A CLI exit 0 with CheckTx `code != 0` can be treated as broadcast success and weaken E2E/deployment evidence.
- **Evidence/reasoning:** `terrad_tx_from` checks process return code and sequence-mismatch stderr only; unlike `wait_tx`, it never validates the sync response code before returning JSON.
- **Remediation:** Require valid JSON, code 0, and nonempty hash in the helper before returning.
- **Working-tree resolution:** The shared helper now requires an object JSON
  response, code zero, and a nonempty transaction hash while retaining bounded
  sequence-mismatch retry behavior; dedicated common-helper tests pass.
- **Status:** **closed in working tree**.

#### M-28 - Fee E2E is outside canonical CI and market-1000 can false-green

- **References:** canonical CI `921859b:.github/workflows/e2e.yml:1-64`; local target `921859b:Makefile:75-76`; fee scripts are standalone; market-1000 omits proxy at `921859b:test-area/fee-e2e-market-1000.sh:96-105` and accepts absent fee at `:121-133`.
- **Impact:** Release checks do not prove fee-enabled behavior; market-1000 may finish after logging that no fee was charged and does not exercise production proxy routing.
- **Evidence/reasoning:** No canonical workflow invokes `fee-e2e-*.sh`; the script branches on empty fee instead of failing and instantiate JSON lacks `proxy`.
- **Remediation:** Add all fee venues to exact-SHA CI, require expected rate/tier/nonzero shares/proxy route/treasury delta, and retain logs/artifacts.
- **Working-tree resolution:** `make local-fee-e2e` and a retained canonical fee
  workflow were added. Market-1000 now requires proxy registration, exact fee
  tier/rate/source, matching nonzero collector shares, and positive treasury
  deltas. The workflow was not run locally in this working tree.
- **Status:** **closed in working tree** as a definition/false-green finding;
  current-SHA execution evidence remains a release gate.

#### M-29 - setup.sh can ignore arbitrary deployment failure if an env file exists

- **References:** `921859b:test-area/setup.sh:16-28`.
- **Impact:** A stale/partial `.env.local` can turn a failed DEX deployment into an apparently ready environment and produce false test results.
- **Evidence/reasoning:** Any nonzero deploy status is ignored solely when the file exists; the script does not validate addresses, code IDs, health, or that only the optional indexer failed.
- **Remediation:** Make the upstream deploy expose a specific optional-indexer status or validate every required deployment output and on-chain contract before continuing.
- **Working-tree resolution:** Setup validates every required generated address
  as a live on-chain contract before accepting a post-core deployment failure;
  an env file alone is no longer sufficient.
- **Status:** **closed in working tree**.

#### M-30 - Release policy lacks ancestry/version consistency and complete fee security gates

- **References:** tag checks `921859b:.github/workflows/release.yml:26-36`; required checks `:37-60`; package versions in workspace Cargo manifests; fee omission/RUSTSEC exception `921859b:.github/workflows/security.yml:31-40`; `921859b:Makefile:29-36`.
- **Impact:** A valid signed tag can point outside the intended main lineage or publish artifacts whose package versions do not match the tag; fee dependencies are unscanned; a global ignored advisory may outlive a narrow justification.
- **Evidence/reasoning:** Workflow checks signed annotated tag and exact SHA but not ancestry or `vX.Y.Z` consistency; security loops omit fee-system and ignore `RUSTSEC-2024-0344` without package/version-scoped enforcement.
- **Remediation:** Require ancestry from protected release branch, clean/version-consistent manifests, fee-system audit/deny, and a documented expiring advisory exception constrained to affected package/version.
- **Working-tree resolution:** Release now checks tag ancestry from `main`, and
  fee-system is included in source/security/release requirements. Package-version
  to tag consistency and narrowing/expiry of the global RustSec exception remain
  unresolved.
- **Status:** **partially closed**.

#### M-31 - Market-grid pause freezes withdrawals

- **References:** withdrawal calls `assert_not_paused` at `921859b:market-grid-system/contracts/grid-vault-swap/src/contract.rs:265-275`; collector redemption also blocked at `:715-744`; admin pause `:590-609`.
- **Impact:** Admin pause creates custodial liveness risk by preventing ordinary LP exits and protocol collection during an incident.
- **Evidence/reasoning:** Pause applies to deposits/rebalances and both redemption paths, with no emergency pro-rata withdrawal.
- **Remediation:** Permit safe pro-rata withdrawals while paused, subject to pending-swap exclusion, or add a separately governed emergency exit with invariant tests.
- **Working-tree resolution:** Owner and collector pro-rata withdrawals are
  permitted while paused, while any pending swap/rebalance continues to block
  withdrawal.
- **Status:** **closed in working tree**.

#### M-32 - Maker/direct limit-grid DEX fee semantics are not proven

- **References:** direct pair order placement `921859b:limit-grid-system/contracts/grid-vault/src/contract.rs:1363-1428`; protocol fee on credited fills `:1802-1876`; no proxy field in limit config/messages `921859b:limit-grid-system/contracts/grid-vault/src/state.rs:10-27` and `msg.rs:5-20`.
- **Impact:** Net execution economics may include maker/direct DEX fees in addition to protocol fees, or differ from documented zero-fee proxy assumptions.
- **Evidence/reasoning:** Limit-grid never routes through the shared proxy; this audit did not independently verify production pair maker fee and recipient semantics.
- **Remediation:** Verify exact deployed pair code/config and fill event amounts externally; document gross/net/protocol fee ordering and add mainnet-equivalent tests.
- **Disposition:** limit-grid is abandoned as a production venue and retained
  only as a PoC artifact. Direct-maker fee ordering was not validated.
- **Status:** **closed by formal production de-scoping**.

This is distinct from registry CL8Y token-query failure semantics, which now
fail to full base/`Lowest` and are closed under M-14. M-32 concerns externally
deployed pair maker/direct fee ordering and amounts.

### Low / Coverage

#### L-33 - Fee artifact path/default-feature behavior is ambiguous and lacks a manifest

- **References:** `921859b:Makefile:49-61`; release collection `921859b:.github/workflows/release.yml:62-89`; optimizer script `921859b:.github/scripts/reproducible-build.sh:28-48`.
- **Impact:** Operators can confuse raw `artifacts/fee-system` Wasms with optimized release artifacts or omit files/features without a machine-checked inventory.
- **Evidence/reasoning:** Default `fee-wasm` builds but does not copy artifacts; `MAINNET=1` copies raw Wasms outside `release/`; no manifest records crate, feature set, hash, code version, and canonical constants.
- **Remediation:** Produce one signed artifact manifest and fail release on unexpected/missing Wasms or feature sets.
- **Working-tree resolution:** Reproducible builds produce feature-suffixed Wasm
  names and per-workspace manifests recording source SHA, feature set, canonical
  inputs, optimizer image, artifact names, and hashes; release checks expected
  artifact/manifest counts.
- **Status:** **closed in working tree**. Complete primary reproducible execution
  was not performed in this update.

#### L-34 - Migration tests do not use frozen released-state fixtures

- **References:** migration tests include `921859b:limit-grid-system/contracts/grid-vault/src/contract.rs:2420-2475`, `921859b:rebalancer-system/contracts/bot-vault/src/contract.rs:1839-1880`, and fee integration helpers, but no immutable prior-release state artifact is identified.
- **Impact:** Tests can pass while silently tracking current structs rather than proving compatibility with deployed historical bytes.
- **Evidence/reasoning:** Tests seed storage using current Rust types/helpers instead of importing frozen old-state fixtures and old contract binaries/schemas.
- **Remediation:** Add fixture snapshots for every released source version and rehearse migrate/query/withdraw/rollback paths.
- **Working-tree resolution:** Frozen raw-state fixtures now cover all seven
  migrate entry points. The tested policy is explicit: market-grid
  `grid-vault-swap` 0.1.x and rebalancer `bot-vault` 0.1.x require redeployment;
  a 0.1.x `swap-proxy` with any routes rejects migration and requires a fresh
  0.2.0 proxy plus route re-registration, while empty compatible proxy state may
  migrate; `bot-liquidity` 0.1.x rejects because no trusted admin can be derived;
  limit `grid-vault` 0.1.0 to 0.1.1 is supported; and initial fee-registry and
  fee-collector fixtures migrate with their state queryable. Incompatible state
  must never be migrated in place.
- **Status:** **closed in working tree**.

#### L-35 - Missing property/fuzz coverage for share/NAV and exit invariants

- **References:** existing limit asset-flow property test `921859b:limit-grid-system/contracts/grid-vault/tests/grid_vault_integration.rs:1619-1815`; fee tests near `:1940-2200`; market share tests `921859b:market-grid-system/contracts/grid-vault-swap/tests/grid_vault_swap_integration.rs:1017-1115`.
- **Impact:** Donation, rounding, fee mint/redemption, and emergency-exit interactions can violate conservation outside hand-picked examples.
- **Evidence/reasoning:** No identified property/fuzz suite jointly covers `sum(holder claims)`, NAV/share, donations, collector redemption, full exit, and zero/maximum boundaries across all venues.
- **Remediation:** Add model-based/property tests for conservation and no-profit round trips, including H-01/H-02 regressions.
- **Working-tree resolution:** Proptest 1.5.0 runs 128 cases over wide `Uint128`
  fee/NAV/share ranges, repeated mints under changing and fixed NAV,
  donation/withdraw no-profit behavior, two-token conservation, emergency
  owner-plus-collector exit, and pause/pending invariants.
- **Status:** **closed in working tree**.

#### L-36 - Shell and Python verification is shallow/path-dependent

- **References:** shell gate is only `bash -n` at `921859b:.github/workflows/ci.yml:59-63`; Python invocation mutates import context through discovery at `:52-58`; tests import local modules directly, e.g. `921859b:rebalancer-system/examples/keeper/test_keeper.py:1-20`.
- **Impact:** Shell defects beyond syntax and Python packaging/import-order failures may escape CI or behave differently when installed as services.
- **Evidence/reasoning:** No ShellCheck gate is run despite shellcheck annotations; tests rely on discovery paths rather than installed-package execution.
- **Remediation:** Add pinned ShellCheck and test Python from built/installed packages in isolated environments.
- **Working-tree resolution:** CI builds sdist and wheel artifacts, installs each
  operator package into an isolated environment, and runs its tests and entry
  points from the installed package. Shell validation remains pinned and
  path-independent package execution is now a required gate.
- **Status:** **closed in working tree**.

#### L-37 - Python has no coverage, lint, or type quality gate

- **References:** `921859b:.github/workflows/ci.yml:44-65`; Python project/config files under `grid-operator-system/services/grid-operator`.
- **Impact:** Untested branches and interface/type drift in transaction-critical operator code are not measured or blocked.
- **Evidence/reasoning:** CI runs `unittest` only; no coverage threshold, Ruff/Flake8, MyPy/Pyright, or package-build gate is required.
- **Remediation:** Add pinned lint/type checks and a justified branch/line coverage threshold focused on broadcast and recovery paths.
- **Working-tree resolution:** CI pins Ruff 0.12.10, MyPy 1.17.1, coverage
  7.10.4, build 1.3.0, and setuptools 80.9.0. A 70% branch-coverage baseline is
  enforced from isolated installed packages. The measured combined branch
  coverage is 72.84% for the rebalancer keeper and 76.08% for the grid operator;
  their suites contain 63 and 71 tests respectively.
- **Status:** **closed in working tree**.

#### L-38 - Documentation contained stale and superseded contradictions

- **References:** corrected working-tree files include `README.md`, `RELEASE.md`, `SECURITY_CHANGELOG.md`, `docs/FEE_TIER_PROTOCOL.md`, `docs/DEPLOY_FEE_SYSTEM.md`, `docs/TEST_RESULTS.md`, grid READMEs/operations, and rebalancer architecture/deployment/protocol docs; historical review is now labelled at `rebalancer-system/docs/AUDIT.md:1-30`.
- **Impact:** Operators could treat pre-production components as deployable, infer exact economic fee value, assume current proxy whitelisting, or overstate ladder/E2E evidence.
- **Evidence/reasoning:** Baseline docs mixed superseded per-LP designs, 180/1800 examples, exact-value claims, and historical local evidence with current readiness statements. The working tree corrects those claims but does not fix code risks.
- **Remediation:** Commit reviewed corrections with this report, add docs claim checks/release ownership, and keep historical evidence explicitly SHA/date scoped.
- **Status:** **closed/doc-corrected in working tree**; ongoing claim ownership
  and exact-SHA review remain governance requirements rather than this finding's
  stale-current-text defect.

## Design Risks And Assumptions

- CL8Y pair/factory query, order disappearance, parked-refund, maker-fee, TWAP, simulation, event, and whitelist semantics are trusted external behavior until independently verified against deployed code and configuration.
- CW20 tokens are assumed standard and non-malicious after address selection; direct donations remain possible and must be included in conservation models.
- Vault/admin, Wasm migration admin, fee governance, collector keeper, proxy admin, and signer infrastructure remain privileged trust domains; compile-time locks reduce but do not remove governance/deployment trust.
- Market-grid is now single-operator: deposits are admin-only and an accepted
  admin transfer migrates admin shares.
- Swap-proxy routing supports multiple vaults per pair. Market-grid/rebalancer
  now require factory registration and pinned runtime pair code; external pair
  behavior and the unexecuted `0.2.0` redeployment remain release risks.
- Registry outage no longer bypasses fees. Vaults intentionally retain the last
  successful effective tier by fee subject during registry unavailability, or
  charge 180 bps with no local history. Registry token-query failure never uses
  historical holding data and grants no discount.
- Confirmation depth is a deployment parameter, not a proof of finality. Reorg, indexer, LCD/RPC, and signer behavior must be monitored and rehearsed.
- Emergency and migration paths are part of the accounting security boundary and require the same invariant coverage as normal deposits/withdrawals.

## Documentation Corrections Made During This Audit

The uncommitted working tree now contains code, test, workflow, operator, and
documentation remediation. The immutable evidence paragraphs above remain
baseline findings; working-tree closure does not equal production evidence:

- Standardizes the canonical production base fee description and recent real-registry market tests from 1,800 to **180 bps**.
- Specifies NAV-priced fee shares in all venues as
  `F=floor(V*bps/10000)`, `x=floor(F*S/(A-F))`, with flooring preventing an
  immediate collector claim above `F`.
- Corrects fee subjects to limit `bot.owner`, market `config.admin`, and rebalancer `config.admin`; removes superseded per-LP/pool enumeration claims.
- Clarifies registry live/Lowest behavior, vault-local `vault_cached`/`lowest`
  outage charging, the explicit availability/revenue tradeoff, checked collector
  cumulative bookkeeping, and absence of an anti-dust threshold.
- Reclassifies market/rebalancer as pre-production, limit-grid as an abandoned
  production venue retained only as PoC, and states the explicit production block.
- Records compile-time environment-gated registry/collector/proxy values,
  manager propagation for future vaults, and
  four-workspace/default+mainnet release definitions.
- Corrects proxy claims: multiple vaults may share a pair, but production proxy
  deployment, pinning, provenance, and DEX whitelist remain prerequisites;
  limit-grid has no proxy.
- Corrects ladder evidence: market-grid has a real-registry full rebalance; limit and rebalancer ladder tests are direct registry queries; rebalancer charge coverage uses mocked rates.
- Separates historical local/E2E results from current-SHA evidence and marks the older rebalancer internal review as historical/superseded.
- Corrects deployment/operations text for required liquidity code ID, manager
  fee propagation, old-vault migration/redeployment, Exit collector backing,
  dedicated fee E2E, and unavailable production addresses.
- Records strong market/rebalancer factory and runtime-code provenance, the
  contract-specific `0.2.0` redeployment/migration boundary, and LocalTerra
  script wiring for factory plus queried pair code ID.
- Records exact-once late-fill reconciliation, typed audited intervention and
  state recovery, production-compatible signer models, frozen migration fixtures,
  property tests, and isolated Python quality/coverage gates.

## Required Remediation Sequence Before Production

1. Obtain approved real registry/collector/proxy addresses, deploy and
   independently verify the proxy/collector topology and proxy whitelist.
2. Keep limit-grid explicitly labelled and distributed as PoC-only; do not
   promote its artifacts to production without reopening M-15 and M-32.
3. Run complete exact-SHA release evidence: four-workspace default/mainnet
   reproducible builds and manifests, security/source gates, canonical fee E2E,
   and artifact inspection. Add package-version/tag consistency and narrow the
   RustSec exception.
4. Redeploy market-grid 0.1.x and bot-vault 0.1.x. Replace any routed 0.1.x swap-proxy
   with a fresh 0.2.0 instance and re-register routes; migrate only an empty
   compatible proxy. Do not migrate bot-liquidity 0.1.x. Rehearse each supported
   migration from its frozen fixture.
5. Add the still-missing real-registry rebalancer full-flow execution evidence
   on the exact candidate SHA.
6. Complete independent contract/operator/release audit, deployed CL8Y semantics
   verification, incident/rollback exercise, and limited-value canary planning.
7. Reconsider production only after partial High/Medium findings are closed or
   formally accepted with exact-SHA evidence and signed
   deployment records.

## Verification Performed

- Static inspection of repository tree and exact `921859b` source references.
- Static comparison of current uncommitted documentation/test corrections against the baseline.
- `make test` passed on the current working tree, including all four Rust
  workspaces and configured `mainnet` runs; rebalancer keeper passed 63 tests and
  grid operator passed 71 tests.
- `make clippy` passed on all four workspaces with `mainnet` and `-D warnings`.
- `bash -n`, `test-area/common-test.sh`, and workflow YAML parsing passed.
- Compilation with a missing `CL8Y_CANONICAL_FEE_REGISTRY` passed the expected
  negative check by failing.
- Isolated sdist/wheel install, Ruff 0.12.10, MyPy 1.17.1, and the 70% branch
  gate passed; measured combined branch coverage was 72.84% for rebalancer and
  76.08% for grid.
- These are working-tree results, not immutable-SHA CI evidence and not proof
  that the economic, release, operator, migration, or external-chain findings
  are closed.

## Verification Not Performed

- No complete Wasm optimizer/reproducibility, SBOM, provenance, or
  artifact-feature inspection was run as primary validation.
  Selected reproducible double builds reported separately are not treated as a
  complete release-set result here.
- No full LocalTerra canonical fee E2E, soak, funded migration,
  hardware/remote-signer, testnet, or mainnet validation was run.
- No independent audit, economic simulation, deployed CL8Y code/config verification, canonical address verification, proxy whitelist verification, GitHub settings review, commit, or push was performed.
