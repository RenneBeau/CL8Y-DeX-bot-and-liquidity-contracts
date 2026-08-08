//! `mainnet` feature-only tests: proves the canonical `cl8y` + `treasury`
//! addresses are pinned at instantiate and cannot be re-pointed by governance.
#![cfg(feature = "mainnet")]

use assert_matches::assert_matches;
use cl8y_fee_registry::contract::{CANONICAL_CL8Y, CANONICAL_TREASURY};
use cl8y_fee_registry::msg::{ExecuteMsg, InstantiateMsg};
use cl8y_fee_registry::ContractError;
use cosmwasm_std::Addr;
use cw_multi_test::{App, ContractWrapper, Executor};

fn registry_contract() -> Box<dyn cw_multi_test::Contract<cosmwasm_std::Empty>> {
    let contract = ContractWrapper::new(
        cl8y_fee_registry::contract::execute,
        cl8y_fee_registry::contract::instantiate,
        cl8y_fee_registry::contract::query,
    )
    .with_migrate(cl8y_fee_registry::contract::migrate);
    Box::new(contract)
}

fn canonical_instantiate_msg(governance: &Addr, fee_collector: &Addr) -> InstantiateMsg {
    InstantiateMsg {
        governance: governance.to_string(),
        cl8y: CANONICAL_CL8Y.to_string(),
        treasury: CANONICAL_TREASURY.to_string(),
        fee_collector: fee_collector.to_string(),
        base_fee_bps: 180,
    }
}

#[test]
fn instantiate_rejects_fake_cl8y() {
    let mut app = App::default();
    let governance = app.api().addr_make("governance");
    let registry_id = app.store_code(registry_contract());

    let err: ContractError = app
        .instantiate_contract(
            registry_id,
            governance.clone(),
            &InstantiateMsg {
                cl8y: app.api().addr_make("fake-cl8y").to_string(),
                treasury: CANONICAL_TREASURY.to_string(),
                fee_collector: app.api().addr_make("collector").to_string(),
                governance: governance.to_string(),
                base_fee_bps: 180,
            },
            &[],
            "registry",
            None,
        )
        .unwrap_err()
        .downcast()
        .unwrap();
    assert_matches!(
        err,
        ContractError::NonCanonicalAddress { field: "cl8y", .. }
    );
}

#[test]
fn instantiate_rejects_fake_treasury() {
    let mut app = App::default();
    let governance = app.api().addr_make("governance");
    let registry_id = app.store_code(registry_contract());

    let err: ContractError = app
        .instantiate_contract(
            registry_id,
            governance.clone(),
            &InstantiateMsg {
                cl8y: CANONICAL_CL8Y.to_string(),
                treasury: app.api().addr_make("fake-treasury").to_string(),
                fee_collector: app.api().addr_make("collector").to_string(),
                governance: governance.to_string(),
                base_fee_bps: 180,
            },
            &[],
            "registry",
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
fn instantiate_with_canonical_addresses_succeeds() {
    let mut app = App::default();
    let governance = app.api().addr_make("governance");
    let registry_id = app.store_code(registry_contract());
    app.instantiate_contract(
        registry_id,
        governance.clone(),
        &canonical_instantiate_msg(&governance, &app.api().addr_make("collector")),
        &[],
        "registry",
        None,
    )
    .unwrap();
}

#[test]
fn update_config_refuses_to_repoint_pinned_fields() {
    let mut app = App::default();
    let governance = app.api().addr_make("governance");
    let registry_id = app.store_code(registry_contract());
    let registry = app
        .instantiate_contract(
            registry_id,
            governance.clone(),
            &canonical_instantiate_msg(&governance, &app.api().addr_make("collector")),
            &[],
            "registry",
            None,
        )
        .unwrap();

    let err: ContractError = app
        .execute_contract(
            governance.clone(),
            registry.clone(),
            &ExecuteMsg::UpdateConfig {
                governance: None,
                cl8y: Some(app.api().addr_make("fake-cl8y").to_string()),
                treasury: None,
                fee_collector: None,
                base_fee_bps: None,
            },
            &[],
        )
        .unwrap_err()
        .downcast()
        .unwrap();
    assert_matches!(
        err,
        ContractError::NonCanonicalAddress { field: "cl8y", .. }
    );

    let err: ContractError = app
        .execute_contract(
            governance.clone(),
            registry.clone(),
            &ExecuteMsg::UpdateConfig {
                governance: None,
                cl8y: None,
                treasury: Some(app.api().addr_make("fake-treasury").to_string()),
                fee_collector: None,
                base_fee_bps: None,
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
fn update_config_still_allows_governance_and_base_fee() {
    let mut app = App::default();
    let governance = app.api().addr_make("governance");
    let next_governance = app.api().addr_make("next-governance");
    let registry_id = app.store_code(registry_contract());
    let registry = app
        .instantiate_contract(
            registry_id,
            governance.clone(),
            &canonical_instantiate_msg(&governance, &app.api().addr_make("collector")),
            &[],
            "registry",
            None,
        )
        .unwrap();

    app.execute_contract(
        governance,
        registry.clone(),
        &ExecuteMsg::UpdateConfig {
            governance: Some(next_governance.to_string()),
            cl8y: None,
            treasury: None,
            fee_collector: Some(app.api().addr_make("new-collector").to_string()),
            base_fee_bps: Some(150),
        },
        &[],
    )
    .unwrap();

    let config: cl8y_fee_registry::msg::ConfigResponse = app
        .wrap()
        .query_wasm_smart(&registry, &cl8y_fee_registry::msg::QueryMsg::Config {})
        .unwrap();
    assert_eq!(config.governance, next_governance);
    assert_eq!(config.cl8y, CANONICAL_CL8Y);
    assert_eq!(config.treasury, CANONICAL_TREASURY);
    assert_eq!(config.base_fee_bps, 150);
}
