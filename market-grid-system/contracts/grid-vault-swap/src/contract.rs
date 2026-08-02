use cosmwasm_std::{
    entry_point, to_json_binary, Addr, Binary, Decimal, Deps, DepsMut, Env, MessageInfo, Reply,
    Response, StdError, StdResult, SubMsg, Uint128, Uint256, WasmMsg,
};
use cw2::set_contract_version;
use cw20::{BalanceResponse, Cw20ExecuteMsg, Cw20QueryMsg, Cw20ReceiveMsg};

use crate::error::ContractError;
use crate::msg::{
    ExecuteMsg, GridStatusResponse, HybridSwapParams, InstantiateMsg, MigrateMsg, PairCw20HookMsg,
    PairQueryMsg, PoolResponse, QueryMsg, ReceiveMsg,
};
use crate::state::{Config, PendingSwap, CONFIG, PAUSED, PENDING_SWAP, SHARES, TOTAL_SHARES};

const CONTRACT_NAME: &str = "crates.io:cl8y-grid-vault-swap";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
const REBALANCE_REPLY_ID: u64 = 1;

const DEFAULT_ALLOCATION_TOLERANCE_BPS: u16 = 100;
const DEFAULT_MAX_TRADE_BPS: u16 = 2_500;
const DEFAULT_MAX_EXECUTION_DEVIATION_BPS: u16 = 500;
const DEFAULT_QUOTE_SLIPPAGE_BPS: u16 = 200;
const DEFAULT_MAX_SPOT_TWAP_DEVIATION_BPS: u16 = 500;
const DEFAULT_MAX_TRADE_POOL_BPS: u16 = 1_000;
const DEFAULT_MAX_SPREAD: Decimal = Decimal::percent(5);
const MAX_SPREAD: Decimal = Decimal::percent(10);
const MAX_TRADE_BPS: u16 = 5_000;
const MAX_EXECUTION_DEVIATION_BPS: u16 = 1_000;
const MAX_QUOTE_SLIPPAGE_BPS: u16 = 500;
const MAX_SPOT_TWAP_DEVIATION_BPS: u16 = 1_000;
const MAX_TRADE_POOL_BPS: u16 = 2_000;
const MAX_ALLOCATION_TOLERANCE_BPS: u16 = 2_000;
const MAX_TWAP_WINDOW_SECONDS: u32 = 86_400;
const MAX_GRID_COUNT: u32 = 500;

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    let admin = deps.api.addr_validate(&msg.admin)?;
    let pair = deps.api.addr_validate(&msg.pair)?;
    let pair_info = deps
        .querier
        .query_wasm_smart::<crate::msg::PairInfo>(&pair, &PairQueryMsg::Pair {})?;
    if pair_info.contract_addr.as_str() != pair.as_str() {
        return Err(ContractError::InvalidPair);
    }
    let [asset_0, asset_1] = pair_info.asset_infos;
    let asset_tokens = [
        token_addr(deps.as_ref(), asset_0)?,
        token_addr(deps.as_ref(), asset_1)?,
    ];
    if asset_tokens[0] == asset_tokens[1] {
        return Err(ContractError::InvalidPair);
    }
    let token_0: cw20::TokenInfoResponse = deps
        .querier
        .query_wasm_smart(&asset_tokens[0], &Cw20QueryMsg::TokenInfo {})?;
    let token_1: cw20::TokenInfoResponse = deps
        .querier
        .query_wasm_smart(&asset_tokens[1], &Cw20QueryMsg::TokenInfo {})?;
    if token_0.decimals != token_1.decimals {
        return Err(ContractError::DecimalMismatch);
    }
    if msg.upper_price <= msg.lower_price || msg.lower_price.is_zero() {
        return Err(ContractError::InvalidGrid);
    }
    if msg.grid_count == 0 || msg.grid_count > MAX_GRID_COUNT {
        return Err(ContractError::InvalidGridCount);
    }
    if !valid_twap_window(msg.twap_window_seconds) {
        return Err(ContractError::InvalidTwapWindow);
    }
    let allocation_tolerance_bps = msg
        .allocation_tolerance_bps
        .unwrap_or(DEFAULT_ALLOCATION_TOLERANCE_BPS);
    validate_allocation_tolerance(allocation_tolerance_bps)?;
    let max_trade_bps = msg.max_trade_bps.unwrap_or(DEFAULT_MAX_TRADE_BPS);
    let max_execution_deviation_bps = msg
        .max_execution_deviation_bps
        .unwrap_or(DEFAULT_MAX_EXECUTION_DEVIATION_BPS);
    let quote_slippage_bps = msg.quote_slippage_bps.unwrap_or(DEFAULT_QUOTE_SLIPPAGE_BPS);
    let max_spot_twap_deviation_bps = msg
        .max_spot_twap_deviation_bps
        .unwrap_or(DEFAULT_MAX_SPOT_TWAP_DEVIATION_BPS);
    let max_trade_pool_bps = msg.max_trade_pool_bps.unwrap_or(DEFAULT_MAX_TRADE_POOL_BPS);
    let max_spread = msg.max_spread.unwrap_or(DEFAULT_MAX_SPREAD);
    validate_risk_controls(
        max_trade_bps,
        max_execution_deviation_bps,
        quote_slippage_bps,
        max_spot_twap_deviation_bps,
        max_trade_pool_bps,
        max_spread,
    )?;
    let initial_price = reference_price(deps.as_ref(), &env, &pair, msg.twap_window_seconds)?;
    let initial_cell = grid_cell(
        initial_price,
        msg.lower_price,
        msg.upper_price,
        msg.grid_count,
    );
    let config = Config {
        admin,
        pending_admin: None,
        pair: pair.clone(),
        asset_tokens,
        decimals: token_0.decimals,
        twap_window_seconds: msg.twap_window_seconds,
        grid_count: msg.grid_count,
        lower_price: msg.lower_price,
        upper_price: msg.upper_price,
        allocation_tolerance_bps,
        max_trade_bps,
        max_execution_deviation_bps,
        quote_slippage_bps,
        max_spot_twap_deviation_bps,
        max_trade_pool_bps,
        max_spread,
        reference_price: initial_price,
        last_cell: initial_cell,
    };
    PAUSED.save(deps.storage, &false)?;
    TOTAL_SHARES.save(deps.storage, &Uint128::zero())?;
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("owner", info.sender)
        .add_attribute("pair", pair.to_string()))
}

