use bot_types::{
    AuthorizedTransfer, LiquidityAuthorizationResponse, RebalancePlanResponse,
    RebalanceStatusResponse, SwapParams, SwapProxyHookMsg, VaultBalancesResponse,
    VaultConfigResponse, VaultExecuteMsg, VaultFeeSharesResponse, VaultPriceResponse,
    VaultQueryMsg,
};
use cl8y_dex::{
    Asset, AssetInfo, HybridSimulationResponse, HybridSwapParams, ObserveResponse, PairInfo,
    PairQueryMsg, PoolResponse,
};
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    entry_point, to_json_binary, Addr, Binary, Decimal, Deps, DepsMut, Env, MessageInfo, Reply,
    Response, StdError, StdResult, SubMsg, Uint128, Uint256, WasmMsg,
};
use cw2::{get_contract_version, set_contract_version};
use cw20::{BalanceResponse, Cw20ExecuteMsg, Cw20QueryMsg, TokenInfoResponse};

use crate::error::ContractError;
use crate::msg::{InstantiateMsg, MigrateMsg};
use crate::state::{
    Config, PendingRebalance, CONFIG, LIQUIDITY_CODE_ID, PAUSED, PENDING_ADMIN, PENDING_REBALANCE,
};

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

#[cw_serde]
enum LiquidityQueryMsg {
    Config {},
    Authorization {},
}

#[cw_serde]
#[allow(dead_code)]
struct LiquidityConfigResponse {
    admin: String,
    pending_admin: Option<String>,
    vault: String,
    asset_tokens: [String; 2],
    minimum_initial_deposit: Uint128,
}

#[cw_serde]
enum LiquidityExecuteMsg {
    MintTo { recipient: String, amount: Uint128 },
}

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    if msg.liquidity_code_id == 0 {
        return Err(ContractError::InvalidLiquidityContract);
    }
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
    if !valid_twap_window(msg.twap_window_seconds) {
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
    let fee_registry = msg
        .fee_registry
        .as_deref()
        .map(|address| deps.api.addr_validate(address))
        .transpose()?;
    let fee_collector = msg
        .fee_collector
        .as_deref()
        .map(|address| deps.api.addr_validate(address))
        .transpose()?;
    let proxy = deps.api.addr_validate(&msg.proxy)?;
    #[cfg(feature = "mainnet")]
    {
        crate::mainnet::assert_fee_collector_canonical_mainnet(fee_collector.as_ref())?;
        crate::mainnet::assert_proxy_canonical_mainnet(&proxy)?;
    }
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
        proxy,
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
        fee_registry,
        fee_collector,
    };
    config.reference_price = query_price(deps.as_ref(), &env, &config)?;
    CONFIG.save(deps.storage, &config)?;
    LIQUIDITY_CODE_ID.save(deps.storage, &msg.liquidity_code_id)?;
    PAUSED.save(deps.storage, &false)?;
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("vault", env.contract.address))
}

#[entry_point]
pub fn migrate(deps: DepsMut, _env: Env, msg: MigrateMsg) -> Result<Response, ContractError> {
    let previous = get_contract_version(deps.storage)?;
    if previous.contract != CONTRACT_NAME {
        return Err(ContractError::Std(StdError::generic_err(
            "unsupported migration source",
        )));
    }
    if msg.liquidity_code_id == 0 {
        return Err(ContractError::InvalidLiquidityContract);
    }
    if PENDING_REBALANCE.may_load(deps.storage)?.is_some() {
        return Err(ContractError::RebalancePending);
    }
    // Legacy bindings were not reciprocally checked or code-pinned. Requiring an
    // explicit rebind is safer than silently preserving custodial authority.
    CONFIG.update(deps.storage, |mut config| -> StdResult<_> {
        config.liquidity_contract = None;
        Ok(config)
    })?;
    LIQUIDITY_CODE_ID.save(deps.storage, &msg.liquidity_code_id)?;
    PAUSED.save(deps.storage, &true)?;
    PENDING_ADMIN.remove(deps.storage);
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    Ok(Response::new()
        .add_attribute("action", "migrate_bot_vault")
        .add_attribute("from_version", previous.version)
        .add_attribute("liquidity_rebind_required", "true"))
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
            execute_set_liquidity(deps, env, info, liquidity_contract)
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
        VaultExecuteMsg::UpdateFeeConfig {
            fee_registry,
            fee_collector,
        } => execute_update_fee_config(deps, info, fee_registry, fee_collector),
        VaultExecuteMsg::TransferAdmin { admin } => execute_transfer_admin(deps, info, admin),
        VaultExecuteMsg::AcceptAdmin {} => execute_accept_admin(deps, info),
        VaultExecuteMsg::CancelAdminTransfer {} => execute_cancel_admin_transfer(deps, info),
        VaultExecuteMsg::RevokeLiquidityContract {} => execute_revoke_liquidity(deps, info),
        VaultExecuteMsg::Pause {} => execute_pause(deps, info),
        VaultExecuteMsg::Resume {} => execute_resume(deps, info),
    }
}

