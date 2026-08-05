use assert_matches::assert_matches;
use cl8y_fee_registry::msg::{
    ConfigResponse, EffectiveFeeResponse, ExecuteMsg, InstantiateMsg, QueryMsg, TierSource,
};
use cl8y_fee_registry::ContractError;
use cosmwasm_std::{Addr, Uint128};
use cw20::{Cw20ExecuteMsg, MinterResponse};
use cw20_base::msg::InstantiateMsg as Cw20InstantiateMsg;
use cw_multi_test::{App, ContractWrapper, Executor};
use serde::de::DeserializeOwned;

const ONE_CL8Y: u128 = 1_000_000_000_000_000_000;

fn registry_contract() -> Box<dyn cw_multi_test::Contract<cosmwasm_std::Empty>> {
    let contract = ContractWrapper::new(
        cl8y_fee_registry::contract::execute,
        cl8y_fee_registry::contract::instantiate,
        cl8y_fee_registry::contract::query,
    )
    .with_migrate(cl8y_fee_registry::contract::migrate);
    Box::new(contract)
}

fn cl8y_token_contract() -> Box<dyn cw_multi_test::Contract<cosmwasm_std::Empty>> {
    let contract = ContractWrapper::new(
        cw20_base::contract::execute,
        cw20_base::contract::instantiate,
        cw20_base::contract::query,
    )
    .with_migrate(cw20_base::contract::migrate);
    Box::new(contract)
}

fn setup() -> (App, Addr, Addr) {
    let mut app = App::default();

    let governance = app.api().addr_make("governance");

    let cl8y_id = app.store_code(cl8y_token_contract());
    let cl8y = app
        .instantiate_contract(
            cl8y_id,
            Addr::unchecked("minter"),
            &Cw20InstantiateMsg {
                name: "CL8Y".to_string(),
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
                base_fee_bps: 180,
            },
            &[],
            "registry",
            None,
        )
        .unwrap();

    (app, registry, cl8y)
}