#[entry_point]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Receive(receive) => execute_receive(deps, env, info, receive),
        ExecuteMsg::Withdraw { shares, recipient } => {
            execute_withdraw(deps, env, info, shares, recipient)
        }
        ExecuteMsg::Rebalance { deadline } => execute_rebalance(deps, env, info, deadline),
        ExecuteMsg::UpdateConfig {
            grid_count,
            lower_price,
            upper_price,
            allocation_tolerance_bps,
            max_trade_bps,
            max_execution_deviation_bps,
            quote_slippage_bps,
            max_spot_twap_deviation_bps,
            max_trade_pool_bps,
            max_spread,
        } => execute_update_config(
            deps,
            env,
            info,
            grid_count,
            lower_price,
            upper_price,
            allocation_tolerance_bps,
            max_trade_bps,
            max_execution_deviation_bps,
            quote_slippage_bps,
            max_spot_twap_deviation_bps,
            max_trade_pool_bps,
            max_spread,
        ),
        ExecuteMsg::TransferAdmin { admin } => execute_transfer_admin(deps, info, admin),
        ExecuteMsg::AcceptAdmin {} => execute_accept_admin(deps, info),
        ExecuteMsg::Pause {} => execute_pause(deps, info),
        ExecuteMsg::Resume {} => execute_resume(deps, info),
    }
}

fn execute_receive(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    receive: Cw20ReceiveMsg,
) -> Result<Response, ContractError> {
    assert_not_paused(deps.as_ref())?;
    if receive.amount.is_zero() {
        return Err(ContractError::ZeroAmount);
    }
    let hook: ReceiveMsg = from_json(&receive.msg)?;
    let ReceiveMsg::Deposit {} = hook;
    let config = CONFIG.load(deps.storage)?;
    let token_index = config
        .asset_tokens
        .iter()
        .position(|token| token.as_str() == info.sender.as_str())
        .ok_or(ContractError::UnsupportedToken)?;
    let price = reference_price(
        deps.as_ref(),
        &env,
        &config.pair,
        config.twap_window_seconds,
    )?;
    let balances = balances(deps.as_ref(), &env.contract.address, &config)?;
    let total_shares = TOTAL_SHARES.load(deps.storage)?;
    let minted = deposit_shares(receive.amount, token_index, price, &balances, total_shares)?;
    if minted.is_zero() {
        return Err(ContractError::ZeroAmount);
    }
    let depositor = deps.api.addr_validate(&receive.sender)?;
    TOTAL_SHARES.save(
        deps.storage,
        &total_shares
            .checked_add(minted)
            .map_err(StdError::overflow)?,
    )?;
    let shares = SHARES
        .may_load(deps.storage, depositor.as_str())?
        .unwrap_or_default()
        .checked_add(minted)
        .map_err(StdError::overflow)?;
    SHARES.save(deps.storage, depositor.as_str(), &shares)?;
    Ok(Response::new()
        .add_attribute("action", "deposit")
        .add_attribute("depositor", depositor)
        .add_attribute("amount", receive.amount)
        .add_attribute("shares", minted))
}

fn execute_withdraw(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    shares: Uint128,
    recipient: Option<String>,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    assert_not_paused(deps.as_ref())?;
    if PENDING_SWAP.may_load(deps.storage)?.is_some() {
        return Err(ContractError::RebalancePending);
    }
    if shares.is_zero() {
        return Err(ContractError::ZeroAmount);
    }
    let config = CONFIG.load(deps.storage)?;
    let owner_shares = SHARES
        .may_load(deps.storage, info.sender.as_str())?
        .unwrap_or_default();
    let total_shares = TOTAL_SHARES.load(deps.storage)?;
    if shares > owner_shares || shares > total_shares {
        return Err(ContractError::InsufficientShares);
    }
    let balances = balances(deps.as_ref(), &env.contract.address, &config)?;
    let recipient = deps
        .api
        .addr_validate(recipient.as_deref().unwrap_or(info.sender.as_str()))?;
    let amounts = [
        balances[0].multiply_ratio(shares, total_shares),
        balances[1].multiply_ratio(shares, total_shares),
    ];
    let mut response = Response::new()
        .add_attribute("action", "withdraw")
        .add_attribute("burned_shares", shares);
    for (index, amount) in amounts.into_iter().enumerate() {
        if !amount.is_zero() {
            response = response.add_message(WasmMsg::Execute {
                contract_addr: config.asset_tokens[index].to_string(),
                msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
                    recipient: recipient.to_string(),
                    amount,
                })?,
                funds: vec![],
            });
        }
    }
    TOTAL_SHARES.save(
        deps.storage,
        &total_shares
            .checked_sub(shares)
            .map_err(StdError::overflow)?,
    )?;
    SHARES.save(
        deps.storage,
        info.sender.as_str(),
        &owner_shares
            .checked_sub(shares)
            .map_err(StdError::overflow)?,
    )?;
    Ok(response)
}