fn execute_set_liquidity(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    liquidity_contract: String,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;
    assert_admin(&config, &info.sender)?;
    if config.liquidity_contract.is_some() {
        return Err(ContractError::LiquidityAlreadyConfigured);
    }
    let candidate = deps.api.addr_validate(&liquidity_contract)?;
    let binding: LiquidityConfigResponse = deps
        .querier
        .query_wasm_smart(&candidate, &LiquidityQueryMsg::Config {})
        .map_err(|_| ContractError::InvalidLiquidityContract)?;
    if binding.vault != env.contract.address.as_str() {
        return Err(ContractError::LiquidityVaultMismatch);
    }
    if binding.asset_tokens != config.asset_tokens.clone().map(|token| token.to_string()) {
        return Err(ContractError::LiquidityAssetsMismatch);
    }
    let info = deps.querier.query_wasm_contract_info(&candidate)?;
    let approved_code_id = LIQUIDITY_CODE_ID.load(deps.storage)?;
    if info.code_id != approved_code_id {
        return Err(ContractError::LiquidityCodeMismatch);
    }
    config.liquidity_contract = Some(candidate);
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
    assert_liquidity(deps.as_ref(), &config, &info.sender)?;
    assert_not_paused(deps.as_ref())?;
    if PENDING_REBALANCE.may_load(deps.storage)?.is_some() {
        return Err(ContractError::RebalancePending);
    }
    let authorization = query_liquidity_authorization(deps.as_ref(), &config)?;
    if authorization.swap.as_ref() != Some(&params) {
        return Err(ContractError::LiquidityOperationNotAuthorized);
    }
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
    assert_liquidity(deps.as_ref(), &config, &info.sender)?;
    if PENDING_REBALANCE.may_load(deps.storage)?.is_some() {
        return Err(ContractError::RebalancePending);
    }
    let token = deps.api.addr_validate(&token)?;
    if !config.asset_tokens.contains(&token) {
        return Err(ContractError::UnsupportedToken);
    }
    if amount.is_zero() {
        return Err(ContractError::ZeroAmount);
    }
    let recipient = deps.api.addr_validate(&recipient)?;
    let requested = AuthorizedTransfer {
        token: token.to_string(),
        amount,
        recipient: recipient.to_string(),
    };
    if !query_liquidity_authorization(deps.as_ref(), &config)?
        .transfers
        .contains(&requested)
    {
        return Err(ContractError::LiquidityOperationNotAuthorized);
    }
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
    assert_liquidity(deps.as_ref(), &config, &info.sender)?;
    assert_not_paused(deps.as_ref())?;
    if PENDING_REBALANCE.may_load(deps.storage)?.is_some() {
        return Err(ContractError::RebalancePending);
    }
    if !query_liquidity_authorization(deps.as_ref(), &config)?.finalize {
        return Err(ContractError::LiquidityOperationNotAuthorized);
    }
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
    assert_not_paused(deps.as_ref())?;
    if PENDING_REBALANCE.may_load(deps.storage)?.is_some() {
        return Err(ContractError::RebalancePending);
    }
    ensure_no_liquidity_operation(deps.as_ref(), &config)?;
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
    assert_not_paused(deps.as_ref())?;
    ensure_no_liquidity_operation(deps.as_ref(), &config)?;
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
    let mut deps = deps;
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
    let offer = pending.offer_index as usize;
    let ask = 1 - offer;
    let proceeds = settled[ask]
        .checked_sub(pending.balances[ask])
        .map_err(StdError::overflow)?;
    let value_in_token0 = if ask == 0 {
        proceeds
    } else {
        checked_ratio(
            proceeds,
            Decimal::one().atomics(),
            pending.captured_twap.atomics(),
        )?
    };
    let mut response = Response::new()
        .add_attribute("action", "complete_rebalance")
        .add_attribute("allocation_deviation_bps", current.to_string())
        .add_attribute("reference_updated", within_tolerance.to_string());
    let (fee_charge, mint_msgs) = charge_fee(&mut deps, &config, value_in_token0)?;
    match fee_charge {
        ChargeFee::None => {}
        ChargeFee::Applied(fee) => {
            response = response
                .add_attribute("fee_bps", fee.fee_bps.to_string())
                .add_attribute("fee_holders", fee.holders.to_string())
                .add_attribute(
                    "fee_tier",
                    fee.tier.map(|t| t.to_string()).unwrap_or_default(),
                )
                .add_attribute("fee_source", fee.source)
                .add_attribute("fee_shares", fee.shares.to_string());
        }
        ChargeFee::Unavailable(reason) => {
            response = response.add_attribute("fee_skipped", reason);
        }
    }
    for mint_msg in mint_msgs {
        response = response.add_message(mint_msg);
    }
    Ok(response)
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
        if !valid_twap_window(value) {
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
    let pending = PENDING_ADMIN
        .may_load(deps.storage)?
        .ok_or(ContractError::Unauthorized)?;
    if info.sender != pending {
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

fn execute_revoke_liquidity(deps: DepsMut, info: MessageInfo) -> Result<Response, ContractError> {
    CONFIG.update(deps.storage, |mut config| -> Result<_, ContractError> {
        assert_admin(&config, &info.sender)?;
        config.liquidity_contract = None;
        Ok(config)
    })?;
    PAUSED.save(deps.storage, &true)?;
    Ok(Response::new().add_attribute("action", "revoke_liquidity_contract"))
}

fn execute_pause(deps: DepsMut, info: MessageInfo) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    assert_admin(&config, &info.sender)?;
    if PAUSED.may_load(deps.storage)?.unwrap_or(true) {
        return Err(ContractError::Paused);
    }
    PAUSED.save(deps.storage, &true)?;
    Ok(Response::new().add_attribute("action", "pause"))
}

fn execute_resume(deps: DepsMut, info: MessageInfo) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    assert_admin(&config, &info.sender)?;
    if !PAUSED.may_load(deps.storage)?.unwrap_or(true) {
        return Err(ContractError::NotPaused);
    }
    let liquidity = config
        .liquidity_contract
        .as_ref()
        .ok_or(ContractError::LiquidityNotConfigured)?;
    let expected = LIQUIDITY_CODE_ID.load(deps.storage)?;
    if deps.querier.query_wasm_contract_info(liquidity)?.code_id != expected {
        return Err(ContractError::LiquidityCodeMismatch);
    }
    PAUSED.save(deps.storage, &false)?;
    Ok(Response::new().add_attribute("action", "resume"))
}

#[entry_point]
pub fn query(deps: Deps, env: Env, msg: VaultQueryMsg) -> StdResult<Binary> {
    let config = CONFIG.load(deps.storage)?;
    match msg {
        VaultQueryMsg::Config {} => to_json_binary(&VaultConfigResponse {
            admin: config.admin.to_string(),
            pending_admin: PENDING_ADMIN
                .may_load(deps.storage)?
                .map(|admin| admin.to_string()),
            keeper: config.keeper.to_string(),
            liquidity_contract: config.liquidity_contract.map(|addr| addr.to_string()),
            liquidity_code_id: LIQUIDITY_CODE_ID.may_load(deps.storage)?,
            paused: PAUSED.may_load(deps.storage)?.unwrap_or(true),
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
            fee_registry: config.fee_registry.as_ref().map(ToString::to_string),
            fee_collector: config.fee_collector.as_ref().map(ToString::to_string),
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
        VaultQueryMsg::Shares { bot_id: _, address } => {
            let address = deps.api.addr_validate(&address)?;
            let shares = match &config.liquidity_contract {
                Some(liquidity) => {
                    let balance: BalanceResponse = deps.querier.query_wasm_smart(
                        liquidity,
                        &Cw20QueryMsg::Balance {
                            address: address.to_string(),
                        },
                    )?;
                    balance.balance
                }
                None => Uint128::zero(),
            };
            to_json_binary(&VaultFeeSharesResponse { shares })
        }
    }
}

fn rebalance_status(deps: Deps, env: &Env, config: &Config) -> StdResult<RebalanceStatusResponse> {
    let current_price = query_price(deps, env, config)?;
    let holdings = balances(deps, &env.contract.address, config)?;
    let allocation_deviation_bps = allocation_deviation(holdings, current_price)?;
    let price_deviation_bps = relative_deviation(current_price, config.reference_price)?;
    Ok(RebalanceStatusResponse {
        should_rebalance: rebalance_required(price_deviation_bps, allocation_deviation_bps, config),
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
    let should_rebalance =
        rebalance_required(price_deviation_bps, allocation_deviation_bps, config);
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

fn assert_not_paused(deps: Deps) -> Result<(), ContractError> {
    if PAUSED.may_load(deps.storage)?.unwrap_or(true) {
        return Err(ContractError::Paused);
    }
    Ok(())
}

fn assert_liquidity(deps: Deps, config: &Config, sender: &Addr) -> Result<(), ContractError> {
    let liquidity = config
        .liquidity_contract
        .as_ref()
        .ok_or(ContractError::LiquidityNotConfigured)?;
    if sender != liquidity {
        return Err(ContractError::Unauthorized);
    }
    let expected = LIQUIDITY_CODE_ID
        .may_load(deps.storage)?
        .ok_or(ContractError::LiquidityCodeMismatch)?;
    if deps.querier.query_wasm_contract_info(liquidity)?.code_id != expected {
        return Err(ContractError::LiquidityCodeMismatch);
    }
    Ok(())
}

fn query_liquidity_authorization(
    deps: Deps,
    config: &Config,
) -> Result<LiquidityAuthorizationResponse, ContractError> {
    let liquidity = config
        .liquidity_contract
        .as_ref()
        .ok_or(ContractError::LiquidityNotConfigured)?;
    deps.querier
        .query_wasm_smart(liquidity, &LiquidityQueryMsg::Authorization {})
        .map_err(ContractError::from)
}

fn ensure_no_liquidity_operation(deps: Deps, config: &Config) -> Result<(), ContractError> {
    if config.liquidity_contract.is_none() {
        return Ok(());
    }
    let authorization = query_liquidity_authorization(deps, config)?;
    if authorization.swap.is_some() || !authorization.transfers.is_empty() || authorization.finalize
    {
        return Err(ContractError::RebalancePending);
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

fn rebalance_required(
    price_deviation_bps: u16,
    allocation_deviation_bps: u16,
    config: &Config,
) -> bool {
    price_deviation_bps >= config.rebalance_threshold_bps
        || allocation_deviation_bps > config.allocation_tolerance_bps
}

fn validate_threshold(value: u16) -> Result<(), ContractError> {
    if value == 0 || value > 10_000 {
        return Err(ContractError::InvalidThreshold);
    }
    Ok(())
}

fn valid_twap_window(value: u32) -> bool {
    (1..=MAX_TWAP_WINDOW_SECONDS).contains(&value)
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
    let spot_atomics = checked_ratio(reserves[1], Decimal::one().atomics(), reserves[0])?;
    let spot = Decimal::from_atomics(spot_atomics, Decimal::DECIMAL_PLACES)
        .map_err(|_| ContractError::ArithmeticOutOfRange)?;
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

#[cw_serde]
enum FeeRegistryQueryMsg {
    EffectiveFee { trader: String },
}

#[cw_serde]
struct FeeRegistryEffectiveFeeResponse {
    fee_bps: u16,
    discount_bps: u16,
    tier_id: Option<u8>,
    /// The registry always returns the holding it used; if the vault fails to
    /// mirror it here, `cw_serde` rejects the response and the fee is skipped.
    holding: Option<Uint128>,
    source: String,
}

struct FeeApplied {
    /// Weighted-average effective fee across all charged LP holders, in bps.
    fee_bps: u16,
    shares: Uint128,
    /// The number of LP holders whose share was charged at their own tier.
    holders: usize,
    tier: Option<u8>,
    source: String,
}

enum ChargeFee {
    None,
    Applied(FeeApplied),
    /// Fee-registry read failed. The rebalance still completes; the fee is
    /// skipped rather than reverting the swap.
    Unavailable(String),
}

/// Protocol fee per executed rebalance swap (token-0 value based), resolved as
/// a **single user** (`FEE_TIER_PROTOCOL.md` §5): the operating user — the
/// address that instantiated / deposited into the vault (`config.admin`) — is
/// billed at THEIR OWN tier for the full fill value. Only the fee is minted as
/// LP to the configured fee-collector — no user LP is minted on a fill (the user's
/// deposited bot-liquidity position simply appreciates via NAV). No fee is charged
/// when the vault has no fee configured, no liquidity contract, or the resolved
/// rate is zero. A transient fee-registry failure is non-blocking (the whole charge
/// is skipped, never a revert). Returns the mint messages to append to the
/// rebalance response.
fn charge_fee(
    deps: &mut DepsMut,
    config: &Config,
    value_in_token0: Uint128,
) -> Result<(ChargeFee, Vec<WasmMsg>), ContractError> {
    let (Some(registry), Some(collector)) = (&config.fee_registry, &config.fee_collector) else {
        return Ok((ChargeFee::None, vec![]));
    };
    if value_in_token0.is_zero() {
        return Ok((ChargeFee::None, vec![]));
    }
    let Some(liquidity) = &config.liquidity_contract else {
        // No pooled LP token to mint into: nothing to charge.
        return Ok((ChargeFee::None, vec![]));
    };

    // Single-user (`FEE_TIER_PROTOCOL.md` §5): the operating user — the address
    // that instantiated / deposited into the vault — is billed at THEIR OWN tier
    // for the full fill value. `config.admin` is the operator identity.
    let user = config.admin.to_string();
    let fee: FeeRegistryEffectiveFeeResponse = match deps.querier.query_wasm_smart(
        registry,
        &FeeRegistryQueryMsg::EffectiveFee {
            trader: user.clone(),
        },
    ) {
        Ok(fee) => fee,
        Err(err) => return Ok((ChargeFee::Unavailable(err.to_string()), vec![])),
    };
    // Defensive bound: cap the effective fee at 100% (10_000 bps).
    let fee_bps = fee.fee_bps.min(10_000);
    if fee_bps == 0 {
        return Ok((ChargeFee::None, vec![]));
    }
    let fee_lp = value_in_token0.multiply_ratio(fee_bps, 10_000u16);
    if fee_lp.is_zero() {
        return Ok((ChargeFee::None, vec![]));
    }

    // Single-user (`FEE_TIER_PROTOCOL.md` §5, model B): a fill mints NO extra LP
    // to the user — their deposited bot-liquidity position appreciates via NAV.
    // Only the protocol fee is minted, straight to the fee-collector.
    let mint_msg = |recipient: &str, amount: Uint128| -> StdResult<WasmMsg> {
        Ok(WasmMsg::Execute {
            contract_addr: liquidity.to_string(),
            msg: to_json_binary(&LiquidityExecuteMsg::MintTo {
                recipient: recipient.to_string(),
                amount,
            })?,
            funds: vec![],
        })
    };
    let messages = vec![mint_msg(collector.as_str(), fee_lp)?];

    Ok((
        ChargeFee::Applied(FeeApplied {
            fee_bps,
            shares: fee_lp,
            holders: 1,
            tier: fee.tier_id,
            source: fee.source,
        }),
        messages,
    ))
}

fn execute_update_fee_config(
    deps: DepsMut,
    info: MessageInfo,
    fee_registry: Option<String>,
    fee_collector: Option<String>,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;
    assert_admin(&config, &info.sender)?;
    #[cfg(feature = "mainnet")]
    if let Some(value) = &fee_collector {
        let parsed = match value.as_str() {
            "" => None,
            address => Some(deps.api.addr_validate(address)?),
        };
        crate::mainnet::assert_fee_collector_canonical_mainnet(parsed.as_ref())?;
    }
    if let Some(value) = fee_registry {
        config.fee_registry = match value.as_str() {
            "" => None,
            address => Some(deps.api.addr_validate(address)?),
        };
    }
    if let Some(value) = fee_collector {
        config.fee_collector = match value.as_str() {
            "" => None,
            address => Some(deps.api.addr_validate(address)?),
        };
    }
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new().add_attribute("action", "update_fee_config"))
}

#[cfg(test)]
fn vault_config(
    liquidity: Option<&str>,
    registry: Option<&str>,
    collector: Option<&str>,
) -> Config {
    Config {
        admin: Addr::unchecked("admin"),
        keeper: Addr::unchecked("keeper"),
        liquidity_contract: liquidity.map(Addr::unchecked),
        proxy: Addr::unchecked("proxy"),
        pair: Addr::unchecked("pair"),
        asset_tokens: [Addr::unchecked("t0"), Addr::unchecked("t1")],
        decimals: 6,
        twap_window_seconds: 300,
        rebalance_threshold_bps: 500,
        allocation_tolerance_bps: 100,
        max_trade_bps: 2_500,
        max_execution_deviation_bps: 500,
        quote_slippage_bps: 200,
        max_spot_twap_deviation_bps: 500,
        max_trade_pool_bps: 1_000,
        max_spread: Decimal::percent(5),
        reference_price: Decimal::one(),
        fee_registry: registry.map(Addr::unchecked),
        fee_collector: collector.map(Addr::unchecked),
    }
}

/// A mock bot-liquidity pool (cw20) plus fee-registry: `holders` is
/// `(address, lp_balance)`; `registry_rates` is `(address, fee_bps)`.
#[cfg(test)]
fn install_pool_mock(
    deps: &mut cosmwasm_std::OwnedDeps<
        cosmwasm_std::testing::MockStorage,
        cosmwasm_std::testing::MockApi,
        cosmwasm_std::testing::MockQuerier,
        cosmwasm_std::Empty,
    >,
    holders: &[(&str, u128)],
    registry_rates: &[(&str, u16)],
) {
    use cosmwasm_std::{from_json, ContractResult, SystemResult, WasmQuery};
    let holders: Vec<(String, u128)> = holders.iter().map(|(a, b)| (a.to_string(), *b)).collect();
    let registry_rates: Vec<(String, u16)> = registry_rates
        .iter()
        .map(|(a, b)| (a.to_string(), *b))
        .collect();
    deps.querier.update_wasm(move |query| match query {
        WasmQuery::Smart { contract_addr, msg } => {
            if contract_addr == "liquidity" {
                pool_smart_response(contract_addr, msg, &holders)
            } else if contract_addr == "registry" {
                let query: FeeRegistryQueryMsg = from_json(msg).unwrap();
                match query {
                    FeeRegistryQueryMsg::EffectiveFee { trader } => {
                        let fee_bps = registry_rates
                            .iter()
                            .find(|(a, _)| *a == trader)
                            .map(|(_, b)| *b)
                            .unwrap_or(0);
                        SystemResult::Ok(ContractResult::Ok(
                            to_json_binary(&FeeRegistryEffectiveFeeResponse {
                                fee_bps,
                                discount_bps: 0,
                                tier_id: None,
                                holding: None,
                                source: "live".to_string(),
                            })
                            .unwrap(),
                        ))
                    }
                }
            } else {
                panic!("unexpected contract {}", contract_addr)
            }
        }
        _ => panic!("unexpected query"),
    });
}

#[cfg(test)]
fn pool_smart_response(
    _contract_addr: &str,
    msg: &Binary,
    holders: &[(String, u128)],
) -> cosmwasm_std::SystemResult<cosmwasm_std::ContractResult<Binary>> {
    use cosmwasm_std::{from_json, ContractResult, SystemResult};
    let query: Cw20QueryMsg = from_json(msg).unwrap();
    match query {
        Cw20QueryMsg::TokenInfo {} => {
            let total_supply = holders.iter().fold(0u128, |acc, (_, b)| acc + b);
            SystemResult::Ok(ContractResult::Ok(
                to_json_binary(&TokenInfoResponse {
                    name: "Pool LP".to_string(),
                    symbol: "PLP".to_string(),
                    decimals: 6,
                    total_supply: Uint128::new(total_supply),
                })
                .unwrap(),
            ))
        }
        Cw20QueryMsg::AllAccounts { .. } => {
            let accounts: Vec<String> = holders.iter().map(|(a, _)| a.clone()).collect();
            SystemResult::Ok(ContractResult::Ok(
                to_json_binary(&cw20::AllAccountsResponse { accounts }).unwrap(),
            ))
        }
        Cw20QueryMsg::Balance { address } => {
            let balance = holders
                .iter()
                .find(|(a, _)| *a == address)
                .map(|(_, b)| Uint128::new(*b))
                .unwrap_or_else(Uint128::zero);
            SystemResult::Ok(ContractResult::Ok(
                to_json_binary(&BalanceResponse { balance }).unwrap(),
            ))
        }
        _ => panic!("unexpected liquidity query"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};
    use cosmwasm_std::{from_json, ContractInfoResponse, ContractResult, SystemResult, WasmQuery};

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
        assert!(!valid_twap_window(0));
        assert!(valid_twap_window(MAX_TWAP_WINDOW_SECONDS));
        assert!(!valid_twap_window(MAX_TWAP_WINDOW_SECONDS + 1));
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
                    fee_registry: None,
                    fee_collector: None,
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
                    fee_registry: None,
                    fee_collector: None,
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
    fn expected_return_reports_extreme_overflow_without_panicking() {
        let maximum_price = Decimal::from_atomics(Uint128::MAX, 18).unwrap();
        assert!(expected_return(Uint128::MAX, 0, maximum_price).is_err());
        let minimum_price = Decimal::from_atomics(Uint128::one(), 18).unwrap();
        assert!(expected_return(Uint128::MAX, 1, minimum_price).is_err());
        assert_eq!(
            expected_return(Uint128::zero(), 0, maximum_price).unwrap(),
            Uint128::zero()
        );
    }

    #[test]
    fn deviation_caps_extreme_intermediate_without_panicking() {
        assert_eq!(
            ratio_deviation(Uint256::MAX, Uint256::one()).unwrap(),
            10_000
        );
    }

    #[test]
    fn liquidity_authority_is_rejected_after_unapproved_migration() {
        let mut deps = mock_dependencies();
        let config = Config {
            admin: Addr::unchecked("admin"),
            keeper: Addr::unchecked("keeper"),
            liquidity_contract: Some(Addr::unchecked("liquidity")),
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
            fee_registry: None,
            fee_collector: None,
        };
        CONFIG.save(deps.as_mut().storage, &config).unwrap();
        LIQUIDITY_CODE_ID.save(deps.as_mut().storage, &7).unwrap();
        PAUSED.save(deps.as_mut().storage, &false).unwrap();
        deps.querier.update_wasm(|query| match query {
            WasmQuery::ContractInfo { .. } => {
                let mut response = ContractInfoResponse::default();
                response.code_id = 8;
                SystemResult::Ok(ContractResult::Ok(to_json_binary(&response).unwrap()))
            }
            _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
        });
        assert_eq!(
            assert_liquidity(deps.as_ref(), &config, &Addr::unchecked("liquidity")).unwrap_err(),
            ContractError::LiquidityCodeMismatch
        );
        deps.querier.update_wasm(|query| match query {
            WasmQuery::ContractInfo { .. } => {
                let mut response = ContractInfoResponse::default();
                response.code_id = 7;
                SystemResult::Ok(ContractResult::Ok(to_json_binary(&response).unwrap()))
            }
            WasmQuery::Smart { .. } => SystemResult::Ok(ContractResult::Ok(
                to_json_binary(&LiquidityAuthorizationResponse {
                    swap: None,
                    transfers: vec![AuthorizedTransfer {
                        token: "token0".into(),
                        amount: Uint128::new(10),
                        recipient: "approved".into(),
                    }],
                    finalize: false,
                })
                .unwrap(),
            )),
            _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
        });
        assert_eq!(
            execute_transfer(
                deps.as_mut(),
                mock_info("liquidity", &[]),
                "token0".into(),
                Uint128::new(10),
                "attacker".into(),
            )
            .unwrap_err(),
            ContractError::LiquidityOperationNotAuthorized
        );
        let valid = execute_transfer(
            deps.as_mut(),
            mock_info("liquidity", &[]),
            "token0".into(),
            Uint128::new(10),
            "approved".into(),
        )
        .unwrap();
        assert_eq!(valid.messages.len(), 1);
        assert_eq!(
            execute_pause(deps.as_mut(), mock_info("attacker", &[])).unwrap_err(),
            ContractError::Unauthorized
        );
        execute_pause(deps.as_mut(), mock_info("admin", &[])).unwrap();
        assert_eq!(
            assert_not_paused(deps.as_ref()).unwrap_err(),
            ContractError::Paused
        );
        execute_resume(deps.as_mut(), mock_info("admin", &[])).unwrap();
        assert!(!PAUSED.load(&deps.storage).unwrap());
        execute_revoke_liquidity(deps.as_mut(), mock_info("admin", &[])).unwrap();
        assert!(CONFIG
            .load(&deps.storage)
            .unwrap()
            .liquidity_contract
            .is_none());
        assert_eq!(LIQUIDITY_CODE_ID.load(&deps.storage).unwrap(), 7);
        assert!(PAUSED.load(&deps.storage).unwrap());
        assert!(rebalance_required(500, 0, &config));
        assert!(rebalance_required(0, 501, &config));
        assert!(!rebalance_required(499, 500, &config));
    }

    #[test]
    fn migration_revokes_liquidity_and_starts_paused() {
        let mut deps = mock_dependencies();
        let config = Config {
            admin: Addr::unchecked("admin"),
            keeper: Addr::unchecked("keeper"),
            liquidity_contract: Some(Addr::unchecked("legacy_liquidity")),
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
            fee_registry: None,
            fee_collector: None,
        };
        CONFIG.save(deps.as_mut().storage, &config).unwrap();
        set_contract_version(deps.as_mut().storage, CONTRACT_NAME, "0.0.1").unwrap();
        migrate(
            deps.as_mut(),
            mock_env(),
            MigrateMsg {
                liquidity_code_id: 42,
            },
        )
        .unwrap();
        assert!(CONFIG
            .load(&deps.storage)
            .unwrap()
            .liquidity_contract
            .is_none());
        assert_eq!(LIQUIDITY_CODE_ID.load(&deps.storage).unwrap(), 42);
        assert!(PAUSED.load(&deps.storage).unwrap());
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

    #[test]
    fn fee_config_update_is_admin_gated_and_can_clear() {
        let mut deps = mock_dependencies();
        CONFIG
            .save(
                &mut deps.storage,
                &Config {
                    admin: Addr::unchecked("admin"),
                    keeper: Addr::unchecked("keeper"),
                    liquidity_contract: None,
                    proxy: Addr::unchecked("proxy"),
                    pair: Addr::unchecked("pair"),
                    asset_tokens: [Addr::unchecked("t0"), Addr::unchecked("t1")],
                    decimals: 6,
                    twap_window_seconds: 300,
                    rebalance_threshold_bps: 500,
                    allocation_tolerance_bps: 100,
                    max_trade_bps: 2_500,
                    max_execution_deviation_bps: 500,
                    quote_slippage_bps: 200,
                    max_spot_twap_deviation_bps: 500,
                    max_trade_pool_bps: 1_000,
                    max_spread: Decimal::percent(5),
                    reference_price: Decimal::one(),
                    fee_registry: None,
                    fee_collector: None,
                },
            )
            .unwrap();

        assert_eq!(
            execute_update_fee_config(
                deps.as_mut(),
                mock_info("attacker", &[]),
                Some("registry".to_string()),
                Some("collector".to_string()),
            )
            .unwrap_err(),
            ContractError::Unauthorized
        );

        execute_update_fee_config(
            deps.as_mut(),
            mock_info("admin", &[]),
            Some("registry".to_string()),
            Some("collector".to_string()),
        )
        .unwrap();
        let config = CONFIG.load(&deps.storage).unwrap();
        assert_eq!(config.fee_registry, Some(Addr::unchecked("registry")));
        assert_eq!(config.fee_collector, Some(Addr::unchecked("collector")));

        execute_update_fee_config(
            deps.as_mut(),
            mock_info("admin", &[]),
            Some(String::new()),
            Some(String::new()),
        )
        .unwrap();
        let config = CONFIG.load(&deps.storage).unwrap();
        assert_eq!(config.fee_registry, None);
        assert_eq!(config.fee_collector, None);
    }

    #[test]
    fn charge_fee_skips_when_not_configured_or_zero_value() {
        let mut deps = mock_dependencies();
        let config = vault_config(None, None, None);
        assert!(matches!(
            charge_fee(&mut deps.as_mut(), &config, Uint128::new(1_000))
                .unwrap()
                .0,
            ChargeFee::None
        ));

        // Configured user + pool but zero fill value => nothing to tax.
        let mut deps = mock_dependencies();
        install_pool_mock(&mut deps, &[], &[("admin", 1_000)]);
        let config = vault_config(Some("liquidity"), Some("registry"), Some("collector"));
        assert!(matches!(
            charge_fee(&mut deps.as_mut(), &config, Uint128::zero())
                .unwrap()
                .0,
            ChargeFee::None
        ));
    }

    #[test]
    fn charge_fee_mints_only_fee_lp_to_collector() {
        let mut deps = mock_dependencies();
        // Single user = the operating vault `admin`, taxed at 1_000 bps (10%).
        // Fill value 1 000 => fee 100 LP to collector, NO user LP minted (model B).
        install_pool_mock(&mut deps, &[], &[("admin", 1_000)]);
        let config = vault_config(Some("liquidity"), Some("registry"), Some("collector"));
        let (charge, messages) =
            charge_fee(&mut deps.as_mut(), &config, Uint128::new(1_000)).unwrap();
        match charge {
            ChargeFee::Applied(fee) => {
                assert_eq!(fee.fee_bps, 1_000, "single user at their exact rate");
                assert_eq!(fee.shares, Uint128::new(100), "fee LP to collector");
                assert_eq!(fee.holders, 1);
                assert_eq!(fee.source, "live");
            }
            _other => panic!("expected Applied fee, got a different ChargeFee variant"),
        }
        assert_eq!(messages.len(), 1, "only the collector fee LP is minted");
        let minted: Vec<(String, Uint128)> = messages
            .iter()
            .map(|m| match m {
                WasmMsg::Execute {
                    contract_addr, msg, ..
                } if contract_addr == "liquidity" => {
                    let parsed: LiquidityExecuteMsg = from_json(msg).unwrap();
                    match parsed {
                        LiquidityExecuteMsg::MintTo { recipient, amount } => (recipient, amount),
                    }
                }
                m => panic!("unexpected message {m:?}"),
            })
            .collect();
        assert!(minted.contains(&("collector".to_string(), Uint128::new(100))));
        assert!(
            !minted.iter().any(|(r, _)| r == "admin"),
            "no user LP is minted on a fill"
        );
    }

    #[test]
    fn charge_fee_applies_every_canonical_ladder_rate() {
        const LADDER: [(u8, u16); 9] = [
            (1, 1_755),
            (2, 1_620),
            (3, 1_440),
            (4, 1_170),
            (5, 900),
            (6, 720),
            (7, 450),
            (8, 270),
            (9, 90),
        ];

        for (tier_id, fee_bps) in LADDER {
            let mut deps = mock_dependencies();
            install_pool_mock(&mut deps, &[], &[("admin", fee_bps)]);
            let config = vault_config(Some("liquidity"), Some("registry"), Some("collector"));
            let fill_value = Uint128::new(100_000);
            let (charge, messages) = charge_fee(&mut deps.as_mut(), &config, fill_value).unwrap();
            let expected_fee_lp = fill_value.multiply_ratio(fee_bps, 10_000u16);

            match charge {
                ChargeFee::Applied(fee) => {
                    assert_eq!(fee.fee_bps, fee_bps, "tier {tier_id} fee_bps");
                    assert_eq!(fee.shares, expected_fee_lp, "tier {tier_id} fee LP");
                    assert_eq!(fee.holders, 1, "tier {tier_id} is single-user model");
                    assert_eq!(fee.source, "live", "tier {tier_id} source");
                }
                _other => panic!("expected Applied fee for tier {tier_id}"),
            }

            assert_eq!(messages.len(), 1, "tier {tier_id} collector mint count");
            match &messages[0] {
                WasmMsg::Execute {
                    contract_addr, msg, ..
                } if contract_addr == "liquidity" => {
                    let parsed: LiquidityExecuteMsg = from_json(msg).unwrap();
                    match parsed {
                        LiquidityExecuteMsg::MintTo { recipient, amount } => {
                            assert_eq!(recipient, "collector", "tier {tier_id} recipient");
                            assert_eq!(amount, expected_fee_lp, "tier {tier_id} mint amount");
                        }
                    }
                }
                other => panic!("unexpected message for tier {tier_id}: {other:?}"),
            }
        }
    }

    #[test]
    fn charge_fee_skips_when_registry_unreachable_or_zero_rate() {
        // A configured pool but the fee-registry query fails => the whole charge
        // is skipped non-blockingly (no fee, no revert).
        let mut deps = mock_dependencies();
        deps.querier.update_wasm(|query| match query {
            WasmQuery::Smart { contract_addr, .. } => {
                if contract_addr == "liquidity" {
                    SystemResult::Ok(ContractResult::Ok(
                        to_json_binary(&TokenInfoResponse {
                            name: "Pool LP".to_string(),
                            symbol: "PLP".to_string(),
                            decimals: 6,
                            total_supply: Uint128::new(2_000),
                        })
                        .unwrap(),
                    ))
                } else {
                    SystemResult::Err(cosmwasm_std::SystemError::InvalidRequest {
                        request: Default::default(),
                        error: "boom".to_string(),
                    })
                }
            }
            _ => panic!("unexpected query"),
        });
        let config = vault_config(Some("liquidity"), Some("registry"), Some("collector"));
        assert!(matches!(
            charge_fee(&mut deps.as_mut(), &config, Uint128::new(1_000))
                .unwrap()
                .0,
            ChargeFee::Unavailable(_)
        ));

        // Zero rate returned by the registry for the user => skip.
        let mut deps = mock_dependencies();
        install_pool_mock(&mut deps, &[], &[("admin", 0)]);
        let config = vault_config(Some("liquidity"), Some("registry"), Some("collector"));
        assert!(matches!(
            charge_fee(&mut deps.as_mut(), &config, Uint128::new(5_000))
                .unwrap()
                .0,
            ChargeFee::None
        ));
    }
}
