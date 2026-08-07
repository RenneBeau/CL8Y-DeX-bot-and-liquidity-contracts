use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult,
    Uint128, WasmMsg,
};
use cw2::set_contract_version;

use crate::error::ContractError;
use crate::msg::{
    ConfigResponse, ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg, VaultSharesResponse,
};
use crate::state::{Config, CONFIG, VAULT_SHARES};

const CONTRACT_NAME: &str = "crates.io:cl8y-fee-collector";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The collector never reads vault state directly; it delegates the redemption
/// to the vault, which is the sole reader of its own shares and balances. The
/// messages below must match the vault integration (grid-vault).
#[cw_serde]
enum VaultQueryMsg {
    Shares { bot_id: u64, address: String },
    Config {},
}

#[cw_serde]
struct VaultSharesResponseRaw {
    shares: Uint128,
}

/// Vault `Config` is vault-specific (grid vs rebalancer) and carries many
/// fields we do not care about. `cw_serde` would `deny_unknown_fields`, so a
/// strict raw type cannot parse a real vault's config. We deserialize only the
/// field we need (`liquidity_contract`) and ignore everything else, matching
/// both the grid-vaults (no liquidity_contract -> internal Shares redemption)
/// and the rebalancer (liquidity_contract set -> external LP withdrawal).
#[derive(serde::Serialize, serde::Deserialize)]
struct VaultConfigResponseRaw {
    liquidity_contract: Option<String>,
}

#[cw_serde]
enum VaultExecuteMsg {
    RedeemShares { bot_id: u64, recipient: String },
}

/// External LP pool realization (rebalancer): the collector owns real
/// bot-liquidity LP; `Withdraw` burns it and returns the underlying tokens.
#[cw_serde]
enum LiquidityExecuteMsg {
    Withdraw {
        shares: Uint128,
        recipient: Option<String>,
        deadline: u64,
        output: WithdrawalTypeRaw,
    },
}

#[cw_serde]
enum WithdrawalTypeRaw {
    ProRata { min_assets: [Uint128; 2] },
}

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    let governance = deps.api.addr_validate(&msg.governance)?;
    let registry = deps.api.addr_validate(&msg.registry)?;
    let keeper = deps.api.addr_validate(&msg.keeper)?;
    let treasury = deps.api.addr_validate(&msg.treasury)?;
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    CONFIG.save(
        deps.storage,
        &Config {
            governance,
            registry,
            keeper,
            treasury,
        },
    )?;
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("creator", info.sender))
}

#[entry_point]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Collect { vault, bot_id } => execute_collect(deps, env, info, vault, bot_id),
        ExecuteMsg::UpdateConfig {
            governance,
            registry,
            keeper,
            treasury,
        } => execute_update_config(deps, info, governance, registry, keeper, treasury),
    }
}

fn assert_keeper(deps: &Deps, info: &MessageInfo) -> Result<(), ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.keeper {
        return Err(ContractError::Unauthorized);
    }
    Ok(())
}

fn execute_collect(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    vault: String,
    bot_id: u64,
) -> Result<Response, ContractError> {
    assert_keeper(&deps.as_ref(), &info)?;
    let config = CONFIG.load(deps.storage)?;
    let vault_addr = deps.api.addr_validate(&vault)?;

    // The collector redeems its OWN shares; the keeper only triggers the call.
    let shares: VaultSharesResponseRaw = deps.querier.query_wasm_smart(
        &vault_addr,
        &VaultQueryMsg::Shares {
            bot_id,
            address: env.contract.address.to_string(),
        },
    )?;
    if shares.shares.is_zero() {
        return Err(ContractError::NoEntitlement { vault, shares: 0 });
    }

    VAULT_SHARES.update(
        deps.storage,
        (&vault_addr, bot_id),
        |existing: Option<u128>| -> StdResult<u128> {
            Ok(existing.unwrap_or_default() + shares.shares.u128())
        },
    )?;

    let vault_addr_str = vault_addr.to_string();
    let config_resp: VaultConfigResponseRaw = deps
        .querier
        .query_wasm_smart(&vault_addr, &VaultQueryMsg::Config {})?;
    let mut response = Response::new()
        .add_attribute("action", "collect")
        .add_attribute("vault", vault)
        .add_attribute("bot_id", bot_id.to_string())
        .add_attribute("shares", shares.shares.to_string());
    if let Some(liquidity) = config_resp.liquidity_contract {
        // Rebalancer: the collector holds external bot-liquidity LP; withdraw it
        // pro-rata to the treasury (the collector is the LP owner).
        response = response.add_message(WasmMsg::Execute {
            contract_addr: liquidity,
            msg: to_json_binary(&LiquidityExecuteMsg::Withdraw {
                shares: shares.shares,
                recipient: Some(config.treasury.to_string()),
                deadline: env.block.time.seconds(),
                output: WithdrawalTypeRaw::ProRata {
                    min_assets: [Uint128::zero(), Uint128::zero()],
                },
            })?,
            funds: vec![],
        });
    } else {
        response = response.add_message(WasmMsg::Execute {
            contract_addr: vault_addr_str,
            msg: to_json_binary(&VaultExecuteMsg::RedeemShares {
                bot_id,
                recipient: config.treasury.to_string(),
            })?,
            funds: vec![],
        });
    }
    Ok(response)
}

fn execute_update_config(
    deps: DepsMut,
    info: MessageInfo,
    governance: Option<String>,
    registry: Option<String>,
    keeper: Option<String>,
    treasury: Option<String>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.governance {
        return Err(ContractError::Unauthorized);
    }
    let mut config = config;
    if let Some(address) = governance {
        config.governance = deps.api.addr_validate(&address)?;
    }
    if let Some(address) = registry {
        config.registry = deps.api.addr_validate(&address)?;
    }
    if let Some(address) = keeper {
        config.keeper = deps.api.addr_validate(&address)?;
    }
    if let Some(address) = treasury {
        config.treasury = deps.api.addr_validate(&address)?;
    }
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new().add_attribute("action", "update_config"))
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&query_config(deps)?),
        QueryMsg::VaultShares { vault, bot_id } => {
            to_json_binary(&query_vault_shares(deps, vault, bot_id)?)
        }
    }
}

fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config = CONFIG.load(deps.storage)?;
    Ok(ConfigResponse {
        governance: config.governance.to_string(),
        registry: config.registry.to_string(),
        keeper: config.keeper.to_string(),
        treasury: config.treasury.to_string(),
    })
}

fn query_vault_shares(deps: Deps, vault: String, bot_id: u64) -> StdResult<VaultSharesResponse> {
    let vault = deps.api.addr_validate(&vault)?;
    let shares = VAULT_SHARES.load(deps.storage, (&vault, bot_id));
    Ok(VaultSharesResponse {
        vault: vault.to_string(),
        bot_id,
        shares: shares.map(Uint128::from).unwrap_or_default(),
    })
}

#[entry_point]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    cw2::ensure_from_older_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    Ok(Response::new().add_attribute("action", "migrate"))
}