fn execute_rebalance(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    deadline: u64,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    assert_not_paused(deps.as_ref())?;
    if PENDING_SWAP.may_load(deps.storage)?.is_some() {
        return Err(ContractError::RebalancePending);
    }
    if deadline < env.block.time.seconds() {
        return Err(ContractError::Expired);
    }
    let config = CONFIG.load(deps.storage)?;
    let plan = rebalance_plan(deps.as_ref(), &env, &config)?;
    if !plan.should_rebalance {
        return Err(ContractError::RebalanceNotRequired);
    }
    let offer_token = plan
        .offer_token
        .ok_or(ContractError::InvalidRebalanceSwap)?;
    let amount = plan.amount.ok_or(ContractError::InvalidRebalanceSwap)?;
    let min_return = plan.min_return.ok_or(ContractError::InvalidRebalanceSwap)?;
    let offer_index = if offer_token == config.asset_tokens[0] {
        0
    } else {
        1
    };
    let pool: PoolResponse = deps
        .querier
        .query_wasm_smart(&config.pair, &PairQueryMsg::Pool {})?;
    validate_pool_safety(
        &pool,
        plan.captured_twap,
        offer_index,
        amount,
        config.max_spot_twap_deviation_bps,
        config.max_trade_pool_bps,
    )?;
    PENDING_SWAP.save(
        deps.storage,
        &PendingSwap {
            captured_twap: plan.captured_twap,
            balances: plan.balances,
            pre_deviation_bps: plan.allocation_deviation_bps,
            offer_index: offer_index as u8,
            amount,
            min_return,
        },
    )?;
    Ok(Response::new()
        .add_submessage(SubMsg::reply_on_success(
            swap_message(&config, &offer_token, amount, min_return, deadline)?,
            REBALANCE_REPLY_ID,
        ))
        .add_attribute("action", "rebalance")
        .add_attribute("offer_token", offer_token)
        .add_attribute("amount", amount)
        .add_attribute("min_return", min_return))
}

#[entry_point]
pub fn reply(deps: DepsMut, env: Env, reply: Reply) -> Result<Response, ContractError> {
    if reply.id != REBALANCE_REPLY_ID {
        return Err(ContractError::UnknownReply);
    }
    let pending = PENDING_SWAP
        .may_load(deps.storage)?
        .ok_or(ContractError::MissingPendingRebalance)?;
    let mut config = CONFIG.load(deps.storage)?;
    let settled = balances(deps.as_ref(), &env.contract.address, &config)?;
    validate_settlement(&pending, settled)?;
    let current = allocation_deviation(settled, pending.captured_twap, &config)?;
    let within_tolerance = validate_rebalance_outcome(
        pending.pre_deviation_bps,
        current,
        config.allocation_tolerance_bps,
    )?;
    if within_tolerance {
        config.reference_price = pending.captured_twap;
        config.last_cell = grid_cell(
            pending.captured_twap,
            config.lower_price,
            config.upper_price,
            config.grid_count,
        );
    }
    CONFIG.save(deps.storage, &config)?;
    PENDING_SWAP.remove(deps.storage);
    Ok(Response::new()
        .add_attribute("action", "complete_rebalance")
        .add_attribute("allocation_deviation_bps", current.to_string())
        .add_attribute("cell_updated", within_tolerance.to_string()))
}

#[allow(clippy::too_many_arguments)]
fn execute_update_config(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    grid_count: Option<u32>,
    lower_price: Option<Decimal>,
    upper_price: Option<Decimal>,
    allocation_tolerance_bps: Option<u16>,
    max_trade_bps: Option<u16>,
    max_execution_deviation_bps: Option<u16>,
    quote_slippage_bps: Option<u16>,
    max_spot_twap_deviation_bps: Option<u16>,
    max_trade_pool_bps: Option<u16>,
    max_spread: Option<Decimal>,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    let mut config = CONFIG.load(deps.storage)?;
    assert_admin(&config, &info.sender)?;
    if PENDING_SWAP.may_load(deps.storage)?.is_some() {
        return Err(ContractError::RebalancePending);
    }
    if let Some(value) = grid_count {
        if value == 0 || value > MAX_GRID_COUNT {
            return Err(ContractError::InvalidGridCount);
        }
        config.grid_count = value;
    }
    if let Some(value) = lower_price {
        config.lower_price = value;
    }
    if let Some(value) = upper_price {
        config.upper_price = value;
    }
    if config.lower_price.is_zero() || config.upper_price <= config.lower_price {
        return Err(ContractError::InvalidGrid);
    }
    if let Some(value) = allocation_tolerance_bps {
        validate_allocation_tolerance(value)?;
        config.allocation_tolerance_bps = value;
    }
    let next_max_trade = max_trade_bps.unwrap_or(config.max_trade_bps);
    let next_execution = max_execution_deviation_bps.unwrap_or(config.max_execution_deviation_bps);
    let next_quote = quote_slippage_bps.unwrap_or(config.quote_slippage_bps);
    let next_spot_twap = max_spot_twap_deviation_bps.unwrap_or(config.max_spot_twap_deviation_bps);
    let next_trade_pool = max_trade_pool_bps.unwrap_or(config.max_trade_pool_bps);
    let next_spread = max_spread.unwrap_or(config.max_spread);
    validate_risk_controls(
        next_max_trade,
        next_execution,
        next_quote,
        next_spot_twap,
        next_trade_pool,
        next_spread,
    )?;
    config.max_trade_bps = next_max_trade;
    config.max_execution_deviation_bps = next_execution;
    config.quote_slippage_bps = next_quote;
    config.max_spot_twap_deviation_bps = next_spot_twap;
    config.max_trade_pool_bps = next_trade_pool;
    config.max_spread = next_spread;
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new().add_attribute("action", "update_config"))
}

fn execute_transfer_admin(
    deps: DepsMut,
    info: MessageInfo,
    admin: String,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    let config = CONFIG.load(deps.storage)?;
    assert_admin(&config, &info.sender)?;
    let pending = deps.api.addr_validate(&admin)?;
    CONFIG.update(deps.storage, |mut config| -> Result<_, ContractError> {
        config.pending_admin = Some(pending.clone());
        Ok(config)
    })?;
    Ok(Response::new()
        .add_attribute("action", "propose_admin")
        .add_attribute("pending_admin", pending))
}

