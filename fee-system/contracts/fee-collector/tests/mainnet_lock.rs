//! `mainnet` feature-only tests: proves the canonical `treasury` (CMM payout
//! target) is pinned at instantiate and cannot be re-pointed by governance.
#![cfg(feature = "mainnet")]

use assert_matches::assert_matches;
use cl8y_fee_collector::contract::CANONICAL_TREASURY;
use cl8y_fee_collector::msg::{ExecuteMsg, InstantiateMsg};
use cl8y_fee_collector::ContractError;
use cosmwasm_std::Addr;
use cw_multi_test::{App, ContractWrapper, Executor};

fn collector_contract() -> Box<dyn cw_multi_test::Contract<cosmwasm_std::Empty>> {
    let contract = ContractWrapper::new(
        cl8y_fee_collector::contract::execute,
        cl8y_fee_collector::contract::instantiate,
        cl8y_fee_collector::contract::query,
    )
    .with_migrate(cl8y_fee_collector::contract::migrate);
    Box::new(contract)
}

fn canonical_instantiate_msg(governance: &Addr, registry: &Addr, keeper: &Addr) -> InstantiateMsg {
    InstantiateMsg {
        governance: governance.to_string(),
        registry: registry.to_string(),
        keeper: keeper.to_string(),
        treasury: CANONICAL_TREASURY.to_string(),
    }
}

#[test]
fn instantiate_rejects_fake_treasury() {
    let mut app = App::default();
    let governance = app.api().addr_make("governance");
    let collector_id = app.store_code(collector_contract());

    let err: ContractError = app
        .instantiate_contract(
            collector_id,
            governance.clone(),
            &InstantiateMsg {
                governance: governance.to_string(),
                registry: app.api().addr_make("registry").to_string(),
                keeper: app.api().addr_make("keeper").to_string(),
                treasury: app.api().addr_make("fake-treasury").to_string(),
            },
            &[],
            "collector",
            None,
        )
        .unwrap_err()
        .downcast()
        .unwrap();
    assert_matches!(
        err,
        ContractError::NonCanonicalAddress {
            field: "treasury",
            ..
        }
    );
}

#[test]
fn instantiate_with_canonical_treasury_succeeds() {
    let mut app = App::default();
    let governance = app.api().addr_make("governance");
    let collector_id = app.store_code(collector_contract());
    app.instantiate_contract(
        collector_id,
        governance.clone(),
        &canonical_instantiate_msg(
            &governance,
            &app.api().addr_make("registry"),
            &app.api().addr_make("keeper"),
        ),
        &[],
        "collector",
        None,
    )
    .unwrap();
}

#[test]
fn update_config_refuses_to_repoint_treasury() {
    let mut app = App::default();
    let governance = app.api().addr_make("governance");
    let collector_id = app.store_code(collector_contract());
    let collector = app
        .instantiate_contract(
            collector_id,
            governance.clone(),
            &canonical_instantiate_msg(
                &governance,
                &app.api().addr_make("registry"),
                &app.api().addr_make("keeper"),
            ),
            &[],
            "collector",
            None,
        )
        .unwrap();

    let err: ContractError = app
        .execute_contract(
            governance.clone(),
            collector.clone(),
            &ExecuteMsg::UpdateConfig {
                governance: None,
                registry: None,
                keeper: None,
                treasury: Some(app.api().addr_make("fake-treasury").to_string()),
            },
            &[],
        )
        .unwrap_err()
        .downcast()
        .unwrap();
    assert_matches!(
        err,
        ContractError::NonCanonicalAddress {
            field: "treasury",
            ..
        }
    );
}

#[test]
fn update_config_still_allows_governance_and_keeper() {
    let mut app = App::default();
    let governance = app.api().addr_make("governance");
    let next_governance = app.api().addr_make("next-governance");
    let next_keeper = app.api().addr_make("next-keeper");
    let collector_id = app.store_code(collector_contract());
    let collector = app
        .instantiate_contract(
            collector_id,
            governance.clone(),
            &canonical_instantiate_msg(
                &governance,
                &app.api().addr_make("registry"),
                &app.api().addr_make("keeper"),
            ),
            &[],
            "collector",
            None,
        )
        .unwrap();

    app.execute_contract(
        governance,
        collector.clone(),
        &ExecuteMsg::UpdateConfig {
            governance: Some(next_governance.to_string()),
            registry: Some(app.api().addr_make("next-registry").to_string()),
            keeper: Some(next_keeper.to_string()),
            treasury: None,
        },
        &[],
    )
    .unwrap();

    let config: cl8y_fee_collector::msg::ConfigResponse = app
        .wrap()
        .query_wasm_smart(&collector, &cl8y_fee_collector::msg::QueryMsg::Config {})
        .unwrap();
    assert_eq!(config.governance, next_governance);
    assert_eq!(config.keeper, next_keeper);
    assert_eq!(config.treasury, CANONICAL_TREASURY);
}
