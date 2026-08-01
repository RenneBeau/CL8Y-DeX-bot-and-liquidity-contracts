use bot_types::{
    RebalancePlanResponse, RebalanceStatusResponse, SwapParams, SwapProxyHookMsg,
    VaultBalancesResponse, VaultConfigResponse, VaultExecuteMsg, VaultPriceResponse, VaultQueryMsg,
};
use cl8y_dex::{
    Asset, AssetInfo, HybridSimulationResponse, HybridSwapParams, ObserveResponse, PairInfo,
    PairQueryMsg, PoolResponse,
};
use cosmwasm_std::{
    entry_point, to_json_binary, Addr, Binary, Decimal, Deps, DepsMut, Env, MessageInfo, Reply,
    Response, StdError, StdResult, SubMsg, Uint128, Uint256, WasmMsg,
};
use cw2::set_contract_version;
use cw20::{BalanceResponse, Cw20ExecuteMsg, Cw20QueryMsg, TokenInfoResponse};

use crate::error::ContractError;
use crate::msg::InstantiateMsg;
use crate::state::{Config, PendingRebalance, CONFIG, PENDING_REBALANCE};

const CONTRACT_NAME: &str = "crates.io:cl8y-bot-vault";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_REBALANCE_THRESHOLD_BPS: u16 = 500;
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
const REBALANCE_REPLY_ID: u64 = 1;

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    let pair = deps.api.addr_validate(&msg.pair)?;
    let pair_info: PairInfo = deps
        .querier
        .query_wasm_smart(&pair, &PairQueryMsg::Pair {})?;
    if pair_info.contract_addr != pair {
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
    let token_0: TokenInfoResponse = deps
        .querier
        .query_wasm_smart(&asset_tokens[0], &Cw20QueryMsg::TokenInfo {})?;
    let token_1: TokenInfoResponse = deps
        .querier
        .query_wasm_smart(&asset_tokens[1], &Cw20QueryMsg::TokenInfo {})?;
    if token_0.decimals != token_1.decimals {
        return Err(ContractError::DecimalMismatch);
    }
    let rebalance_threshold_bps = msg
        .rebalance_threshold_bps
        .unwrap_or(DEFAULT_REBALANCE_THRESHOLD_BPS);
    let allocation_tolerance_bps = msg
        .allocation_tolerance_bps
        .unwrap_or(DEFAULT_ALLOCATION_TOLERANCE_BPS);
    validate_threshold(rebalance_threshold_bps)?;
    validate_allocation_tolerance(allocation_tolerance_bps)?;
    if msg.twap_window_seconds == 0 {
        return Err(ContractError::InvalidTwapWindow);
    }
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
    let mut config = Config {
        admin: deps.api.addr_validate(&msg.admin)?,
        keeper: deps.api.addr_validate(&msg.keeper)?,
        liquidity_contract: None,
        proxy: deps.api.addr_validate(&msg.proxy)?,
        pair,
        asset_tokens,
        decimals: token_0.decimals,
        twap_window_seconds: msg.twap_window_seconds,
        rebalance_threshold_bps,
        allocation_tolerance_bps,
        max_trade_bps,
        max_execution_deviation_bps,
        quote_slippage_bps,
        max_spot_twap_deviation_bps,
        max_trade_pool_bps,
        max_spread,
        reference_price: Decimal::one(),
    };
    config.reference_price = query_price(deps.as_ref(), &env, &config)?;
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("vault", env.contract.address))
}

#[entry_point]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: VaultExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        VaultExecuteMsg::SetLiquidityContract { liquidity_contract } => {
            execute_set_liquidity(deps, info, liquidity_contract)
        }
        VaultExecuteMsg::LiquiditySwap { params } => {
            execute_liquidity_swap(deps, env, info, params)
        }
        VaultExecuteMsg::TransferTo {
            token,
            amount,
            recipient,
        } => execute_transfer(deps, info, token, amount, recipient),
        VaultExecuteMsg::FinalizeLiquidityOperation {} => execute_finalize(deps, info),
        VaultExecuteMsg::Rebalance { deadline } => execute_rebalance(deps, env, info, deadline),
        VaultExecuteMsg::SyncReference {} => execute_sync_reference(deps, env, info),
        VaultExecuteMsg::UpdateKeeper { keeper } => execute_update_keeper(deps, info, keeper),
        VaultExecuteMsg::UpdateThresholds {
            rebalance_threshold_bps,
            allocation_tolerance_bps,
            max_trade_bps,
            max_execution_deviation_bps,
            quote_slippage_bps,
            max_spot_twap_deviation_bps,
            max_trade_pool_bps,
            max_spread,
            twap_window_seconds,
        } => execute_update_thresholds(
            deps,
            env,
            info,
            rebalance_threshold_bps,
            allocation_tolerance_bps,
            max_trade_bps,
            max_execution_deviation_bps,
            quote_slippage_bps,
            max_spot_twap_deviation_bps,
            max_trade_pool_bps,
            max_spread,
            twap_window_seconds,
        ),
        VaultExecuteMsg::TransferAdmin { admin } => execute_transfer_admin(deps, info, admin),
    }
}

