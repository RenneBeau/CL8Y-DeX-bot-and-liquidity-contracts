use bot_types::{SwapProxyHookMsg, VaultQueryMsg};
use cl8y_dex::{
    AssetInfo, FactoryQueryMsg, HybridSwapParams, PairCw20HookMsg, PairInfo, PairQueryMsg,
    PairResponse,
};
use cosmwasm_std::{
    entry_point, from_json, to_json_binary, Addr, Binary, Decimal, Deps, DepsMut, Env, MessageInfo,
    Order, Response, StdError, StdResult, WasmMsg,
};
use cw2::{get_contract_version, set_contract_version};
use cw20::{Cw20ExecuteMsg, Cw20ReceiveMsg};
use semver::Version;

use crate::error::ContractError;
use crate::msg::{ConfigResponse, ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg, RouteResponse};
use crate::state::{Config, Route, CONFIG, PENDING_ADMIN, ROUTES};

const CONTRACT_NAME: &str = "crates.io:cl8y-swap-proxy";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_SPREAD: Decimal = Decimal::percent(10);

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    CONFIG.save(
        deps.storage,
        &Config {
            admin: deps.api.addr_validate(&msg.admin)?,
        },
    )?;
    Ok(Response::new().add_attribute("action", "instantiate"))
}

#[entry_point]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    let previous = get_contract_version(deps.storage)?;
    if previous.contract != CONTRACT_NAME {
        return Err(ContractError::Std(StdError::generic_err(
            "unsupported migration source",
        )));
    }
    let source = Version::parse(&previous.version)
        .map_err(|_| ContractError::Std(StdError::generic_err("unsupported migration source")))?;
    if source.major != 0 || source.minor != 1 {
        return Err(ContractError::Std(StdError::generic_err(
            "unsupported migration source",
        )));
    }
    CONFIG.load(deps.storage)?;
    if ROUTES
        .keys_raw(deps.storage, None, None, Order::Ascending)
        .next()
        .is_some()
    {
        return Err(ContractError::Std(StdError::generic_err(
            "legacy routes require a fresh proxy deployment and re-registration",
        )));
    }
    PENDING_ADMIN.remove(deps.storage);
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    Ok(Response::new()
        .add_attribute("action", "migrate_swap_proxy")
        .add_attribute("from_version", previous.version))
}

#[entry_point]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Receive(msg) => execute_receive(deps, env, info, msg),
        ExecuteMsg::RegisterVault { vault, pair } => {
            execute_register_vault(deps, env, info, vault, pair)
        }
        ExecuteMsg::RemoveVault { vault } => execute_remove_vault(deps, info, vault),
        ExecuteMsg::TransferAdmin { admin } => execute_transfer_admin(deps, info, admin),
        ExecuteMsg::AcceptAdmin {} => execute_accept_admin(deps, info),
        ExecuteMsg::CancelAdminTransfer {} => execute_cancel_admin_transfer(deps, info),
    }
}

/// Vault config is vault-specific: the rebalancer (`bot-vault`) returns a
/// `VaultConfigResponse`, but a market-grid (`grid-vault-swap`) returns its own
/// config with extra fields. `cw_serde` deny_unknown_fields is too strict for a
/// shared provider, so we deserialize only what `register_vault` needs and
/// ignore everything else.
#[derive(serde::Serialize, serde::Deserialize)]
struct VaultConfigRaw {
    factory: String,
    pair: String,
    pair_code_id: u64,
    proxy: Option<String>,
    asset_tokens: [String; 2],
}

