use bot_types::{
    AuthorizedTransfer, LiquidityAuthorizationResponse, RebalanceStatusResponse, SwapParams,
    VaultBalancesResponse, VaultConfigResponse, VaultExecuteMsg, VaultPriceResponse, VaultQueryMsg,
    WithdrawalType,
};
use cosmwasm_std::{
    entry_point, to_json_binary, Addr, Binary, Decimal, Deps, DepsMut, Env, MessageInfo, Reply,
    Response, StdError, StdResult, SubMsg, Uint128, Uint256, WasmMsg,
};
use cw2::{get_contract_version, set_contract_version};
use cw20::{Cw20ExecuteMsg, Cw20QueryMsg, MinterResponse};
use cw20_base::msg::{ExecuteMsg as BaseExecuteMsg, InstantiateMsg as BaseInstantiateMsg};
use cw20_base::state::{BALANCES, TOKEN_INFO};
use semver::Version;

use crate::error::ContractError;
use crate::msg::{ConfigResponse, ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};
use crate::state::{Config, PendingOperation, CONFIG, PENDING, PENDING_ADMIN};

const CONTRACT_NAME: &str = "crates.io:cl8y-bot-liquidity";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEPOSIT_REPLY_ID: u64 = 1;
const WITHDRAW_REPLY_ID: u64 = 2;
const TRANSFER_REPLY_ID: u64 = 3;
const LOCKED_INITIAL_SHARES: Uint128 = Uint128::new(1_000);

