#[path = "fixtures/v0_1_0.rs"]
mod v0_1_0;

use cl8y_fee_collector::contract::{migrate, query};
use cl8y_fee_collector::msg::{ConfigResponse, MigrateMsg, QueryMsg};
use cosmwasm_std::testing::{mock_dependencies, mock_env};
use cosmwasm_std::{from_json, Order, Storage};

fn snapshot(storage: &dyn Storage) -> Vec<(Vec<u8>, Vec<u8>)> {
    storage.range(None, None, Order::Ascending).collect()
}

fn storage_with(metadata: &[u8]) -> cosmwasm_std::MemoryStorage {
    let _: v0_1_0::LegacyConfig = from_json(v0_1_0::CONFIG).unwrap();
    let mut storage = cosmwasm_std::MemoryStorage::new();
    storage.set(b"contract_info", metadata);
    storage.set(b"config", v0_1_0::CONFIG);
    storage
}

#[test]
fn frozen_initial_release_is_queryable_and_repeat_migration_preserves_state() {
    let mut deps = mock_dependencies();
    deps.storage = storage_with(v0_1_0::CONTRACT_INFO);
    let config: ConfigResponse =
        from_json(query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap()).unwrap();
    assert_eq!(config.keeper, "keeper");
    let config_before = deps.storage.get(b"config").unwrap();
    migrate(deps.as_mut(), mock_env(), MigrateMsg {}).unwrap();
    assert_eq!(deps.storage.get(b"config").unwrap(), config_before);
    let normalized = snapshot(&deps.storage);
    migrate(deps.as_mut(), mock_env(), MigrateMsg {}).unwrap();
    assert_eq!(snapshot(&deps.storage), normalized);
}

#[test]
fn wrong_name_and_newer_version_are_rejected_without_mutation() {
    for metadata in [
        br#"{"contract":"wrong","version":"0.0.1"}"#.as_slice(),
        br#"{"contract":"crates.io:cl8y-fee-collector","version":"99.0.0"}"#.as_slice(),
    ] {
        let mut deps = mock_dependencies();
        deps.storage = storage_with(metadata);
        let before = snapshot(&deps.storage);
        assert!(migrate(deps.as_mut(), mock_env(), MigrateMsg {}).is_err());
        assert_eq!(snapshot(&deps.storage), before);
    }
}