fn execute_accept_admin(deps: DepsMut, info: MessageInfo) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    CONFIG.update(deps.storage, |mut config| -> Result<_, ContractError> {
        if config.pending_admin.as_ref() != Some(&info.sender) {
            return Err(ContractError::Unauthorized);
        }
        config.admin = info.sender.clone();
        config.pending_admin = None;
        Ok(config)
    })?;
    Ok(Response::new().add_attribute("action", "accept_admin"))
}

fn execute_pause(deps: DepsMut, info: MessageInfo) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    let config = CONFIG.load(deps.storage)?;
    assert_admin(&config, &info.sender)?;
    if PAUSED.load(deps.storage)? {
        return Err(ContractError::Paused);
    }
    PAUSED.save(deps.storage, &true)?;
    Ok(Response::new().add_attribute("action", "pause"))
}

fn execute_resume(deps: DepsMut, info: MessageInfo) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    let config = CONFIG.load(deps.storage)?;
    assert_admin(&config, &info.sender)?;
    if !PAUSED.load(deps.storage)? {
        return Err(ContractError::NotPaused);
    }
    PAUSED.save(deps.storage, &false)?;
    Ok(Response::new().add_attribute("action", "resume"))
}

#[entry_point]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&query_config(deps)?),
        QueryMsg::GridStatus {} => to_json_binary(&grid_status(deps, &env)?),
        QueryMsg::Shares { address } => {
            let address = deps.api.addr_validate(&address)?;
            let shares = SHARES
                .may_load(deps.storage, address.as_str())?
                .unwrap_or_default();
            to_json_binary(&crate::msg::SharesResponse { shares })
        }
        QueryMsg::Vault {} => to_json_binary(&query_vault(deps, &env)?),
    }
}

fn query_config(deps: Deps) -> StdResult<crate::msg::ConfigResponse> {
    let config = CONFIG.load(deps.storage)?;
    Ok(crate::msg::ConfigResponse {
        admin: config.admin.to_string(),
        pending_admin: config.pending_admin.as_ref().map(ToString::to_string),
        pair: config.pair.to_string(),
        asset_tokens: [
            config.asset_tokens[0].to_string(),
            config.asset_tokens[1].to_string(),
        ],
        decimals: config.decimals,
        twap_window_seconds: config.twap_window_seconds,
        grid_count: config.grid_count,
        lower_price: config.lower_price,
        upper_price: config.upper_price,
        allocation_tolerance_bps: config.allocation_tolerance_bps,
        max_trade_bps: config.max_trade_bps,
        max_execution_deviation_bps: config.max_execution_deviation_bps,
        quote_slippage_bps: config.quote_slippage_bps,
        max_spot_twap_deviation_bps: config.max_spot_twap_deviation_bps,
        max_trade_pool_bps: config.max_trade_pool_bps,
        max_spread: config.max_spread,
        reference_price: config.reference_price,
        last_cell: config.last_cell,
        paused: PAUSED.may_load(deps.storage)?.unwrap_or(true),
    })
}

fn grid_status(deps: Deps, env: &Env) -> StdResult<GridStatusResponse> {
    let config = CONFIG.load(deps.storage)?;
    let pending_swap = PENDING_SWAP.may_load(deps.storage)?.is_some();
    let mut status = GridStatusResponse {
        current_cell: config.last_cell,
        target_weight_bps: 0,
        allocation_deviation_bps: 0,
        should_rebalance: false,
        captured_twap: config.reference_price,
        balances: [Uint128::zero(), Uint128::zero()],
        offer_token: None,
        amount: None,
        min_return: None,
        pending_swap,
    };
    if pending_swap {
        return Ok(status);
    }
    let plan = rebalance_plan(deps, env, &config)?;
    status.current_cell = grid_cell(
        plan.captured_twap,
        config.lower_price,
        config.upper_price,
        config.grid_count,
    );
    status.target_weight_bps = target_weight_bps(status.current_cell, &config);
    status.allocation_deviation_bps = plan.allocation_deviation_bps;
    status.should_rebalance = plan.should_rebalance;
    status.offer_token = plan.offer_token;
    status.amount = plan.amount;
    status.min_return = plan.min_return;
    Ok(status)
}

fn query_vault(deps: Deps, env: &Env) -> StdResult<crate::msg::VaultResponse> {
    let config = CONFIG.load(deps.storage)?;
    let balances = balances(deps, &env.contract.address, &config)?;
    let price = reference_price(deps, env, &config.pair, config.twap_window_seconds)?;
    let token_0_value = checked_ratio(balances[0], price.atomics(), Decimal::one().atomics())?;
    let value_in_token_1 = Uint256::from(token_0_value)
        .checked_add(Uint256::from(balances[1]))
        .map_err(|_| StdError::generic_err("value overflow"))?
        .try_into()
        .map_err(|_| StdError::generic_err("value overflow"))?;
    Ok(crate::msg::VaultResponse {
        balances,
        total_shares: TOTAL_SHARES.load(deps.storage)?,
        value_in_token_1,
    })
}

