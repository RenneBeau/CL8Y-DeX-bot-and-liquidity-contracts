use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Order, Reply, Response,
    StdResult, SubMsg, WasmMsg,
};
use cw2::set_contract_version;
use cw_utils::parse_reply_instantiate_data;

use crate::error::ContractError;
use crate::msg::{
    ConfigResponse, ExecuteMsg, InstantiateMsg, QueryMsg, VaultInstantiateMsg, VaultResponse,
};
use crate::state::{
    Config, PendingVault, Vault, CONFIG, NEXT_VAULT_ID, OWNER_VAULTS, PENDING_VAULTS, VAULTS,
};

const CONTRACT_NAME: &str = "crates.io:cl8y-grid-manager";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    if msg.vault_code_id == 0
        || msg.gas_denom.trim().is_empty()
        || msg.keeper_reward.is_zero()
        || msg.order_timeout_seconds == 0
        || msg.max_grid_count < 2
        || msg.max_orders_per_reconcile == 0
        || msg.max_active_orders_per_vault < msg.max_grid_count
    {
        return Err(ContractError::InvalidConfig);
    }
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    CONFIG.save(
        deps.storage,
        &Config {
            admin: deps.api.addr_validate(&msg.admin)?,
            pending_admin: None,
            keeper: deps.api.addr_validate(&msg.keeper)?,
            dex_factory: deps.api.addr_validate(&msg.dex_factory)?,
            vault_code_id: msg.vault_code_id,
            gas_denom: msg.gas_denom,
            keeper_reward: msg.keeper_reward,
            minimum_gas_reserve: msg.minimum_gas_reserve,
            order_timeout_seconds: msg.order_timeout_seconds,
            max_grid_count: msg.max_grid_count,
            max_orders_per_reconcile: msg.max_orders_per_reconcile,
            max_active_orders_per_vault: msg.max_active_orders_per_vault,
        },
    )?;
    NEXT_VAULT_ID.save(deps.storage, &1)?;
    Ok(Response::new().add_attribute("action", "instantiate_grid_manager"))
}

#[entry_point]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    match msg {
        ExecuteMsg::CreateVault { label } => execute_create_vault(deps, env, info, label),
        ExecuteMsg::UpdateConfig {
            keeper,
            vault_code_id,
        } => execute_update_config(deps, info, keeper, vault_code_id),
        ExecuteMsg::TransferAdmin { admin } => execute_transfer_admin(deps, info, admin),
        ExecuteMsg::AcceptAdmin {} => execute_accept_admin(deps, info),
    }
}

fn execute_create_vault(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    label: Option<String>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let vault_id = NEXT_VAULT_ID.load(deps.storage)?;
    NEXT_VAULT_ID.save(
        deps.storage,
        &vault_id
            .checked_add(1)
            .ok_or(ContractError::InvalidConfig)?,
    )?;
    PENDING_VAULTS.save(
        deps.storage,
        vault_id,
        &PendingVault {
            vault_id,
            owner: info.sender.clone(),
        },
    )?;
    let instantiate = VaultInstantiateMsg {
        admin: config.admin.to_string(),
        owner: info.sender.to_string(),
        keeper: config.keeper.to_string(),
        factory: config.dex_factory.to_string(),
        gas_denom: config.gas_denom,
        keeper_reward: config.keeper_reward,
        minimum_gas_reserve: config.minimum_gas_reserve,
        order_timeout_seconds: config.order_timeout_seconds,
        max_grid_count: config.max_grid_count,
        max_orders_per_reconcile: config.max_orders_per_reconcile,
        max_active_orders_per_bot: config.max_active_orders_per_vault,
    };
    Ok(Response::new()
        .add_submessage(SubMsg::reply_on_success(
            WasmMsg::Instantiate {
                admin: Some(info.sender.to_string()),
                code_id: config.vault_code_id,
                msg: to_json_binary(&instantiate)?,
                funds: vec![],
                label: label.unwrap_or_else(|| format!("grid-vault-{vault_id}")),
            },
            vault_id,
        ))
        .add_attribute("action", "create_grid_vault")
        .add_attribute("vault_id", vault_id.to_string())
        .add_attribute("owner", info.sender)
        .add_attribute("manager", env.contract.address))
}

fn execute_update_config(
    deps: DepsMut,
    info: MessageInfo,
    keeper: Option<String>,
    vault_code_id: Option<u64>,
) -> Result<Response, ContractError> {
    CONFIG.update(deps.storage, |mut config| -> Result<_, ContractError> {
        if info.sender != config.admin {
            return Err(ContractError::Unauthorized);
        }
        if let Some(keeper) = keeper {
            config.keeper = deps.api.addr_validate(&keeper)?;
        }
        if let Some(code_id) = vault_code_id {
            if code_id == 0 {
                return Err(ContractError::InvalidConfig);
            }
            config.vault_code_id = code_id;
        }
        Ok(config)
    })?;
    Ok(Response::new().add_attribute("action", "update_grid_manager"))
}

fn execute_transfer_admin(
    deps: DepsMut,
    info: MessageInfo,
    admin: String,
) -> Result<Response, ContractError> {
    let pending = deps.api.addr_validate(&admin)?;
    CONFIG.update(deps.storage, |mut config| -> Result<_, ContractError> {
        if info.sender != config.admin {
            return Err(ContractError::Unauthorized);
        }
        config.pending_admin = Some(pending.clone());
        Ok(config)
    })?;
    Ok(Response::new()
        .add_attribute("action", "transfer_grid_manager_admin")
        .add_attribute("pending_admin", pending))
}

