#[path = "fixtures/v0_1_0.rs"]
mod v0_1_0;

use cl8y_bot_liquidity::contract::migrate;
use cl8y_bot_liquidity::msg::MigrateMsg;
use cl8y_bot_liquidity::state::{Config, CONFIG};
use cosmwasm_std::testing::{mock_dependencies, mock_env};
use cosmwasm_std::{from_json, Addr, Storage, Uint128};
use cw2::set_contract_version;

fn legacy_storage() -> cosmwasm_std::MemoryStorage {
    let _: v0_1_0::LegacyConfig = from_json(v0_1_0::CONFIG).unwrap();
    let mut storage = cosmwasm_std::MemoryStorage::new();
    storage.set(b"contract_info", v0_1_0::CONTRACT_INFO);
    storage.set(b"config", v0_1_0::CONFIG);
    storage
}

#[test]
fn frozen_0_1_0_schema_rejects_without_mutating_state() {
    let mut deps = mock_dependencies();
    deps.storage = legacy_storage();
    let metadata_before = deps.storage.get(b"contract_info").unwrap();
    assert!(migrate(deps.as_mut(), mock_env(), MigrateMsg {}).is_err());
    assert_eq!(deps.storage.get(b"config").unwrap(), v0_1_0::CONFIG);
    assert_eq!(deps.storage.get(b"contract_info").unwrap(), metadata_before);
}

#[test]
fn wrong_contract_is_rejected_without_touching_frozen_state() {
    let mut deps = mock_dependencies();
    deps.storage = legacy_storage();
    deps.storage.set(
        b"contract_info",
        br#"{"contract":"wrong","version":"0.1.0"}"#,
    );
    let config_before = deps.storage.get(b"config").unwrap();
    let metadata_before = deps.storage.get(b"contract_info").unwrap();
    assert!(migrate(deps.as_mut(), mock_env(), MigrateMsg {}).is_err());
    assert_eq!(deps.storage.get(b"config").unwrap(), config_before);
    assert_eq!(deps.storage.get(b"contract_info").unwrap(), metadata_before);

    deps.storage = legacy_storage();
    deps.storage.set(
        b"contract_info",
        br#"{"contract":"crates.io:cl8y-bot-liquidity","version":"99.0.0"}"#,
    );
    let metadata_before = deps.storage.get(b"contract_info").unwrap();
    assert!(migrate(deps.as_mut(), mock_env(), MigrateMsg {}).is_err());
    assert_eq!(deps.storage.get(b"contract_info").unwrap(), metadata_before);
    assert_eq!(deps.storage.get(b"config").unwrap(), v0_1_0::CONFIG);
}

#[test]
fn compatible_0_2_state_migrates_once() {
    let mut deps = mock_dependencies();
    let config = Config {
        admin: Addr::unchecked("admin"),
        vault: Addr::unchecked("vault"),
        asset_tokens: [Addr::unchecked("token0"), Addr::unchecked("token1")],
        minimum_initial_deposit: Uint128::new(2_000),
    };
    CONFIG.save(deps.as_mut().storage, &config).unwrap();
    set_contract_version(
        deps.as_mut().storage,
        "crates.io:cl8y-bot-liquidity",
        "0.2.0-rc.1",
    )
    .unwrap();

    migrate(deps.as_mut(), mock_env(), MigrateMsg {}).unwrap();
    assert_eq!(CONFIG.load(&deps.storage).unwrap(), config);
    let metadata = deps.storage.get(b"contract_info").unwrap();
    assert!(migrate(deps.as_mut(), mock_env(), MigrateMsg {}).is_err());
    assert_eq!(deps.storage.get(b"contract_info").unwrap(), metadata);
}