fn rebalance_plan(
    deps: Deps,
    env: &Env,
    config: &Config,
) -> StdResult<crate::msg::GridStatusResponse> {
    let captured_twap = reference_price(deps, env, &config.pair, config.twap_window_seconds)?;
    let holdings = balances(deps, &env.contract.address, config)?;
    let current_cell = grid_cell(
        captured_twap,
        config.lower_price,
        config.upper_price,
        config.grid_count,
    );
    let deviation_bps = allocation_deviation(holdings, captured_twap, config)?;
    let should_rebalance =
        current_cell != config.last_cell || deviation_bps > config.allocation_tolerance_bps;
    let offer = if should_rebalance {
        planned_offer(
            holdings,
            captured_twap,
            current_cell,
            config.grid_count,
            config.max_trade_bps,
        )?
    } else {
        None
    };
    let (offer_token, amount, min_return) = if let Some((offer_index, amount)) = offer {
        let token = config.asset_tokens[offer_index].to_string();
        let twap_return = expected_return(amount, offer_index, captured_twap)?;
        let execution_floor = twap_return.multiply_ratio(
            10_000u128 - u128::from(config.max_execution_deviation_bps),
            10_000u128,
        );
        let simulation: crate::msg::HybridSimulationResponse = deps.querier.query_wasm_smart(
            &config.pair,
            &PairQueryMsg::HybridSimulation {
                offer_asset: crate::msg::Asset {
                    info: crate::msg::AssetInfo::Token {
                        contract_addr: token.clone(),
                    },
                    amount,
                },
                hybrid: HybridSwapParams::pool_only(amount),
                trader: None,
                sender: None,
                belief_price: None,
            },
        )?;
        let quote_floor = simulation.return_amount.multiply_ratio(
            10_000u128 - u128::from(config.quote_slippage_bps),
            10_000u128,
        );
        (
            Some(token),
            Some(amount),
            Some(execution_floor.max(quote_floor)),
        )
    } else {
        (None, None, None)
    };
    Ok(crate::msg::GridStatusResponse {
        current_cell,
        target_weight_bps: target_weight_bps(current_cell, config),
        allocation_deviation_bps: deviation_bps,
        should_rebalance,
        captured_twap,
        balances: holdings,
        offer_token,
        amount,
        min_return,
        pending_swap: false,
    })
}

fn reference_price(deps: Deps, _env: &Env, pair: &Addr, window: u32) -> StdResult<Decimal> {
    let response: crate::msg::ObserveResponse = deps.querier.query_wasm_smart(
        pair,
        &PairQueryMsg::Observe {
            seconds_ago: vec![0, window],
        },
    )?;
    if response.price_a_cumulatives.len() != 2
        || response.price_a_cumulatives[0] <= response.price_a_cumulatives[1]
    {
        return Err(StdError::generic_err("empty TWAP history"));
    }
    twap_from_observation(&response, window)
}

fn twap_from_observation(
    response: &crate::msg::ObserveResponse,
    window: u32,
) -> StdResult<Decimal> {
    let difference = response.price_a_cumulatives[0] - response.price_a_cumulatives[1];
    let atomics = difference.checked_div(Uint128::from(window))?;
    let price = Decimal::from_atomics(atomics, 18)
        .map_err(|error| StdError::generic_err(error.to_string()))?;
    if price.is_zero() {
        return Err(StdError::generic_err("empty TWAP price"));
    }
    Ok(price)
}

fn grid_cell(price: Decimal, lower_price: Decimal, upper_price: Decimal, grid_count: u32) -> u32 {
    if price <= lower_price {
        return 0;
    }
    if price >= upper_price {
        return grid_count;
    }
    let span = upper_price - lower_price;
    let offset = price - lower_price;
    let step = span
        .checked_div(Decimal::from_ratio(grid_count, 1u8))
        .unwrap_or(Decimal::zero());
    if step.is_zero() {
        return 0;
    }
    let cell = offset.checked_div(step).unwrap_or(Decimal::zero());
    let cell_u128 = cell.to_uint_floor();
    cell_u128.min(Uint128::from(grid_count)).u128() as u32
}

fn target_weight_bps(cell: u32, config: &Config) -> u16 {
    if config.grid_count == 0 {
        return 0;
    }
    let weight = Decimal::from_ratio(cell, config.grid_count);
    (weight * Decimal::from_ratio(10_000u128, 1u8))
        .to_uint_floor()
        .u128()
        .min(10_000) as u16
}

fn allocation_deviation(holdings: [Uint128; 2], price: Decimal, config: &Config) -> StdResult<u16> {
    if holdings[0].is_zero() && holdings[1].is_zero() {
        return Ok(0);
    }
    let cell = grid_cell(
        price,
        config.lower_price,
        config.upper_price,
        config.grid_count,
    );
    let weight = Decimal::from_ratio(cell, config.grid_count);
    let total_value = Uint256::from(holdings[0])
        .checked_mul(Uint256::from(price.atomics()))?
        .checked_add(
            Uint256::from(holdings[1]).checked_mul(Uint256::from(Decimal::one().atomics()))?,
        )?;
    let target = total_value
        .checked_mul(Uint256::from(weight.atomics()))?
        .checked_div(Uint256::from(Decimal::one().atomics()))?;
    let actual = Uint256::from(holdings[1]).checked_mul(Uint256::from(Decimal::one().atomics()))?;
    ratio_deviation(actual, target)
}

fn planned_offer(
    holdings: [Uint128; 2],
    price: Decimal,
    cell: u32,
    grid_count: u32,
    max_trade_bps: u16,
) -> StdResult<Option<(usize, Uint128)>> {
    let weight = Decimal::from_ratio(cell, grid_count.max(1));
    let token0_value = Uint256::from(holdings[0]) * Uint256::from(price.atomics());
    let token1_value = Uint256::from(holdings[1]) * Uint256::from(Decimal::one().atomics());
    let total_value = token0_value + token1_value;
    let target_token1 =
        total_value * Uint256::from(weight.atomics()) / Uint256::from(Decimal::one().atomics());
    let (index, uncapped) = match token1_value.cmp(&target_token1) {
        std::cmp::Ordering::Greater => {
            let amount = (token1_value - target_token1)
                / Uint256::from(Decimal::one().atomics())
                / Uint256::from(2u8);
            (1, amount)
        }
        std::cmp::Ordering::Less => {
            let amount = (target_token1 - token1_value)
                / Uint256::from(price.atomics())
                / Uint256::from(2u8);
            (0, amount)
        }
        std::cmp::Ordering::Equal => {
            return Ok(None);
        }
    };
    let cap = holdings[index].multiply_ratio(max_trade_bps, 10_000u16);
    let uncapped: Uint128 = uncapped
        .try_into()
        .map_err(|_| StdError::generic_err("rebalance amount overflow"))?;
    let amount = uncapped.min(cap);
    Ok((!amount.is_zero()).then_some((index, amount)))
}