#[entry_point]
pub fn instantiate(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    if msg.minimum_initial_deposit <= LOCKED_INITIAL_SHARES {
        return Err(ContractError::InvalidMinimumInitialDeposit);
    }
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    let admin = deps.api.addr_validate(&msg.admin)?;
    let vault = deps.api.addr_validate(&msg.vault)?;
    let vault_config: VaultConfigResponse = deps
        .querier
        .query_wasm_smart(&vault, &VaultQueryMsg::Config {})?;
    if vault_config.decimals != msg.decimals {
        return Err(ContractError::InvalidVault);
    }
    let asset_tokens = [
        deps.api.addr_validate(&vault_config.asset_tokens[0])?,
        deps.api.addr_validate(&vault_config.asset_tokens[1])?,
    ];
    cw20_base::contract::instantiate(
        deps.branch(),
        env.clone(),
        info,
        BaseInstantiateMsg {
            name: msg.name,
            symbol: msg.symbol,
            decimals: msg.decimals,
            initial_balances: vec![],
            mint: Some(MinterResponse {
                minter: env.contract.address.to_string(),
                cap: None,
            }),
            marketing: msg.marketing,
        },
    )
    .map_err(cw20_error)?;
    CONFIG.save(
        deps.storage,
        &Config {
            admin,
            vault,
            asset_tokens,
            minimum_initial_deposit: msg.minimum_initial_deposit,
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
    let target = Version::parse(CONTRACT_VERSION)
        .map_err(|_| ContractError::Std(StdError::generic_err("invalid contract version")))?;
    if source.major != 0 || source.minor != 2 || source >= target {
        return Err(ContractError::Std(StdError::generic_err(
            "unsupported migration source",
        )));
    }
    CONFIG.load(deps.storage)?;
    assert_no_pending(deps.as_ref())?;
    PENDING_ADMIN.remove(deps.storage);
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    Ok(Response::new()
        .add_attribute("action", "migrate_bot_liquidity")
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
        ExecuteMsg::Deposit {
            amounts,
            min_shares,
            deadline,
            swap,
        } => execute_deposit(deps, env, info, amounts, min_shares, deadline, swap),
        ExecuteMsg::Withdraw {
            shares,
            recipient,
            deadline,
            output,
        } => execute_withdraw(deps, env, info, shares, recipient, deadline, output),
        ExecuteMsg::UpdateConfig {
            minimum_initial_deposit,
        } => execute_update_config(deps, info, minimum_initial_deposit),
        ExecuteMsg::TransferAdmin { admin } => execute_transfer_admin(deps, info, admin),
        ExecuteMsg::AcceptAdmin {} => execute_accept_admin(deps, info),
        ExecuteMsg::CancelAdminTransfer {} => execute_cancel_admin_transfer(deps, info),
        ExecuteMsg::MintTo { recipient, amount } => {
            execute_mint_to(deps, env, info, recipient, amount)
        }
        other => execute_cw20(deps, env, info, other),
    }
}

fn execute_mint_to(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    recipient: String,
    amount: Uint128,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.vault {
        return Err(ContractError::Unauthorized);
    }
    if amount.is_zero() {
        return Err(ContractError::ZeroAmount);
    }
    deps.api.addr_validate(&recipient)?;
    mint_shares(deps, &env, recipient.clone(), amount)?;
    Ok(Response::new()
        .add_attribute("action", "liquidity_mint_to")
        .add_attribute("recipient", recipient)
        .add_attribute("amount", amount))
}

fn execute_deposit(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    amounts: [Uint128; 2],
    min_shares: Uint128,
    deadline: u64,
    swap: Option<SwapParams>,
) -> Result<Response, ContractError> {
    assert_no_pending(deps.as_ref())?;
    if amounts[0].is_zero() && amounts[1].is_zero() {
        return Err(ContractError::ZeroAmount);
    }
    if deadline < env.block.time.seconds() {
        return Err(ContractError::Expired);
    }
    let config = CONFIG.load(deps.storage)?;
    if let Some(params) = &swap {
        let offer = deps.api.addr_validate(&params.offer_token)?;
        let index = config
            .asset_tokens
            .iter()
            .position(|token| token == offer)
            .ok_or(ContractError::InvalidDepositSwap)?;
        if params.amount.is_zero() || params.amount > amounts[index] || params.deadline > deadline {
            return Err(ContractError::InvalidDepositSwap);
        }
    }
    let pre_balances = vault_balances(deps.as_ref(), &config)?;
    let price: VaultPriceResponse = deps
        .querier
        .query_wasm_smart(&config.vault, &VaultQueryMsg::Price {})?;
    let pre_supply = TOKEN_INFO.load(deps.storage)?.total_supply;
    PENDING.save(
        deps.storage,
        &PendingOperation::Deposit {
            depositor: info.sender.clone(),
            pre_balances,
            pre_supply,
            price: price.token1_per_token0,
            min_shares,
            swap: swap.clone(),
        },
    )?;
    let mut response = Response::new()
        .add_attribute("action", "deposit")
        .add_attribute("depositor", info.sender.clone());
    for (token, amount) in config.asset_tokens.iter().zip(amounts) {
        if !amount.is_zero() {
            response = response.add_message(WasmMsg::Execute {
                contract_addr: token.to_string(),
                msg: to_json_binary(&Cw20ExecuteMsg::TransferFrom {
                    owner: info.sender.to_string(),
                    recipient: config.vault.to_string(),
                    amount,
                })?,
                funds: vec![],
            });
        }
    }
    let finalize = match swap {
        Some(params) => VaultExecuteMsg::LiquiditySwap { params },
        None => VaultExecuteMsg::FinalizeLiquidityOperation {},
    };
    Ok(response.add_submessage(SubMsg::reply_on_success(
        WasmMsg::Execute {
            contract_addr: config.vault.to_string(),
            msg: to_json_binary(&finalize)?,
            funds: vec![],
        },
        DEPOSIT_REPLY_ID,
    )))
}

fn execute_withdraw(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    shares: Uint128,
    recipient: Option<String>,
    deadline: u64,
    output: WithdrawalType,
) -> Result<Response, ContractError> {
    assert_no_pending(deps.as_ref())?;
    if shares.is_zero() {
        return Err(ContractError::ZeroAmount);
    }
    if deadline < env.block.time.seconds() {
        return Err(ContractError::Expired);
    }
    let config = CONFIG.load(deps.storage)?;
    let owner_balance = BALANCES
        .may_load(deps.storage, &info.sender)?
        .unwrap_or_default();
    if shares > owner_balance {
        return Err(ContractError::InsufficientShares);
    }
    let supply = TOKEN_INFO.load(deps.storage)?.total_supply;
    let vault_balances = vault_balances(deps.as_ref(), &config)?;
    let claims = [
        checked_ratio(vault_balances[0], shares, supply)?,
        checked_ratio(vault_balances[1], shares, supply)?,
    ];
    let recipient = deps
        .api
        .addr_validate(recipient.as_deref().unwrap_or(info.sender.as_str()))?;
    match output {
        WithdrawalType::ProRata { min_assets } => {
            if claims[0] < min_assets[0] || claims[1] < min_assets[1] {
                return Err(ContractError::MinimumNotMet);
            }
            let transfers: Vec<AuthorizedTransfer> = config
                .asset_tokens
                .iter()
                .zip(claims)
                .filter(|(_, amount)| !amount.is_zero())
                .map(|(token, amount)| AuthorizedTransfer {
                    token: token.to_string(),
                    amount,
                    recipient: recipient.to_string(),
                })
                .collect();
            if transfers.is_empty() {
                return Err(ContractError::ZeroAmount);
            }
            PENDING.save(
                deps.storage,
                &PendingOperation::AuthorizedTransfers {
                    replies_remaining: transfers.len() as u8,
                    transfers: transfers.clone(),
                },
            )?;
            burn_shares(deps.branch(), env.clone(), info.clone(), shares)?;
            let mut response = Response::new().add_attribute("action", "withdraw_pro_rata");
            for transfer in transfers {
                response = response.add_submessage(SubMsg::reply_on_success(
                    vault_transfer(
                        &config,
                        &Addr::unchecked(transfer.token),
                        transfer.amount,
                        &recipient,
                    )?,
                    TRANSFER_REPLY_ID,
                ));
            }
            Ok(response)
        }
        WithdrawalType::Token0 { min_amount, swap } => execute_single_withdraw(
            deps, info, config, recipient, shares, claims, 0, min_amount, swap, deadline,
        ),
        WithdrawalType::Token1 { min_amount, swap } => execute_single_withdraw(
            deps, info, config, recipient, shares, claims, 1, min_amount, swap, deadline,
        ),
    }
}

fn execute_update_config(
    deps: DepsMut,
    info: MessageInfo,
    minimum_initial_deposit: Option<Uint128>,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;
    assert_admin(&config, &info.sender)?;
    if let Some(value) = minimum_initial_deposit {
        if value <= LOCKED_INITIAL_SHARES {
            return Err(ContractError::InvalidMinimumInitialDeposit);
        }
        if !TOKEN_INFO.load(deps.storage)?.total_supply.is_zero() {
            return Err(ContractError::BootstrapComplete);
        }
        config.minimum_initial_deposit = value;
    }
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new().add_attribute("action", "update_config"))
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

#[allow(clippy::too_many_arguments)]
fn execute_single_withdraw(
    deps: DepsMut,
    info: MessageInfo,
    config: Config,
    recipient: Addr,
    shares: Uint128,
    claims: [Uint128; 2],
    payout_index: usize,
    min_amount: Uint128,
    swap: SwapParams,
    deadline: u64,
) -> Result<Response, ContractError> {
    let unwanted_index = 1 - payout_index;
    if swap.offer_token != config.asset_tokens[unwanted_index]
        || swap.amount != claims[unwanted_index]
        || swap.deadline > deadline
        || swap.amount.is_zero()
    {
        return Err(ContractError::InvalidWithdrawalSwap);
    }
    let pre_payout_balance = vault_balances(deps.as_ref(), &config)?[payout_index];
    PENDING.save(
        deps.storage,
        &PendingOperation::WithdrawSingle {
            owner: info.sender.clone(),
            shares,
            recipient,
            payout_token: config.asset_tokens[payout_index].clone(),
            base_amount: claims[payout_index],
            pre_payout_balance,
            min_amount,
            swap: swap.clone(),
        },
    )?;
    Ok(Response::new()
        .add_submessage(SubMsg::reply_always(
            WasmMsg::Execute {
                contract_addr: config.vault.to_string(),
                msg: to_json_binary(&VaultExecuteMsg::LiquiditySwap { params: swap })?,
                funds: vec![],
            },
            WITHDRAW_REPLY_ID,
        ))
        .add_attribute("action", "withdraw_single"))
}

#[entry_point]
pub fn reply(deps: DepsMut, env: Env, reply: Reply) -> Result<Response, ContractError> {
    match reply.id {
        DEPOSIT_REPLY_ID => complete_deposit(deps, env),
        WITHDRAW_REPLY_ID => complete_single_withdraw(deps, env, reply),
        TRANSFER_REPLY_ID => complete_authorized_transfer(deps),
        _ => Err(ContractError::UnknownReply),
    }
}

fn complete_deposit(mut deps: DepsMut, env: Env) -> Result<Response, ContractError> {
    let pending = PENDING.load(deps.storage)?;
    let PendingOperation::Deposit {
        depositor,
        pre_balances,
        pre_supply,
        price,
        min_shares,
        swap: _,
    } = pending
    else {
        return Err(ContractError::UnknownReply);
    };
    let config = CONFIG.load(deps.storage)?;
    let post_balances = vault_balances(deps.as_ref(), &config)?;
    if post_balances[0] < pre_balances[0] || post_balances[1] < pre_balances[1] {
        return Err(ContractError::InvalidDepositSettlement);
    }
    let status: RebalanceStatusResponse = deps
        .querier
        .query_wasm_smart(&config.vault, &VaultQueryMsg::RebalanceStatus {})?;
    let vault_config: VaultConfigResponse = deps
        .querier
        .query_wasm_smart(&config.vault, &VaultQueryMsg::Config {})?;
    if status.allocation_deviation_bps > vault_config.allocation_tolerance_bps {
        return Err(ContractError::AllocationOutsideTolerance);
    }
    let pre_nav = nav(pre_balances, price)?;
    let post_nav = nav(post_balances, price)?;
    let deposit_value = post_nav
        .checked_sub(pre_nav)
        .map_err(|_| ContractError::InvalidDepositSettlement)?;
    let user_shares = if pre_supply.is_zero() {
        if deposit_value < config.minimum_initial_deposit || deposit_value <= LOCKED_INITIAL_SHARES
        {
            return Err(ContractError::InitialDepositTooSmall);
        }
        let locked_shares = pre_nav
            .checked_add(LOCKED_INITIAL_SHARES)
            .map_err(StdError::overflow)?;
        mint_shares(
            deps.branch(),
            &env,
            env.contract.address.to_string(),
            locked_shares,
        )?;
        deposit_value - LOCKED_INITIAL_SHARES
    } else {
        proportional_deposit_shares(pre_balances, post_balances, pre_supply)?
    };
    if user_shares.is_zero() {
        return Err(ContractError::ZeroShares);
    }
    if user_shares < min_shares {
        return Err(ContractError::MinimumNotMet);
    }
    mint_shares(deps.branch(), &env, depositor.to_string(), user_shares)?;
    PENDING.remove(deps.storage);
    Ok(Response::new()
        .add_attribute("action", "complete_deposit")
        .add_attribute("depositor", depositor)
        .add_attribute("deposit_value", deposit_value)
        .add_attribute("shares", user_shares))
}

fn complete_single_withdraw(
    mut deps: DepsMut,
    env: Env,
    reply: Reply,
) -> Result<Response, ContractError> {
    let pending = PENDING.load(deps.storage)?;
    let PendingOperation::WithdrawSingle {
        owner,
        shares,
        recipient,
        payout_token,
        base_amount,
        pre_payout_balance,
        min_amount,
        swap: _,
    } = pending
    else {
        return Err(ContractError::UnknownReply);
    };
    if reply.result.is_err() {
        PENDING.remove(deps.storage);
        return Ok(Response::new()
            .add_attribute("action", "withdraw_single_failed")
            .add_attribute("owner", owner));
    }
    let config = CONFIG.load(deps.storage)?;
    let current = cw20_balance(deps.as_ref(), &payout_token, &config.vault)?;
    let received = current
        .checked_sub(pre_payout_balance)
        .map_err(|_| ContractError::InvalidDepositSettlement)?;
    let payout = base_amount
        .checked_add(received)
        .map_err(StdError::overflow)?;
    if payout < min_amount {
        return Err(ContractError::MinimumNotMet);
    }
    burn_shares(
        deps.branch(),
        env,
        MessageInfo {
            sender: owner,
            funds: vec![],
        },
        shares,
    )?;
    let transfer = AuthorizedTransfer {
        token: payout_token.to_string(),
        amount: payout,
        recipient: recipient.to_string(),
    };
    PENDING.save(
        deps.storage,
        &PendingOperation::AuthorizedTransfers {
            transfers: vec![transfer],
            replies_remaining: 1,
        },
    )?;
    Ok(Response::new()
        .add_submessage(SubMsg::reply_on_success(
            vault_transfer(&config, &payout_token, payout, &recipient)?,
            TRANSFER_REPLY_ID,
        ))
        .add_attribute("action", "complete_single_withdraw")
        .add_attribute("recipient", recipient)
        .add_attribute("amount", payout))
}

fn complete_authorized_transfer(deps: DepsMut) -> Result<Response, ContractError> {
    let PendingOperation::AuthorizedTransfers {
        transfers: _,
        replies_remaining,
    } = PENDING.load(deps.storage)?
    else {
        return Err(ContractError::UnknownReply);
    };
    let remaining = replies_remaining
        .checked_sub(1)
        .ok_or(ContractError::UnknownReply)?;
    if remaining == 0 {
        PENDING.remove(deps.storage);
    } else {
        PENDING.update(deps.storage, |pending| -> Result<_, ContractError> {
            let PendingOperation::AuthorizedTransfers { transfers, .. } = pending else {
                return Err(ContractError::UnknownReply);
            };
            Ok(PendingOperation::AuthorizedTransfers {
                transfers,
                replies_remaining: remaining,
            })
        })?;
    }
    Ok(Response::new()
        .add_attribute("action", "complete_authorized_transfer")
        .add_attribute("remaining", remaining.to_string()))
}

fn execute_cw20(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    assert_no_pending(deps.as_ref())?;
    let msg = match msg {
        ExecuteMsg::Transfer { recipient, amount } => {
            BaseExecuteMsg::Transfer { recipient, amount }
        }
        ExecuteMsg::Burn { amount } => BaseExecuteMsg::Burn { amount },
        ExecuteMsg::Send {
            contract,
            amount,
            msg,
        } => BaseExecuteMsg::Send {
            contract,
            amount,
            msg,
        },
        ExecuteMsg::IncreaseAllowance {
            spender,
            amount,
            expires,
        } => BaseExecuteMsg::IncreaseAllowance {
            spender,
            amount,
            expires,
        },
        ExecuteMsg::DecreaseAllowance {
            spender,
            amount,
            expires,
        } => BaseExecuteMsg::DecreaseAllowance {
            spender,
            amount,
            expires,
        },
        ExecuteMsg::TransferFrom {
            owner,
            recipient,
            amount,
        } => BaseExecuteMsg::TransferFrom {
            owner,
            recipient,
            amount,
        },
        ExecuteMsg::BurnFrom { owner, amount } => BaseExecuteMsg::BurnFrom { owner, amount },
        ExecuteMsg::SendFrom {
            owner,
            contract,
            amount,
            msg,
        } => BaseExecuteMsg::SendFrom {
            owner,
            contract,
            amount,
            msg,
        },
        ExecuteMsg::UpdateMarketing {
            project,
            description,
            marketing,
        } => BaseExecuteMsg::UpdateMarketing {
            project,
            description,
            marketing,
        },
        ExecuteMsg::UploadLogo(logo) => BaseExecuteMsg::UploadLogo(logo),
        ExecuteMsg::UpdateConfig { .. }
        | ExecuteMsg::TransferAdmin { .. }
        | ExecuteMsg::AcceptAdmin {}
        | ExecuteMsg::CancelAdminTransfer {}
        | ExecuteMsg::MintTo { .. }
        | ExecuteMsg::Deposit { .. }
        | ExecuteMsg::Withdraw { .. } => {
            return Err(ContractError::UnsupportedMessage);
        }
    };
    cw20_base::contract::execute(deps, env, info, msg).map_err(cw20_error)
}

#[entry_point]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => {
            let config = CONFIG.load(deps.storage)?;
            to_json_binary(&ConfigResponse {
                admin: config.admin.to_string(),
                pending_admin: PENDING_ADMIN
                    .may_load(deps.storage)?
                    .map(|admin| admin.to_string()),
                vault: config.vault.to_string(),
                asset_tokens: config.asset_tokens.map(|addr| addr.to_string()),
                minimum_initial_deposit: config.minimum_initial_deposit,
            })
        }
        QueryMsg::Authorization {} => to_json_binary(&liquidity_authorization(deps)?),
        QueryMsg::Balance { address } => {
            cw20_query(deps, env, cw20_base::msg::QueryMsg::Balance { address })
        }
        QueryMsg::TokenInfo {} => cw20_query(deps, env, cw20_base::msg::QueryMsg::TokenInfo {}),
        QueryMsg::Minter {} => cw20_query(deps, env, cw20_base::msg::QueryMsg::Minter {}),
        QueryMsg::Allowance { owner, spender } => cw20_query(
            deps,
            env,
            cw20_base::msg::QueryMsg::Allowance { owner, spender },
        ),
        QueryMsg::AllAllowances {
            owner,
            start_after,
            limit,
        } => cw20_query(
            deps,
            env,
            cw20_base::msg::QueryMsg::AllAllowances {
                owner,
                start_after,
                limit,
            },
        ),
        QueryMsg::AllSpenderAllowances {
            spender,
            start_after,
            limit,
        } => cw20_query(
            deps,
            env,
            cw20_base::msg::QueryMsg::AllSpenderAllowances {
                spender,
                start_after,
                limit,
            },
        ),
        QueryMsg::AllAccounts { start_after, limit } => cw20_query(
            deps,
            env,
            cw20_base::msg::QueryMsg::AllAccounts { start_after, limit },
        ),
        QueryMsg::MarketingInfo {} => {
            cw20_query(deps, env, cw20_base::msg::QueryMsg::MarketingInfo {})
        }
        QueryMsg::DownloadLogo {} => {
            cw20_query(deps, env, cw20_base::msg::QueryMsg::DownloadLogo {})
        }
    }
}

