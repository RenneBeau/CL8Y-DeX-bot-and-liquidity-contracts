use std::collections::BTreeSet;

use cosmwasm_std::{
    entry_point, from_json, to_json_binary, Addr, BankMsg, Binary, Coin, Decimal, Deps, DepsMut,
    Env, Fraction, MessageInfo, Order, Reply, Response, StdError, StdResult, SubMsg, Uint128,
    WasmMsg,
};
use cw2::set_contract_version;
use cw20::{Cw20ExecuteMsg, Cw20QueryMsg, Cw20ReceiveMsg, TokenInfoResponse};
use cw_storage_plus::Bound;

use crate::error::ContractError;
use crate::msg::{
    AssetInfo, BotResponse, ConfigResponse, ExecuteMsg, ExpiredLimitRefundResponse,
    FactoryQueryMsg, InstantiateMsg, LimitOrderConfigResponse, LimitOrderPlacementItem,
    LimitOrderResponse, LimitOrderSide, OrderFillReport, OrderResponse, PairCw20HookMsg,
    PairExecuteMsg, PairInfo, PairQueryMsg, PairResponse, PoolResponse, QueryMsg, ReceiveMsg,
    RungResponse, ShareResponse,
};
use crate::state::{
    Bot, Config, GridOrder, PlacementPlan, Rung, BOTS, CONFIG, NEXT_BOT_ID, NEXT_REPLY_ID, ORDERS,
    PLACEMENTS, RUNGS, SHARES,
};

const CONTRACT_NAME: &str = "crates.io:cl8y-grid-manager-experimental";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
const FIRST_REPLY_ID: u64 = 1;
const MAX_ADJUST_STEPS: u32 = 64;
const MAX_GRID_COUNT: u32 = 100;
const MAX_ORDERS_PER_RECONCILE: u32 = 100;
const MAX_ACTIVE_ORDERS_PER_BOT: u32 = 500;

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    if msg.gas_denom.trim().is_empty()
        || msg.max_grid_count < 2
        || msg.max_grid_count > MAX_GRID_COUNT
        || msg.max_orders_per_reconcile == 0
        || msg.max_orders_per_reconcile > MAX_ORDERS_PER_RECONCILE
        || msg.max_active_orders_per_bot < msg.max_grid_count
        || msg.max_active_orders_per_bot > MAX_ACTIVE_ORDERS_PER_BOT
        || msg.keeper_reward.is_zero()
    {
        return Err(ContractError::InvalidGrid);
    }
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    CONFIG.save(
        deps.storage,
        &Config {
            admin: deps.api.addr_validate(&msg.admin)?,
            keeper: deps.api.addr_validate(&msg.keeper)?,
            factory: deps.api.addr_validate(&msg.factory)?,
            gas_denom: msg.gas_denom,
            keeper_reward: msg.keeper_reward,
            minimum_gas_reserve: msg.minimum_gas_reserve,
            max_grid_count: msg.max_grid_count,
            max_orders_per_reconcile: msg.max_orders_per_reconcile,
            max_active_orders_per_bot: msg.max_active_orders_per_bot,
        },
    )?;
    NEXT_BOT_ID.save(deps.storage, &1)?;
    NEXT_REPLY_ID.save(deps.storage, &FIRST_REPLY_ID)?;
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
        ExecuteMsg::CreateBot {
            pair,
            lower_price,
            upper_price,
            grid_count,
        } => execute_create_bot(deps, env, info, pair, lower_price, upper_price, grid_count),
        ExecuteMsg::Receive(receive) => execute_receive(deps, info, receive),
        ExecuteMsg::FundGas { bot_id } => execute_fund_gas(deps, info, bot_id),
        ExecuteMsg::WithdrawGas {
            bot_id,
            amount,
            recipient,
        } => execute_withdraw_gas(deps, info, bot_id, amount, recipient),
        ExecuteMsg::Allocate { bot_id } => execute_allocate(deps, info, bot_id),
        ExecuteMsg::Reconcile { bot_id, reports } => {
            execute_reconcile(deps, env, info, bot_id, reports)
        }
        ExecuteMsg::CancelAll { bot_id } => execute_cancel_all(deps, env, info, bot_id),
        ExecuteMsg::Withdraw {
            bot_id,
            shares,
            recipient,
        } => execute_withdraw(deps, info, bot_id, shares, recipient),
        ExecuteMsg::UpdateKeeper { keeper } => execute_update_keeper(deps, info, keeper),
    }
}

fn execute_create_bot(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    pair: String,
    lower_price: Decimal,
    upper_price: Decimal,
    grid_count: u32,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if grid_count < 2 || grid_count > config.max_grid_count || lower_price >= upper_price {
        return Err(ContractError::InvalidGrid);
    }
    let pair = deps.api.addr_validate(&pair)?;
    let pair_info: PairInfo = deps
        .querier
        .query_wasm_smart(&pair, &PairQueryMsg::Pair {})?;
    if deps.api.addr_validate(&pair_info.contract_addr)? != pair {
        return Err(ContractError::InvalidPair);
    }
    let registered: PairResponse = deps.querier.query_wasm_smart(
        &config.factory,
        &FactoryQueryMsg::Pair {
            asset_infos: pair_info.asset_infos.clone(),
        },
    )?;
    if deps.api.addr_validate(&registered.pair.contract_addr)? != pair {
        return Err(ContractError::InvalidPair);
    }
    let asset_tokens = [
        token_address(deps.as_ref(), &pair_info.asset_infos[0])?,
        token_address(deps.as_ref(), &pair_info.asset_infos[1])?,
    ];
    if asset_tokens[0] == asset_tokens[1] {
        return Err(ContractError::InvalidPair);
    }
    let token_0: TokenInfoResponse = deps
        .querier
        .query_wasm_smart(&asset_tokens[0], &Cw20QueryMsg::TokenInfo {})?;
    let token_1: TokenInfoResponse = deps
        .querier
        .query_wasm_smart(&asset_tokens[1], &Cw20QueryMsg::TokenInfo {})?;
    if token_0.decimals != token_1.decimals {
        return Err(ContractError::InvalidPair);
    }
    let limit_config: LimitOrderConfigResponse = deps
        .querier
        .query_wasm_smart(&pair, &PairQueryMsg::LimitOrderConfig {})?;
    if grid_count > limit_config.max_batch_rungs {
        return Err(ContractError::InvalidGrid);
    }
    let pool: PoolResponse = deps
        .querier
        .query_wasm_smart(&pair, &PairQueryMsg::Pool {})?;
    if pool.assets[0].info != pair_info.asset_infos[0]
        || pool.assets[1].info != pair_info.asset_infos[1]
    {
        return Err(ContractError::InvalidPair);
    }
    let reference_price = pool_price(&pool)?;
    if reference_price <= lower_price || reference_price >= upper_price {
        return Err(ContractError::InvalidGrid);
    }
    let prices = grid_prices(lower_price, upper_price, grid_count)?;
    let minimum_price = Decimal::from_atomics(Uint128::new(1_000_000_000), 18)
        .map_err(|_| ContractError::InvalidGrid)?;
    let maximum_price = Decimal::from_ratio(1_000_000_000u128, 1u128);
    if prices
        .iter()
        .any(|price| *price < minimum_price || *price > maximum_price)
        || prices.windows(2).any(|window| window[0] == window[1])
    {
        return Err(ContractError::InvalidGrid);
    }
    let sides: Vec<Option<LimitOrderSide>> = prices
        .iter()
        .map(|price| {
            if *price < reference_price {
                Some(LimitOrderSide::Bid)
            } else if *price > reference_price {
                Some(LimitOrderSide::Ask)
            } else {
                None
            }
        })
        .collect();
    if !sides.contains(&Some(LimitOrderSide::Bid)) || !sides.contains(&Some(LimitOrderSide::Ask)) {
        return Err(ContractError::EmptySide);
    }
    let gas_credit = paid_amount(&info, &config.gas_denom)?;
    if gas_credit
        < config
            .minimum_gas_reserve
            .checked_add(config.keeper_reward)?
    {
        return Err(ContractError::InsufficientGasCredit);
    }
    let bot_id = NEXT_BOT_ID.load(deps.storage)?;
    NEXT_BOT_ID.save(
        deps.storage,
        &bot_id
            .checked_add(1)
            .ok_or_else(|| StdError::generic_err("bot id overflow"))?,
    )?;
    BOTS.save(
        deps.storage,
        bot_id,
        &Bot {
            owner: info.sender.clone(),
            pair,
            asset_tokens,
            lower_price,
            upper_price,
            grid_count,
            reference_price,
            free_balances: [Uint128::zero(), Uint128::zero()],
            total_shares: Uint128::zero(),
            gas_credit,
            active_orders: 0,
            pair_batch_limit: limit_config.max_batch_rungs,
        },
    )?;
    for (index, (price, side)) in prices.into_iter().zip(sides).enumerate() {
        RUNGS.save(deps.storage, (bot_id, index as u32), &Rung { price, side })?;
    }
    Ok(Response::new()
        .add_attribute("action", "create_grid_bot")
        .add_attribute("bot_id", bot_id.to_string())
        .add_attribute("owner", info.sender)
        .add_attribute("gas_credit", gas_credit))
}