fn execute_set_liquidity(
    deps: DepsMut,
    info: MessageInfo,
    liquidity_contract: String,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;
    assert_admin(&config, &info.sender)?;
    if config.liquidity_contract.is_some() {
        return Err(ContractError::LiquidityAlreadyConfigured);
    }
    config.liquidity_contract = Some(deps.api.addr_validate(&liquidity_contract)?);
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new()
        .add_attribute("action", "set_liquidity_contract")
        .add_attribute("liquidity_contract", liquidity_contract))
}

fn execute_liquidity_swap(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    params: SwapParams,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    assert_liquidity(&config, &info.sender)?;
    validate_swap(deps.as_ref(), &env, &config, &params)?;
    Ok(Response::new()
        .add_message(proxy_swap_message(&config, params.clone())?)
        .add_attribute("action", "liquidity_swap")
        .add_attribute("offer_token", params.offer_token)
        .add_attribute("amount", params.amount))
}

fn execute_transfer(
    deps: DepsMut,
    info: MessageInfo,
    token: String,
    amount: Uint128,
    recipient: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    assert_liquidity(&config, &info.sender)?;
    let token = deps.api.addr_validate(&token)?;
    if !config.asset_tokens.contains(&token) {
        return Err(ContractError::UnsupportedToken);
    }
    if amount.is_zero() {
        return Err(ContractError::ZeroAmount);
    }
    let recipient = deps.api.addr_validate(&recipient)?;
    Ok(Response::new()
        .add_message(WasmMsg::Execute {
            contract_addr: token.to_string(),
            msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
                recipient: recipient.to_string(),
                amount,
            })?,
            funds: vec![],
        })
        .add_attribute("action", "transfer_to")
        .add_attribute("recipient", recipient))
}

fn execute_finalize(deps: DepsMut, info: MessageInfo) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    assert_liquidity(&config, &info.sender)?;
    Ok(Response::new().add_attribute("action", "finalize_liquidity_operation"))
}

fn execute_rebalance(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    deadline: u64,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.keeper {
        return Err(ContractError::Unauthorized);
    }
    if PENDING_REBALANCE.may_load(deps.storage)?.is_some() {
        return Err(ContractError::RebalancePending);
    }
    if deadline < env.block.time.seconds() {
        return Err(ContractError::Expired);
    }
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
    let params = SwapParams {
        offer_token,
        amount,
        min_return,
        max_spread: config.max_spread,
        deadline,
    };
    PENDING_REBALANCE.save(
        deps.storage,
        &PendingRebalance {
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
            proxy_swap_message(&config, params)?,
            REBALANCE_REPLY_ID,
        ))
        .add_attribute("action", "rebalance"))
}

fn execute_sync_reference(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;
    if info.sender != config.keeper {
        return Err(ContractError::Unauthorized);
    }
    let status = rebalance_status(deps.as_ref(), &env, &config)?;
    if !status.should_rebalance {
        return Err(ContractError::RebalanceNotRequired);
    }
    if status.allocation_deviation_bps > config.allocation_tolerance_bps {
        return Err(ContractError::AllocationOutsideTolerance);
    }
    config.reference_price = status.current_price;
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new().add_attribute("action", "sync_reference"))
}