fn mint_cl8y(app: &mut App, cl8y: &Addr, to: &Addr, amount: u128) {
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

#[test]
fn instantiate_seeds_standard_ladder() {
    let (app, registry, _cl8y) = setup();
    let config: ConfigResponse = query_smart(&app, &registry, &QueryMsg::Config {});
    assert_eq!(config.base_fee_bps, 180);
    assert_eq!(config.ladder_version, 1);
    assert_eq!(config.treasury, app.api().addr_make("treasury").to_string());

    let tiers: Vec<cl8y_fee_registry::msg::TierEntry> =
        query_smart(&app, &registry, &QueryMsg::Tiers {});
    assert_eq!(tiers.len(), 11);
    let tier9 = tiers.iter().find(|t| t.tier_id == 9).unwrap();
    assert_eq!(tier9.min_cl8y_balance, Uint128::new(ONE_CL8Y * 7_500));
    assert_eq!(tier9.discount_bps, 9_500);
    assert!(!tier9.governance_only);
    let tier0 = tiers.iter().find(|t| t.tier_id == 0).unwrap();
    assert!(tier0.governance_only);
}

#[test]
fn effective_fee_reads_live_holding() {
    let (mut app, registry, cl8y) = setup();
    let trader = app.api().addr_make("trader");
    mint_cl8y(&mut app, &cl8y, &trader, 200 * ONE_CL8Y);

    app.execute_contract(
        Addr::unchecked("anyone"),
        registry.clone(),
        &ExecuteMsg::RefreshHolding {
            trader: trader.to_string(),
        },
        &[],
    )
    .unwrap();

    // Live read succeeds => fee must reflect the current balance (Live), not the
    // persisted snapshot.
    let fee: EffectiveFeeResponse = query_smart(
        &app,
        &registry,
        &QueryMsg::EffectiveFee {
            trader: trader.to_string(),
        },
    );
    assert_matches!(fee.source, TierSource::Live);
    assert_eq!(fee.tier_id, Some(5));
    assert_eq!(fee.discount_bps, 5_000);
    assert_eq!(fee.fee_bps, 90);
    assert_eq!(fee.holding, Some(Uint128::new(200 * ONE_CL8Y)));
}

#[test]
fn effective_fee_falls_back_to_saved_holding_when_live_read_fails() {
    let (mut app, registry, cl8y) = setup();
    let governance = app.api().addr_make("governance");
    let trader = app.api().addr_make("trader");
    mint_cl8y(&mut app, &cl8y, &trader, 200 * ONE_CL8Y);
    app.execute_contract(
        Addr::unchecked("anyone"),
        registry.clone(),
        &ExecuteMsg::RefreshHolding {
            trader: trader.to_string(),
        },
        &[],
    )
    .unwrap();

    // Point the registry at a dead token so the live read must fail.
    app.execute_contract(
        governance.clone(),
        registry.clone(),
        &ExecuteMsg::UpdateConfig {
            governance: None,
            cl8y: Some(app.api().addr_make("dead-token").to_string()),
            treasury: None,
            fee_collector: None,
            base_fee_bps: None,
        },
        &[],
    )
    .unwrap();

    let fee: EffectiveFeeResponse = query_smart(
        &app,
        &registry,
        &QueryMsg::EffectiveFee {
            trader: trader.to_string(),
        },
    );
    assert_matches!(fee.source, TierSource::Cached);
    assert_eq!(fee.tier_id, Some(5));
    assert_eq!(fee.fee_bps, 90);
    assert_eq!(fee.holding, Some(Uint128::new(200 * ONE_CL8Y)));

    // A trader with no saved holding and a dead live source is the lowest tier
    // (full base fee) -- never under-fee.
    let stranger = app.api().addr_make("stranger");
    let fee: EffectiveFeeResponse = query_smart(
        &app,
        &registry,
        &QueryMsg::EffectiveFee {
            trader: stranger.to_string(),
        },
    );
    assert_matches!(fee.source, TierSource::Lowest);
    assert_eq!(fee.fee_bps, 180);
}

#[test]
fn effective_fee_live_read_without_persisted_holding() {
    let (app, registry, _cl8y) = setup();
    // No RefreshHolding: the query falls back to a one-off live read.
    let trader = app.api().addr_make("trader");
    let fee: EffectiveFeeResponse = query_smart(
        &app,
        &registry,
        &QueryMsg::EffectiveFee {
            trader: trader.to_string(),
        },
    );
    // 0 balance => no eligible tier => full base fee, but still a live read.
    assert_matches!(fee.source, TierSource::Live);
    assert_eq!(fee.discount_bps, 0);
    assert_eq!(fee.fee_bps, 180);
    assert_eq!(fee.tier_id, None);
}

#[test]
fn governance_only_tiers_never_auto_applied() {
    let (mut app, registry, cl8y) = setup();
    // Tier 0 would give 100% discount; it must never auto-apply.
    let trader = app.api().addr_make("trader");
    mint_cl8y(&mut app, &cl8y, &trader, 0);
    app.execute_contract(
        Addr::unchecked("anyone"),
        registry.clone(),
        &ExecuteMsg::RefreshHolding {
            trader: trader.to_string(),
        },
        &[],
    )
    .unwrap();
    let fee: EffectiveFeeResponse = query_smart(
        &app,
        &registry,
        &QueryMsg::EffectiveFee {
            trader: trader.to_string(),
        },
    );
    assert_eq!(fee.discount_bps, 0);
    assert_eq!(fee.fee_bps, 180);
}

#[test]
fn tier_crud_is_governance_only_and_bumps_ladder() {
    let (mut app, registry, _cl8y) = setup();
    let governance = app.api().addr_make("governance");
    let attacker = app.api().addr_make("attacker");

    let err: ContractError = app
        .execute_contract(
            attacker.clone(),
            registry.clone(),
            &ExecuteMsg::AddTier {
                tier_id: 10,
                min_cl8y_balance: Uint128::new(100),
                discount_bps: 500,
                governance_only: false,
            },
            &[],
        )
        .unwrap_err()
        .downcast()
        .unwrap();
    assert_matches!(err, ContractError::Unauthorized);

    app.execute_contract(
        governance.clone(),
        registry.clone(),
        &ExecuteMsg::AddTier {
            tier_id: 10,
            min_cl8y_balance: Uint128::new(100),
            discount_bps: 500,
            governance_only: false,
        },
        &[],
    )
    .unwrap();

    let tiers: Vec<cl8y_fee_registry::msg::TierEntry> =
        query_smart(&app, &registry, &QueryMsg::Tiers {});
    assert_eq!(tiers.len(), 12);
    let config: ConfigResponse = query_smart(&app, &registry, &QueryMsg::Config {});
    assert_eq!(config.ladder_version, 2);

    app.execute_contract(
        governance.clone(),
        registry.clone(),
        &ExecuteMsg::UpdateTier {
            tier_id: 10,
            min_cl8y_balance: None,
            discount_bps: Some(700),
            governance_only: None,
        },
        &[],
    )
    .unwrap();
    let tier: cl8y_fee_registry::msg::TierEntry =
        query_smart(&app, &registry, &QueryMsg::Tier { tier_id: 10 });
    assert_eq!(tier.discount_bps, 700);

    app.execute_contract(
        governance,
        registry.clone(),
        &ExecuteMsg::RemoveTier { tier_id: 10 },
        &[],
    )
    .unwrap();
    let tiers: Vec<cl8y_fee_registry::msg::TierEntry> =
        query_smart(&app, &registry, &QueryMsg::Tiers {});
    assert_eq!(tiers.len(), 11);
}

#[test]
fn reserved_tier_ids_must_be_governance_only() {
    let (mut app, registry, _cl8y) = setup();
    let governance = app.api().addr_make("governance");
    let err: ContractError = app
        .execute_contract(
            governance,
            registry.clone(),
            &ExecuteMsg::AddTier {
                tier_id: 0,
                min_cl8y_balance: Uint128::zero(),
                discount_bps: 10_000,
                governance_only: false,
            },
            &[],
        )
        .unwrap_err()
        .downcast()
        .unwrap();
    assert_matches!(err, ContractError::ReservedTierId { tier_id: 0 });
}

#[test]
fn invalid_discount_rejected() {
    let (mut app, registry, _cl8y) = setup();
    let governance = app.api().addr_make("governance");
    let err: ContractError = app
        .execute_contract(
            governance,
            registry.clone(),
            &ExecuteMsg::AddTier {
                tier_id: 11,
                min_cl8y_balance: Uint128::new(1),
                discount_bps: 10_001,
                governance_only: false,
            },
            &[],
        )
        .unwrap_err()
        .downcast()
        .unwrap();
    assert_matches!(
        err,
        ContractError::InvalidDiscountBps { value: 10_001 }
    );
}

#[test]
fn update_config_is_governance_only() {
    let (mut app, registry, _cl8y) = setup();
    let governance = app.api().addr_make("governance");
    let attacker = app.api().addr_make("attacker");

    let err: ContractError = app
        .execute_contract(
            attacker,
            registry.clone(),
            &ExecuteMsg::UpdateConfig {
                governance: None,
                cl8y: None,
                treasury: None,
                fee_collector: None,
                base_fee_bps: Some(50),
            },
            &[],
        )
        .unwrap_err()
        .downcast()
        .unwrap();
    assert_matches!(err, ContractError::Unauthorized);

    app.execute_contract(
        governance,
        registry.clone(),
        &ExecuteMsg::UpdateConfig {
            governance: None,
            cl8y: None,
            treasury: None,
            fee_collector: None,
            base_fee_bps: Some(50),
        },
        &[],
    )
    .unwrap();
    let config: ConfigResponse = query_smart(&app, &registry, &QueryMsg::Config {});
    assert_eq!(config.base_fee_bps, 50);
}

#[test]
fn holding_query_returns_persisted_value() {
    let (mut app, registry, cl8y) = setup();
    let trader = app.api().addr_make("trader");
    mint_cl8y(&mut app, &cl8y, &trader, 7_500 * ONE_CL8Y);
    app.execute_contract(
        Addr::unchecked("anyone"),
        registry.clone(),
        &ExecuteMsg::RefreshHolding {
            trader: trader.to_string(),
        },
        &[],
    )
    .unwrap();

    let holding: cl8y_fee_registry::msg::HoldingResponse =
        query_smart(&app, &registry, &QueryMsg::Holding {
            trader: trader.to_string(),
        });
    assert_eq!(holding.holding, Some(Uint128::new(7_500 * ONE_CL8Y)));
    assert!(holding.at_height.is_some());
}