fn execute_register_vault(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    vault: String,
    pair: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    assert_admin(&config, &info.sender)?;
    let vault = deps.api.addr_validate(&vault)?;
    let pair = deps.api.addr_validate(&pair)?;
    let pair_info: PairInfo = deps
        .querier
        .query_wasm_smart(&pair, &PairQueryMsg::Pair {})?;
    if pair_info.contract_addr != pair {
        return Err(ContractError::InvalidRoute);
    }
    let [asset_0, asset_1] = pair_info.asset_infos.clone();
    let asset_tokens = [
        token_addr(deps.as_ref(), asset_0)?,
        token_addr(deps.as_ref(), asset_1)?,
    ];
    if asset_tokens[0] == asset_tokens[1] {
        return Err(ContractError::InvalidRoute);
    }
    let vault_config_raw: VaultConfigRaw = deps
        .querier
        .query_wasm_smart(&vault, &VaultQueryMsg::Config {})?;
    let factory = deps.api.addr_validate(&vault_config_raw.factory)?;
    let registered: PairResponse = deps.querier.query_wasm_smart(
        factory,
        &FactoryQueryMsg::Pair {
            asset_infos: pair_info.asset_infos.clone(),
        },
    )?;
    if vault_config_raw.pair != pair
        || vault_config_raw.pair_code_id == 0
        || deps.querier.query_wasm_contract_info(&pair)?.code_id != vault_config_raw.pair_code_id
        || registered.pair.contract_addr != pair
        || vault_config_raw.proxy.as_deref() != Some(env.contract.address.as_str())
        || vault_config_raw.asset_tokens != asset_tokens.clone().map(|addr| addr.to_string())
    {
        return Err(ContractError::InvalidRoute);
    }
    ROUTES.save(
        deps.storage,
        &vault,
        &Route {
            pair: pair.clone(),
            pair_code_id: vault_config_raw.pair_code_id,
            asset_tokens,
        },
    )?;
    Ok(Response::new()
        .add_attribute("action", "register_vault")
        .add_attribute("vault", vault)
        .add_attribute("pair", pair))
}

fn execute_remove_vault(
    deps: DepsMut,
    info: MessageInfo,
    vault: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    assert_admin(&config, &info.sender)?;
    let vault = deps.api.addr_validate(&vault)?;
    if !ROUTES.has(deps.storage, &vault) {
        return Err(ContractError::UnregisteredVault);
    }
    ROUTES.remove(deps.storage, &vault);
    Ok(Response::new()
        .add_attribute("action", "remove_vault")
        .add_attribute("vault", vault))
}

fn execute_receive(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    receive: Cw20ReceiveMsg,
) -> Result<Response, ContractError> {
    if receive.amount.is_zero() {
        return Err(ContractError::ZeroAmount);
    }
    let vault = deps.api.addr_validate(&receive.sender)?;
    let route = ROUTES
        .may_load(deps.storage, &vault)?
        .ok_or(ContractError::UnregisteredVault)?;
    if deps.querier.query_wasm_contract_info(&route.pair)?.code_id != route.pair_code_id {
        return Err(ContractError::InvalidRoute);
    }
    if !route.asset_tokens.contains(&info.sender) {
        return Err(ContractError::UnsupportedToken);
    }
    let hook: SwapProxyHookMsg = from_json(receive.msg)?;
    let SwapProxyHookMsg::Swap {
        pair,
        min_return,
        max_spread,
        deadline,
    } = hook;
    if pair != route.pair {
        return Err(ContractError::InvalidRoute);
    }
    if deadline < env.block.time.seconds() {
        return Err(ContractError::Expired);
    }
    if max_spread > MAX_SPREAD {
        return Err(ContractError::ExcessiveSpread);
    }
    let pair_hook = PairCw20HookMsg::Swap {
        belief_price: None,
        max_spread: Some(max_spread),
        min_return: Some(min_return),
        to: Some(vault.to_string()),
        deadline: Some(deadline),
        trader: None,
        hybrid: Some(HybridSwapParams::pool_only(receive.amount)),
    };
    Ok(Response::new()
        .add_message(WasmMsg::Execute {
            contract_addr: info.sender.to_string(),
            msg: to_json_binary(&Cw20ExecuteMsg::Send {
                contract: route.pair.to_string(),
                amount: receive.amount,
                msg: to_json_binary(&pair_hook)?,
            })?,
            funds: vec![],
        })
        .add_attribute("action", "proxy_swap")
        .add_attribute("vault", vault)
        .add_attribute("pair", route.pair)
        .add_attribute("offer_token", info.sender)
        .add_attribute("amount", receive.amount))
}