fn execute_receive(
    mut deps: DepsMut,
    info: MessageInfo,
    receive: Cw20ReceiveMsg,
) -> Result<Response, ContractError> {
    if receive.amount.is_zero() {
        return Err(ContractError::ZeroAmount);
    }
    let hook: ReceiveMsg = from_json(receive.msg)?;
    match hook {
        ReceiveMsg::Deposit { bot_id } => {
            let depositor = deps.api.addr_validate(&receive.sender)?;
            let mut bot = BOTS.load(deps.storage, bot_id)?;
            if depositor != bot.owner {
                return Err(ContractError::Unauthorized);
            }
            let config = CONFIG.load(deps.storage)?;
            if bot.gas_credit
                < config
                    .minimum_gas_reserve
                    .checked_add(config.keeper_reward)?
            {
                return Err(ContractError::InsufficientGasCredit);
            }
            let token_index = bot
                .asset_tokens
                .iter()
                .position(|token| token == info.sender)
                .ok_or(ContractError::UnsupportedToken)?;
            bot.free_balances[token_index] =
                bot.free_balances[token_index].checked_add(receive.amount)?;
            let minted = deposit_shares(receive.amount, token_index, bot.reference_price)?;
            if minted.is_zero() {
                return Err(ContractError::ZeroAmount);
            }
            bot.total_shares = bot.total_shares.checked_add(minted)?;
            let shares = SHARES
                .may_load(deps.storage, (bot_id, &depositor))?
                .unwrap_or_default()
                .checked_add(minted)?;
            SHARES.save(deps.storage, (bot_id, &depositor), &shares)?;
            BOTS.save(deps.storage, bot_id, &bot)?;
            let response = Response::new()
                .add_attribute("action", "deposit_grid_bot")
                .add_attribute("bot_id", bot_id.to_string())
                .add_attribute("amount", receive.amount)
                .add_attribute("shares", minted);
            let side = if token_index == 0 {
                LimitOrderSide::Ask
            } else {
                LimitOrderSide::Bid
            };
            allocate_side(deps.branch(), response, bot_id, &bot, side)
        }
    }
}

fn execute_fund_gas(
    deps: DepsMut,
    info: MessageInfo,
    bot_id: u64,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let amount = paid_amount(&info, &config.gas_denom)?;
    if amount.is_zero() {
        return Err(ContractError::MissingGasFunds);
    }
    BOTS.update(deps.storage, bot_id, |bot| -> Result<_, ContractError> {
        let mut bot = bot.ok_or_else(|| StdError::not_found("bot"))?;
        bot.gas_credit = bot.gas_credit.checked_add(amount)?;
        Ok(bot)
    })?;
    Ok(Response::new()
        .add_attribute("action", "fund_grid_gas")
        .add_attribute("bot_id", bot_id.to_string())
        .add_attribute("amount", amount))
}

fn execute_withdraw_gas(
    deps: DepsMut,
    info: MessageInfo,
    bot_id: u64,
    amount: Uint128,
    recipient: Option<String>,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    if amount.is_zero() {
        return Err(ContractError::ZeroAmount);
    }
    let config = CONFIG.load(deps.storage)?;
    let recipient = deps
        .api
        .addr_validate(recipient.as_deref().unwrap_or(info.sender.as_str()))?;
    BOTS.update(deps.storage, bot_id, |bot| -> Result<_, ContractError> {
        let mut bot = bot.ok_or_else(|| StdError::not_found("bot"))?;
        if info.sender != bot.owner {
            return Err(ContractError::Unauthorized);
        }
        let remaining = bot.gas_credit.checked_sub(amount)?;
        let required = config
            .minimum_gas_reserve
            .checked_add(config.keeper_reward)?;
        if (bot.active_orders != 0 || !bot.total_shares.is_zero()) && remaining < required {
            return Err(ContractError::InsufficientGasCredit);
        }
        bot.gas_credit = remaining;
        Ok(bot)
    })?;
    Ok(Response::new()
        .add_message(BankMsg::Send {
            to_address: recipient.to_string(),
            amount: vec![Coin::new(amount.u128(), config.gas_denom)],
        })
        .add_attribute("action", "withdraw_grid_gas")
        .add_attribute("bot_id", bot_id.to_string())
        .add_attribute("amount", amount))
}

fn execute_allocate(
    mut deps: DepsMut,
    info: MessageInfo,
    bot_id: u64,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    let bot = BOTS.load(deps.storage, bot_id)?;
    if info.sender != bot.owner {
        return Err(ContractError::Unauthorized);
    }
    let mut response = Response::new()
        .add_attribute("action", "allocate_grid")
        .add_attribute("bot_id", bot_id.to_string());
    response = allocate_side(deps.branch(), response, bot_id, &bot, LimitOrderSide::Bid)?;
    response = allocate_side(deps, response, bot_id, &bot, LimitOrderSide::Ask)?;
    if response.messages.is_empty() {
        return Err(ContractError::NothingToAllocate);
    }
    Ok(response)
}

