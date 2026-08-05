use assert_matches::assert_matches;
use cl8y_fee_collector::msg::{
    ConfigResponse, ExecuteMsg, InstantiateMsg, QueryMsg, VaultSharesResponse,
};
use cl8y_fee_collector::ContractError;
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult,
    Uint128,
};
use cw2::set_contract_version;
use cw20::{Cw20ExecuteMsg, MinterResponse};
use cw20_base::msg::InstantiateMsg as Cw20InstantiateMsg;
use cw_multi_test::{App, ContractWrapper, Executor};
use cw_storage_plus::{Item, Map};

// ---------- Mock vault: a tiny contract the collector redeems LP from. -------

#[cw_serde]
struct VaultConfig {
    admin: String,
    token: String,
}

#[cw_serde]
enum VaultExecuteMsg {
    GiveShares { bot_id: u64, holder: String, amount: Uint128 },
    RedeemShares { bot_id: u64, recipient: String },
}

#[cw_serde]
enum VaultQueryMsg {
    Shares { bot_id: u64, address: String },
}

#[cw_serde]
struct VaultSharesRaw {
    shares: Uint128,
}

const V_CONFIG: Item<VaultConfig> = Item::new("vault_config");
const V_SHARES: Map<(&str, u64), Uint128> = Map::new("vault_shares");

#[entry_point]
fn vault_instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: VaultConfig,
) -> StdResult<Response> {
    set_contract_version(deps.storage, "test:vault", "0.1.0")?;
    V_CONFIG.save(deps.storage, &msg)?;
    Ok(Response::new())
}

#[entry_point]
fn vault_execute(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: VaultExecuteMsg,
) -> StdResult<Response> {
    let config = V_CONFIG.load(deps.storage)?;
    match msg {
        VaultExecuteMsg::GiveShares {
            bot_id,
            holder,
            amount,
        } => {
            if info.sender != Addr::unchecked(&config.admin) {
                return Err(cosmwasm_std::StdError::generic_err("unauthorized"));
            }
            let key = (holder.as_str(), bot_id);
            V_SHARES.update(
                deps.storage,
                key,
                |cur: Option<Uint128>| -> StdResult<Uint128> {
                    Ok(cur.unwrap_or_default() + amount)
                },
            )?;
            Ok(Response::new().add_attribute("action", "give_shares"))
        }
        VaultExecuteMsg::RedeemShares { bot_id, recipient } => {
            let key = (info.sender.as_str(), bot_id);
            let shares = V_SHARES
                .may_load(deps.storage, key)?
                .ok_or_else(|| cosmwasm_std::StdError::generic_err("no shares"))?;
            V_SHARES.remove(deps.storage, key);
            Ok(Response::new()
                .add_attribute("action", "redeem_shares")
                .add_message(cosmwasm_std::WasmMsg::Execute {
                    contract_addr: config.token,
                    msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
                        recipient,
                        amount: shares,
                    })?,
                    funds: vec![],
                }))
        }
    }
}

#[entry_point]
fn vault_query(deps: Deps, _env: Env, msg: VaultQueryMsg) -> StdResult<Binary> {
    match msg {
        VaultQueryMsg::Shares { bot_id, address } => {
            let shares = V_SHARES
                .may_load(deps.storage, (address.as_str(), bot_id))?
                .unwrap_or_default();
            to_json_binary(&VaultSharesRaw {
                shares,
            })
        }
    }
}

fn vault_contract() -> Box<dyn cw_multi_test::Contract<cosmwasm_std::Empty>> {
    Box::new(ContractWrapper::new(vault_execute, vault_instantiate, vault_query))
}

fn collector_contract() -> Box<dyn cw_multi_test::Contract<cosmwasm_std::Empty>> {
    let contract = ContractWrapper::new(
        cl8y_fee_collector::contract::execute,
        cl8y_fee_collector::contract::instantiate,
        cl8y_fee_collector::contract::query,
    )
    .with_migrate(cl8y_fee_collector::contract::migrate);
    Box::new(contract)
}

fn cw20_contract() -> Box<dyn cw_multi_test::Contract<cosmwasm_std::Empty>> {
    Box::new(ContractWrapper::new(
        cw20_base::contract::execute,
        cw20_base::contract::instantiate,
        cw20_base::contract::query,
    ))
}