fn liquidity_authorization(deps: Deps) -> StdResult<LiquidityAuthorizationResponse> {
    let authorization = match PENDING.may_load(deps.storage)? {
        Some(PendingOperation::Deposit { swap, .. }) => LiquidityAuthorizationResponse {
            finalize: swap.is_none(),
            swap,
            transfers: vec![],
        },
        Some(PendingOperation::WithdrawSingle { swap, .. }) => LiquidityAuthorizationResponse {
            swap: Some(swap),
            transfers: vec![],
            finalize: false,
        },
        Some(PendingOperation::AuthorizedTransfers { transfers, .. }) => {
            LiquidityAuthorizationResponse {
                swap: None,
                transfers,
                finalize: false,
            }
        }
        None => LiquidityAuthorizationResponse {
            swap: None,
            transfers: vec![],
            finalize: false,
        },
    };
    Ok(authorization)
}

fn cw20_query(deps: Deps, env: Env, msg: cw20_base::msg::QueryMsg) -> StdResult<Binary> {
    cw20_base::contract::query(deps, env, msg)
}

fn vault_balances(deps: Deps, config: &Config) -> StdResult<[Uint128; 2]> {
    let response: VaultBalancesResponse = deps
        .querier
        .query_wasm_smart(&config.vault, &VaultQueryMsg::Balances {})?;
    Ok(response.balances)
}