fn execute_transfer_admin(
    deps: DepsMut,
    info: MessageInfo,
    admin: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    assert_admin(&config, &info.sender)?;
    let pending = deps.api.addr_validate(&admin)?;
    PENDING_ADMIN.save(deps.storage, &pending)?;
    Ok(Response::new()
        .add_attribute("action", "propose_admin")
        .add_attribute("current_admin", config.admin)
        .add_attribute("pending_admin", pending))
}

fn execute_accept_admin(deps: DepsMut, info: MessageInfo) -> Result<Response, ContractError> {
    if PENDING_ADMIN.may_load(deps.storage)?.as_ref() != Some(&info.sender) {
        return Err(ContractError::Unauthorized);
    }
    let previous = CONFIG.load(deps.storage)?.admin;
    CONFIG.update(deps.storage, |mut config| -> Result<_, ContractError> {
        config.admin = info.sender.clone();
        Ok(config)
    })?;
    PENDING_ADMIN.remove(deps.storage);
    Ok(Response::new()
        .add_attribute("action", "accept_admin")
        .add_attribute("previous_admin", previous)
        .add_attribute("admin", info.sender))
}

fn execute_cancel_admin_transfer(
    deps: DepsMut,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    assert_admin(&config, &info.sender)?;
    PENDING_ADMIN.remove(deps.storage);
    Ok(Response::new().add_attribute("action", "cancel_admin_transfer"))
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => {
            let config = CONFIG.load(deps.storage)?;
            to_json_binary(&ConfigResponse {
                admin: config.admin.to_string(),
                pending_admin: PENDING_ADMIN
                    .may_load(deps.storage)?
                    .map(|admin| admin.to_string()),
            })
        }
        QueryMsg::Route { vault } => {
            let vault = deps.api.addr_validate(&vault)?;
            let route = ROUTES.load(deps.storage, &vault)?;
            to_json_binary(&RouteResponse {
                vault: vault.to_string(),
                pair: route.pair.to_string(),
                pair_code_id: route.pair_code_id,
                asset_tokens: route.asset_tokens.map(|addr| addr.to_string()),
            })
        }
    }
}

fn token_addr(deps: Deps, info: AssetInfo) -> Result<Addr, ContractError> {
    match info {
        AssetInfo::Token { contract_addr } => Ok(deps.api.addr_validate(&contract_addr)?),
        AssetInfo::NativeToken { .. } => Err(ContractError::InvalidRoute),
    }
}

