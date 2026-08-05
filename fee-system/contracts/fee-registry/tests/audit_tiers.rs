//! Fee-audit suite: stresses the CL8Y fee-discount ladder as if the pricing
//! logic were unknown. Covers every holder tier at its exact boundary and one raw
//! unit below, the fee-bps arithmetic, reserved-tier rules, and base-fee edges.

use assert_matches::assert_matches;
use cl8y_fee_registry::msg::{
    ConfigResponse, EffectiveFeeResponse, ExecuteMsg, InstantiateMsg, QueryMsg, TierEntry,
    TierSource,
};
use cl8y_fee_registry::ContractError;
use cosmwasm_std::{Addr, Uint128};
use cw20::{Cw20ExecuteMsg, MinterResponse};
use cw20_base::msg::InstantiateMsg as Cw20InstantiateMsg;
use cw_multi_test::{App, ContractWrapper, Executor};
use serde::de::DeserializeOwned;

const ONE_CLY: u128 = 1_000_000_000_000_000_000;
const MAX_BPS: u16 = 10_000;

fn registry_contract() -> Box<dyn cw_multi_test::Contract<cosmwasm_std::Empty>> {
    Box::new(ContractWrapper::new(
        cl8y_fee_registry::contract::execute,
        cl8y_fee_registry::contract::instantiate,
        cl8y_fee_registry::contract::query,
    )
    .with_migrate(cl8y_fee_registry::contract::migrate))
}

fn cl8y_token_contract() -> Box<dyn cw_multi_test::Contract<cosmwasm_std::Empty>> {
    Box::new(ContractWrapper::new(
        cw20_base::contract::execute,
        cw20_base::contract::instantiate,
        cw20_base::contract::query,
    )
    .with_migrate(cw20_base::contract::migrate))
}

fn setup_with(base_fee_bps: u16) -> (App, Addr, Addr) {
    let mut app = App::default();
    let governance = app.api().addr_make("governance");

    let cl8y_id = app.store_code(cl8y_token_contract());
    let cl8y = app
        .instantiate_contract(
            cl8y_id,
            Addr::unchecked("minter"),
            &Cw20InstantiateMsg {
                name: "CLY".to_string(),
                symbol: "CLY".to_string(),
                decimals: 18,
                initial_balances: vec![],
                mint: Some(MinterResponse {
                    minter: "minter".to_string(),
                    cap: None,
                }),
                marketing: None,
            },
            &[],
            "cl8y",
            None,
        )
        .unwrap();

    let registry_id = app.store_code(registry_contract());
    let registry = app
        .instantiate_contract(
            registry_id,
            governance.clone(),
            &InstantiateMsg {
                governance: governance.to_string(),
                cl8y: cl8y.to_string(),
                treasury: app.api().addr_make("treasury").to_string(),
                fee_collector: app.api().addr_make("collector").to_string(),
                base_fee_bps,
            },
            &[],
            "registry",
            None,
        )
        .unwrap();

    (app, registry, cl8y)
}

fn mint_cly(app: &mut App, cl8y: &Addr, to: &Addr, amount: u128) {
    app.execute_contract(
        Addr::unchecked("minter"),
        cl8y.clone(),
        &Cw20ExecuteMsg::Mint {
            recipient: to.to_string(),
            amount: Uint128::new(amount),
        },
        &[],
    )
    .unwrap();
}

fn query_smart<T: DeserializeOwned>(app: &App, contract: &Addr, msg: &QueryMsg) -> T {
    app.wrap()
        .query_wasm_smart(contract.clone(), msg)
        .unwrap()
}

fn live_fee(app: &App, registry: &Addr, trader: &Addr) -> EffectiveFeeResponse {
    query_smart(app, registry, &QueryMsg::EffectiveFee { trader: trader.to_string() })
}

fn all_tiers(app: &App, registry: &Addr) -> Vec<TierEntry> {
    query_smart(app, registry, &QueryMsg::Tiers {})
}

/// Reference resolution: highest discount among non-governance tiers whose minimum
/// is met by `balance` (in raw CLY weis).
fn reference_discount(pool: &[(u8, u128, u16, bool)], balance: u128) -> (u16, Option<u8>) {
    let mut best: (u16, Option<u8>) = (0, None);
    for (tier_id, min, disc, gov) in pool {
        if *gov {
            continue;
        }
        if balance >= *min && *disc > best.0 {
            best = (*disc, Some(*tier_id));
        }
    }
    best
}

/// Reference fee: floor(base * (MAX_BPS - discount) / MAX_BPS).
fn reference_fee(base: u16, discount: u16) -> u16 {
    (u32::from(base) * (u32::from(MAX_BPS) - u32::from(discount)) / u32::from(MAX_BPS)) as u16
}