fn nav(balances: [Uint128; 2], price: Decimal) -> Result<Uint128, ContractError> {
    if price.is_zero() {
        return Err(ContractError::InvalidVault);
    }
    let token1_in_token0 = Uint256::from(balances[1]) * Uint256::from(Decimal::one().atomics())
        / Uint256::from(price.atomics());
    let total = Uint256::from(balances[0]) + token1_in_token0;
    total
        .to_string()
        .parse()
        .map_err(|_| ContractError::Std(StdError::generic_err("NAV overflow")))
}

fn proportional_deposit_shares(
    pre_balances: [Uint128; 2],
    post_balances: [Uint128; 2],
    supply: Uint128,
) -> Result<Uint128, ContractError> {
    if pre_balances[0].is_zero() || pre_balances[1].is_zero() {
        return Err(ContractError::InvalidDepositSettlement);
    }
    let added_0 = post_balances[0]
        .checked_sub(pre_balances[0])
        .map_err(|_| ContractError::InvalidDepositSettlement)?;
    let added_1 = post_balances[1]
        .checked_sub(pre_balances[1])
        .map_err(|_| ContractError::InvalidDepositSettlement)?;
    Ok(
        checked_ratio(added_0, supply, pre_balances[0])?.min(checked_ratio(
            added_1,
            supply,
            pre_balances[1],
        )?),
    )
}