#[entry_point]
pub fn reply(deps: DepsMut, env: Env, reply: Reply) -> Result<Response, ContractError> {
    if reply.id != REBALANCE_REPLY_ID {
        return Err(ContractError::UnknownReply);
    }
    let pending = PENDING_REBALANCE
        .may_load(deps.storage)?
        .ok_or(ContractError::MissingPendingRebalance)?;
    let mut config = CONFIG.load(deps.storage)?;
    let settled = balances(deps.as_ref(), &env.contract.address, &config)?;
    validate_settlement(&pending, settled)?;
    let current = allocation_deviation(settled, pending.captured_twap)?;
    let within_tolerance = validate_rebalance_outcome(
        pending.pre_deviation_bps,
        current,
        config.allocation_tolerance_bps,
    )?;
    if within_tolerance {
        config.reference_price = pending.captured_twap;
    }
    CONFIG.save(deps.storage, &config)?;
    PENDING_REBALANCE.remove(deps.storage);
    Ok(Response::new()
        .add_attribute("action", "complete_rebalance")
        .add_attribute("allocation_deviation_bps", current.to_string())
        .add_attribute("reference_updated", within_tolerance.to_string()))
}

fn validate_settlement(
    pending: &PendingRebalance,
    settled: [Uint128; 2],
) -> Result<(), ContractError> {
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

fn execute_update_keeper(
    deps: DepsMut,
    info: MessageInfo,
    keeper: String,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;
    assert_admin(&config, &info.sender)?;
    config.keeper = deps.api.addr_validate(&keeper)?;
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new().add_attribute("action", "update_keeper"))
}