fn setup() -> (App, Addr, Addr, Addr) {
    let mut app = App::default();
    let governance = app.api().addr_make("governance");
    let keeper = app.api().addr_make("keeper");
    let treasury = app.api().addr_make("treasury");
    let vault_admin = app.api().addr_make("vault_admin");

    let te_id = app.store_code(cw20_contract());
    let te = app
        .instantiate_contract(
            te_id,
            Addr::unchecked("minter"),
            &Cw20InstantiateMsg {
                name: "TestToken".to_string(),
                symbol: "TES".to_string(),
                decimals: 6,
                initial_balances: vec![],
                mint: Some(MinterResponse {
                    minter: "minter".to_string(),
                    cap: None,
                }),
                marketing: None,
            },
            &[],
            "te",
            None,
        )
        .unwrap();

    let vault_id = app.store_code(vault_contract());
    let vault = app
        .instantiate_contract(
            vault_id,
            vault_admin.clone(),
            &VaultConfig {
                admin: vault_admin.to_string(),
                token: te.to_string(),
            },
            &[],
            "vault",
            None,
        )
        .unwrap();

    let collector_id = app.store_code(collector_contract());
    let collector = app
        .instantiate_contract(
            collector_id,
            governance.clone(),
            &InstantiateMsg {
                governance: governance.to_string(),
                registry: app.api().addr_make("registry").to_string(),
                keeper: keeper.to_string(),
                treasury: treasury.to_string(),
            },
            &[],
            "collector",
            None,
        )
        .unwrap();

    (app, collector, vault, te)
}

use cosmwasm_std::Addr;

#[test]
fn only_keeper_can_collect() {
    let (mut app, collector, vault, _te) = setup();
    let attacker = app.api().addr_make("attacker");

    let err: ContractError = app
        .execute_contract(
            attacker,
            collector.clone(),
            &ExecuteMsg::Collect {
                vault: vault.to_string(),
                bot_id: 1,
            },
            &[],
        )
        .unwrap_err()
        .downcast()
        .unwrap();
    assert_matches!(err, ContractError::Unauthorized);
}

#[test]
fn collect_redeems_into_treasury_and_books_ledger() {
    let (mut app, collector, vault, te) = setup();
    let treasury = app.api().addr_make("treasury");
    let keeper = app.api().addr_make("keeper");

    // Fund the vault so redemption can pay out, then credit the collector LP.
    app.execute_contract(
        Addr::unchecked("minter"),
        te.clone(),
        &Cw20ExecuteMsg::Mint {
            recipient: vault.to_string(),
            amount: Uint128::new(1_000_000),
        },
        &[],
    )
    .unwrap();
    app.execute_contract(
        app.api().addr_make("vault_admin"),
        vault.clone(),
        &VaultExecuteMsg::GiveShares {
            bot_id: 1,
            holder: collector.to_string(),
            amount: Uint128::new(250_000),
        },
        &[],
    )
    .unwrap();

    app.execute_contract(
        keeper,
        collector.clone(),
        &ExecuteMsg::Collect {
            vault: vault.to_string(),
            bot_id: 1,
        },
        &[],
    )
    .unwrap();

    let balance: cw20::BalanceResponse =
        app.wrap()
            .query_wasm_smart(&te, &cw20::Cw20QueryMsg::Balance {
                address: treasury.to_string(),
            })
            .unwrap();
    assert_eq!(balance.balance, Uint128::new(250_000));

    let ledger: VaultSharesResponse = app
        .wrap()
        .query_wasm_smart(
            &collector,
            &QueryMsg::VaultShares {
                vault: vault.to_string(),
                bot_id: 1,
            },
        )
        .unwrap();
    assert_eq!(ledger.shares, Uint128::new(250_000));
}

#[test]
fn collect_with_no_entitlement_errors() {
    let (mut app, collector, vault, _te) = setup();
    let keeper = app.api().addr_make("keeper");

    let err: ContractError = app
        .execute_contract(
            keeper,
            collector.clone(),
            &ExecuteMsg::Collect {
                vault: vault.to_string(),
                bot_id: 99,
            },
            &[],
        )
        .unwrap_err()
        .downcast()
        .unwrap();
    assert_matches!(err, ContractError::NoEntitlement { .. });
}

#[test]
fn update_config_is_governance_only() {
    let (mut app, collector, _vault, _te) = setup();
    let governance = app.api().addr_make("governance");
    let attacker = app.api().addr_make("attacker");

    let err: ContractError = app
        .execute_contract(
            attacker,
            collector.clone(),
            &ExecuteMsg::UpdateConfig {
                governance: None,
                registry: None,
                keeper: None,
                treasury: None,
            },
            &[],
        )
        .unwrap_err()
        .downcast()
        .unwrap();
    assert_matches!(err, ContractError::Unauthorized);

    let new_treasury = app.api().addr_make("treasury2").to_string();
    app.execute_contract(
        governance,
        collector.clone(),
        &ExecuteMsg::UpdateConfig {
            governance: None,
            registry: None,
            keeper: None,
            treasury: Some(new_treasury.clone()),
        },
        &[],
    )
    .unwrap();

    let config: ConfigResponse = app
        .wrap()
        .query_wasm_smart(&collector, &QueryMsg::Config {})
        .unwrap();
    assert_eq!(config.treasury, new_treasury);
}