fn checked_ratio(value: Uint128, numerator: Uint128, denominator: Uint128) -> StdResult<Uint128> {
    if denominator.is_zero() {
        return Err(StdError::generic_err("arithmetic denominator is zero"));
    }
    let result = Uint256::from(value) * Uint256::from(numerator) / Uint256::from(denominator);
    result
        .try_into()
        .map_err(|_| StdError::generic_err("arithmetic result overflow"))
}

fn vault_transfer(
    config: &Config,
    token: &Addr,
    amount: Uint128,
    recipient: &Addr,
) -> StdResult<WasmMsg> {
    Ok(WasmMsg::Execute {
        contract_addr: config.vault.to_string(),
        msg: to_json_binary(&VaultExecuteMsg::TransferTo {
            token: token.to_string(),
            amount,
            recipient: recipient.to_string(),
        })?,
        funds: vec![],
    })
}

fn cw20_balance(deps: Deps, token: &Addr, account: &Addr) -> StdResult<Uint128> {
    let response: cw20::BalanceResponse = deps.querier.query_wasm_smart(
        token,
        &Cw20QueryMsg::Balance {
            address: account.to_string(),
        },
    )?;
    Ok(response.balance)
}

fn mint_shares(
    deps: DepsMut,
    env: &Env,
    recipient: String,
    amount: Uint128,
) -> Result<(), ContractError> {
    cw20_base::contract::execute(
        deps,
        env.clone(),
        MessageInfo {
            sender: env.contract.address.clone(),
            funds: vec![],
        },
        BaseExecuteMsg::Mint { recipient, amount },
    )
    .map_err(cw20_error)?;
    Ok(())
}

