#[path = "fixtures/v0_1_0.rs"]
mod v0_1_0;

use cl8y_bot_vault::contract::migrate;
use cl8y_bot_vault::msg::MigrateMsg;
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
fn frozen_0_1_0_without_pair_provenance_is_rejected_and_repeat_safe() {
    let mut deps = mock_dependencies();
    deps.storage = legacy_storage();
    let before = snapshot(&deps.storage);

    assert!(migrate(
        deps.as_mut(),
        mock_env(),
        MigrateMsg {
            liquidity_code_id: 42,
        },
    )
    .is_err());
    assert_eq!(snapshot(&deps.storage), before);

    assert!(migrate(
        deps.as_mut(),
        mock_env(),
        MigrateMsg {
            liquidity_code_id: 42,
        },
    )
    .is_err());
    assert_eq!(snapshot(&deps.storage), before);
}

#[test]
fn wrong_name_and_invalid_request_leave_legacy_state_unchanged() {
    for (metadata, code_id) in [
        (br#"{"contract":"wrong","version":"0.1.0"}"#.as_slice(), 42),
        (
            br#"{"contract":"crates.io:cl8y-bot-vault","version":"0.2.0"}"#.as_slice(),
            42,
        ),
        (
            br#"{"contract":"crates.io:cl8y-bot-vault","version":"99.0.0"}"#.as_slice(),
            42,
        ),
        (v0_1_0::CONTRACT_INFO, 0),
    ] {
        let mut deps = mock_dependencies();
        deps.storage = legacy_storage();
        deps.storage.set(b"contract_info", metadata);
        let before = snapshot(&deps.storage);
        assert!(migrate(
            deps.as_mut(),
            mock_env(),
            MigrateMsg {
                liquidity_code_id: code_id,
            },
        )
        .is_err());
        assert_eq!(snapshot(&deps.storage), before);
    }
}