fn allocate_side(
    deps: DepsMut,
    response: Response,
    bot_id: u64,
    bot: &Bot,
    side: LimitOrderSide,
) -> Result<Response, ContractError> {
    let rungs = side_rungs(deps.as_ref(), bot_id, bot.grid_count, side.clone())?;
    if rungs.is_empty() {
        return Err(ContractError::EmptySide);
    }
    let token_index = match side {
        LimitOrderSide::Ask => 0,
        LimitOrderSide::Bid => 1,
    };
    let current = BOTS.load(deps.storage, bot_id)?;
    let amount_each = current.free_balances[token_index].multiply_ratio(1u128, rungs.len() as u128);
    add_placement(deps, response, bot_id, bot, side, rungs, amount_each)
}

fn execute_reconcile(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    bot_id: u64,
    reports: Vec<OrderFillReport>,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    let config = CONFIG.load(deps.storage)?;
    let mut bot = BOTS.load(deps.storage, bot_id)?;
    if info.sender != config.keeper {
        return Err(ContractError::Unauthorized);
    }
    if reports.is_empty() || reports.len() > config.max_orders_per_reconcile as usize {
        return Err(ContractError::InvalidFillReport);
    }
    if bot.gas_credit
        < config
            .minimum_gas_reserve
            .checked_add(config.keeper_reward)?
    {
        return Err(ContractError::InsufficientGasCredit);
    }
    let mut seen = BTreeSet::new();
    let mut placements: Vec<(LimitOrderSide, u32, Uint128)> = vec![];
    let mut parked_claims = vec![];
    for report in &reports {
        if deps.api.addr_validate(&report.pair)? != bot.pair {
            return Err(ContractError::InvalidFillReport);
        }
        if !seen.insert(report.order_id) {
            return Err(ContractError::InvalidFillReport);
        }
        let mut order = ORDERS.load(deps.storage, (bot_id, report.order_id))?;
        let active: StdResult<LimitOrderResponse> = deps.querier.query_wasm_smart(
            &bot.pair,
            &PairQueryMsg::LimitOrder {
                order_id: report.order_id,
            },
        );
        let (current_remaining, terminal, parked_refund) = match active {
            Ok(on_chain) => {
                validate_order(&env.contract.address, report.order_id, &order, &on_chain)?;
                (on_chain.remaining, false, Uint128::zero())
            }
            Err(_) => {
                let parked: Option<ExpiredLimitRefundResponse> = deps.querier.query_wasm_smart(
                    &bot.pair,
                    &PairQueryMsg::ExpiredLimitRefund {
                        order_id: report.order_id,
                    },
                )?;
                if let Some(refund) = parked {
                    if refund.owner != env.contract.address
                        || refund.order_id != report.order_id
                        || refund.side != order.side
                        || refund.remaining > order.remaining
                    {
                        return Err(ContractError::InvalidOrder);
                    }
                    (refund.remaining, true, refund.remaining)
                } else {
                    (Uint128::zero(), true, Uint128::zero())
                }
            }
        };
        let consumed = order.remaining.checked_sub(current_remaining)?;
        let (reported_input, output) = validate_indexed_report(&order, report, terminal)?;
        if reported_input != consumed || (consumed.is_zero() && !terminal) {
            return Err(ContractError::InvalidFillReport);
        }
        let (output_index, opposite, opposite_rung) = match order.side {
            LimitOrderSide::Ask => (
                1usize,
                LimitOrderSide::Bid,
                order.rung_index.saturating_sub(1),
            ),
            LimitOrderSide::Bid => (
                0usize,
                LimitOrderSide::Ask,
                (order.rung_index + 1).min(bot.grid_count - 1),
            ),
        };
        bot.free_balances[output_index] = bot.free_balances[output_index].checked_add(output)?;
        if terminal {
            if !parked_refund.is_zero() {
                let input_index = match order.side {
                    LimitOrderSide::Ask => 0,
                    LimitOrderSide::Bid => 1,
                };
                bot.free_balances[input_index] =
                    bot.free_balances[input_index].checked_add(parked_refund)?;
                parked_claims.push(report.order_id);
            }
            ORDERS.remove(deps.storage, (bot_id, report.order_id));
            bot.active_orders = bot
                .active_orders
                .checked_sub(1)
                .ok_or(ContractError::InvalidOrder)?;
        } else {
            order.remaining = current_remaining;
            ORDERS.save(deps.storage, (bot_id, report.order_id), &order)?;
        }
        if !output.is_zero() {
            placements.push((opposite, opposite_rung, output));
        }
    }
    BOTS.save(deps.storage, bot_id, &bot)?;
    let mut response = Response::new()
        .add_attribute("action", "reconcile_grid")
        .add_attribute("bot_id", bot_id.to_string())
        .add_attribute("changed_orders", reports.len().to_string());
    if !parked_claims.is_empty() {
        let pair_batch_limit = query_pair_batch_limit(deps.as_ref(), &bot.pair)?;
        response = add_pair_batches(response, &bot.pair, &[], &parked_claims, pair_batch_limit)?;
    }
    for (side, rung, amount) in placements {
        let latest = BOTS.load(deps.storage, bot_id)?;
        response = add_placement(
            deps.branch(),
            response,
            bot_id,
            &latest,
            side,
            vec![rung],
            amount,
        )?;
    }
    BOTS.update(deps.storage, bot_id, |bot| -> Result<_, ContractError> {
        let mut bot = bot.ok_or_else(|| StdError::not_found("bot"))?;
        bot.gas_credit = bot.gas_credit.checked_sub(config.keeper_reward)?;
        Ok(bot)
    })?;
    response = response
        .add_message(BankMsg::Send {
            to_address: info.sender.to_string(),
            amount: vec![Coin::new(config.keeper_reward.u128(), config.gas_denom)],
        })
        .add_attribute("keeper_reward", config.keeper_reward);
    Ok(response)
}