fn expected_return(amount: Uint128, offer_index: usize, price: Decimal) -> StdResult<Uint128> {
    if offer_index == 0 {
        checked_ratio(amount, price.atomics(), Decimal::one().atomics())
    } else {
        checked_ratio(amount, Decimal::one().atomics(), price.atomics())
    }
}

fn relative_deviation(current: Decimal, reference: Decimal) -> StdResult<u16> {
    ratio_deviation(
        Uint256::from(current.atomics()),
        Uint256::from(reference.atomics()),
    )
}

fn ratio_deviation(actual: Uint256, expected: Uint256) -> StdResult<u16> {
    if expected.is_zero() {
        if actual.is_zero() {
            return Ok(0);
        }
        return Err(StdError::generic_err("empty reference"));
    }
    let difference = if actual >= expected {
        actual - expected
    } else {
        expected - actual
    };
    if difference >= expected {
        return Ok(10_000);
    }
    let value = difference
        .checked_multiply_ratio(Uint256::from(10_000u16), expected)
        .map_err(|_| StdError::generic_err("deviation overflow"))?
        .min(Uint256::from(10_000u16));
    value
        .to_string()
        .parse()
        .map_err(|_| StdError::generic_err("deviation overflow"))
}

fn balances(deps: Deps, vault: &Addr, config: &Config) -> StdResult<[Uint128; 2]> {
    Ok([
        cw20_balance(deps, &config.asset_tokens[0], vault)?,
        cw20_balance(deps, &config.asset_tokens[1], vault)?,
    ])
}

fn cw20_balance(deps: Deps, token: &Addr, account: &Addr) -> StdResult<Uint128> {
    let response: BalanceResponse = deps.querier.query_wasm_smart(
        token,
        &Cw20QueryMsg::Balance {
            address: account.to_string(),
        },
    )?;
    Ok(response.balance)
}

fn swap_message(
    config: &Config,
    offer_token: &str,
    amount: Uint128,
    min_return: Uint128,
    deadline: u64,
) -> StdResult<WasmMsg> {
    let hook = PairCw20HookMsg::Swap {
        belief_price: None,
        max_spread: Some(config.max_spread),
        min_return: Some(min_return),
        to: None,
        deadline: Some(deadline),
        trader: None,
        hybrid: Some(HybridSwapParams::pool_only(amount)),
    };
    Ok(WasmMsg::Execute {
        contract_addr: offer_token.to_string(),
        msg: to_json_binary(&Cw20ExecuteMsg::Send {
            contract: config.pair.to_string(),
            amount,
            msg: to_json_binary(&hook)?,
        })?,
        funds: vec![],
    })
}

fn validate_settlement(pending: &PendingSwap, settled: [Uint128; 2]) -> Result<(), ContractError> {
    let offer = pending.offer_index as usize;
    let ask = 1 - offer;
    let expected_offer = pending.balances[offer]
        .checked_sub(pending.amount)
        .map_err(StdError::overflow)?;
    let minimum_ask = pending.balances[ask]
        .checked_add(pending.min_return)
        .map_err(StdError::overflow)?;
    if settled[offer] != expected_offer || settled[ask] < minimum_ask {
        return Err(ContractError::AllocationDidNotImprove);
    }
    Ok(())
}

fn validate_rebalance_outcome(
    previous: u16,
    current: u16,
    tolerance: u16,
) -> Result<bool, ContractError> {
    if current >= previous && current > tolerance {
        return Err(ContractError::AllocationDidNotImprove);
    }
    Ok(current <= tolerance)
}