fn burn_shares(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    amount: Uint128,
) -> Result<(), ContractError> {
    cw20_base::contract::execute(deps, env, info, BaseExecuteMsg::Burn { amount })
        .map_err(cw20_error)?;
    Ok(())
}

fn assert_no_pending(deps: Deps) -> Result<(), ContractError> {
    if PENDING.may_load(deps.storage)?.is_some() {
        return Err(ContractError::OperationPending);
    }
    Ok(())
}

fn assert_admin(config: &Config, sender: &Addr) -> Result<(), ContractError> {
    if sender != config.admin {
        return Err(ContractError::Unauthorized);
    }
    Ok(())
}

fn cw20_error(error: cw20_base::ContractError) -> ContractError {
    ContractError::Cw20(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};
    use proptest::prelude::*;
    use proptest::test_runner::Config as ProptestConfig;

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 128,
            ..ProptestConfig::default()
        })]

        #[test]
        fn deposit_donation_withdraw_rounding_preserves_value_without_free_profit(
            assets in 1u128..=u64::MAX as u128,
            supply in 1u128..=u64::MAX as u128,
            deposit in 1u128..=u64::MAX as u128,
            donation in 0u128..=u64::MAX as u128,
        ) {
            let minted = Uint256::from(deposit) * Uint256::from(supply) / Uint256::from(assets);
            prop_assume!(minted > Uint256::zero());
            let post_supply = Uint256::from(supply) + minted;
            let post_assets = Uint256::from(assets) + Uint256::from(deposit) + Uint256::from(donation);
            let claim = post_assets * minted / post_supply;

            // A depositor can receive only their deposit plus their pro-rata donation;
            // the donation is external value, not protocol-created profit.
            prop_assert!(claim * post_supply
                <= Uint256::from(deposit) * post_supply + Uint256::from(donation) * minted);
            prop_assert_eq!(claim + (post_assets - claim), post_assets);
            prop_assert_eq!(Uint256::from(supply) + minted, post_supply);
        }

        #[test]
        fn proportional_two_token_withdrawal_conserves_each_balance(
            balances in (any::<u128>(), any::<u128>()),
            supply in 1u128..=u128::MAX,
            shares in any::<u128>(),
        ) {
            let shares = shares.min(supply);
            for balance in [balances.0, balances.1] {
                let paid = Uint256::from(balance) * Uint256::from(shares) / Uint256::from(supply);
                let remaining = Uint256::from(balance) - paid;
                prop_assert_eq!(paid + remaining, Uint256::from(balance));
            }
            prop_assert_eq!(shares + (supply - shares), supply);
        }
    }
    use cosmwasm_std::{from_json, ContractResult, Reply, SubMsgResult, SystemResult};
    use cw20_base::state::TokenInfo;

    fn test_config() -> Config {
        Config {
            admin: Addr::unchecked("admin"),
            vault: Addr::unchecked("vault"),
            asset_tokens: [Addr::unchecked("token0"), Addr::unchecked("token1")],
            minimum_initial_deposit: Uint128::new(1_000),
        }
    }

    #[test]
    fn nav_values_token1_at_fixed_price() {
        let value = nav(
            [Uint128::new(100), Uint128::new(200)],
            Decimal::percent(200),
        )
        .unwrap();
        assert_eq!(value, Uint128::new(200));
    }

    #[test]
    fn nav_rejects_zero_price() {
        assert_eq!(
            nav([Uint128::one(), Uint128::one()], Decimal::zero()).unwrap_err(),
            ContractError::InvalidVault
        );
    }

    #[test]
    fn established_deposit_uses_minimum_proportional_contribution() {
        assert_eq!(
            proportional_deposit_shares(
                [Uint128::new(100), Uint128::new(200)],
                [Uint128::new(110), Uint128::new(260)],
                Uint128::new(1_000),
            )
            .unwrap(),
            Uint128::new(100)
        );
    }

    #[test]
    fn proportional_shares_reports_overflow_without_panicking() {
        assert!(proportional_deposit_shares(
            [Uint128::one(), Uint128::one()],
            [Uint128::MAX, Uint128::MAX],
            Uint128::MAX,
        )
        .is_err());
    }

    #[test]
    fn admin_transfer_requires_acceptance_and_can_be_cancelled() {
        let mut deps = mock_dependencies();
        CONFIG.save(deps.as_mut().storage, &test_config()).unwrap();
        execute_transfer_admin(deps.as_mut(), mock_info("admin", &[]), "next".into()).unwrap();
        assert_eq!(
            CONFIG.load(&deps.storage).unwrap().admin,
            Addr::unchecked("admin")
        );
        assert_eq!(
            execute_accept_admin(deps.as_mut(), mock_info("other", &[])).unwrap_err(),
            ContractError::Unauthorized
        );
        execute_cancel_admin_transfer(deps.as_mut(), mock_info("admin", &[])).unwrap();
        assert!(PENDING_ADMIN.may_load(&deps.storage).unwrap().is_none());
        execute_transfer_admin(deps.as_mut(), mock_info("admin", &[]), "next".into()).unwrap();
        execute_accept_admin(deps.as_mut(), mock_info("next", &[])).unwrap();
        assert_eq!(
            CONFIG.load(&deps.storage).unwrap().admin,
            Addr::unchecked("next")
        );
        assert_eq!(
            execute_update_config(deps.as_mut(), mock_info("admin", &[]), None).unwrap_err(),
            ContractError::Unauthorized
        );
    }

    #[test]
    fn authorization_is_scoped_to_pending_operation_and_cleared_by_reply() {
        let mut deps = mock_dependencies();
        let transfer = AuthorizedTransfer {
            token: "token0".into(),
            amount: Uint128::new(25),
            recipient: "recipient".into(),
        };
        PENDING
            .save(
                deps.as_mut().storage,
                &PendingOperation::AuthorizedTransfers {
                    transfers: vec![transfer.clone()],
                    replies_remaining: 1,
                },
            )
            .unwrap();
        assert_eq!(
            liquidity_authorization(deps.as_ref()).unwrap(),
            LiquidityAuthorizationResponse {
                swap: None,
                transfers: vec![transfer],
                finalize: false,
            }
        );
        assert_eq!(
            execute_cw20(
                deps.as_mut(),
                mock_env(),
                mock_info("holder", &[]),
                ExecuteMsg::Transfer {
                    recipient: "other".into(),
                    amount: Uint128::one(),
                },
            )
            .unwrap_err(),
            ContractError::OperationPending
        );
        complete_authorized_transfer(deps.as_mut()).unwrap();
        assert!(PENDING.may_load(&deps.storage).unwrap().is_none());
        assert_eq!(
            liquidity_authorization(deps.as_ref()).unwrap(),
            LiquidityAuthorizationResponse {
                swap: None,
                transfers: vec![],
                finalize: false,
            }
        );
    }

    #[test]
    fn migration_preserves_config_and_rejects_pending_settlement() {
        let mut deps = mock_dependencies();
        CONFIG.save(deps.as_mut().storage, &test_config()).unwrap();
        set_contract_version(deps.as_mut().storage, CONTRACT_NAME, "0.2.0-rc.1").unwrap();
        migrate(deps.as_mut(), mock_env(), MigrateMsg {}).unwrap();
        assert_eq!(CONFIG.load(&deps.storage).unwrap(), test_config());

        assert!(migrate(deps.as_mut(), mock_env(), MigrateMsg {}).is_err());
        set_contract_version(deps.as_mut().storage, CONTRACT_NAME, "0.2.0-rc.1").unwrap();
        PENDING
            .save(
                deps.as_mut().storage,
                &PendingOperation::AuthorizedTransfers {
                    transfers: vec![],
                    replies_remaining: 1,
                },
            )
            .unwrap();
        assert_eq!(
            migrate(deps.as_mut(), mock_env(), MigrateMsg {}).unwrap_err(),
            ContractError::OperationPending
        );
    }

    #[test]
    fn update_config_is_admin_gated_and_updates_minimum() {
        let mut deps = mock_dependencies();
        CONFIG.save(deps.as_mut().storage, &test_config()).unwrap();
        TOKEN_INFO
            .save(
                deps.as_mut().storage,
                &TokenInfo {
                    name: "Liquidity".into(),
                    symbol: "LIQ".into(),
                    decimals: 6,
                    total_supply: Uint128::zero(),
                    mint: None,
                },
            )
            .unwrap();
        let error = execute_update_config(
            deps.as_mut(),
            mock_info("attacker", &[]),
            Some(Uint128::new(5_000)),
        )
        .unwrap_err();
        assert_eq!(error, ContractError::Unauthorized);
        execute_update_config(
            deps.as_mut(),
            mock_info("admin", &[]),
            Some(Uint128::new(5_000)),
        )
        .unwrap();
        assert_eq!(
            CONFIG.load(&deps.storage).unwrap().minimum_initial_deposit,
            Uint128::new(5_000)
        );
        TOKEN_INFO
            .update(deps.as_mut().storage, |mut token| -> StdResult<_> {
                token.total_supply = Uint128::new(10_000);
                Ok(token)
            })
            .unwrap();
        let error = execute_update_config(
            deps.as_mut(),
            mock_info("admin", &[]),
            Some(Uint128::new(6_000)),
        )
        .unwrap_err();
        assert_eq!(error, ContractError::BootstrapComplete);
        execute_update_config(deps.as_mut(), mock_info("admin", &[]), None).unwrap();
        assert_eq!(
            CONFIG.load(&deps.storage).unwrap().minimum_initial_deposit,
            Uint128::new(5_000)
        );
    }

    #[test]
    fn instantiate_rejects_ineffective_initial_minimum() {
        let mut deps = mock_dependencies();
        let error = instantiate(
            deps.as_mut(),
            mock_env(),
            mock_info("creator", &[]),
            InstantiateMsg {
                admin: "admin".into(),
                vault: "vault".into(),
                name: "Liquidity".into(),
                symbol: "LIQ".into(),
                decimals: 6,
                minimum_initial_deposit: LOCKED_INITIAL_SHARES,
                marketing: None,
            },
        )
        .unwrap_err();
        assert_eq!(error, ContractError::InvalidMinimumInitialDeposit);
    }

    #[test]
    fn failed_withdraw_reply_clears_pending_without_burning() {
        let mut deps = mock_dependencies();
        CONFIG.save(deps.as_mut().storage, &test_config()).unwrap();
        PENDING
            .save(
                deps.as_mut().storage,
                &PendingOperation::WithdrawSingle {
                    owner: Addr::unchecked("owner"),
                    shares: Uint128::new(100),
                    recipient: Addr::unchecked("recipient"),
                    payout_token: Addr::unchecked("token0"),
                    base_amount: Uint128::new(50),
                    pre_payout_balance: Uint128::new(1_000),
                    min_amount: Uint128::new(40),
                    swap: SwapParams {
                        offer_token: "token1".into(),
                        amount: Uint128::new(50),
                        min_return: Uint128::new(40),
                        max_spread: Decimal::percent(5),
                        deadline: u64::MAX,
                    },
                },
            )
            .unwrap();
        let response = complete_single_withdraw(
            deps.as_mut(),
            mock_env(),
            Reply {
                id: WITHDRAW_REPLY_ID,
                result: SubMsgResult::Err("swap failed".to_string()),
            },
        )
        .unwrap();
        assert_eq!(response.attributes[0].value, "withdraw_single_failed");
        assert!(PENDING.may_load(&deps.storage).unwrap().is_none());
    }

    #[test]
    fn withdraw_single_uses_reply_always() {
        let mut deps = mock_dependencies();
        CONFIG.save(deps.as_mut().storage, &test_config()).unwrap();
        deps.querier.update_wasm(|query| match query {
            cosmwasm_std::WasmQuery::Smart { contract_addr, msg } if contract_addr == "vault" => {
                let query: VaultQueryMsg = from_json(msg).unwrap();
                match query {
                    VaultQueryMsg::Balances {} => SystemResult::Ok(ContractResult::Ok(
                        to_json_binary(&VaultBalancesResponse {
                            balances: [Uint128::new(1_000), Uint128::new(2_000)],
                        })
                        .unwrap(),
                    )),
                    _ => SystemResult::Ok(ContractResult::Err("unsupported".to_string())),
                }
            }
            _ => SystemResult::Ok(ContractResult::Err("unsupported".to_string())),
        });
        let response = execute_single_withdraw(
            deps.as_mut(),
            mock_info("owner", &[]),
            test_config(),
            Addr::unchecked("recipient"),
            Uint128::new(100),
            [Uint128::new(50), Uint128::new(50)],
            0,
            Uint128::new(40),
            SwapParams {
                offer_token: "token1".to_string(),
                amount: Uint128::new(50),
                min_return: Uint128::new(40),
                max_spread: Decimal::percent(1),
                deadline: u64::MAX,
            },
            u64::MAX,
        )
        .unwrap();
        let submsg = response.messages[0].clone();
        assert_eq!(submsg.id, WITHDRAW_REPLY_ID);
        assert!(matches!(submsg.reply_on, cosmwasm_std::ReplyOn::Always));
    }
}