fn execute_cancel_all(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    bot_id: u64,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    let mut bot = BOTS.load(deps.storage, bot_id)?;
    if info.sender != bot.owner {
        return Err(ContractError::Unauthorized);
    }
    let tracked: Vec<(u64, GridOrder)> = ORDERS
        .prefix(bot_id)
        .range(deps.storage, None, None, Order::Ascending)
        .take(CONFIG.load(deps.storage)?.max_orders_per_reconcile as usize)
        .collect::<StdResult<_>>()?;
    if tracked.is_empty() {
        return Ok(Response::new().add_attribute("action", "cancel_grid_orders"));
    }
    let processed_orders = tracked.len() as u32;
    let mut cancel_ids = Vec::with_capacity(tracked.len());
    for (order_id, order) in tracked {
        let on_chain: LimitOrderResponse = deps
            .querier
            .query_wasm_smart(&bot.pair, &PairQueryMsg::LimitOrder { order_id })
            .map_err(|_| ContractError::UnsettledOrder)?;
        validate_order(&env.contract.address, order_id, &order, &on_chain)?;
        if on_chain.remaining != order.remaining {
            return Err(ContractError::UnsettledOrder);
        }
        let input_index = match order.side {
            LimitOrderSide::Ask => 0,
            LimitOrderSide::Bid => 1,
        };
        bot.free_balances[input_index] =
            bot.free_balances[input_index].checked_add(on_chain.remaining)?;
        cancel_ids.push(order_id);
        ORDERS.remove(deps.storage, (bot_id, order_id));
    }
    bot.active_orders = bot
        .active_orders
        .checked_sub(processed_orders)
        .ok_or(ContractError::InvalidOrder)?;
    BOTS.save(deps.storage, bot_id, &bot)?;
    let pair_batch_limit = query_pair_batch_limit(deps.as_ref(), &bot.pair)?;
    Ok(add_pair_batches(
        Response::new(),
        &bot.pair,
        &cancel_ids,
        &[],
        pair_batch_limit,
    )?
    .add_attribute("action", "cancel_grid_orders")
    .add_attribute("bot_id", bot_id.to_string())
    .add_attribute("remaining_orders", bot.active_orders.to_string()))
}

fn execute_withdraw(
    deps: DepsMut,
    info: MessageInfo,
    bot_id: u64,
    shares: Uint128,
    recipient: Option<String>,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    let mut bot = BOTS.load(deps.storage, bot_id)?;
    if info.sender != bot.owner {
        return Err(ContractError::Unauthorized);
    }
    if bot.active_orders != 0 {
        return Err(ContractError::ActiveOrders);
    }
    if shares.is_zero() {
        return Err(ContractError::ZeroAmount);
    }
    let owner_shares = SHARES
        .may_load(deps.storage, (bot_id, &info.sender))?
        .unwrap_or_default();
    if shares > owner_shares || shares > bot.total_shares {
        return Err(ContractError::InsufficientShares);
    }
    let amounts = [
        bot.free_balances[0].multiply_ratio(shares, bot.total_shares),
        bot.free_balances[1].multiply_ratio(shares, bot.total_shares),
    ];
    let recipient = deps
        .api
        .addr_validate(recipient.as_deref().unwrap_or(info.sender.as_str()))?;
    let mut response = Response::new()
        .add_attribute("action", "withdraw_grid")
        .add_attribute("bot_id", bot_id.to_string())
        .add_attribute("burned_shares", shares);
    for (index, amount) in amounts.into_iter().enumerate() {
        if !amount.is_zero() {
            bot.free_balances[index] = bot.free_balances[index].checked_sub(amount)?;
            response = response.add_message(WasmMsg::Execute {
                contract_addr: bot.asset_tokens[index].to_string(),
                msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
                    recipient: recipient.to_string(),
                    amount,
                })?,
                funds: vec![],
            });
        }
    }
    bot.total_shares = bot.total_shares.checked_sub(shares)?;
    SHARES.save(
        deps.storage,
        (bot_id, &info.sender),
        &owner_shares.checked_sub(shares)?,
    )?;
    BOTS.save(deps.storage, bot_id, &bot)?;
    Ok(response)
}

fn execute_update_keeper(
    deps: DepsMut,
    info: MessageInfo,
    keeper: String,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    CONFIG.update(deps.storage, |mut config| -> Result<_, ContractError> {
        if info.sender != config.admin {
            return Err(ContractError::Unauthorized);
        }
        config.keeper = deps.api.addr_validate(&keeper)?;
        Ok(config)
    })?;
    Ok(Response::new().add_attribute("action", "update_grid_keeper"))
}

fn add_placement(
    deps: DepsMut,
    response: Response,
    bot_id: u64,
    bot: &Bot,
    side: LimitOrderSide,
    rungs: Vec<u32>,
    amount_each: Uint128,
) -> Result<Response, ContractError> {
    if amount_each.is_zero() || rungs.is_empty() {
        return Ok(response);
    }
    let total = amount_each.checked_mul(Uint128::from(rungs.len() as u128))?;
    let token_index = match side {
        LimitOrderSide::Ask => 0,
        LimitOrderSide::Bid => 1,
    };
    let mut current = BOTS.load(deps.storage, bot_id)?;
    let config = CONFIG.load(deps.storage)?;
    if current.active_orders.saturating_add(rungs.len() as u32) > config.max_active_orders_per_bot {
        return Ok(response.add_attribute("allocation_deferred", "active_order_limit"));
    }
    if total > current.free_balances[token_index] {
        return Err(ContractError::InsufficientBalance);
    }
    current.free_balances[token_index] = current.free_balances[token_index].checked_sub(total)?;
    current.active_orders = current
        .active_orders
        .checked_add(rungs.len() as u32)
        .ok_or(ContractError::ActiveOrderLimit)?;
    BOTS.save(deps.storage, bot_id, &current)?;
    let mut orders = Vec::with_capacity(rungs.len());
    for rung_index in &rungs {
        let rung = RUNGS.load(deps.storage, (bot_id, *rung_index))?;
        orders.push(LimitOrderPlacementItem {
            price: rung.price,
            amount: amount_each,
            max_adjust_steps: MAX_ADJUST_STEPS,
            expires_at: None,
            hint_after_order_id: None,
        });
    }
    let reply_id = NEXT_REPLY_ID.load(deps.storage)?;
    NEXT_REPLY_ID.save(
        deps.storage,
        &reply_id
            .checked_add(1)
            .ok_or_else(|| StdError::generic_err("reply id overflow"))?,
    )?;
    PLACEMENTS.save(
        deps.storage,
        reply_id,
        &PlacementPlan {
            bot_id,
            side: side.clone(),
            rungs,
            gross_amounts: vec![amount_each; orders.len()],
        },
    )?;
    let hook = PairCw20HookMsg::PlaceLimitOrderBatch { side, orders };
    Ok(response.add_submessage(SubMsg::reply_always(
        WasmMsg::Execute {
            contract_addr: bot.asset_tokens[token_index].to_string(),
            msg: to_json_binary(&Cw20ExecuteMsg::Send {
                contract: bot.pair.to_string(),
                amount: total,
                msg: to_json_binary(&hook)?,
            })?,
            funds: vec![],
        },
        reply_id,
    )))
}