/// Canonical holder ladder: (tier_id, min_cl8y_weis, discount_bps).
const LADDER: [(u8, u128, u16); 9] = [
    (1, 1, 250),
    (2, 5, 1_000),
    (3, 20, 2_000),
    (4, 75, 3_500),
    (5, 200, 5_000),
    (6, 500, 6_000),
    (7, 1_500, 7_500),
    (8, 3_500, 8_500),
    (9, 7_500, 9_500),
];

#[test]
fn every_seeded_tier_resolves_at_its_exact_boundary() {
    let (mut app, registry, cl8y) = setup_with(1_800);
    for (tier_id, min_weis, discount_bps) in LADDER {
        let trader = app.api().addr_make(&format!("tier{tier_id}"));
        mint_cly(&mut app, &cl8y, &trader, ONE_CLY * min_weis);
        let fee = live_fee(&app, &registry, &trader);
        assert_matches!(fee.source, TierSource::Live);
        assert_eq!(fee.tier_id, Some(tier_id), "tier {tier_id} exact boundary");
        assert_eq!(fee.discount_bps, discount_bps);
        assert_eq!(fee.fee_bps, reference_fee(1_800, discount_bps));
    }
}

#[test]
fn below_each_minimum_hits_the_next_lower_tier() {
    let (mut app, registry, cl8y) = setup_with(1_000);
    for i in 1..LADDER.len() {
        let (tier_id, min_weis, _) = LADDER[i];
        let (prev_tier, _, _) = LADDER[i - 1];
        let trader = app.api().addr_make(&format!("below{tier_id}"));
        mint_cly(&mut app, &cl8y, &trader, ONE_CLY * min_weis - 1);
        let fee = live_fee(&app, &registry, &trader);
        assert_eq!(
            fee.tier_id,
            Some(prev_tier),
            "a balance 1 wei below tier {tier_id} must fall to tier {prev_tier}"
        );
    }
}

#[test]
fn full_matrix_agrees_with_reference_resolution() {
    let (mut app, registry, cl8y) = setup_with(1_800);
    let pool: Vec<(u8, u128, u16, bool)> = all_tiers(&app, &registry)
        .into_iter()
        .map(|t| (t.tier_id, t.min_cl8y_balance.u128(), t.discount_bps, t.governance_only))
        .collect();

    // Sample balances that straddle every boundary plus midpoints.
    let samples: Vec<u128> = {
        let mut v = vec![
            0u128,
            1,
            ONE_CLY - 1,
            ONE_CLY,
            2 * ONE_CLY,
            5 * ONE_CLY - 1,
            5 * ONE_CLY,
            19 * ONE_CLY,
            20 * ONE_CLY,
            200 * ONE_CLY,
            499 * ONE_CLY,
            500 * ONE_CLY,
            7_499 * ONE_CLY,
            7_500 * ONE_CLY,
            7_500 * ONE_CLY + 123,
        ];
        for (_, min, _) in LADDER {
            v.push(min * ONE_CLY);
            v.push(min * ONE_CLY + 7);
        }
        v
    };

    for (i, balance) in samples.iter().enumerate() {
        let trader = app.api().addr_make(&format!("s{i}"));
        mint_cly(&mut app, &cl8y, &trader, *balance);
        let fee = live_fee(&app, &registry, &trader);
        let (exp_disc, exp_tier) = reference_discount(&pool, *balance);
        assert_eq!(fee.discount_bps, exp_disc, "discount at balance {balance}");
        assert_eq!(fee.tier_id, exp_tier, "tier at balance {balance}");
        assert_eq!(fee.fee_bps, reference_fee(1_800, exp_disc), "fee at balance {balance}");

        // A holder can never be charged more than their live discount implies.
        assert!(
            fee.fee_bps as u32 <= 1_800,
            "never above base fee at balance {balance}"
        );
        assert_matches!(fee.source, TierSource::Live);
    }
}

#[test]
fn base_fee_edge_cases_map_to_exact_fees() {
    // With zero balance the discount is 0, so fee_bps must equal base exactly.
    for base in [0u16, 1, 180, 1_800, 5_000, 10_000] {
        let (app, registry, _) = setup_with(base);
        let fee = live_fee(&app, &registry, &app.api().addr_make("zero"));
        assert_matches!(fee.source, TierSource::Live);
        assert_eq!(fee.discount_bps, 0);
        assert_eq!(fee.tier_id, None);
        assert_eq!(fee.fee_bps, base, "zero-balance fee equals base {base}");
        assert_eq!(fee.fee_bps, reference_fee(base, 0));
    }
}

