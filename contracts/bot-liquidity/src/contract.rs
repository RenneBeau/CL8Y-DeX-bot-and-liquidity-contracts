use bot_types::{
    RebalanceStatusResponse, SwapParams, VaultBalancesResponse, VaultConfigResponse,
    VaultExecuteMsg, VaultPriceResponse, VaultQueryMsg, WithdrawalType,
};
use cosmwasm_std::{
    entry_point, to_json_binary, Addr, Binary, Decimal, Deps, DepsMut, Env, MessageInfo, Reply,
    Response, StdError, StdResult, SubMsg, Uint128, Uint256, WasmMsg,
};
use cw2::set_contract_version;
use cw20::{Cw20ExecuteMsg, Cw20QueryMsg, MinterResponse};
use cw20_base::msg::{ExecuteMsg as BaseExecuteMsg, InstantiateMsg as BaseInstantiateMsg};
use cw20_base::state::{BALANCES, TOKEN_INFO};

use crate::error::ContractError;
use crate::msg::{ConfigResponse, ExecuteMsg, InstantiateMsg, QueryMsg};
use crate::state::{Config, PendingOperation, CONFIG, PENDING};

const CONTRACT_NAME: &str = "crates.io:cl8y-bot-liquidity";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEPOSIT_REPLY_ID: u64 = 1;
const WITHDRAW_REPLY_ID: u64 = 2;
const LOCKED_INITIAL_SHARES: Uint128 = Uint128::new(1_000);

#[entry_point]
pub fn instantiate(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
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
            vault,
            asset_tokens,
            minimum_initial_deposit: msg.minimum_initial_deposit,
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
        other => execute_cw20(deps, env, info, other),
    }
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
        vault_balances[0].multiply_ratio(shares, supply),
        vault_balances[1].multiply_ratio(shares, supply),
    ];
    let recipient = deps
        .api
        .addr_validate(recipient.as_deref().unwrap_or(info.sender.as_str()))?;
    burn_shares(deps.branch(), env.clone(), info.clone(), shares)?;
    match output {
        WithdrawalType::ProRata { min_assets } => {
            if claims[0] < min_assets[0] || claims[1] < min_assets[1] {
                return Err(ContractError::MinimumNotMet);
            }
            let mut response = Response::new().add_attribute("action", "withdraw_pro_rata");
            for (token, amount) in config.asset_tokens.iter().zip(claims) {
                if !amount.is_zero() {
                    response =
                        response.add_message(vault_transfer(&config, token, amount, &recipient)?);
                }
            }
            Ok(response)
        }
        WithdrawalType::Token0 { min_amount, swap } => execute_single_withdraw(
            deps, config, recipient, claims, 0, min_amount, swap, deadline,
        ),
        WithdrawalType::Token1 { min_amount, swap } => execute_single_withdraw(
            deps, config, recipient, claims, 1, min_amount, swap, deadline,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_single_withdraw(
    deps: DepsMut,
    config: Config,
    recipient: Addr,
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
            recipient,
            payout_token: config.asset_tokens[payout_index].clone(),
            base_amount: claims[payout_index],
            pre_payout_balance,
            min_amount,
        },
    )?;
    Ok(Response::new()
        .add_submessage(SubMsg::reply_on_success(
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
        WITHDRAW_REPLY_ID => complete_single_withdraw(deps),
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
        if pre_nav.is_zero() {
            return Err(ContractError::InvalidDepositSettlement);
        }
        deposit_value.multiply_ratio(pre_supply, pre_nav)
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

fn complete_single_withdraw(deps: DepsMut) -> Result<Response, ContractError> {
    let pending = PENDING.load(deps.storage)?;
    let PendingOperation::WithdrawSingle {
        recipient,
        payout_token,
        base_amount,
        pre_payout_balance,
        min_amount,
    } = pending
    else {
        return Err(ContractError::UnknownReply);
    };
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
    PENDING.remove(deps.storage);
    Ok(Response::new()
        .add_message(vault_transfer(&config, &payout_token, payout, &recipient)?)
        .add_attribute("action", "complete_single_withdraw")
        .add_attribute("recipient", recipient)
        .add_attribute("amount", payout))
}

fn execute_cw20(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
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
        ExecuteMsg::Deposit { .. } | ExecuteMsg::Withdraw { .. } => unreachable!(),
    };
    cw20_base::contract::execute(deps, env, info, msg).map_err(cw20_error)
}

#[entry_point]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => {
            let config = CONFIG.load(deps.storage)?;
            to_json_binary(&ConfigResponse {
                vault: config.vault.to_string(),
                asset_tokens: config.asset_tokens.map(|addr| addr.to_string()),
                minimum_initial_deposit: config.minimum_initial_deposit,
            })
        }
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

fn cw20_error(error: cw20_base::ContractError) -> ContractError {
    ContractError::Cw20(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