#[entry_point]
pub fn reply(deps: DepsMut, env: Env, reply: Reply) -> Result<Response, ContractError> {
    let plan = PLACEMENTS
        .may_load(deps.storage, reply.id)?
        .ok_or(ContractError::UnknownReply)?;
    let result = match reply.result.into_result() {
        Ok(result) => result,
        Err(error) if plan.rungs.len() == 1 => {
            return restore_single_placement(deps, reply.id, &plan, error)
        }
        Err(error) => return Err(StdError::generic_err(error).into()),
    };
    let ids: Vec<u64> = result
        .events
        .iter()
        .flat_map(|event| event.attributes.iter())
        .filter(|attribute| attribute.key == "limit_order_placed")
        .map(|attribute| attribute.value.parse::<u64>())
        .collect::<Result<_, _>>()
        .map_err(|_| ContractError::InvalidPlacementReply)?;
    if ids.is_empty() && plan.rungs.len() == 1 {
        return restore_single_placement(
            deps,
            reply.id,
            &plan,
            "pair skipped the placement".to_string(),
        );
    }
    if ids.len() != plan.rungs.len() {
        return Err(ContractError::InvalidPlacementReply);
    }
    let bot = BOTS.load(deps.storage, plan.bot_id)?;
    for ((order_id, rung_index), gross) in ids.into_iter().zip(plan.rungs).zip(plan.gross_amounts) {
        if ORDERS
            .may_load(deps.storage, (plan.bot_id, order_id))?
            .is_some()
        {
            return Err(ContractError::InvalidPlacementReply);
        }
        let on_chain: LimitOrderResponse = deps
            .querier
            .query_wasm_smart(&bot.pair, &PairQueryMsg::LimitOrder { order_id })?;
        let rung = RUNGS.load(deps.storage, (plan.bot_id, rung_index))?;
        if on_chain.owner != env.contract.address
            || on_chain.side != plan.side
            || on_chain.price != rung.price
            || on_chain.remaining > gross
        {
            return Err(ContractError::InvalidOrder);
        }
        ORDERS.save(
            deps.storage,
            (plan.bot_id, order_id),
            &GridOrder {
                rung_index,
                side: plan.side.clone(),
                price: on_chain.price,
                remaining: on_chain.remaining,
            },
        )?;
    }
    PLACEMENTS.remove(deps.storage, reply.id);
    Ok(Response::new()
        .add_attribute("action", "record_grid_orders")
        .add_attribute("bot_id", plan.bot_id.to_string()))
}

fn restore_single_placement(
    deps: DepsMut,
    reply_id: u64,
    plan: &PlacementPlan,
    reason: String,
) -> Result<Response, ContractError> {
    let amount = *plan
        .gross_amounts
        .first()
        .ok_or(ContractError::InvalidPlacementReply)?;
    BOTS.update(
        deps.storage,
        plan.bot_id,
        |bot| -> Result<_, ContractError> {
            let mut bot = bot.ok_or_else(|| StdError::not_found("bot"))?;
            let token_index = match plan.side {
                LimitOrderSide::Ask => 0,
                LimitOrderSide::Bid => 1,
            };
            bot.free_balances[token_index] = bot.free_balances[token_index].checked_add(amount)?;
            bot.active_orders = bot
                .active_orders
                .checked_sub(1)
                .ok_or(ContractError::InvalidOrder)?;
            Ok(bot)
        },
    )?;
    PLACEMENTS.remove(deps.storage, reply_id);
    Ok(Response::new()
        .add_attribute("action", "defer_grid_placement")
        .add_attribute("bot_id", plan.bot_id.to_string())
        .add_attribute("reason", reason))
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => {
            let config = CONFIG.load(deps.storage)?;
            to_json_binary(&ConfigResponse {
                admin: config.admin.to_string(),
                keeper: config.keeper.to_string(),
                factory: config.factory.to_string(),
                gas_denom: config.gas_denom,
                keeper_reward: config.keeper_reward,
                minimum_gas_reserve: config.minimum_gas_reserve,
                max_grid_count: config.max_grid_count,
                max_orders_per_reconcile: config.max_orders_per_reconcile,
                max_active_orders_per_bot: config.max_active_orders_per_bot,
            })
        }
        QueryMsg::Bot { bot_id } => {
            let bot = BOTS.load(deps.storage, bot_id)?;
            to_json_binary(&BotResponse {
                bot_id,
                owner: bot.owner.to_string(),
                pair: bot.pair.to_string(),
                asset_tokens: bot.asset_tokens.map(|token| token.to_string()),
                lower_price: bot.lower_price,
                upper_price: bot.upper_price,
                grid_count: bot.grid_count,
                reference_price: bot.reference_price,
                free_balances: bot.free_balances,
                total_shares: bot.total_shares,
                gas_credit: bot.gas_credit,
                active_orders: bot.active_orders,
                pair_batch_limit: bot.pair_batch_limit,
            })
        }
        QueryMsg::Rungs { bot_id } => to_json_binary(
            &RUNGS
                .prefix(bot_id)
                .range(deps.storage, None, None, Order::Ascending)
                .map(|item| {
                    item.map(|(index, rung)| RungResponse {
                        index,
                        price: rung.price,
                        side: rung.side,
                    })
                })
                .collect::<StdResult<Vec<_>>>()?,
        ),
        QueryMsg::Orders { bot_id } => to_json_binary(
            &ORDERS
                .prefix(bot_id)
                .range(deps.storage, None, None, Order::Ascending)
                .map(|item| {
                    item.map(|(order_id, order)| OrderResponse {
                        order_id,
                        rung_index: order.rung_index,
                        side: order.side,
                        price: order.price,
                        remaining: order.remaining,
                    })
                })
                .collect::<StdResult<Vec<_>>>()?,
        ),
        QueryMsg::Shares { bot_id, address } => {
            let address = deps.api.addr_validate(&address)?;
            to_json_binary(&ShareResponse {
                shares: SHARES
                    .may_load(deps.storage, (bot_id, &address))?
                    .unwrap_or_default(),
            })
        }
    }
}

fn token_address(deps: Deps, asset: &AssetInfo) -> Result<Addr, ContractError> {
    match asset {
        AssetInfo::Token { contract_addr } => Ok(deps.api.addr_validate(contract_addr)?),
        AssetInfo::NativeToken { .. } => Err(ContractError::UnsupportedAsset),
    }
}

fn pool_price(pool: &PoolResponse) -> Result<Decimal, ContractError> {
    if pool.assets[0].amount.is_zero() || pool.assets[1].amount.is_zero() {
        return Err(ContractError::InvalidPair);
    }
    Ok(Decimal::from_ratio(
        pool.assets[1].amount,
        pool.assets[0].amount,
    ))
}

fn grid_prices(lower: Decimal, upper: Decimal, count: u32) -> Result<Vec<Decimal>, ContractError> {
    if count < 2 || lower >= upper {
        return Err(ContractError::InvalidGrid);
    }
    let difference = upper.checked_sub(lower)?;
    (0..count)
        .map(|index| {
            let offset = difference
                .atomics()
                .multiply_ratio(index as u128, (count - 1) as u128);
            lower
                .checked_add(
                    Decimal::from_atomics(offset, 18)
                        .map_err(|error| StdError::generic_err(error.to_string()))?,
                )
                .map_err(ContractError::from)
        })
        .collect()
}

