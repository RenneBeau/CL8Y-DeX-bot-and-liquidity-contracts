#[path = "fixtures/v0_1_0.rs"]
mod v0_1_0;

use cl8y_swap_proxy::contract::migrate;
use cl8y_swap_proxy::msg::MigrateMsg;
use cosmwasm_std::testing::{mock_dependencies, mock_env};
use cosmwasm_std::{from_json, Storage};

fn legacy_storage() -> cosmwasm_std::MemoryStorage {
    let _: v0_1_0::LegacyConfig = from_json(v0_1_0::CONFIG).unwrap();
    let _: v0_1_0::LegacyRoute = from_json(v0_1_0::ROUTE).unwrap();
    let mut storage = cosmwasm_std::MemoryStorage::new();
    storage.set(b"contract_info", v0_1_0::CONTRACT_INFO);
    storage.set(b"config", v0_1_0::CONFIG);
    storage.set(v0_1_0::ROUTE_KEY, v0_1_0::ROUTE);
    storage
}

#[test]
fn frozen_0_1_0_route_rejects_without_mutating_state() {
    let mut deps = mock_dependencies();
    deps.storage = legacy_storage();
    let config_before = deps.storage.get(b"config").unwrap();
    let metadata_before = deps.storage.get(b"contract_info").unwrap();
    assert!(migrate(deps.as_mut(), mock_env(), MigrateMsg {}).is_err());
    assert_eq!(deps.storage.get(b"config").unwrap(), config_before);
    assert_eq!(deps.storage.get(b"contract_info").unwrap(), metadata_before);
    assert_eq!(deps.storage.get(v0_1_0::ROUTE_KEY).unwrap(), v0_1_0::ROUTE);
}

#[test]
fn empty_compatible_0_1_0_state_migrates_once() {
    let mut deps = mock_dependencies();
    deps.storage = legacy_storage();
    deps.storage.remove(v0_1_0::ROUTE_KEY);
    migrate(deps.as_mut(), mock_env(), MigrateMsg {}).unwrap();
    assert_eq!(deps.storage.get(b"config").unwrap(), v0_1_0::CONFIG);

    let metadata = deps.storage.get(b"contract_info").unwrap();
    assert!(migrate(deps.as_mut(), mock_env(), MigrateMsg {}).is_err());
    assert_eq!(deps.storage.get(b"contract_info").unwrap(), metadata);
}

#[test]
fn empty_0_1_0_state_with_incompatible_config_is_unchanged() {
    let mut deps = mock_dependencies();
    deps.storage = legacy_storage();
    deps.storage.remove(v0_1_0::ROUTE_KEY);
    deps.storage.set(b"config", br#"{"vault":"vault"}"#);
    let config_before = deps.storage.get(b"config").unwrap();
    let metadata_before = deps.storage.get(b"contract_info").unwrap();

    assert!(migrate(deps.as_mut(), mock_env(), MigrateMsg {}).is_err());
    assert_eq!(deps.storage.get(b"config").unwrap(), config_before);
    assert_eq!(deps.storage.get(b"contract_info").unwrap(), metadata_before);
}

#[test]
fn unsupported_sources_do_not_mutate_frozen_state() {
    let mut deps = mock_dependencies();
    deps.storage = legacy_storage();
    deps.storage.set(
        b"contract_info",
        br#"{"contract":"wrong","version":"0.1.0"}"#,
    );
    let before = deps.storage.get(b"contract_info").unwrap();
    assert!(migrate(deps.as_mut(), mock_env(), MigrateMsg {}).is_err());
    assert_eq!(deps.storage.get(b"contract_info").unwrap(), before);

    deps.storage = legacy_storage();
    deps.storage.set(
        b"contract_info",
        br#"{"contract":"crates.io:cl8y-swap-proxy","version":"99.0.0"}"#,
    );
    let metadata_before = deps.storage.get(b"contract_info").unwrap();
    assert!(migrate(deps.as_mut(), mock_env(), MigrateMsg {}).is_err());
    assert_eq!(deps.storage.get(b"contract_info").unwrap(), metadata_before);
    assert_eq!(deps.storage.get(v0_1_0::ROUTE_KEY).unwrap(), v0_1_0::ROUTE);
}
