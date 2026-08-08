#[path = "fixtures/v0_1_0.rs"]
mod v0_1_0;

use cl8y_grid_vault_swap::contract::migrate;
use cl8y_grid_vault_swap::error::ContractError;
use cl8y_grid_vault_swap::msg::MigrateMsg;
use cosmwasm_std::testing::{mock_dependencies, mock_env};
use cosmwasm_std::{from_json, Order, Storage};

fn snapshot(storage: &dyn Storage) -> Vec<(Vec<u8>, Vec<u8>)> {
    storage.range(None, None, Order::Ascending).collect()
}

fn legacy_storage() -> cosmwasm_std::MemoryStorage {
    let _: v0_1_0::LegacyConfig = from_json(v0_1_0::CONFIG).unwrap();
    let mut storage = cosmwasm_std::MemoryStorage::new();
    storage.set(b"contract_info", v0_1_0::CONTRACT_INFO);
    storage.set(b"config", v0_1_0::CONFIG);
    storage
}

#[test]
fn frozen_0_1_0_provenance_schema_requires_redeployment_without_mutation() {
    let mut deps = mock_dependencies();
    deps.storage = legacy_storage();
    let before = snapshot(&deps.storage);

    assert_eq!(
        migrate(deps.as_mut(), mock_env(), MigrateMsg {}),
        Err(ContractError::LegacySchemaRequiresRedeploy)
    );
    assert_eq!(snapshot(&deps.storage), before);

    assert_eq!(
        migrate(deps.as_mut(), mock_env(), MigrateMsg {}),
        Err(ContractError::LegacySchemaRequiresRedeploy)
    );
    assert_eq!(snapshot(&deps.storage), before);
}

#[test]
fn wrong_name_and_non_older_versions_leave_legacy_state_unchanged() {
    for metadata in [
        br#"{"contract":"wrong","version":"0.1.0"}"#.as_slice(),
        br#"{"contract":"crates.io:cl8y-grid-vault-swap","version":"0.2.0"}"#.as_slice(),
        br#"{"contract":"crates.io:cl8y-grid-vault-swap","version":"not-semver"}"#.as_slice(),
    ] {
        let mut deps = mock_dependencies();
        deps.storage = legacy_storage();
        deps.storage.set(b"contract_info", metadata);
        let before = snapshot(&deps.storage);
        assert!(matches!(
            migrate(deps.as_mut(), mock_env(), MigrateMsg {}),
            Err(ContractError::InvalidMigration { .. })
        ));
        assert_eq!(snapshot(&deps.storage), before);
    }
}