fn side_rungs(
    deps: Deps,
    bot_id: u64,
    grid_count: u32,
    side: LimitOrderSide,
) -> StdResult<Vec<u32>> {
    RUNGS
        .prefix(bot_id)
        .range(
            deps.storage,
            None,
            Some(Bound::inclusive(grid_count - 1)),
            Order::Ascending,
        )
        .filter_map(|item| match item {
            Ok((index, rung)) if rung.side == Some(side.clone()) => Some(Ok(index)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

fn paid_amount(info: &MessageInfo, denom: &str) -> Result<Uint128, ContractError> {
    if info.funds.iter().any(|coin| coin.denom != denom) {
        return Err(ContractError::UnexpectedFunds);
    }
    Ok(info
        .funds
        .iter()
        .find(|coin| coin.denom == denom)
        .map(|coin| coin.amount)
        .unwrap_or_default())
}

fn assert_no_funds(info: &MessageInfo) -> Result<(), ContractError> {
    if info.funds.is_empty() {
        Ok(())
    } else {
        Err(ContractError::UnexpectedFunds)
    }
}

fn deposit_shares(
    amount: Uint128,
    token_index: usize,
    price: Decimal,
) -> Result<Uint128, ContractError> {
    if token_index == 0 {
        Ok(amount)
    } else {
        Ok(amount * price.inv().ok_or(ContractError::InvalidPair)?)
    }
}

fn add_pair_batches(
    mut response: Response,
    pair: &Addr,
    cancel_ids: &[u64],
    claim_ids: &[u64],
    batch_limit: u32,
) -> StdResult<Response> {
    let limit = batch_limit.max(1) as usize;
    for ids in cancel_ids.chunks(limit) {
        response = response.add_message(WasmMsg::Execute {
            contract_addr: pair.to_string(),
            msg: to_json_binary(&PairExecuteMsg::CancelLimitOrders {
                order_ids: ids.to_vec(),
            })?,
            funds: vec![],
        });
    }
    for ids in claim_ids.chunks(limit) {
        response = response.add_message(WasmMsg::Execute {
            contract_addr: pair.to_string(),
            msg: to_json_binary(&PairExecuteMsg::ClaimExpiredLimitOrders {
                order_ids: ids.to_vec(),
            })?,
            funds: vec![],
        });
    }
    Ok(response)
}

fn query_pair_batch_limit(deps: Deps, pair: &Addr) -> StdResult<u32> {
    let config: LimitOrderConfigResponse = deps
        .querier
        .query_wasm_smart(pair, &PairQueryMsg::LimitOrderConfig {})?;
    Ok(config.max_batch_rungs.max(1))
}

fn validate_order(
    manager: &Addr,
    order_id: u64,
    recorded: &GridOrder,
    on_chain: &LimitOrderResponse,
) -> Result<(), ContractError> {
    if on_chain.owner != manager.as_str() {
        return Err(ContractError::InvalidOrderOwner);
    }
    if on_chain.order_id != order_id
        || on_chain.side != recorded.side
        || on_chain.price != recorded.price
        || on_chain.remaining > recorded.remaining
    {
        return Err(ContractError::InvalidOrder);
    }
    Ok(())
}

fn validate_indexed_report(
    order: &GridOrder,
    report: &OrderFillReport,
    terminal: bool,
) -> Result<(Uint128, Uint128), ContractError> {
    if report.fill_count == 0 {
        if terminal && report.input_amount.is_zero() && report.output_amount.is_zero() {
            return Ok((Uint128::zero(), Uint128::zero()));
        }
        return Err(ContractError::InvalidFillReport);
    }
    if report.input_amount.is_zero() || report.output_amount.is_zero() {
        return Err(ContractError::InvalidFillReport);
    }
    // Summing per-fill floors can differ from flooring the aggregate by at most n - 1 units.
    let aggregate_floor = match order.side {
        LimitOrderSide::Ask => order.price * report.input_amount,
        LimitOrderSide::Bid => order.price * report.output_amount,
    };
    let indexed_floor = match order.side {
        LimitOrderSide::Ask => report.output_amount,
        LimitOrderSide::Bid => report.input_amount,
    };
    if indexed_floor > aggregate_floor
        || aggregate_floor.checked_sub(indexed_floor)? >= Uint128::from(report.fill_count as u128)
    {
        return Err(ContractError::InvalidFillReport);
    }
    Ok((report.input_amount, report.output_amount))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};
    use cosmwasm_std::{coin, from_json, ContractResult, SubMsgResult, SystemResult, WasmQuery};

    fn instantiate_default(deps: DepsMut) {
        instantiate(
            deps,
            mock_env(),
            mock_info("creator", &[]),
            InstantiateMsg {
                admin: "admin".into(),
                keeper: "keeper".into(),
                factory: "factory".into(),
                gas_denom: "uluna".into(),
                keeper_reward: Uint128::new(20),
                minimum_gas_reserve: Uint128::new(100),
                max_grid_count: 20,
                max_orders_per_reconcile: 10,
                max_active_orders_per_bot: 40,
            },
        )
        .unwrap();
    }

    fn install_pair_querier(
        deps: &mut cosmwasm_std::OwnedDeps<
            cosmwasm_std::MemoryStorage,
            cosmwasm_std::testing::MockApi,
            cosmwasm_std::testing::MockQuerier,
        >,
    ) {
        deps.querier.update_wasm(|query| match query {
            WasmQuery::Smart { contract_addr, msg } if contract_addr == "factory" => {
                let _: FactoryQueryMsg = from_json(msg).unwrap();
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&PairResponse {
                        pair: PairInfo {
                            asset_infos: [
                                AssetInfo::Token {
                                    contract_addr: "token_a".into(),
                                },
                                AssetInfo::Token {
                                    contract_addr: "token_b".into(),
                                },
                            ],
                            contract_addr: "pair".into(),
                            liquidity_token: "lp".into(),
                        },
                    })
                    .unwrap(),
                ))
            }
            WasmQuery::Smart { contract_addr, msg }
                if contract_addr == "token_a" || contract_addr == "token_b" =>
            {
                let _: Cw20QueryMsg = from_json(msg).unwrap();
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&TokenInfoResponse {
                        name: contract_addr.clone(),
                        symbol: "TOK".into(),
                        decimals: 6,
                        total_supply: Uint128::new(1_000_000),
                    })
                    .unwrap(),
                ))
            }
            WasmQuery::Smart { msg, .. } => {
                let query: PairQueryMsg = from_json(msg).unwrap();
                let response = match query {
                    PairQueryMsg::Pair {} => to_json_binary(&PairInfo {
                        asset_infos: [
                            AssetInfo::Token {
                                contract_addr: "token_a".into(),
                            },
                            AssetInfo::Token {
                                contract_addr: "token_b".into(),
                            },
                        ],
                        contract_addr: "pair".into(),
                        liquidity_token: "lp".into(),
                    })
                    .unwrap(),
                    PairQueryMsg::Pool {} => to_json_binary(&PoolResponse {
                        assets: [
                            crate::msg::Asset {
                                info: AssetInfo::Token {
                                    contract_addr: "token_a".into(),
                                },
                                amount: Uint128::new(1_000),
                            },
                            crate::msg::Asset {
                                info: AssetInfo::Token {
                                    contract_addr: "token_b".into(),
                                },
                                amount: Uint128::new(2_000),
                            },
                        ],
                        total_share: Uint128::new(1_000),
                    })
                    .unwrap(),
                    PairQueryMsg::LimitOrderConfig {} => {
                        to_json_binary(&LimitOrderConfigResponse {
                            max_batch_rungs: 20,
                        })
                        .unwrap()
                    }
                    PairQueryMsg::LimitOrder { .. } => {
                        return SystemResult::Ok(ContractResult::Err("order not installed".into()))
                    }
                    PairQueryMsg::ExpiredLimitRefund { .. } => {
                        to_json_binary(&Option::<ExpiredLimitRefundResponse>::None).unwrap()
                    }
                };
                SystemResult::Ok(ContractResult::Ok(response))
            }
            _ => SystemResult::Ok(ContractResult::Err("unsupported query".into())),
        });
    }

    fn create_bot(deps: DepsMut, owner: &str) {
        execute(
            deps,
            mock_env(),
            mock_info(owner, &[coin(200, "uluna")]),
            ExecuteMsg::CreateBot {
                pair: "pair".into(),
                lower_price: Decimal::one(),
                upper_price: Decimal::from_ratio(3u128, 1u128),
                grid_count: 5,
            },
        )
        .unwrap();
    }

    #[test]
    fn creates_user_owned_grid_with_separate_gas_credit() {
        let mut deps = mock_dependencies();
        install_pair_querier(&mut deps);
        instantiate_default(deps.as_mut());
        create_bot(deps.as_mut(), "alice");
        create_bot(deps.as_mut(), "bob");

        let alice = BOTS.load(&deps.storage, 1).unwrap();
        let bob = BOTS.load(&deps.storage, 2).unwrap();
        assert_eq!(alice.owner, Addr::unchecked("alice"));
        assert_eq!(bob.owner, Addr::unchecked("bob"));
        assert_eq!(alice.gas_credit, Uint128::new(200));
        assert_eq!(bob.gas_credit, Uint128::new(200));
        assert_eq!(alice.reference_price, Decimal::percent(200));

        let sides: Vec<Option<LimitOrderSide>> = (0..5)
            .map(|index| RUNGS.load(&deps.storage, (1, index)).unwrap().side)
            .collect();
        assert_eq!(
            sides,
            vec![
                Some(LimitOrderSide::Bid),
                Some(LimitOrderSide::Bid),
                None,
                Some(LimitOrderSide::Ask),
                Some(LimitOrderSide::Ask),
            ]
        );
    }

    #[test]
    fn rejects_prices_outside_standard_pair_bounds() {
        let mut deps = mock_dependencies();
        install_pair_querier(&mut deps);
        instantiate_default(deps.as_mut());
        let error = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("alice", &[coin(200, "uluna")]),
            ExecuteMsg::CreateBot {
                pair: "pair".into(),
                lower_price: Decimal::zero(),
                upper_price: Decimal::from_ratio(3u128, 1u128),
                grid_count: 5,
            },
        )
        .unwrap_err();
        assert_eq!(error, ContractError::InvalidGrid);
    }

    #[test]
    fn failed_single_opposite_placement_returns_free_balance() {
        let mut deps = mock_dependencies();
        BOTS.save(
            deps.as_mut().storage,
            1,
            &Bot {
                owner: Addr::unchecked("alice"),
                pair: Addr::unchecked("pair"),
                asset_tokens: [Addr::unchecked("token_a"), Addr::unchecked("token_b")],
                lower_price: Decimal::one(),
                upper_price: Decimal::percent(300),
                grid_count: 5,
                reference_price: Decimal::percent(200),
                free_balances: [Uint128::zero(), Uint128::zero()],
                total_shares: Uint128::new(1_000),
                gas_credit: Uint128::new(200),
                active_orders: 1,
                pair_batch_limit: 20,
            },
        )
        .unwrap();
        PLACEMENTS
            .save(
                deps.as_mut().storage,
                7,
                &PlacementPlan {
                    bot_id: 1,
                    side: LimitOrderSide::Bid,
                    rungs: vec![1],
                    gross_amounts: vec![Uint128::new(25)],
                },
            )
            .unwrap();

        let response = reply(
            deps.as_mut(),
            mock_env(),
            Reply {
                id: 7,
                result: SubMsgResult::Err("book walk exhausted".into()),
            },
        )
        .unwrap();
        let bot = BOTS.load(&deps.storage, 1).unwrap();
        assert_eq!(bot.free_balances, [Uint128::zero(), Uint128::new(25)]);
        assert_eq!(bot.active_orders, 0);
        assert!(!PLACEMENTS.has(&deps.storage, 7));
        assert_eq!(response.attributes[0].value, "defer_grid_placement");
    }

    #[test]
    fn deposits_and_allocates_each_asset_by_its_own_grid_count() {
        let mut deps = mock_dependencies();
        install_pair_querier(&mut deps);
        instantiate_default(deps.as_mut());
        create_bot(deps.as_mut(), "alice");

        let ask_response = execute_receive(
            deps.as_mut(),
            mock_info("token_a", &[]),
            Cw20ReceiveMsg {
                sender: "alice".into(),
                amount: Uint128::new(1_000),
                msg: to_json_binary(&ReceiveMsg::Deposit { bot_id: 1 }).unwrap(),
            },
        )
        .unwrap();
        let bid_response = execute_receive(
            deps.as_mut(),
            mock_info("token_b", &[]),
            Cw20ReceiveMsg {
                sender: "alice".into(),
                amount: Uint128::new(4_000),
                msg: to_json_binary(&ReceiveMsg::Deposit { bot_id: 1 }).unwrap(),
            },
        )
        .unwrap();

        assert_eq!(ask_response.messages.len(), 1);
        assert_eq!(bid_response.messages.len(), 1);
        let bot = BOTS.load(&deps.storage, 1).unwrap();
        assert_eq!(bot.free_balances, [Uint128::zero(), Uint128::zero()]);
        let ask = PLACEMENTS.load(&deps.storage, 1).unwrap();
        let bid = PLACEMENTS.load(&deps.storage, 2).unwrap();
        assert_eq!(bid.gross_amounts, vec![Uint128::new(2_000); 2]);
        assert_eq!(ask.gross_amounts, vec![Uint128::new(500); 2]);
        assert_eq!(
            SHARES
                .load(&deps.storage, (1, &Addr::unchecked("alice")))
                .unwrap(),
            Uint128::new(3_000)
        );
    }

    #[test]
    fn bot_owner_cannot_deposit_into_another_users_bot() {
        let mut deps = mock_dependencies();
        install_pair_querier(&mut deps);
        instantiate_default(deps.as_mut());
        create_bot(deps.as_mut(), "alice");

        let error = execute_receive(
            deps.as_mut(),
            mock_info("token_a", &[]),
            Cw20ReceiveMsg {
                sender: "bob".into(),
                amount: Uint128::new(100),
                msg: to_json_binary(&ReceiveMsg::Deposit { bot_id: 1 }).unwrap(),
            },
        )
        .unwrap_err();
        assert_eq!(error, ContractError::Unauthorized);
        assert_eq!(
            BOTS.load(&deps.storage, 1).unwrap().free_balances[0],
            Uint128::zero()
        );
    }

    #[test]
    fn withdrawal_burns_only_that_bots_shares() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        for (bot_id, owner) in [(1, "alice"), (2, "bob")] {
            BOTS.save(
                deps.as_mut().storage,
                bot_id,
                &Bot {
                    owner: Addr::unchecked(owner),
                    pair: Addr::unchecked("pair"),
                    asset_tokens: [Addr::unchecked("token_a"), Addr::unchecked("token_b")],
                    lower_price: Decimal::one(),
                    upper_price: Decimal::percent(300),
                    grid_count: 5,
                    reference_price: Decimal::percent(200),
                    free_balances: [Uint128::new(1_000), Uint128::new(2_000)],
                    total_shares: Uint128::new(1_000),
                    gas_credit: Uint128::new(200),
                    active_orders: 0,
                    pair_batch_limit: 20,
                },
            )
            .unwrap();
            SHARES
                .save(
                    deps.as_mut().storage,
                    (bot_id, &Addr::unchecked(owner)),
                    &Uint128::new(1_000),
                )
                .unwrap();
        }

        let response = execute_withdraw(
            deps.as_mut(),
            mock_info("alice", &[]),
            1,
            Uint128::new(500),
            None,
        )
        .unwrap();
        assert_eq!(response.messages.len(), 2);
        assert_eq!(
            BOTS.load(&deps.storage, 1).unwrap().free_balances,
            [Uint128::new(500), Uint128::new(1_000)]
        );
        assert_eq!(
            BOTS.load(&deps.storage, 2).unwrap().free_balances,
            [Uint128::new(1_000), Uint128::new(2_000)]
        );
        assert_eq!(
            SHARES
                .load(&deps.storage, (1, &Addr::unchecked("alice")))
                .unwrap(),
            Uint128::new(500)
        );
        assert_eq!(
            SHARES
                .load(&deps.storage, (2, &Addr::unchecked("bob")))
                .unwrap(),
            Uint128::new(1_000)
        );
    }

    #[test]
    fn partial_bid_uses_exact_output_when_escrow_rounding_is_not_invertible() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        let bot = Bot {
            owner: Addr::unchecked("alice"),
            pair: Addr::unchecked("pair"),
            asset_tokens: [Addr::unchecked("token_a"), Addr::unchecked("token_b")],
            lower_price: Decimal::one(),
            upper_price: Decimal::from_ratio(3u128, 1u128),
            grid_count: 5,
            reference_price: Decimal::percent(200),
            free_balances: [Uint128::zero(), Uint128::zero()],
            total_shares: Uint128::new(1_000),
            gas_credit: Uint128::new(200),
            active_orders: 1,
            pair_batch_limit: 20,
        };
        BOTS.save(deps.as_mut().storage, 1, &bot).unwrap();
        for index in 0..5 {
            let price = Decimal::percent(100 + index as u64 * 50);
            RUNGS
                .save(
                    deps.as_mut().storage,
                    (1, index),
                    &Rung {
                        price,
                        side: if index < 2 {
                            Some(LimitOrderSide::Bid)
                        } else if index > 2 {
                            Some(LimitOrderSide::Ask)
                        } else {
                            None
                        },
                    },
                )
                .unwrap();
        }
        ORDERS
            .save(
                deps.as_mut().storage,
                (1, 77),
                &GridOrder {
                    rung_index: 1,
                    side: LimitOrderSide::Bid,
                    price: Decimal::percent(150),
                    remaining: Uint128::new(100),
                },
            )
            .unwrap();
        deps.querier.update_wasm(|query| match query {
            WasmQuery::Smart { msg, .. } => {
                let query: PairQueryMsg = from_json(msg).unwrap();
                match query {
                    PairQueryMsg::LimitOrder { order_id } => SystemResult::Ok(ContractResult::Ok(
                        to_json_binary(&LimitOrderResponse {
                            order_id,
                            owner: mock_env().contract.address.to_string(),
                            side: LimitOrderSide::Bid,
                            price: Decimal::percent(150),
                            remaining: Uint128::new(99),
                            expires_at: None,
                            prev: None,
                            next: None,
                        })
                        .unwrap(),
                    )),
                    _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
                }
            }
            _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
        });

        let response = execute_reconcile(
            deps.as_mut(),
            mock_env(),
            mock_info("keeper", &[]),
            1,
            vec![OrderFillReport {
                pair: "pair".into(),
                order_id: 77,
                input_amount: Uint128::one(),
                output_amount: Uint128::one(),
                fill_count: 1,
            }],
        )
        .unwrap();
        assert_eq!(
            ORDERS.load(&deps.storage, (1, 77)).unwrap().remaining,
            Uint128::new(99)
        );
        let opposite = PLACEMENTS.load(&deps.storage, 1).unwrap();
        assert_eq!(opposite.side, LimitOrderSide::Ask);
        assert_eq!(opposite.rungs, vec![2]);
        assert_eq!(opposite.gross_amounts, vec![Uint128::one()]);
        assert_eq!(
            BOTS.load(&deps.storage, 1).unwrap().gas_credit,
            Uint128::new(180)
        );
        assert_eq!(response.messages.len(), 2);
    }

    #[test]
    fn keeper_is_not_paid_for_empty_fill_report() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        BOTS.save(
            deps.as_mut().storage,
            1,
            &Bot {
                owner: Addr::unchecked("alice"),
                pair: Addr::unchecked("pair"),
                asset_tokens: [Addr::unchecked("token_a"), Addr::unchecked("token_b")],
                lower_price: Decimal::one(),
                upper_price: Decimal::percent(300),
                grid_count: 5,
                reference_price: Decimal::percent(200),
                free_balances: [Uint128::zero(), Uint128::zero()],
                total_shares: Uint128::zero(),
                gas_credit: Uint128::new(200),
                active_orders: 0,
                pair_batch_limit: 20,
            },
        )
        .unwrap();
        let error = execute_reconcile(
            deps.as_mut(),
            mock_env(),
            mock_info("keeper", &[]),
            1,
            vec![],
        )
        .unwrap_err();
        assert_eq!(error, ContractError::InvalidFillReport);
        assert_eq!(
            BOTS.load(&deps.storage, 1).unwrap().gas_credit,
            Uint128::new(200)
        );
    }
}