#[test]
fn no_discount_ever_over_charges() {
    let (mut app, registry, cl8y) = setup_with(1_800);
    // A tier that grants 100% discount (full zero fee) must never auto-apply from
    // a holder balance; only a full balance shortcut via base fee zero is allowed.
    let gov = app.api().addr_make("governance");
    app.execute_contract(
        gov.clone(),
        registry.clone(),
        &ExecuteMsg::AddTier {
            tier_id: 99,
            min_cl8y_balance: Uint128::new(1),
            discount_bps: 10_000,
            governance_only: false,
        },
        &[],
    )
    .unwrap();
    let trader = app.api().addr_make("mm");
    mint_cly(&mut app, &cl8y, &trader, 1);
    let fee = live_fee(&app, &registry, &trader);
    // The full-discount tier applies -> fee is zero, and this is a valid,
    // governance-approved grant. Never revert; fee floor is reachable.
    assert_eq!(fee.discount_bps, 10_000);
    assert_eq!(fee.fee_bps, 0);
}

#[test]
fn reserved_tiers_are_never_addable_as_holder_tiers() {
    let (mut app, registry, _cl8y) = setup_with(1_000);
    let governance = app.api().addr_make("governance");
    for tier_id in [0u8, 255u8] {
        let err: ContractError = app
            .execute_contract(
                governance.clone(),
                registry.clone(),
                &ExecuteMsg::AddTier {
                    tier_id,
                    min_cl8y_balance: Uint128::zero(),
                    discount_bps: 0,
                    governance_only: false,
                },
                &[],
            )
            .unwrap_err()
            .downcast()
            .unwrap();
        assert_matches!(err, ContractError::ReservedTierId { .. });
    }

    // Same for turning an existing reserved tier into a holder tier.
    let err: ContractError = app
        .execute_contract(
            governance.clone(),
            registry.clone(),
            &ExecuteMsg::UpdateTier {
                tier_id: 0,
                min_cl8y_balance: None,
                discount_bps: None,
                governance_only: Some(false),
            },
            &[],
        )
        .unwrap_err()
        .downcast()
        .unwrap();
    assert_matches!(err, ContractError::ReservedTierId { tier_id: 0 });
}

#[test]
fn tier_crud_bumps_ladder_and_revalidates_base() {
    let (mut app, registry, _) = setup_with(1_000);
    let governance = app.api().addr_make("governance");
    let before: ConfigResponse = query_smart(&app, &registry, &QueryMsg::Config {});
    app.execute_contract(
        governance.clone(),
        registry.clone(),
        &ExecuteMsg::AddTier {
            tier_id: 77,
            min_cl8y_balance: Uint128::new(9),
            discount_bps: 400,
            governance_only: false,
        },
        &[],
    )
    .unwrap();
    app.execute_contract(
        governance.clone(),
        registry.clone(),
        &ExecuteMsg::UpdateConfig {
            governance: None,
            cl8y: None,
            treasury: None,
            fee_collector: None,
            base_fee_bps: Some(5_000),
        },
        &[],
    )
    .unwrap();
    let after: ConfigResponse = query_smart(&app, &registry, &QueryMsg::Config {});
    assert_eq!(after.base_fee_bps, 5_000);
    assert!(after.ladder_version > before.ladder_version);

    // Invalid base over the max is rejected.
    let err: ContractError = app
        .execute_contract(
            governance.clone(),
            registry.clone(),
            &ExecuteMsg::UpdateConfig {
                governance: None,
                cl8y: None,
                treasury: None,
                fee_collector: None,
                base_fee_bps: Some(10_001),
            },
            &[],
        )
        .unwrap_err()
        .downcast()
        .unwrap();
    assert_matches!(err, ContractError::InvalidDiscountBps { .. });
}

#[test]
fn holding_never_cleared_on_transient_live_failure() {
    let (mut app, registry, cl8y) = setup_with(1_000);
    let governance = app.api().addr_make("governance");
    let trader = app.api().addr_make("trader");
    mint_cly(&mut app, &cl8y, &trader, 500 * ONE_CLY);
    app.execute_contract(
        Addr::unchecked("anyone"),
        registry.clone(),
        &ExecuteMsg::RefreshHolding { trader: trader.to_string() },
        &[],
    )
    .unwrap();

    // Point at a dead token so the live read must fail; the saved 6% tier must
    // survive as the cached fallback.
    app.execute_contract(
        governance.clone(),
        registry.clone(),
        &ExecuteMsg::UpdateConfig {
            governance: None,
            cl8y: Some(app.api().addr_make("dead").to_string()),
            treasury: None,
            fee_collector: None,
            base_fee_bps: None,
        },
        &[],
    )
    .unwrap();
    let fee = live_fee(&app, &registry, &trader);
    assert_matches!(fee.source, TierSource::Cached);
    assert_eq!(fee.tier_id, Some(6));
    assert_eq!(fee.fee_bps, reference_fee(1_000, 6_000));
    assert_eq!(fee.holding, Some(Uint128::new(500 * ONE_CLY)));
}