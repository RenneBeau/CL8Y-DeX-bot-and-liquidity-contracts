#[path = "fixtures/v0_1_0.rs"]
mod v0_1_0;

use cl8y_grid_vault::contract::{migrate, query};
use cl8y_grid_vault::error::ContractError;
use cl8y_grid_vault::msg::{ConfigResponse, MigrateMsg, QueryMsg};
use cosmwasm_std::testing::{mock_dependencies, mock_env};
use cosmwasm_std::{from_json, Order, Storage};

fn snapshot(storage: &dyn Storage) -> Vec<(Vec<u8>, Vec<u8>)> {
    storage.range(None, None, Order::Ascending).collect()
}

fn legacy_storage() -> cosmwasm_std::MemoryStorage {
    let _: v0_1_0::LegacyConfig = from_json(v0_1_0::CONFIG).unwrap();
    let _: v0_1_0::LegacyVaultMode = from_json(v0_1_0::VAULT_MODE).unwrap();
    let mut storage = cosmwasm_std::MemoryStorage::new();
    storage.set(b"contract_info", v0_1_0::CONTRACT_INFO);
    storage.set(b"config", v0_1_0::CONFIG);
    storage.set(b"vault_mode", v0_1_0::VAULT_MODE);
    storage
}

#[test]
fn frozen_0_1_0_migrates_and_config_remains_queryable() {
    let mut deps = mock_dependencies();
    deps.storage = legacy_storage();
    migrate(deps.as_mut(), mock_env(), MigrateMsg {}).unwrap();

    let config: ConfigResponse =
        from_json(query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap()).unwrap();
    assert_eq!(config.owner, "owner");
    assert_eq!(config.keeper_reward.u128(), 1_000);
    assert_eq!(deps.storage.get(b"config").unwrap(), v0_1_0::CONFIG);
}

#[test]
fn wrong_versions_name_and_repeat_are_rejected_without_mutation() {
    for metadata in [
        br#"{"contract":"wrong","version":"0.1.0"}"#.as_slice(),
        br#"{"contract":"crates.io:cl8y-grid-vault","version":"0.0.9"}"#.as_slice(),
        br#"{"contract":"crates.io:cl8y-grid-vault","version":"0.1.1"}"#.as_slice(),
        br#"{"contract":"crates.io:cl8y-grid-vault","version":"99.0.0"}"#.as_slice(),
    ] {
        let mut deps = mock_dependencies();
        deps.storage = legacy_storage();
        deps.storage.set(b"contract_info", metadata);
        let before = snapshot(&deps.storage);
        assert_eq!(
            migrate(deps.as_mut(), mock_env(), MigrateMsg {}),
            Err(ContractError::UnsupportedMigrationSource)
        );
        assert_eq!(snapshot(&deps.storage), before);
    }

    let mut deps = mock_dependencies();
    deps.storage = legacy_storage();
    migrate(deps.as_mut(), mock_env(), MigrateMsg {}).unwrap();
    let before = snapshot(&deps.storage);
    assert_eq!(
        migrate(deps.as_mut(), mock_env(), MigrateMsg {}),
        Err(ContractError::UnsupportedMigrationSource)
    );
    assert_eq!(snapshot(&deps.storage), before);
}
