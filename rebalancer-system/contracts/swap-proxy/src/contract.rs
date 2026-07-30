use bot_types::{SwapProxyHookMsg, VaultConfigResponse, VaultQueryMsg};
use cl8y_dex::{AssetInfo, HybridSwapParams, PairCw20HookMsg, PairInfo, PairQueryMsg};
use cosmwasm_std::{
    entry_point, from_json, to_json_binary, Addr, Binary, Decimal, Deps, DepsMut, Env, MessageInfo,
    Response, StdResult, Uint128, WasmMsg,
};
use cw2::set_contract_version;
use cw20::{Cw20ExecuteMsg, Cw20ReceiveMsg};

use crate::error::ContractError;
use crate::msg::{ConfigResponse, ExecuteMsg, InstantiateMsg, QueryMsg, RouteResponse};
use crate::state::{Config, Route, CONFIG, PAIR_VAULTS, ROUTES};

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
            cl8y_token: deps.api.addr_validate(&msg.cl8y_token)?,
            fee_registry: deps.api.addr_validate(&msg.fee_registry)?,
        },
    )?;
    Ok(Response::new().add_attribute("action", "instantiate"))
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
        ExecuteMsg::WithdrawCl8y { amount, recipient } => {
            execute_withdraw_cl8y(deps, info, amount, recipient)
        }
        ExecuteMsg::TransferAdmin { admin } => execute_transfer_admin(deps, info, admin),
    }
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
    if PAIR_VAULTS.has(deps.storage, &pair) {
        return Err(ContractError::PairAlreadyRegistered);
    }
    let pair_info: PairInfo = deps
        .querier
        .query_wasm_smart(&pair, &PairQueryMsg::Pair {})?;
    if pair_info.contract_addr != pair {
        return Err(ContractError::InvalidRoute);
    }
    let [asset_0, asset_1] = pair_info.asset_infos;
    let asset_tokens = [
        token_addr(deps.as_ref(), asset_0)?,
        token_addr(deps.as_ref(), asset_1)?,
    ];
    if asset_tokens[0] == asset_tokens[1] {
        return Err(ContractError::InvalidRoute);
    }
    let vault_config: VaultConfigResponse = deps
        .querier
        .query_wasm_smart(&vault, &VaultQueryMsg::Config {})?;
    if vault_config.pair != pair
        || vault_config.proxy != env.contract.address
        || vault_config.asset_tokens != asset_tokens.clone().map(|addr| addr.to_string())
    {
        return Err(ContractError::InvalidRoute);
    }
    ROUTES.save(
        deps.storage,
        &vault,
        &Route {
            pair: pair.clone(),
            asset_tokens,
        },
    )?;
    PAIR_VAULTS.save(deps.storage, &pair, &vault)?;
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
    let route = ROUTES
        .may_load(deps.storage, &vault)?
        .ok_or(ContractError::UnregisteredVault)?;
    ROUTES.remove(deps.storage, &vault);
    PAIR_VAULTS.remove(deps.storage, &route.pair);
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

fn execute_withdraw_cl8y(
    deps: DepsMut,
    info: MessageInfo,
    amount: Uint128,
    recipient: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    assert_admin(&config, &info.sender)?;
    if amount.is_zero() {
        return Err(ContractError::ZeroAmount);
    }
    let recipient = deps.api.addr_validate(&recipient)?;
    Ok(Response::new()
        .add_message(WasmMsg::Execute {
            contract_addr: config.cl8y_token.to_string(),
            msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
                recipient: recipient.to_string(),
                amount,
            })?,
            funds: vec![],
        })
        .add_attribute("action", "withdraw_cl8y"))
}

fn execute_transfer_admin(
    deps: DepsMut,
    info: MessageInfo,
    admin: String,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;
    assert_admin(&config, &info.sender)?;
    config.admin = deps.api.addr_validate(&admin)?;
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new()
        .add_attribute("action", "transfer_admin")
        .add_attribute("admin", config.admin))
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => {
            let config = CONFIG.load(deps.storage)?;
            to_json_binary(&ConfigResponse {
                admin: config.admin.to_string(),
                cl8y_token: config.cl8y_token.to_string(),
                fee_registry: config.fee_registry.to_string(),
            })
        }
        QueryMsg::Route { vault } => {
            let vault = deps.api.addr_validate(&vault)?;
            let route = ROUTES.load(deps.storage, &vault)?;
            to_json_binary(&RouteResponse {
                vault: vault.to_string(),
                pair: route.pair.to_string(),
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
                cl8y_token: "cl8y".to_string(),
                fee_registry: "registry".to_string(),
            },
        )
        .unwrap();
        deps
    }

    #[test]
    fn non_admin_cannot_withdraw_cl8y() {
        let mut deps = setup();
        let error = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("attacker", &[]),
            ExecuteMsg::WithdrawCl8y {
                amount: Uint128::one(),
                recipient: "attacker".to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(error, ContractError::Unauthorized);
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
}