fn assert_admin(config: &Config, sender: &Addr) -> Result<(), ContractError> {
    if sender != config.admin {
        return Err(ContractError::Unauthorized);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};
    use cosmwasm_std::{ContractInfoResponse, ContractResult, SystemResult, Uint128, WasmQuery};
    use cw_storage_plus::Map;

    fn setup() -> cosmwasm_std::OwnedDeps<
        cosmwasm_std::MemoryStorage,
        cosmwasm_std::testing::MockApi,
        cosmwasm_std::testing::MockQuerier,
    > {
        let mut deps = mock_dependencies();
        instantiate(
            deps.as_mut(),
            mock_env(),
            mock_info("creator", &[]),
            InstantiateMsg {
                admin: "admin".to_string(),
            },
        )
        .unwrap();
        deps
    }

    fn install_registration_mocks(
        deps: &mut cosmwasm_std::OwnedDeps<
            cosmwasm_std::MemoryStorage,
            cosmwasm_std::testing::MockApi,
            cosmwasm_std::testing::MockQuerier,
        >,
    ) {
        deps.querier.update_wasm(|query| match query {
            WasmQuery::Smart { contract_addr, msg } if contract_addr == "pair" => {
                let _: PairQueryMsg = from_json(msg).unwrap();
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&PairInfo {
                        asset_infos: [
                            AssetInfo::Token {
                                contract_addr: "token0".to_string(),
                            },
                            AssetInfo::Token {
                                contract_addr: "token1".to_string(),
                            },
                        ],
                        contract_addr: Addr::unchecked("pair"),
                        liquidity_token: Addr::unchecked("pair-lp"),
                    })
                    .unwrap(),
                ))
            }
            WasmQuery::Smart { contract_addr, msg } if contract_addr == "factory" => {
                let _: FactoryQueryMsg = from_json(msg).unwrap();
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&PairResponse {
                        pair: PairInfo {
                            asset_infos: [
                                AssetInfo::Token {
                                    contract_addr: "token0".to_string(),
                                },
                                AssetInfo::Token {
                                    contract_addr: "token1".to_string(),
                                },
                            ],
                            contract_addr: Addr::unchecked("pair"),
                            liquidity_token: Addr::unchecked("pair-lp"),
                        },
                    })
                    .unwrap(),
                ))
            }
            WasmQuery::Smart { contract_addr, msg }
                if contract_addr == "vault1" || contract_addr == "vault2" =>
            {
                let _: VaultQueryMsg = from_json(msg).unwrap();
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&VaultConfigRaw {
                        factory: "factory".to_string(),
                        pair: "pair".to_string(),
                        pair_code_id: 7,
                        proxy: Some(mock_env().contract.address.to_string()),
                        asset_tokens: ["token0".to_string(), "token1".to_string()],
                    })
                    .unwrap(),
                ))
            }
            WasmQuery::ContractInfo { contract_addr } if contract_addr == "pair" => {
                let mut response = ContractInfoResponse::default();
                response.code_id = 7;
                SystemResult::Ok(ContractResult::Ok(to_json_binary(&response).unwrap()))
            }
            other => panic!("unexpected query {other:?}"),
        });
    }

    fn assert_routes(deps: DepsMut, vault: &str) {
        let response = execute_receive(
            deps,
            mock_env(),
            mock_info("token0", &[]),
            Cw20ReceiveMsg {
                sender: vault.to_string(),
                amount: Uint128::new(25),
                msg: to_json_binary(&SwapProxyHookMsg::Swap {
                    pair: "pair".to_string(),
                    min_return: Uint128::new(20),
                    max_spread: Decimal::percent(1),
                    deadline: u64::MAX,
                })
                .unwrap(),
            },
        )
        .unwrap();
        let cosmwasm_std::CosmosMsg::Wasm(WasmMsg::Execute { msg, .. }) = &response.messages[0].msg
        else {
            panic!("expected token send");
        };
        let Cw20ExecuteMsg::Send { contract, .. } = from_json(msg).unwrap() else {
            panic!("expected cw20 send");
        };
        assert_eq!(contract, "pair");
    }

    #[test]
    fn unregistered_sender_cannot_proxy_swap() {
        let mut deps = setup();
        let error = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("token", &[]),
            ExecuteMsg::Receive(Cw20ReceiveMsg {
                sender: "unknown-vault".to_string(),
                amount: Uint128::one(),
                msg: to_json_binary(&SwapProxyHookMsg::Swap {
                    pair: "pair".to_string(),
                    min_return: Uint128::one(),
                    max_spread: Decimal::percent(1),
                    deadline: u64::MAX,
                })
                .unwrap(),
            }),
        )
        .unwrap_err();
        assert_eq!(error, ContractError::UnregisteredVault);
    }

    #[test]
    fn admin_transfer_is_two_step_and_replaceable() {
        let mut deps = setup();
        execute_transfer_admin(deps.as_mut(), mock_info("admin", &[]), "wrong".into()).unwrap();
        execute_transfer_admin(deps.as_mut(), mock_info("admin", &[]), "next".into()).unwrap();
        assert_eq!(
            CONFIG.load(&deps.storage).unwrap().admin,
            Addr::unchecked("admin")
        );
        assert_eq!(
            execute_accept_admin(deps.as_mut(), mock_info("wrong", &[])).unwrap_err(),
            ContractError::Unauthorized
        );
        execute_accept_admin(deps.as_mut(), mock_info("next", &[])).unwrap();
        assert_eq!(
            CONFIG.load(&deps.storage).unwrap().admin,
            Addr::unchecked("next")
        );
    }

    #[test]
    fn migration_rejects_routes_without_mutating_state() {
        let mut deps = setup();
        let config = CONFIG.load(&deps.storage).unwrap();
        let legacy_pair_vaults: Map<&Addr, Addr> = Map::new("pair_vaults");
        legacy_pair_vaults
            .save(
                deps.as_mut().storage,
                &Addr::unchecked("pair"),
                &Addr::unchecked("legacy-vault"),
            )
            .unwrap();
        ROUTES
            .save(
                deps.as_mut().storage,
                &Addr::unchecked("vault"),
                &Route {
                    pair: Addr::unchecked("pair"),
                    pair_code_id: 7,
                    asset_tokens: [Addr::unchecked("token0"), Addr::unchecked("token1")],
                },
            )
            .unwrap();
        set_contract_version(deps.as_mut().storage, CONTRACT_NAME, "0.1.0").unwrap();
        let metadata = get_contract_version(&deps.storage).unwrap();
        assert!(migrate(deps.as_mut(), mock_env(), MigrateMsg {}).is_err());
        assert_eq!(CONFIG.load(&deps.storage).unwrap(), config);
        assert!(ROUTES.has(&deps.storage, &Addr::unchecked("vault")));
        assert_eq!(get_contract_version(&deps.storage).unwrap(), metadata);
    }

    #[test]
    fn multiple_vaults_for_one_pair_route_independently() {
        let mut deps = setup();
        install_registration_mocks(&mut deps);

        for vault in ["vault1", "vault2"] {
            execute_register_vault(
                deps.as_mut(),
                mock_env(),
                mock_info("admin", &[]),
                vault.to_string(),
                "pair".to_string(),
            )
            .unwrap();
        }
        assert_routes(deps.as_mut(), "vault1");
        assert_routes(deps.as_mut(), "vault2");

        execute_remove_vault(deps.as_mut(), mock_info("admin", &[]), "vault1".to_string()).unwrap();
        assert!(!ROUTES.has(&deps.storage, &Addr::unchecked("vault1")));
        assert_routes(deps.as_mut(), "vault2");
    }

    #[test]
    fn registered_route_rejects_pair_code_drift() {
        let mut deps = setup();
        install_registration_mocks(&mut deps);
        execute_register_vault(
            deps.as_mut(),
            mock_env(),
            mock_info("admin", &[]),
            "vault1".to_string(),
            "pair".to_string(),
        )
        .unwrap();
        deps.querier.update_wasm(|query| match query {
            WasmQuery::ContractInfo { .. } => {
                let mut response = ContractInfoResponse::default();
                response.code_id = 8;
                SystemResult::Ok(ContractResult::Ok(to_json_binary(&response).unwrap()))
            }
            _ => panic!("unexpected query"),
        });

        let error = execute_receive(
            deps.as_mut(),
            mock_env(),
            mock_info("token0", &[]),
            Cw20ReceiveMsg {
                sender: "vault1".to_string(),
                amount: Uint128::new(25),
                msg: to_json_binary(&SwapProxyHookMsg::Swap {
                    pair: "pair".to_string(),
                    min_return: Uint128::new(20),
                    max_spread: Decimal::percent(1),
                    deadline: u64::MAX,
                })
                .unwrap(),
            },
        )
        .unwrap_err();
        assert_eq!(error, ContractError::InvalidRoute);
    }

    #[test]
    fn legacy_pair_index_does_not_block_registration() {
        let mut deps = setup();
        let legacy_pair_vaults: Map<&Addr, Addr> = Map::new("pair_vaults");
        legacy_pair_vaults
            .save(
                deps.as_mut().storage,
                &Addr::unchecked("pair"),
                &Addr::unchecked("old-vault"),
            )
            .unwrap();
        set_contract_version(deps.as_mut().storage, CONTRACT_NAME, "0.1.9").unwrap();
        migrate(deps.as_mut(), mock_env(), MigrateMsg {}).unwrap();
        install_registration_mocks(&mut deps);

        execute_register_vault(
            deps.as_mut(),
            mock_env(),
            mock_info("admin", &[]),
            "vault1".to_string(),
            "pair".to_string(),
        )
        .unwrap();
        assert_routes(deps.as_mut(), "vault1");
    }
}