fn validate_pool_safety(
    pool: &PoolResponse,
    twap: Decimal,
    offer_index: usize,
    amount: Uint128,
    max_spot_twap_deviation_bps: u16,
    max_trade_pool_bps: u16,
) -> Result<(), ContractError> {
    let reserves = [pool.assets[0].amount, pool.assets[1].amount];
    if reserves[0].is_zero() || reserves[1].is_zero() {
        return Err(ContractError::InsufficientPoolDepth);
    }
    let spot_atomics = checked_ratio(reserves[1], Decimal::one().atomics(), reserves[0])?;
    let spot = Decimal::from_atomics(spot_atomics, Decimal::DECIMAL_PLACES)
        .map_err(|_| ContractError::Std(StdError::generic_err("spot price overflow")))?;
    if relative_deviation(spot, twap)? > max_spot_twap_deviation_bps {
        return Err(ContractError::UnsafePoolPrice);
    }
    if Uint256::from(amount) * Uint256::from(10_000u16)
        > Uint256::from(reserves[offer_index]) * Uint256::from(max_trade_pool_bps)
    {
        return Err(ContractError::InsufficientPoolDepth);
    }
    Ok(())
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

fn token_addr(deps: Deps, info: crate::msg::AssetInfo) -> Result<Addr, ContractError> {
    match info {
        crate::msg::AssetInfo::Token { contract_addr } => {
            Ok(deps.api.addr_validate(&contract_addr)?)
        }
        crate::msg::AssetInfo::NativeToken { .. } => Err(ContractError::InvalidPair),
    }
}

fn valid_twap_window(value: u32) -> bool {
    (1..=MAX_TWAP_WINDOW_SECONDS).contains(&value)
}

fn validate_allocation_tolerance(value: u16) -> Result<(), ContractError> {
    if value == 0 || value > MAX_ALLOCATION_TOLERANCE_BPS {
        return Err(ContractError::InvalidRiskControl);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_risk_controls(
    max_trade_bps: u16,
    max_execution_deviation_bps: u16,
    quote_slippage_bps: u16,
    max_spot_twap_deviation_bps: u16,
    max_trade_pool_bps: u16,
    max_spread: Decimal,
) -> Result<(), ContractError> {
    if max_trade_bps == 0
        || max_trade_bps > MAX_TRADE_BPS
        || max_execution_deviation_bps > MAX_EXECUTION_DEVIATION_BPS
        || quote_slippage_bps > MAX_QUOTE_SLIPPAGE_BPS
        || max_spot_twap_deviation_bps == 0
        || max_spot_twap_deviation_bps > MAX_SPOT_TWAP_DEVIATION_BPS
        || max_trade_pool_bps == 0
        || max_trade_pool_bps > MAX_TRADE_POOL_BPS
        || max_spread.is_zero()
        || max_spread > MAX_SPREAD
    {
        return Err(ContractError::InvalidRiskControl);
    }
    Ok(())
}

fn assert_admin(config: &Config, sender: &Addr) -> Result<(), ContractError> {
    if sender != config.admin {
        return Err(ContractError::Unauthorized);
    }
    Ok(())
}

fn assert_not_paused(deps: Deps) -> Result<(), ContractError> {
    if PAUSED.may_load(deps.storage)?.unwrap_or(true) {
        return Err(ContractError::Paused);
    }
    Ok(())
}

fn assert_no_funds(info: &MessageInfo) -> Result<(), ContractError> {
    if info.funds.is_empty() {
        Ok(())
    } else {
        Err(ContractError::Unauthorized)
    }
}

fn deposit_shares(
    amount: Uint128,
    token_index: usize,
    price: Decimal,
    balances: &[Uint128; 2],
    total_shares: Uint128,
) -> Result<Uint128, ContractError> {
    let deposit_value_t0 = deposit_value(amount, token_index, price)?;
    if total_shares.is_zero() {
        return Ok(deposit_value_t0);
    }
    let prior_token_0 = if token_index == 0 {
        balances[0]
            .checked_sub(amount)
            .map_err(StdError::overflow)?
    } else {
        balances[0]
    };
    let prior_token_1 = if token_index == 1 {
        balances[1]
            .checked_sub(amount)
            .map_err(StdError::overflow)?
    } else {
        balances[1]
    };
    let token_1_value_t0 = checked_ratio(prior_token_1, Decimal::one().atomics(), price.atomics())
        .map_err(ContractError::from)?;
    let vault_value_t0 = prior_token_0
        .checked_add(token_1_value_t0)
        .map_err(StdError::overflow)?;
    if vault_value_t0.is_zero() {
        return Ok(deposit_value_t0);
    }
    Ok(deposit_value_t0
        .checked_multiply_ratio(total_shares, vault_value_t0)
        .map_err(|_| StdError::generic_err("share mint overflow"))?)
}

fn deposit_value(
    amount: Uint128,
    token_index: usize,
    price: Decimal,
) -> Result<Uint128, ContractError> {
    if token_index == 0 {
        Ok(amount)
    } else {
        if price.is_zero() {
            return Err(ContractError::InvalidGrid);
        }
        checked_ratio(amount, Decimal::one().atomics(), price.atomics())
            .map_err(ContractError::from)
    }
}

fn from_json<T>(value: &[u8]) -> StdResult<T>
where
    T: serde::de::DeserializeOwned,
{
    cosmwasm_std::from_json(value)
}

pub fn migrate(_deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    Ok(Response::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::{Asset, AssetInfo};
    use std::str::FromStr;

    #[test]
    fn ratio_deviation_uses_reference_basis() {
        assert_eq!(
            ratio_deviation(Uint256::from(45u8), Uint256::from(55u8)).unwrap(),
            1_818
        );
        assert_eq!(
            ratio_deviation(Uint256::from(95u8), Uint256::from(100u8)).unwrap(),
            500
        );
    }

    #[test]
    fn grid_cell_clamps_to_bounds() {
        let config = Config {
            admin: Addr::unchecked("admin"),
            pending_admin: None,
            pair: Addr::unchecked("pair"),
            asset_tokens: [Addr::unchecked("t0"), Addr::unchecked("t1")],
            decimals: 6,
            twap_window_seconds: 300,
            grid_count: 4,
            lower_price: Decimal::from_atomics(100u128, 0).unwrap(),
            upper_price: Decimal::from_atomics(200u128, 0).unwrap(),
            allocation_tolerance_bps: 100,
            max_trade_bps: 2_500,
            max_execution_deviation_bps: 500,
            quote_slippage_bps: 200,
            max_spot_twap_deviation_bps: 500,
            max_trade_pool_bps: 1_000,
            max_spread: Decimal::percent(5),
            reference_price: Decimal::from_atomics(150u128, 0).unwrap(),
            last_cell: 2,
        };
        assert_eq!(
            grid_cell(
                Decimal::from_atomics(99u128, 0).unwrap(),
                config.lower_price,
                config.upper_price,
                config.grid_count
            ),
            0
        );
        assert_eq!(
            grid_cell(
                Decimal::from_atomics(201u128, 0).unwrap(),
                config.lower_price,
                config.upper_price,
                config.grid_count
            ),
            4
        );
        assert_eq!(
            grid_cell(
                Decimal::from_atomics(150u128, 0).unwrap(),
                config.lower_price,
                config.upper_price,
                config.grid_count
            ),
            2
        );
        assert_eq!(
            grid_cell(
                Decimal::from_atomics(175u128, 0).unwrap(),
                config.lower_price,
                config.upper_price,
                config.grid_count
            ),
            3
        );
        assert_eq!(
            grid_cell(
                Decimal::from_atomics(100u128, 0).unwrap(),
                config.lower_price,
                config.upper_price,
                config.grid_count
            ),
            0
        );
        assert_eq!(
            grid_cell(
                Decimal::from_atomics(200u128, 0).unwrap(),
                config.lower_price,
                config.upper_price,
                config.grid_count
            ),
            4
        );
    }

    #[test]
    fn target_weight_is_linear_across_cells() {
        assert_eq!(target_weight_bps(0, &cell_config(4)), 0);
        assert_eq!(target_weight_bps(2, &cell_config(4)), 5_000);
        assert_eq!(target_weight_bps(4, &cell_config(4)), 10_000);
        assert_eq!(target_weight_bps(1, &cell_config(10)), 1_000);
    }

    #[test]
    fn target_weight_clamps_at_ten_thousand_bps() {
        assert_eq!(target_weight_bps(99, &cell_config(4)), 10_000);
        assert_eq!(target_weight_bps(4, &cell_config(4)), 10_000);
    }

    #[test]
    fn planned_offer_caps_at_max_trade_bps() {
        // 60t0 + 60t1 at price 1.75, cell 3 -> target 75% token1.
        // Uncap = (123.75 - 60) / 1.75 / 2 = 18.21e9, but max_trade_bps 2500
        // caps it to 25% of the 60e9 token0 balance = 15e9.
        let holdings = [Uint128::new(60_000_000_000), Uint128::new(60_000_000_000)];
        let (index, amount) =
            planned_offer(holdings, Decimal::from_str("1.75").unwrap(), 3, 4, 2_500)
                .unwrap()
                .unwrap();
        assert_eq!(index, 0);
        assert_eq!(amount, Uint128::new(15_000_000_000));
    }

    #[test]
    fn planned_offer_skips_when_balanced() {
        // 60t0 + 90t1 at price 1.5, cell 2 -> target 50% token1 = 90e9.
        let holdings = [Uint128::new(60_000_000_000), Uint128::new(90_000_000_000)];
        assert!(
            planned_offer(holdings, Decimal::from_str("1.5").unwrap(), 2, 4, 2_500)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn allocation_deviation_is_bounded_and_relative() {
        // cell_config uses prices 100-200; price 150 is cell 2 (50% token1).
        let config = cell_config(4);
        // Perfectly balanced at price 150 -> 0 bps.
        assert_eq!(
            allocation_deviation(
                [Uint128::new(1_000_000_000), Uint128::new(150_000_000_000)],
                Decimal::from_atomics(150u128, 0).unwrap(),
                &config,
            )
            .unwrap(),
            0
        );
        // All token1 with a 50% target -> 10_000 bps (100% deviation).
        assert_eq!(
            allocation_deviation(
                [Uint128::zero(), Uint128::new(100_000_000_000)],
                Decimal::from_atomics(150u128, 0).unwrap(),
                &config,
            )
            .unwrap(),
            10_000
        );
    }

    #[test]
    fn validate_pool_safety_rejects_spot_twap_drift() {
        let pool = PoolResponse {
            assets: [
                Asset {
                    info: AssetInfo::Token {
                        contract_addr: "t0".to_string(),
                    },
                    amount: Uint128::new(1_000_000_000_000),
                },
                Asset {
                    info: AssetInfo::Token {
                        contract_addr: "t1".to_string(),
                    },
                    amount: Uint128::new(1_200_000_000_000),
                },
            ],
            total_share: Uint128::new(1_000_000),
        };
        // Spot 1.2 vs TWAP 1.5 is a 20% drift, beyond the 500 bps bound.
        let err = validate_pool_safety(
            &pool,
            Decimal::from_str("1.5").unwrap(),
            0,
            Uint128::new(10_000_000_000),
            500,
            1_000,
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::UnsafePoolPrice));
    }

    #[test]
    fn validate_pool_safety_rejects_oversized_trade() {
        let pool = PoolResponse {
            assets: [
                Asset {
                    info: AssetInfo::Token {
                        contract_addr: "t0".to_string(),
                    },
                    amount: Uint128::new(1_000_000_000),
                },
                Asset {
                    info: AssetInfo::Token {
                        contract_addr: "t1".to_string(),
                    },
                    amount: Uint128::new(1_500_000_000),
                },
            ],
            total_share: Uint128::new(1_000_000),
        };
        // Trade of 300e6 against a 1e9 reserve exceeds the 1_000 bps pool cap.
        let err = validate_pool_safety(
            &pool,
            Decimal::from_str("1.5").unwrap(),
            0,
            Uint128::new(300_000_000),
            500,
            1_000,
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::InsufficientPoolDepth));
    }

    #[test]
    fn validate_rebalance_outcome_requires_improvement() {
        // Worsening allocation is always rejected, even under the tolerance.
        assert!(matches!(
            validate_rebalance_outcome(100, 400, 300),
            Err(ContractError::AllocationDidNotImprove)
        ));
        // A worsening but tiny move inside the tolerance is accepted.
        assert!(validate_rebalance_outcome(100, 250, 300).unwrap());
        // Improvement that lands inside the tolerance is accepted.
        assert!(validate_rebalance_outcome(300, 100, 200).unwrap());
        // Improvement that stays outside the tolerance is not accepted.
        assert!(!validate_rebalance_outcome(300, 250, 200).unwrap());
    }

    fn cell_config(grid_count: u32) -> Config {
        Config {
            admin: Addr::unchecked("admin"),
            pending_admin: None,
            pair: Addr::unchecked("pair"),
            asset_tokens: [Addr::unchecked("t0"), Addr::unchecked("t1")],
            decimals: 6,
            twap_window_seconds: 300,
            grid_count,
            lower_price: Decimal::from_atomics(100u128, 0).unwrap(),
            upper_price: Decimal::from_atomics(200u128, 0).unwrap(),
            allocation_tolerance_bps: 100,
            max_trade_bps: 2_500,
            max_execution_deviation_bps: 500,
            quote_slippage_bps: 200,
            max_spot_twap_deviation_bps: 500,
            max_trade_pool_bps: 1_000,
            max_spread: Decimal::percent(5),
            reference_price: Decimal::from_atomics(150u128, 0).unwrap(),
            last_cell: 2,
        }
    }
}