#[allow(clippy::too_many_arguments)]
fn execute_update_thresholds(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    rebalance: Option<u16>,
    allocation: Option<u16>,
    max_trade_bps: Option<u16>,
    max_execution_deviation_bps: Option<u16>,
    quote_slippage_bps: Option<u16>,
    max_spot_twap_deviation_bps: Option<u16>,
    max_trade_pool_bps: Option<u16>,
    max_spread: Option<Decimal>,
    twap_window_seconds: Option<u32>,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;
    assert_admin(&config, &info.sender)?;
    if let Some(value) = rebalance {
        validate_threshold(value)?;
        config.rebalance_threshold_bps = value;
    }
    if let Some(value) = allocation {
        validate_allocation_tolerance(value)?;
        config.allocation_tolerance_bps = value;
    }
    if let Some(value) = twap_window_seconds {
        if value == 0 || value > MAX_TWAP_WINDOW_SECONDS {
            return Err(ContractError::InvalidTwapWindow);
        }
        let mut proposed = config.clone();
        proposed.twap_window_seconds = value;
        config.reference_price = query_price(deps.as_ref(), &env, &proposed)?;
        config.twap_window_seconds = value;
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
    Ok(Response::new().add_attribute("action", "update_thresholds"))
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
    Ok(Response::new().add_attribute("action", "transfer_admin"))
}

#[entry_point]
pub fn query(deps: Deps, env: Env, msg: VaultQueryMsg) -> StdResult<Binary> {
    let config = CONFIG.load(deps.storage)?;
    match msg {
        VaultQueryMsg::Config {} => to_json_binary(&VaultConfigResponse {
            admin: config.admin.to_string(),
            keeper: config.keeper.to_string(),
            liquidity_contract: config.liquidity_contract.map(|addr| addr.to_string()),
            proxy: config.proxy.to_string(),
            pair: config.pair.to_string(),
            asset_tokens: config.asset_tokens.map(|addr| addr.to_string()),
            decimals: config.decimals,
            twap_window_seconds: config.twap_window_seconds,
            rebalance_threshold_bps: config.rebalance_threshold_bps,
            allocation_tolerance_bps: config.allocation_tolerance_bps,
            max_trade_bps: config.max_trade_bps,
            max_execution_deviation_bps: config.max_execution_deviation_bps,
            quote_slippage_bps: config.quote_slippage_bps,
            max_spot_twap_deviation_bps: config.max_spot_twap_deviation_bps,
            max_trade_pool_bps: config.max_trade_pool_bps,
            max_spread: config.max_spread,
        }),
        VaultQueryMsg::Balances {} => to_json_binary(&VaultBalancesResponse {
            balances: balances(deps, &env.contract.address, &config)?,
        }),
        VaultQueryMsg::Price {} => to_json_binary(&VaultPriceResponse {
            token1_per_token0: query_price(deps, &env, &config)?,
        }),
        VaultQueryMsg::RebalanceStatus {} => {
            to_json_binary(&rebalance_status(deps, &env, &config)?)
        }
        VaultQueryMsg::RebalancePlan {} => to_json_binary(&rebalance_plan(deps, &env, &config)?),
    }
}

fn rebalance_status(deps: Deps, env: &Env, config: &Config) -> StdResult<RebalanceStatusResponse> {
    let current_price = query_price(deps, env, config)?;
    let holdings = balances(deps, &env.contract.address, config)?;
    let allocation_deviation_bps = allocation_deviation(holdings, current_price)?;
    let price_deviation_bps = relative_deviation(current_price, config.reference_price)?;
    Ok(RebalanceStatusResponse {
        should_rebalance: price_deviation_bps >= config.rebalance_threshold_bps,
        price_deviation_bps,
        allocation_deviation_bps,
        reference_price: config.reference_price,
        current_price,
    })
}

fn rebalance_plan(deps: Deps, env: &Env, config: &Config) -> StdResult<RebalancePlanResponse> {
    let captured_twap = query_price(deps, env, config)?;
    let holdings = balances(deps, &env.contract.address, config)?;
    let price_deviation_bps = relative_deviation(captured_twap, config.reference_price)?;
    let allocation_deviation_bps = allocation_deviation(holdings, captured_twap)?;
    let should_rebalance = price_deviation_bps >= config.rebalance_threshold_bps;
    let offer = if should_rebalance {
        planned_offer(holdings, captured_twap, config.max_trade_bps)?
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
        let simulation: HybridSimulationResponse = deps.querier.query_wasm_smart(
            &config.pair,
            &PairQueryMsg::HybridSimulation {
                offer_asset: Asset {
                    info: AssetInfo::Token {
                        contract_addr: token.clone(),
                    },
                    amount,
                },
                hybrid: HybridSwapParams::pool_only(amount),
                trader: Some(config.proxy.to_string()),
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
    Ok(RebalancePlanResponse {
        should_rebalance,
        captured_twap,
        balances: holdings,
        price_deviation_bps,
        allocation_deviation_bps,
        reference_price: config.reference_price,
        offer_token,
        amount,
        min_return,
        max_spread: config.max_spread,
    })
}

fn query_price(deps: Deps, _env: &Env, config: &Config) -> StdResult<Decimal> {
    let response: ObserveResponse = deps.querier.query_wasm_smart(
        &config.pair,
        &PairQueryMsg::Observe {
            seconds_ago: vec![0, config.twap_window_seconds],
        },
    )?;
    if response.price_a_cumulatives.len() != 2
        || response.price_a_cumulatives[0] <= response.price_a_cumulatives[1]
    {
        return Err(StdError::generic_err("empty TWAP history"));
    }
    twap_from_observation(&response, config.twap_window_seconds)
}

fn twap_from_observation(response: &ObserveResponse, window: u32) -> StdResult<Decimal> {
    let difference = response.price_a_cumulatives[0] - response.price_a_cumulatives[1];
    let atomics = difference.checked_div(Uint128::from(window))?;
    let price = Decimal::from_atomics(atomics, 18)
        .map_err(|error| StdError::generic_err(error.to_string()))?;
    if price.is_zero() {
        return Err(StdError::generic_err("empty TWAP price"));
    }
    Ok(price)
}

fn allocation_deviation(holdings: [Uint128; 2], price: Decimal) -> StdResult<u16> {
    if holdings[0].is_zero() && holdings[1].is_zero() {
        return Ok(0);
    }
    if holdings[0].is_zero() || holdings[1].is_zero() {
        return Ok(10_000);
    }
    let expected = Uint256::from(price.atomics()) * Uint256::from(holdings[0]);
    let actual = Uint256::from(holdings[1]) * Uint256::from(Decimal::one().atomics());
    ratio_deviation(actual, expected)
}

fn planned_offer(
    holdings: [Uint128; 2],
    price: Decimal,
    max_trade_bps: u16,
) -> StdResult<Option<(usize, Uint128)>> {
    let token0_value = Uint256::from(holdings[0]) * Uint256::from(price.atomics());
    let token1_value = Uint256::from(holdings[1]) * Uint256::from(Decimal::one().atomics());
    let (index, uncapped) = match token1_value.cmp(&token0_value) {
        std::cmp::Ordering::Greater => {
            let amount = (token1_value - token0_value)
                / Uint256::from(Decimal::one().atomics())
                / Uint256::from(2u8);
            (1, amount)
        }
        std::cmp::Ordering::Less => {
            let amount =
                (token0_value - token1_value) / Uint256::from(price.atomics()) / Uint256::from(2u8);
            (0, amount)
        }
        std::cmp::Ordering::Equal => return Ok(None),
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
        Ok(price * amount)
    } else {
        Ok(amount.multiply_ratio(Decimal::one().atomics(), price.atomics()))
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
        return Err(StdError::generic_err("empty reference"));
    }
    let difference = if actual >= expected {
        actual - expected
    } else {
        expected - actual
    };
    let value = (difference * Uint256::from(10_000u16) / expected).min(Uint256::from(10_000u16));
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

fn proxy_swap_message(config: &Config, params: SwapParams) -> StdResult<WasmMsg> {
    let hook = SwapProxyHookMsg::Swap {
        pair: config.pair.to_string(),
        min_return: params.min_return,
        max_spread: params.max_spread,
        deadline: params.deadline,
    };
    Ok(WasmMsg::Execute {
        contract_addr: params.offer_token,
        msg: to_json_binary(&Cw20ExecuteMsg::Send {
            contract: config.proxy.to_string(),
            amount: params.amount,
            msg: to_json_binary(&hook)?,
        })?,
        funds: vec![],
    })
}

fn validate_swap(
    deps: Deps,
    env: &Env,
    config: &Config,
    params: &SwapParams,
) -> Result<(), ContractError> {
    let token = deps.api.addr_validate(&params.offer_token)?;
    if !config.asset_tokens.contains(&token) {
        return Err(ContractError::UnsupportedToken);
    }
    if params.amount.is_zero() {
        return Err(ContractError::ZeroAmount);
    }
    if params.deadline < env.block.time.seconds() {
        return Err(ContractError::Expired);
    }
    if params.max_spread > MAX_SPREAD {
        return Err(ContractError::ExcessiveSpread);
    }
    Ok(())
}

fn token_addr(deps: Deps, info: AssetInfo) -> Result<Addr, ContractError> {
    match info {
        AssetInfo::Token { contract_addr } => Ok(deps.api.addr_validate(&contract_addr)?),
        AssetInfo::NativeToken { .. } => Err(ContractError::InvalidPair),
    }
}

fn assert_admin(config: &Config, sender: &Addr) -> Result<(), ContractError> {
    if sender != config.admin {
        return Err(ContractError::Unauthorized);
    }
    Ok(())
}

fn assert_liquidity(config: &Config, sender: &Addr) -> Result<(), ContractError> {
    let liquidity = config
        .liquidity_contract
        .as_ref()
        .ok_or(ContractError::LiquidityNotConfigured)?;
    if sender != liquidity {
        return Err(ContractError::Unauthorized);
    }
    Ok(())
}

fn validate_threshold(value: u16) -> Result<(), ContractError> {
    if value == 0 || value > 10_000 {
        return Err(ContractError::InvalidThreshold);
    }
    Ok(())
}

fn validate_allocation_tolerance(value: u16) -> Result<(), ContractError> {
    if value == 0 || value > MAX_ALLOCATION_TOLERANCE_BPS {
        return Err(ContractError::InvalidThreshold);
    }
    Ok(())
}

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
    let spot = Decimal::from_ratio(reserves[1], reserves[0]);
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

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};
    use cosmwasm_std::{from_json, ContractResult, SystemResult, WasmQuery};

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
    fn thresholds_are_bounded() {
        assert_eq!(validate_threshold(0), Err(ContractError::InvalidThreshold));
        assert!(validate_threshold(500).is_ok());
        assert_eq!(
            validate_threshold(10_001),
            Err(ContractError::InvalidThreshold)
        );
    }

    #[test]
    fn threshold_updates_apply_the_correct_bounds() {
        let mut deps = mock_dependencies();
        CONFIG
            .save(
                deps.as_mut().storage,
                &Config {
                    admin: Addr::unchecked("admin"),
                    keeper: Addr::unchecked("keeper"),
                    liquidity_contract: None,
                    proxy: Addr::unchecked("proxy"),
                    pair: Addr::unchecked("pair"),
                    asset_tokens: [Addr::unchecked("token0"), Addr::unchecked("token1")],
                    decimals: 6,
                    twap_window_seconds: 0,
                    rebalance_threshold_bps: 500,
                    allocation_tolerance_bps: 500,
                    max_trade_bps: 2_500,
                    max_execution_deviation_bps: 500,
                    quote_slippage_bps: 200,
                    max_spot_twap_deviation_bps: 500,
                    max_trade_pool_bps: 1_000,
                    max_spread: Decimal::percent(5),
                    reference_price: Decimal::one(),
                },
            )
            .unwrap();
        execute_update_thresholds(
            deps.as_mut(),
            mock_env(),
            mock_info("admin", &[]),
            Some(5_000),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            CONFIG.load(&deps.storage).unwrap().rebalance_threshold_bps,
            5_000
        );
        let error = execute_update_thresholds(
            deps.as_mut(),
            mock_env(),
            mock_info("admin", &[]),
            None,
            Some(2_001),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap_err();
        assert_eq!(error, ContractError::InvalidThreshold);
    }

    #[test]
    fn twap_window_update_is_admin_gated_and_validated() {
        let mut deps = mock_dependencies();
        deps.querier.update_wasm(|query| match query {
            WasmQuery::Smart { msg, .. } => {
                let query: PairQueryMsg = from_json(msg).unwrap();
                match query {
                    PairQueryMsg::Observe { seconds_ago } => {
                        let window = Uint128::from(seconds_ago[1]);
                        SystemResult::Ok(ContractResult::Ok(
                            to_json_binary(&ObserveResponse {
                                price_a_cumulatives: vec![
                                    Decimal::from_ratio(2u128, 1u128).atomics() * window,
                                    Uint128::zero(),
                                ],
                                price_b_cumulatives: vec![Uint128::zero(), Uint128::zero()],
                            })
                            .unwrap(),
                        ))
                    }
                    _ => panic!("unexpected query"),
                }
            }
            _ => panic!("unexpected query"),
        });
        CONFIG
            .save(
                deps.as_mut().storage,
                &Config {
                    admin: Addr::unchecked("admin"),
                    keeper: Addr::unchecked("keeper"),
                    liquidity_contract: None,
                    proxy: Addr::unchecked("proxy"),
                    pair: Addr::unchecked("pair"),
                    asset_tokens: [Addr::unchecked("token0"), Addr::unchecked("token1")],
                    decimals: 6,
                    twap_window_seconds: 60,
                    rebalance_threshold_bps: 500,
                    allocation_tolerance_bps: 500,
                    max_trade_bps: 2_500,
                    max_execution_deviation_bps: 500,
                    quote_slippage_bps: 200,
                    max_spot_twap_deviation_bps: 500,
                    max_trade_pool_bps: 1_000,
                    max_spread: Decimal::percent(5),
                    reference_price: Decimal::one(),
                },
            )
            .unwrap();
        execute_update_thresholds(
            deps.as_mut(),
            mock_env(),
            mock_info("admin", &[]),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(300),
        )
        .unwrap();
        assert_eq!(CONFIG.load(&deps.storage).unwrap().twap_window_seconds, 300);
        assert_eq!(
            CONFIG.load(&deps.storage).unwrap().reference_price,
            Decimal::from_ratio(2u128, 1u128)
        );
        let error = execute_update_thresholds(
            deps.as_mut(),
            mock_env(),
            mock_info("admin", &[]),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(0),
        )
        .unwrap_err();
        assert_eq!(error, ContractError::InvalidTwapWindow);
        deps.querier.update_wasm(|_| {
            SystemResult::Ok(ContractResult::Err(
                "insufficient observation history".into(),
            ))
        });
        let error = execute_update_thresholds(
            deps.as_mut(),
            mock_env(),
            mock_info("admin", &[]),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(400),
        )
        .unwrap_err();
        assert!(matches!(error, ContractError::Std(_)));
        let unchanged = CONFIG.load(&deps.storage).unwrap();
        assert_eq!(unchanged.twap_window_seconds, 300);
        assert_eq!(unchanged.reference_price, Decimal::from_ratio(2u128, 1u128));
        let error = execute_update_thresholds(
            deps.as_mut(),
            mock_env(),
            mock_info("admin", &[]),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(MAX_TWAP_WINDOW_SECONDS + 1),
        )
        .unwrap_err();
        assert_eq!(error, ContractError::InvalidTwapWindow);
        let error = execute_update_thresholds(
            deps.as_mut(),
            mock_env(),
            mock_info("keeper", &[]),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(300),
        )
        .unwrap_err();
        assert_eq!(error, ContractError::Unauthorized);
    }

    #[test]
    fn cl8y_price_a_twap_is_token1_per_token0() {
        // Pinned CL8Y fad8011 accumulates price_a as reserve_b / reserve_a.
        let response = ObserveResponse {
            price_a_cumulatives: [Uint128::new(180), Uint128::new(60)].to_vec(),
            price_b_cumulatives: [Uint128::zero(), Uint128::zero()].to_vec(),
        };
        assert_eq!(
            twap_from_observation(&response, 60).unwrap(),
            Decimal::from_atomics(2u128, 18).unwrap()
        );
    }

    #[test]
    fn twap_controls_direction_amount_and_trade_cap() {
        let holdings = [Uint128::new(200), Uint128::new(100)];
        assert_eq!(
            planned_offer(holdings, Decimal::one(), 5_000).unwrap(),
            Some((0, Uint128::new(50)))
        );
        assert_eq!(
            planned_offer(holdings, Decimal::percent(50), 5_000).unwrap(),
            None
        );
        assert_eq!(
            planned_offer(
                [Uint128::new(1_000), Uint128::zero()],
                Decimal::one(),
                1_000
            )
            .unwrap(),
            Some((0, Uint128::new(100)))
        );
    }

    #[test]
    fn execution_floor_uses_twap_orientation() {
        assert_eq!(
            expected_return(Uint128::new(100), 0, Decimal::percent(200)).unwrap(),
            Uint128::new(200)
        );
        assert_eq!(
            expected_return(Uint128::new(100), 1, Decimal::percent(200)).unwrap(),
            Uint128::new(50)
        );
    }

    #[test]
    fn risk_controls_have_hard_bounds() {
        assert!(
            validate_risk_controls(5_000, 1_000, 500, 1_000, 2_000, Decimal::percent(10)).is_ok()
        );
        assert_eq!(
            validate_risk_controls(5_001, 1_000, 500, 1_000, 2_000, Decimal::percent(10)),
            Err(ContractError::InvalidRiskControl)
        );
        assert_eq!(
            validate_risk_controls(5_000, 1_001, 500, 1_000, 2_000, Decimal::percent(10)),
            Err(ContractError::InvalidRiskControl)
        );
    }

    #[test]
    fn pool_safety_bounds_spot_deviation_and_trade_depth() {
        let pool = PoolResponse {
            assets: [
                Asset {
                    info: AssetInfo::Token {
                        contract_addr: "token0".to_string(),
                    },
                    amount: Uint128::new(1_000),
                },
                Asset {
                    info: AssetInfo::Token {
                        contract_addr: "token1".to_string(),
                    },
                    amount: Uint128::new(1_000),
                },
            ],
            total_share: Uint128::new(1_000),
        };
        assert!(
            validate_pool_safety(&pool, Decimal::one(), 0, Uint128::new(100), 500, 1_000,).is_ok()
        );

        let mut manipulated = pool.clone();
        manipulated.assets[1].amount = Uint128::new(1_200);
        assert_eq!(
            validate_pool_safety(
                &manipulated,
                Decimal::one(),
                0,
                Uint128::new(100),
                500,
                1_000,
            ),
            Err(ContractError::UnsafePoolPrice)
        );
        assert_eq!(
            validate_pool_safety(&pool, Decimal::one(), 0, Uint128::new(101), 500, 1_000,),
            Err(ContractError::InsufficientPoolDepth)
        );
    }

    #[test]
    fn partial_improvement_commits_without_updating_reference() {
        assert!(!validate_rebalance_outcome(2_000, 1_500, 100).unwrap());
        assert!(validate_rebalance_outcome(2_000, 100, 100).unwrap());
        assert_eq!(
            validate_rebalance_outcome(2_000, 2_000, 100),
            Err(ContractError::AllocationDidNotImprove)
        );
    }

    #[test]
    fn reply_requires_exact_spend_and_minimum_output() {
        let pending = PendingRebalance {
            captured_twap: Decimal::one(),
            balances: [Uint128::new(200), Uint128::new(100)],
            pre_deviation_bps: 5_000,
            offer_index: 0,
            amount: Uint128::new(50),
            min_return: Uint128::new(48),
        };
        assert!(validate_settlement(&pending, [Uint128::new(150), Uint128::new(148)]).is_ok());
        assert_eq!(
            validate_settlement(&pending, [Uint128::new(150), Uint128::new(147)]),
            Err(ContractError::AllocationDidNotImprove)
        );
        assert_eq!(
            validate_settlement(&pending, [Uint128::new(151), Uint128::new(148)]),
            Err(ContractError::AllocationDidNotImprove)
        );
    }
}