fn execute_accept_admin(deps: DepsMut, info: MessageInfo) -> Result<Response, ContractError> {
    CONFIG.update(deps.storage, |mut config| -> Result<_, ContractError> {
        if config.pending_admin.as_ref() != Some(&info.sender) {
            return Err(ContractError::Unauthorized);
        }
        config.admin = info.sender;
        config.pending_admin = None;
        Ok(config)
    })?;
    Ok(Response::new().add_attribute("action", "accept_grid_manager_admin"))
}

#[entry_point]
pub fn reply(deps: DepsMut, _env: Env, reply: Reply) -> Result<Response, ContractError> {
    let pending = PENDING_VAULTS
        .may_load(deps.storage, reply.id)?
        .ok_or(ContractError::UnknownReply)?;
    let data = parse_reply_instantiate_data(reply)?;
    let address = deps.api.addr_validate(&data.contract_address)?;
    VAULTS.save(
        deps.storage,
        pending.vault_id,
        &Vault {
            owner: pending.owner.clone(),
            address: address.clone(),
        },
    )?;
    OWNER_VAULTS.save(deps.storage, (&pending.owner, pending.vault_id), &address)?;
    PENDING_VAULTS.remove(deps.storage, pending.vault_id);
    Ok(Response::new()
        .add_attribute("action", "register_grid_vault")
        .add_attribute("vault_id", pending.vault_id.to_string())
        .add_attribute("vault", address))
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => {
            let config = CONFIG.load(deps.storage)?;
            to_json_binary(&ConfigResponse {
                admin: config.admin.to_string(),
                pending_admin: config.pending_admin.map(|value| value.to_string()),
                keeper: config.keeper.to_string(),
                dex_factory: config.dex_factory.to_string(),
                vault_code_id: config.vault_code_id,
                gas_denom: config.gas_denom,
                keeper_reward: config.keeper_reward,
                minimum_gas_reserve: config.minimum_gas_reserve,
                order_timeout_seconds: config.order_timeout_seconds,
                max_grid_count: config.max_grid_count,
                max_orders_per_reconcile: config.max_orders_per_reconcile,
                max_active_orders_per_vault: config.max_active_orders_per_vault,
            })
        }
        QueryMsg::Vault { vault_id } => to_json_binary(
            &VAULTS
                .may_load(deps.storage, vault_id)?
                .map(|vault| vault_response(vault_id, vault)),
        ),
        QueryMsg::VaultsByOwner { owner } => {
            let owner = deps.api.addr_validate(&owner)?;
            let vaults = OWNER_VAULTS
                .prefix(&owner)
                .range(deps.storage, None, None, Order::Ascending)
                .map(|item| {
                    item.map(|(vault_id, address)| VaultResponse {
                        vault_id,
                        owner: owner.to_string(),
                        address: address.to_string(),
                    })
                })
                .collect::<StdResult<Vec<_>>>()?;
            to_json_binary(&vaults)
        }
    }
}

fn vault_response(vault_id: u64, vault: Vault) -> VaultResponse {
    VaultResponse {
        vault_id,
        owner: vault.owner.to_string(),
        address: vault.address.to_string(),
    }
}

fn assert_no_funds(info: &MessageInfo) -> Result<(), ContractError> {
    if info.funds.is_empty() {
        Ok(())
    } else {
        Err(ContractError::UnexpectedFunds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};
    use cosmwasm_std::{coin, from_json, CosmosMsg, Uint128};

    fn instantiate_default(deps: DepsMut) {
        instantiate(
            deps,
            mock_env(),
            mock_info("creator", &[]),
            InstantiateMsg {
                admin: "admin".into(),
                keeper: "keeper".into(),
                dex_factory: "dex_factory".into(),
                vault_code_id: 7,
                gas_denom: "uluna".into(),
                keeper_reward: Uint128::new(20),
                minimum_gas_reserve: Uint128::new(100),
                order_timeout_seconds: 86_400,
                max_grid_count: 20,
                max_orders_per_reconcile: 10,
                max_active_orders_per_vault: 40,
            },
        )
        .unwrap();
    }

    #[test]
    fn factory_creates_zero_fund_owner_bound_vault() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        let response = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("alice", &[]),
            ExecuteMsg::CreateVault { label: None },
        )
        .unwrap();

        let CosmosMsg::Wasm(WasmMsg::Instantiate {
            code_id,
            msg,
            funds,
            ..
        }) = &response.messages[0].msg
        else {
            panic!("expected vault instantiation")
        };
        let vault: VaultInstantiateMsg = from_json(msg).unwrap();
        assert_eq!(*code_id, 7);
        assert!(funds.is_empty());
        assert_eq!(vault.owner, "alice");
        assert_eq!(vault.order_timeout_seconds, 86_400);
        assert_eq!(
            PENDING_VAULTS.load(&deps.storage, 1).unwrap().owner,
            info_addr("alice")
        );
    }

    #[test]
    fn manager_rejects_user_funds() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        let error = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("alice", &[coin(1, "uluna")]),
            ExecuteMsg::CreateVault { label: None },
        )
        .unwrap_err();
        assert_eq!(error, ContractError::UnexpectedFunds);
    }

    fn info_addr(value: &str) -> cosmwasm_std::Addr {
        cosmwasm_std::Addr::unchecked(value)
    }
}
