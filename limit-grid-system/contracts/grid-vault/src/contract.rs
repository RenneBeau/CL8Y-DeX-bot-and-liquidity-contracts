use std::{cmp::Ordering as CmpOrdering, collections::BTreeSet};

use cl8y_grid_manager::limits::valid_vault_limits;
use cosmwasm_std::{
    entry_point, from_json, to_json_binary, Addr, BankMsg, Binary, Coin, Decimal, Deps, DepsMut,
    Env, MessageInfo, Order, Reply, Response, StdError, StdResult, SubMsg, SubMsgResponse, Uint128,
    Uint256, WasmMsg,
};
use cw2::{get_contract_version, set_contract_version};
use cw20::{BalanceResponse, Cw20ExecuteMsg, Cw20QueryMsg, Cw20ReceiveMsg, TokenInfoResponse};
use cw_storage_plus::Bound;
use semver::Version;

use cosmwasm_schema::cw_serde;

use crate::error::ContractError;
use crate::msg::{
    AssetInfo, BotResponse, CancelledOrderResponse, CancelledOrdersResponse, ConfigResponse,
    ExecuteMsg, ExpiredLimitRefundResponse, FactoryQueryMsg, InstantiateMsg,
    LimitOrderConfigResponse, LimitOrderPlacementItem, LimitOrderResponse, LimitOrderSide,
    MigrateMsg, OrderResponse, PairCw20HookMsg, PairExecuteMsg, PairInfo, PairQueryMsg,
    PairResponse, PoolResponse, QueryMsg, ReceiveMsg, RungResponse, ShareResponse,
    SolvencyResponse, TokenPolicyResponse, VaultModeResponse,
};
use crate::state::{
    Bot, CancelledOrder, Config, GridOrder, PageKind, PendingPage, PendingPageEntry, PlacementPlan,
    Rung, VaultMode, ALLOWED_TOKENS, BOTS, CANCELLED_ORDERS, CONFIG, NEXT_BOT_ID, NEXT_REPLY_ID,
    ORDERS, PENDING_PAGES, PLACEMENTS, QUARANTINE, RUNGS, SHARES, TOKEN_POLICY_ENABLED, VAULT_MODE,
};

const CONTRACT_NAME: &str = "crates.io:cl8y-grid-vault";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
const FIRST_REPLY_ID: u64 = 1;
const MAX_ADJUST_STEPS: u32 = 64;
const BOT_ID: u64 = 1;
const MIN_MIGRATION_VERSION: &str = "0.1.0";
const MAX_CANCELLED_ORDERS_PAGE_SIZE: u32 = 100;

/// Minimal schema of the protocol fee-registry `EffectiveFee` query.
#[cw_serde]
enum FeeRegistryQueryMsg {
    EffectiveFee { trader: String },
}

#[cw_serde]
struct FeeRegistryEffectiveFeeResponse {
    fee_bps: u16,
    discount_bps: u16,
    tier_id: Option<u8>,
    /// The registry returns the holding it used; `cw_serde` rejects unknown
    /// fields, so it must be mirrored here.
    holding: Option<Uint128>,
    source: FeeRegistryTierSource,
}

#[cw_serde]
enum FeeRegistryTierSource {
    Live,
    Cached,
    Lowest,
}

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    if msg.gas_denom.trim().is_empty()
        || !valid_vault_limits(
            msg.max_grid_count,
            msg.max_orders_per_reconcile,
            msg.max_active_orders_per_bot,
        )
        || msg.keeper_reward.is_zero()
        || msg.order_timeout_seconds == 0
    {
        return Err(ContractError::InvalidGrid);
    }
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    CONFIG.save(
        deps.storage,
        &Config {
            admin: deps.api.addr_validate(&msg.admin)?,
            owner: deps.api.addr_validate(&msg.owner)?,
            pending_admin: None,
            keeper: deps.api.addr_validate(&msg.keeper)?,
            factory: deps.api.addr_validate(&msg.factory)?,
            gas_denom: msg.gas_denom,
            keeper_reward: msg.keeper_reward,
            minimum_gas_reserve: msg.minimum_gas_reserve,
            order_timeout_seconds: msg.order_timeout_seconds,
            max_grid_count: msg.max_grid_count,
            max_orders_per_reconcile: msg.max_orders_per_reconcile,
            max_active_orders_per_bot: msg.max_active_orders_per_bot,
            fee_registry: msg
                .fee_registry
                .map(|s| deps.api.addr_validate(&s))
                .transpose()?,
            fee_collector: msg
                .fee_collector
                .map(|s| deps.api.addr_validate(&s))
                .transpose()?,
        },
    )?;
    NEXT_BOT_ID.save(deps.storage, &1)?;
    NEXT_REPLY_ID.save(deps.storage, &FIRST_REPLY_ID)?;
    VAULT_MODE.save(deps.storage, &VaultMode::Active)?;
    TOKEN_POLICY_ENABLED.save(deps.storage, &false)?;
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
        ExecuteMsg::Receive(receive) => execute_receive(deps, env, info, receive),
        ExecuteMsg::FundGas { bot_id } => execute_fund_gas(deps, info, bot_id),
        ExecuteMsg::WithdrawGas {
            bot_id,
            amount,
            recipient,
        } => execute_withdraw_gas(deps, info, bot_id, amount, recipient),
        ExecuteMsg::Allocate { bot_id } => execute_allocate(deps, env, info, bot_id),
        ExecuteMsg::SyncBalances { bot_id } => execute_sync_balances(deps, env, info, bot_id),
        ExecuteMsg::Reconcile { bot_id, order_ids } => {
            execute_reconcile(deps, env, info, bot_id, order_ids)
        }
        ExecuteMsg::RecoverOrder {
            bot_id,
            order_id,
            rung_index,
        } => execute_recover_order(deps, env, info, bot_id, order_id, rung_index),
        ExecuteMsg::CancelAll { bot_id } => execute_cancel_all(deps, env, info, bot_id),
        ExecuteMsg::Withdraw {
            bot_id,
            shares,
            recipient,
        } => execute_withdraw(deps, info, bot_id, shares, recipient),
        ExecuteMsg::RedeemShares { bot_id, recipient } => {
            execute_redeem_shares(deps, info, bot_id, recipient)
        }
        ExecuteMsg::UpdateKeeper { keeper } => execute_update_keeper(deps, info, keeper),
        ExecuteMsg::UpdatePairCode { bot_id, code_id } => {
            execute_update_pair_code(deps, info, bot_id, code_id)
        }
        ExecuteMsg::AddAllowedToken { token } => execute_add_allowed_token(deps, info, token),
        ExecuteMsg::RemoveAllowedToken { token } => execute_remove_allowed_token(deps, info, token),
        ExecuteMsg::QuarantineToken { token } => execute_quarantine_token(deps, info, token),
        ExecuteMsg::UnquarantineToken { token } => execute_unquarantine_token(deps, info, token),
        ExecuteMsg::TransferAdmin { admin } => execute_transfer_admin(deps, info, admin),
        ExecuteMsg::AcceptAdmin {} => execute_accept_admin(deps, info),
        ExecuteMsg::Pause {} => execute_pause(deps, info),
        ExecuteMsg::Resume {} => execute_resume(deps, info),
        ExecuteMsg::EnterExit { bot_id } => execute_enter_exit(deps, info, bot_id),
        ExecuteMsg::EmergencyCancel { bot_id } => execute_emergency_cancel(deps, env, info, bot_id),
        ExecuteMsg::EmergencyWithdraw { bot_id, recipient } => {
            execute_emergency_withdraw(deps, env, info, bot_id, recipient)
        }
    }
}

#[entry_point]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    let previous = get_contract_version(deps.storage)?;
    if previous.contract != CONTRACT_NAME {
        return Err(ContractError::UnsupportedMigrationSource);
    }
    let previous_version =
        Version::parse(&previous.version).map_err(|_| ContractError::UnsupportedMigrationSource)?;
    let current_version =
        Version::parse(CONTRACT_VERSION).map_err(|_| ContractError::UnsupportedMigrationSource)?;
    let minimum_version = Version::parse(MIN_MIGRATION_VERSION)
        .map_err(|_| ContractError::UnsupportedMigrationSource)?;
    if previous_version < minimum_version || previous_version >= current_version {
        return Err(ContractError::UnsupportedMigrationSource);
    }
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    Ok(Response::new()
        .add_attribute("action", "migrate_grid_vault")
        .add_attribute("from_version", previous.version))
}

fn execute_sync_balances(
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
    let credited = sync_free_balances(deps.as_ref(), &env, &mut bot)?;
    BOTS.save(deps.storage, bot_id, &bot)?;
    Ok(Response::new()
        .add_attribute("action", "sync_grid_balances")
        .add_attribute("bot_id", bot_id.to_string())
        .add_attribute("credited_token_0", credited[0])
        .add_attribute("credited_token_1", credited[1]))
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
    require_active(deps.as_ref())?;
    if BOTS.has(deps.storage, BOT_ID) {
        return Err(ContractError::BotAlreadyExists);
    }
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }
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
    let pair_code_id = deps.querier.query_wasm_contract_info(&pair)?.code_id;
    if pair_code_id == 0 {
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
    if !TOKEN_POLICY_ENABLED.load(deps.storage)? {
        for token in &asset_tokens {
            ALLOWED_TOKENS.save(deps.storage, token, &())?;
        }
        TOKEN_POLICY_ENABLED.save(deps.storage, &true)?;
    }
    for token in &asset_tokens {
        require_token_available(deps.as_ref(), token)?;
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
        .map(|price| match price.cmp(&reference_price) {
            CmpOrdering::Less => Some(LimitOrderSide::Bid),
            CmpOrdering::Greater => Some(LimitOrderSide::Ask),
            CmpOrdering::Equal => None,
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
    let bot_id = BOT_ID;
    NEXT_BOT_ID.save(deps.storage, &(BOT_ID + 1))?;
    BOTS.save(
        deps.storage,
        bot_id,
        &Bot {
            owner: info.sender.clone(),
            pair,
            pair_code_id,
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
    env: Env,
    info: MessageInfo,
    receive: Cw20ReceiveMsg,
) -> Result<Response, ContractError> {
    require_active(deps.as_ref())?;
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
            assert_pair_code(deps.as_ref(), &bot)?;
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
            if QUARANTINE.has(deps.storage, &info.sender) {
                return Err(ContractError::TokenQuarantined);
            }
            let actual = query_token_balance(deps.as_ref(), &info.sender, &env.contract.address)?;
            let expected = bot.free_balances[token_index].checked_add(receive.amount)?;
            if actual != expected {
                return Err(ContractError::UnsupportedTokenBehavior);
            }
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
            let expires_at = env
                .block
                .time
                .seconds()
                .checked_add(config.order_timeout_seconds)
                .ok_or(ContractError::InvalidGrid)?;
            allocate_side(deps.branch(), response, bot_id, &bot, side, expires_at)
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
    env: Env,
    info: MessageInfo,
    bot_id: u64,
) -> Result<Response, ContractError> {
    require_active(deps.as_ref())?;
    assert_no_funds(&info)?;
    let bot = BOTS.load(deps.storage, bot_id)?;
    if info.sender != bot.owner {
        return Err(ContractError::Unauthorized);
    }
    assert_pair_code(deps.as_ref(), &bot)?;
    for token in &bot.asset_tokens {
        if QUARANTINE.has(deps.storage, token) {
            return Err(ContractError::TokenQuarantined);
        }
    }
    let expires_at = env
        .block
        .time
        .seconds()
        .checked_add(CONFIG.load(deps.storage)?.order_timeout_seconds)
        .ok_or(ContractError::InvalidGrid)?;
    let mut response = Response::new()
        .add_attribute("action", "allocate_grid")
        .add_attribute("bot_id", bot_id.to_string());
    response = allocate_side(
        deps.branch(),
        response,
        bot_id,
        &bot,
        LimitOrderSide::Bid,
        expires_at,
    )?;
    response = allocate_side(
        deps,
        response,
        bot_id,
        &bot,
        LimitOrderSide::Ask,
        expires_at,
    )?;
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
    expires_at: u64,
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
    add_placement(deps, response, bot_id, side, rungs, amount_each, expires_at)
}

fn execute_reconcile(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    bot_id: u64,
    order_ids: Vec<u64>,
) -> Result<Response, ContractError> {
    require_active(deps.as_ref())?;
    assert_no_funds(&info)?;
    let config = CONFIG.load(deps.storage)?;
    let mut bot = BOTS.load(deps.storage, bot_id)?;
    if order_ids.is_empty() || order_ids.len() > config.max_orders_per_reconcile as usize {
        return Err(ContractError::InvalidFillReport);
    }
    assert_pair_code(deps.as_ref(), &bot)?;
    if bot.gas_credit
        < config
            .minimum_gas_reserve
            .checked_add(config.keeper_reward)?
    {
        return Err(ContractError::InsufficientGasCredit);
    }
    let credited = sync_free_balances(deps.as_ref(), &env, &mut bot)?;
    let mut seen = BTreeSet::new();
    let mut claim_entries = vec![];
    let mut changed_orders = 0usize;
    let mut executed_count = 0usize;
    let mut cancelled_count = 0usize;
    for order_id in &order_ids {
        if !seen.insert(*order_id) {
            return Err(ContractError::InvalidFillReport);
        }
        let mut order = ORDERS.load(deps.storage, (bot_id, *order_id))?;
        let active: StdResult<LimitOrderResponse> = deps.querier.query_wasm_smart(
            &bot.pair,
            &PairQueryMsg::LimitOrder {
                order_id: *order_id,
            },
        );
        let (current_remaining, terminal, parked_refund) = match active {
            Ok(on_chain) => {
                validate_order(&env.contract.address, *order_id, &order, &on_chain)?;
                (
                    on_chain.remaining,
                    on_chain.remaining.is_zero(),
                    Uint128::zero(),
                )
            }
            Err(_) => {
                let parked: Option<ExpiredLimitRefundResponse> = deps
                    .querier
                    .query_wasm_smart(
                        &bot.pair,
                        &PairQueryMsg::ExpiredLimitRefund {
                            order_id: *order_id,
                        },
                    )
                    .map_err(|_| ContractError::OrderStatusUnverifiable)?;
                if let Some(refund) = parked {
                    if refund.owner != env.contract.address
                        || refund.order_id != *order_id
                        || refund.side != order.side
                        || refund.remaining > order.remaining
                    {
                        return Err(ContractError::InvalidOrder);
                    }
                    (refund.remaining, true, refund.remaining)
                } else if CANCELLED_ORDERS.has(deps.storage, (bot_id, *order_id)) {
                    // The vault already cancelled this order, so its escrow was
                    // returned to the vault when the cancel was confirmed.
                    cancelled_count += 1;
                    (Uint128::zero(), true, Uint128::zero())
                } else {
                    // The vault never cancelled this order, yet the pair no
                    // longer reports it as active or parked. Orders can only
                    // leave the pair through execution or a vault-initiated
                    // cancel, so this order was fully executed. Its fill
                    // proceeds were already credited to the vault balance by
                    // sync_free_balances above.
                    executed_count += 1;
                    (Uint128::zero(), true, Uint128::zero())
                }
            }
        };
        let consumed = order.remaining.checked_sub(current_remaining)?;
        if !consumed.is_zero() || terminal {
            changed_orders += 1;
        }
        if terminal {
            if !parked_refund.is_zero() {
                let input_index = match order.side {
                    LimitOrderSide::Ask => 0,
                    LimitOrderSide::Bid => 1,
                };
                claim_entries.push(PendingPageEntry {
                    order_id: *order_id,
                    token_index: input_index,
                    refund: parked_refund,
                });
            } else {
                ORDERS.remove(deps.storage, (bot_id, *order_id));
                bot.active_orders = bot
                    .active_orders
                    .checked_sub(1)
                    .ok_or(ContractError::InvalidOrder)?;
            }
        } else {
            order.remaining = current_remaining;
            ORDERS.save(deps.storage, (bot_id, *order_id), &order)?;
        }
    }
    if changed_orders == 0 && credited == [Uint128::zero(), Uint128::zero()] {
        return Err(ContractError::NothingToReconcile);
    }
    let mut response = Response::new()
        .add_attribute("action", "reconcile_grid")
        .add_attribute("bot_id", bot_id.to_string())
        .add_attribute("changed_orders", changed_orders.to_string())
        .add_attribute("fully_executed", executed_count.to_string())
        .add_attribute("cancelled", cancelled_count.to_string())
        .add_attribute("credited_token_0", credited[0])
        .add_attribute("credited_token_1", credited[1]);
    match charge_fee(&mut deps, &config, bot_id, &mut bot, &credited)? {
        ChargeFee::Applied(fee) => {
            response = response
                .add_attribute("fee_bps", fee.fee_bps.to_string())
                .add_attribute("fee_shares", fee.shares.to_string())
                .add_attribute(
                    "fee_tier",
                    fee.tier.map(|t| t.to_string()).unwrap_or_default(),
                )
                .add_attribute("fee_source", fee.source);
        }
        // Non-blocking: the reconcile completes even if the fee-registry is
        // unreachable; the fill is processed without a fee.
        ChargeFee::Unavailable(reason) => {
            response = response.add_attribute("fee_skipped", reason);
        }
        ChargeFee::None => {}
    }
    BOTS.save(deps.storage, bot_id, &bot)?;
    if !claim_entries.is_empty() {
        let pair_batch_limit = query_pair_batch_limit(deps.as_ref(), &bot.pair)?;
        response = add_confirmable_pages(
            &mut deps,
            response,
            bot_id,
            &bot.pair,
            &[],
            &claim_entries,
            pair_batch_limit,
        )?;
    }
    if info.sender == config.keeper {
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
    }
    Ok(response)
}

fn execute_recover_order(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    bot_id: u64,
    order_id: u64,
    rung_index: u32,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    let mut bot = BOTS.load(deps.storage, bot_id)?;
    if info.sender != bot.owner {
        return Err(ContractError::Unauthorized);
    }
    if ORDERS.has(deps.storage, (bot_id, order_id)) {
        return Err(ContractError::InvalidOrder);
    }
    if CANCELLED_ORDERS.has(deps.storage, (bot_id, order_id)) {
        return Err(ContractError::InvalidOrder);
    }
    assert_pair_code(deps.as_ref(), &bot)?;
    let rung = RUNGS.load(deps.storage, (bot_id, rung_index))?;
    let side = rung.side.ok_or(ContractError::InvalidOrder)?;
    let active: StdResult<LimitOrderResponse> = deps
        .querier
        .query_wasm_smart(&bot.pair, &PairQueryMsg::LimitOrder { order_id });
    let (remaining, parked) = match active {
        Ok(on_chain) => {
            let recovered = GridOrder {
                rung_index,
                side: side.clone(),
                price: rung.price,
                remaining: on_chain.remaining,
            };
            validate_order(&env.contract.address, order_id, &recovered, &on_chain)?;
            if on_chain.remaining.is_zero() {
                return Err(ContractError::InvalidOrder);
            }
            (on_chain.remaining, false)
        }
        Err(_) => {
            let refund: Option<ExpiredLimitRefundResponse> = deps
                .querier
                .query_wasm_smart(&bot.pair, &PairQueryMsg::ExpiredLimitRefund { order_id })
                .map_err(|_| ContractError::OrderStatusUnverifiable)?;
            let refund = refund.ok_or(ContractError::OrderStatusUnverifiable)?;
            if refund.owner != env.contract.address
                || refund.order_id != order_id
                || refund.side != side
                || refund.remaining.is_zero()
            {
                return Err(ContractError::InvalidOrder);
            }
            (refund.remaining, true)
        }
    };
    ORDERS.save(
        deps.storage,
        (bot_id, order_id),
        &GridOrder {
            rung_index,
            side: side.clone(),
            price: rung.price,
            remaining,
        },
    )?;
    bot.active_orders = bot
        .active_orders
        .checked_add(1)
        .ok_or(ContractError::ArithmeticOutOfRange)?;
    BOTS.save(deps.storage, bot_id, &bot)?;

    let mut response = Response::new()
        .add_attribute("action", "recover_grid_order")
        .add_attribute("bot_id", bot_id.to_string())
        .add_attribute("order_id", order_id.to_string())
        .add_attribute("status", if parked { "parked" } else { "active" });
    if parked {
        let token_index = match side {
            LimitOrderSide::Ask => 0,
            LimitOrderSide::Bid => 1,
        };
        response = add_confirmable_pages(
            &mut deps,
            response,
            bot_id,
            &bot.pair,
            &[],
            &[PendingPageEntry {
                order_id,
                token_index,
                refund: remaining,
            }],
            bot.pair_batch_limit,
        )?;
    }
    Ok(response)
}

fn execute_cancel_all(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    bot_id: u64,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    let bot = BOTS.load(deps.storage, bot_id)?;
    if info.sender != bot.owner {
        return Err(ContractError::Unauthorized);
    }
    assert_pair_code(deps.as_ref(), &bot)?;
    let tracked: Vec<(u64, GridOrder)> = ORDERS
        .prefix(bot_id)
        .range(deps.storage, None, None, Order::Ascending)
        .take(CONFIG.load(deps.storage)?.max_orders_per_reconcile as usize)
        .collect::<StdResult<_>>()?;
    if tracked.is_empty() {
        return Ok(Response::new().add_attribute("action", "cancel_grid_orders"));
    }
    let mut cancel_entries = Vec::with_capacity(tracked.len());
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
        cancel_entries.push(PendingPageEntry {
            order_id,
            token_index: input_index,
            refund: on_chain.remaining,
        });
    }
    let pair_batch_limit = query_pair_batch_limit(deps.as_ref(), &bot.pair)?;
    Ok(add_confirmable_pages(
        &mut deps,
        Response::new(),
        bot_id,
        &bot.pair,
        &cancel_entries,
        &[],
        pair_batch_limit,
    )?
    .add_attribute("action", "cancel_grid_orders")
    .add_attribute("bot_id", bot_id.to_string())
    .add_attribute("orders", cancel_entries.len().to_string()))
}

fn execute_withdraw(
    deps: DepsMut,
    info: MessageInfo,
    bot_id: u64,
    shares: Uint128,
    recipient: Option<String>,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    if VAULT_MODE.load(deps.storage)? == VaultMode::Exit {
        return Err(ContractError::InvalidMode);
    }
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

/// The configured fee-collector redeems its own LP from a bot, sending the
/// underlying assets to `recipient` (or itself). This is the redemption side of
/// the per-fill protocol fee: the collector holds the LP the vault minted and
/// forwards the proceeds to the CMM treasury.
fn execute_redeem_shares(
    deps: DepsMut,
    info: MessageInfo,
    bot_id: u64,
    recipient: Option<String>,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    if VAULT_MODE.load(deps.storage)? == VaultMode::Exit {
        return Err(ContractError::InvalidMode);
    }
    let config = CONFIG.load(deps.storage)?;
    let collector = config.fee_collector.ok_or(ContractError::Unauthorized)?;
    if info.sender != collector {
        return Err(ContractError::Unauthorized);
    }
    let mut bot = BOTS.load(deps.storage, bot_id)?;
    let shares = SHARES
        .may_load(deps.storage, (bot_id, &info.sender))?
        .unwrap_or_default();
    if shares.is_zero() {
        return Err(ContractError::InsufficientShares);
    }
    if bot.active_orders != 0 {
        return Err(ContractError::ActiveOrders);
    }
    let amounts = [
        bot.free_balances[0].multiply_ratio(shares, bot.total_shares),
        bot.free_balances[1].multiply_ratio(shares, bot.total_shares),
    ];
    let recipient = deps
        .api
        .addr_validate(recipient.as_deref().unwrap_or(info.sender.as_str()))?;
    let mut response = Response::new()
        .add_attribute("action", "redeem_grid_shares")
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
    SHARES.save(deps.storage, (bot_id, &info.sender), &Uint128::zero())?;
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

fn execute_update_pair_code(
    deps: DepsMut,
    info: MessageInfo,
    bot_id: u64,
    code_id: u64,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    assert_admin(deps.as_ref(), &info.sender)?;
    if code_id == 0 {
        return Err(ContractError::InvalidPair);
    }
    BOTS.update(deps.storage, bot_id, |bot| -> Result<_, ContractError> {
        let mut bot = bot.ok_or_else(|| StdError::not_found("bot"))?;
        bot.pair_code_id = code_id;
        Ok(bot)
    })?;
    Ok(Response::new()
        .add_attribute("action", "update_grid_pair_code")
        .add_attribute("bot_id", bot_id.to_string())
        .add_attribute("code_id", code_id.to_string()))
}

fn execute_transfer_admin(
    deps: DepsMut,
    info: MessageInfo,
    admin: String,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    let pending = deps.api.addr_validate(&admin)?;
    CONFIG.update(deps.storage, |mut config| -> Result<_, ContractError> {
        if info.sender != config.admin {
            return Err(ContractError::Unauthorized);
        }
        config.pending_admin = Some(pending.clone());
        Ok(config)
    })?;
    Ok(Response::new()
        .add_attribute("action", "transfer_grid_admin")
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
    Ok(Response::new().add_attribute("action", "accept_grid_admin"))
}

fn execute_pause(deps: DepsMut, info: MessageInfo) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    assert_admin(deps.as_ref(), &info.sender)?;
    if VAULT_MODE.load(deps.storage)? != VaultMode::Active {
        return Err(ContractError::InvalidMode);
    }
    VAULT_MODE.save(deps.storage, &VaultMode::Paused)?;
    Ok(Response::new().add_attribute("action", "pause_grid_vault"))
}

fn execute_resume(deps: DepsMut, info: MessageInfo) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    assert_admin(deps.as_ref(), &info.sender)?;
    if VAULT_MODE.load(deps.storage)? != VaultMode::Paused {
        return Err(ContractError::InvalidMode);
    }
    VAULT_MODE.save(deps.storage, &VaultMode::Active)?;
    Ok(Response::new().add_attribute("action", "resume_grid_vault"))
}

fn execute_enter_exit(
    deps: DepsMut,
    info: MessageInfo,
    bot_id: u64,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    let bot = BOTS.load(deps.storage, bot_id)?;
    if info.sender != bot.owner {
        return Err(ContractError::Unauthorized);
    }
    if VAULT_MODE.load(deps.storage)? == VaultMode::Exit {
        return Err(ContractError::InvalidMode);
    }
    VAULT_MODE.save(deps.storage, &VaultMode::Exit)?;
    Ok(Response::new()
        .add_attribute("action", "enter_grid_exit")
        .add_attribute("bot_id", bot_id.to_string()))
}

fn execute_emergency_cancel(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    bot_id: u64,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    if VAULT_MODE.load(deps.storage)? != VaultMode::Exit {
        return Err(ContractError::InvalidMode);
    }
    let bot = BOTS.load(deps.storage, bot_id)?;
    if info.sender != bot.owner {
        return Err(ContractError::Unauthorized);
    }
    assert_pair_code(deps.as_ref(), &bot)?;
    let limit = CONFIG.load(deps.storage)?.max_orders_per_reconcile as usize;
    let tracked: Vec<(u64, GridOrder)> = ORDERS
        .prefix(bot_id)
        .range(deps.storage, None, None, Order::Ascending)
        .take(limit)
        .collect::<StdResult<_>>()?;
    let mut cancel_entries = vec![];
    let mut claim_entries = vec![];
    for (order_id, order) in &tracked {
        let active: StdResult<LimitOrderResponse> = deps.querier.query_wasm_smart(
            &bot.pair,
            &PairQueryMsg::LimitOrder {
                order_id: *order_id,
            },
        );
        match active {
            Ok(on_chain) => {
                validate_order(&env.contract.address, *order_id, order, &on_chain)?;
                let token_index = match order.side {
                    LimitOrderSide::Ask => 0,
                    LimitOrderSide::Bid => 1,
                };
                cancel_entries.push(PendingPageEntry {
                    order_id: *order_id,
                    token_index,
                    refund: on_chain.remaining,
                });
            }
            Err(_) => {
                let parked: Option<ExpiredLimitRefundResponse> = deps
                    .querier
                    .query_wasm_smart(
                        &bot.pair,
                        &PairQueryMsg::ExpiredLimitRefund {
                            order_id: *order_id,
                        },
                    )
                    .map_err(|_| ContractError::OrderStatusUnverifiable)?;
                if let Some(refund) = parked {
                    if refund.owner != env.contract.address
                        || refund.order_id != *order_id
                        || refund.side != order.side
                        || refund.remaining > order.remaining
                    {
                        return Err(ContractError::InvalidOrder);
                    }
                    let token_index = match order.side {
                        LimitOrderSide::Ask => 0,
                        LimitOrderSide::Bid => 1,
                    };
                    claim_entries.push(PendingPageEntry {
                        order_id: *order_id,
                        token_index,
                        refund: refund.remaining,
                    });
                } else {
                    return Err(ContractError::OrderStatusUnverifiable);
                }
            }
        }
    }
    let has_more = ORDERS
        .prefix(bot_id)
        .keys(deps.storage, None, None, Order::Ascending)
        .next()
        .transpose()?
        .is_some();
    let response = add_confirmable_pages(
        &mut deps,
        Response::new(),
        bot_id,
        &bot.pair,
        &cancel_entries,
        &claim_entries,
        bot.pair_batch_limit,
    )?;
    Ok(response
        .add_attribute("action", "emergency_cancel_grid_orders")
        .add_attribute("bot_id", bot_id.to_string())
        .add_attribute("processed_orders", tracked.len().to_string())
        .add_attribute("orders_remain", has_more.to_string()))
}

fn execute_emergency_withdraw(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    bot_id: u64,
    recipient: Option<String>,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    if VAULT_MODE.load(deps.storage)? != VaultMode::Exit {
        return Err(ContractError::InvalidMode);
    }
    let mut bot = BOTS.load(deps.storage, bot_id)?;
    if info.sender != bot.owner {
        return Err(ContractError::Unauthorized);
    }
    if ORDERS
        .prefix(bot_id)
        .keys(deps.storage, None, None, Order::Ascending)
        .next()
        .transpose()?
        .is_some()
    {
        return Err(ContractError::ExitOrdersRemain);
    }
    let recipient = deps
        .api
        .addr_validate(recipient.as_deref().unwrap_or(info.sender.as_str()))?;
    let mut response = Response::new()
        .add_attribute("action", "emergency_withdraw_grid")
        .add_attribute("bot_id", bot_id.to_string());
    for token in &bot.asset_tokens {
        let balance: BalanceResponse = deps.querier.query_wasm_smart(
            token,
            &Cw20QueryMsg::Balance {
                address: env.contract.address.to_string(),
            },
        )?;
        if !balance.balance.is_zero() {
            response = response.add_message(WasmMsg::Execute {
                contract_addr: token.to_string(),
                msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
                    recipient: recipient.to_string(),
                    amount: balance.balance,
                })?,
                funds: vec![],
            });
        }
    }
    SHARES.remove(deps.storage, (bot_id, &bot.owner));
    bot.free_balances = [Uint128::zero(), Uint128::zero()];
    bot.total_shares = Uint128::zero();
    bot.active_orders = 0;
    BOTS.save(deps.storage, bot_id, &bot)?;
    Ok(response)
}

fn assert_admin(deps: Deps, sender: &Addr) -> Result<(), ContractError> {
    if CONFIG.load(deps.storage)?.admin != *sender {
        return Err(ContractError::Unauthorized);
    }
    Ok(())
}

fn require_token_available(deps: Deps, token: &Addr) -> Result<(), ContractError> {
    if QUARANTINE.has(deps.storage, token) {
        return Err(ContractError::TokenQuarantined);
    }
    if TOKEN_POLICY_ENABLED.load(deps.storage)? && !ALLOWED_TOKENS.has(deps.storage, token) {
        return Err(ContractError::TokenNotAllowed);
    }
    Ok(())
}

fn execute_add_allowed_token(
    deps: DepsMut,
    info: MessageInfo,
    token: String,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    assert_admin(deps.as_ref(), &info.sender)?;
    let token = deps.api.addr_validate(&token)?;
    ALLOWED_TOKENS.save(deps.storage, &token, &())?;
    TOKEN_POLICY_ENABLED.save(deps.storage, &true)?;
    Ok(Response::new()
        .add_attribute("action", "add_allowed_token")
        .add_attribute("token", token))
}

fn execute_remove_allowed_token(
    deps: DepsMut,
    info: MessageInfo,
    token: String,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    assert_admin(deps.as_ref(), &info.sender)?;
    let token = deps.api.addr_validate(&token)?;
    ALLOWED_TOKENS.remove(deps.storage, &token);
    Ok(Response::new()
        .add_attribute("action", "remove_allowed_token")
        .add_attribute("token", token))
}

fn execute_quarantine_token(
    deps: DepsMut,
    info: MessageInfo,
    token: String,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    assert_admin(deps.as_ref(), &info.sender)?;
    let token = deps.api.addr_validate(&token)?;
    QUARANTINE.save(deps.storage, &token, &())?;
    Ok(Response::new()
        .add_attribute("action", "quarantine_token")
        .add_attribute("token", token))
}

fn execute_unquarantine_token(
    deps: DepsMut,
    info: MessageInfo,
    token: String,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;
    assert_admin(deps.as_ref(), &info.sender)?;
    let token = deps.api.addr_validate(&token)?;
    QUARANTINE.remove(deps.storage, &token);
    Ok(Response::new()
        .add_attribute("action", "unquarantine_token")
        .add_attribute("token", token))
}

fn assert_pair_code(deps: Deps, bot: &Bot) -> Result<(), ContractError> {
    let current = deps.querier.query_wasm_contract_info(&bot.pair)?.code_id;
    if current != bot.pair_code_id {
        return Err(ContractError::PairCodeMismatch);
    }
    Ok(())
}

fn require_active(deps: Deps) -> Result<(), ContractError> {
    if VAULT_MODE.load(deps.storage)? != VaultMode::Active {
        return Err(ContractError::InvalidMode);
    }
    Ok(())
}

fn add_placement(
    deps: DepsMut,
    response: Response,
    bot_id: u64,
    side: LimitOrderSide,
    rungs: Vec<u32>,
    amount_each: Uint128,
    expires_at: u64,
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
            expires_at: Some(expires_at),
            hint_after_order_id: None,
        });
    }
    let reply_id = next_reply_id(deps.storage)?;
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
            contract_addr: current.asset_tokens[token_index].to_string(),
            msg: to_json_binary(&Cw20ExecuteMsg::Send {
                contract: current.pair.to_string(),
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
    if let Some(page) = PENDING_PAGES.may_load(deps.storage, reply.id)? {
        return handle_pending_page(deps, &env, reply.id, &page, reply.result.into_result());
    }
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
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => {
            let config = CONFIG.load(deps.storage)?;
            let mode = match VAULT_MODE.load(deps.storage)? {
                VaultMode::Active => VaultModeResponse::Active,
                VaultMode::Paused => VaultModeResponse::Paused,
                VaultMode::Exit => VaultModeResponse::Exit,
            };
            to_json_binary(&ConfigResponse {
                admin: config.admin.to_string(),
                owner: config.owner.to_string(),
                pending_admin: config.pending_admin.map(|admin| admin.to_string()),
                keeper: config.keeper.to_string(),
                factory: config.factory.to_string(),
                gas_denom: config.gas_denom,
                keeper_reward: config.keeper_reward,
                minimum_gas_reserve: config.minimum_gas_reserve,
                order_timeout_seconds: config.order_timeout_seconds,
                max_grid_count: config.max_grid_count,
                max_orders_per_reconcile: config.max_orders_per_reconcile,
                max_active_orders_per_bot: config.max_active_orders_per_bot,
                fee_registry: config.fee_registry.map(|registry| registry.to_string()),
                fee_collector: config.fee_collector.map(|collector| collector.to_string()),
                mode,
            })
        }
        QueryMsg::Bot { bot_id } => {
            let bot = BOTS.load(deps.storage, bot_id)?;
            to_json_binary(&BotResponse {
                bot_id,
                owner: bot.owner.to_string(),
                pair: bot.pair.to_string(),
                pair_code_id: bot.pair_code_id,
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
        QueryMsg::CancelledOrders {
            bot_id,
            start_after,
            limit,
        } => {
            let limit = limit
                .unwrap_or(MAX_CANCELLED_ORDERS_PAGE_SIZE)
                .min(MAX_CANCELLED_ORDERS_PAGE_SIZE);
            let start = start_after.map(Bound::exclusive);
            let mut rows = Vec::with_capacity(limit as usize);
            let mut next_cursor = None;
            for item in CANCELLED_ORDERS
                .prefix(bot_id)
                .range(deps.storage, start, None, Order::Ascending)
                .take((limit as usize).saturating_add(1))
            {
                let (order_id, cancelled) = item?;
                if rows.len() as u32 >= limit {
                    next_cursor = Some(order_id);
                    break;
                }
                rows.push(CancelledOrderResponse {
                    order_id,
                    rung_index: cancelled.rung_index,
                    side: cancelled.side,
                    price: cancelled.price,
                    remaining: cancelled.remaining,
                    cancelled_at: cancelled.cancelled_at,
                });
            }
            to_json_binary(&CancelledOrdersResponse { rows, next_cursor })
        }
        QueryMsg::Shares { bot_id, address } => {
            let address = deps.api.addr_validate(&address)?;
            to_json_binary(&ShareResponse {
                shares: SHARES
                    .may_load(deps.storage, (bot_id, &address))?
                    .unwrap_or_default(),
            })
        }
        QueryMsg::Solvency { bot_id } => to_json_binary(&check_solvency(deps, &env, bot_id)?),
        QueryMsg::TokenPolicy {} => {
            let allowed_tokens = ALLOWED_TOKENS
                .keys(deps.storage, None, None, Order::Ascending)
                .collect::<StdResult<Vec<_>>>()?
                .into_iter()
                .map(|addr| addr.to_string())
                .collect();
            let quarantined_tokens = QUARANTINE
                .keys(deps.storage, None, None, Order::Ascending)
                .collect::<StdResult<Vec<_>>>()?
                .into_iter()
                .map(|addr| addr.to_string())
                .collect();
            to_json_binary(&TokenPolicyResponse {
                enabled: TOKEN_POLICY_ENABLED.load(deps.storage)?,
                allowed_tokens,
                quarantined_tokens,
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
    let atomics = checked_ratio(
        pool.assets[1].amount,
        Decimal::one().atomics(),
        pool.assets[0].amount,
    )?;
    Decimal::from_atomics(atomics, Decimal::DECIMAL_PLACES)
        .map_err(|_| ContractError::ArithmeticOutOfRange)
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
        if price.is_zero() {
            return Err(ContractError::InvalidPair);
        }
        checked_ratio(amount, Decimal::one().atomics(), price.atomics())
    }
}

fn checked_ratio(
    value: Uint128,
    numerator: Uint128,
    denominator: Uint128,
) -> Result<Uint128, ContractError> {
    if denominator.is_zero() {
        return Err(ContractError::ArithmeticOutOfRange);
    }
    let result = Uint256::from(value) * Uint256::from(numerator) / Uint256::from(denominator);
    result
        .try_into()
        .map_err(|_| ContractError::ArithmeticOutOfRange)
}

struct FeeApplied {
    fee_bps: u16,
    shares: Uint128,
    tier: Option<u8>,
    source: String,
}

/// Outcome of a protocol-fee charge attempt. Keeping it an enum rather than
/// `Option` lets reconcile distinguish "no fee configured/applicable" from a
/// fee that could not be charged because the fee-registry is unreachable.
enum ChargeFee {
    None,
    Applied(FeeApplied),
    /// Fee-registry read failed. The reconcile must NOT revert -- the bot's
    /// fill is processed anyway and the fee is skipped for this fill.
    Unavailable(String),
}

/// Protocol fee per fill (v1, value-based): on a reconcile that credited fill
/// proceeds, the fee-registry resolves the bot owner's effective fee bps from
/// their CL18Y holding, and the vault mints that fraction of the credited value
/// as LP to the configured fee-collector, diluting existing holders. No fee is
/// charged when the vault has no configured or the resolved fee is zero.
/// A transient fee-registry failure is non-blocking: the reconcile proceeds and
/// the fee is skipped rather than reverting the trader's fill.
fn charge_fee(
    deps: &mut DepsMut,
    config: &Config,
    bot_id: u64,
    bot: &mut Bot,
    credited: &[Uint128; 2],
) -> Result<ChargeFee, ContractError> {
    let (Some(registry), Some(collector)) = (&config.fee_registry, &config.fee_collector) else {
        return Ok(ChargeFee::None);
    };
    let fee: FeeRegistryEffectiveFeeResponse = match deps.querier.query_wasm_smart(
        registry,
        &FeeRegistryQueryMsg::EffectiveFee {
            trader: bot.owner.to_string(),
        },
    ) {
        Ok(fee) => fee,
        Err(err) => {
            // Non-blocking: record why and carry on with the reconcile.
            return Ok(ChargeFee::Unavailable(err.to_string()));
        }
    };
    // Defensive bound: cap the effective fee at 100% (10_000 bps) so a corrupt
    // registry response can never dilute holders beyond the credited value.
    let fee_bps = fee.fee_bps.min(10_000);
    if fee_bps == 0 {
        return Ok(ChargeFee::None);
    }
    if fee_bps == 0 {
        return Ok(ChargeFee::None);
    }
    // Value the credited proceeds in token-0 terms using the bot's reference
    // price (shares are token-0 normalized, same as `deposit_shares`).
    let mut value_in_token0 = credited[0];
    if !credited[1].is_zero() {
        if bot.reference_price.is_zero() {
            return Err(ContractError::InvalidPair);
        }
        let token1_as_token0 = checked_ratio(
            credited[1],
            Decimal::one().atomics(),
            bot.reference_price.atomics(),
        )?;
        value_in_token0 = value_in_token0.checked_add(token1_as_token0)?;
    }
    let shares = value_in_token0.multiply_ratio(fee_bps, 10_000u16);
    if shares.is_zero() {
        return Ok(ChargeFee::None);
    }
    let previous = SHARES
        .may_load(deps.storage, (bot_id, collector))?
        .unwrap_or_default();
    SHARES.save(
        deps.storage,
        (bot_id, collector),
        &previous.checked_add(shares)?,
    )?;
    bot.total_shares = bot.total_shares.checked_add(shares)?;
    Ok(ChargeFee::Applied(FeeApplied {
        fee_bps,
        shares,
        tier: fee.tier_id,
        source: format!("{:?}", fee.source),
    }))
}

fn next_reply_id(storage: &mut dyn cosmwasm_std::Storage) -> StdResult<u64> {
    let reply_id = NEXT_REPLY_ID.load(storage)?;
    NEXT_REPLY_ID.save(
        storage,
        &reply_id
            .checked_add(1)
            .ok_or_else(|| StdError::generic_err("reply id overflow"))?,
    )?;
    Ok(reply_id)
}

fn add_confirmable_pages(
    deps: &mut DepsMut,
    mut response: Response,
    bot_id: u64,
    pair: &Addr,
    cancel_entries: &[PendingPageEntry],
    claim_entries: &[PendingPageEntry],
    batch_limit: u32,
) -> Result<Response, ContractError> {
    let limit = batch_limit.max(1) as usize;
    for (kind, entries) in [
        (PageKind::Cancel, cancel_entries),
        (PageKind::Claim, claim_entries),
    ] {
        for chunk in entries.chunks(limit) {
            let reply_id = next_reply_id(deps.storage)?;
            PENDING_PAGES.save(
                deps.storage,
                reply_id,
                &PendingPage {
                    bot_id,
                    kind: kind.clone(),
                    entries: chunk.to_vec(),
                },
            )?;
            let msg = match kind {
                PageKind::Cancel => PairExecuteMsg::CancelLimitOrders {
                    order_ids: chunk.iter().map(|entry| entry.order_id).collect(),
                },
                PageKind::Claim => PairExecuteMsg::ClaimExpiredLimitOrders {
                    order_ids: chunk.iter().map(|entry| entry.order_id).collect(),
                },
            };
            response = response.add_submessage(SubMsg::reply_always(
                WasmMsg::Execute {
                    contract_addr: pair.to_string(),
                    msg: to_json_binary(&msg)?,
                    funds: vec![],
                },
                reply_id,
            ));
        }
    }
    Ok(response)
}

fn handle_pending_page(
    deps: DepsMut,
    env: &Env,
    reply_id: u64,
    page: &PendingPage,
    result: Result<SubMsgResponse, String>,
) -> Result<Response, ContractError> {
    let reason = match result {
        Ok(_) => {
            let mut bot = BOTS.load(deps.storage, page.bot_id)?;
            for entry in &page.entries {
                bot.free_balances[entry.token_index as usize] =
                    bot.free_balances[entry.token_index as usize].checked_add(entry.refund)?;
                if page.kind == PageKind::Cancel {
                    if let Some(order) =
                        ORDERS.may_load(deps.storage, (page.bot_id, entry.order_id))?
                    {
                        CANCELLED_ORDERS.save(
                            deps.storage,
                            (page.bot_id, entry.order_id),
                            &CancelledOrder {
                                rung_index: order.rung_index,
                                side: order.side,
                                price: order.price,
                                remaining: entry.refund,
                                cancelled_at: env.block.height,
                            },
                        )?;
                    }
                }
                ORDERS.remove(deps.storage, (page.bot_id, entry.order_id));
            }
            bot.active_orders = bot
                .active_orders
                .checked_sub(page.entries.len() as u32)
                .ok_or(ContractError::InvalidOrder)?;
            BOTS.save(deps.storage, page.bot_id, &bot)?;
            None
        }
        Err(error) => Some(error),
    };
    PENDING_PAGES.remove(deps.storage, reply_id);
    let kind_label = match page.kind {
        PageKind::Cancel => "cancel",
        PageKind::Claim => "claim",
    };
    let response = Response::new()
        .add_attribute("bot_id", page.bot_id.to_string())
        .add_attribute("kind", kind_label);
    match reason {
        Some(error) => Ok(response
            .add_attribute("action", "reverted_grid_page")
            .add_attribute("reason", error)),
        None => Ok(response
            .add_attribute("action", "confirmed_grid_page")
            .add_attribute("orders", page.entries.len().to_string())),
    }
}

fn query_pair_batch_limit(deps: Deps, pair: &Addr) -> StdResult<u32> {
    let config: LimitOrderConfigResponse = deps
        .querier
        .query_wasm_smart(pair, &PairQueryMsg::LimitOrderConfig {})?;
    Ok(config.max_batch_rungs.max(1))
}

fn query_token_balance(deps: Deps, token: &Addr, account: &Addr) -> StdResult<Uint128> {
    let response: BalanceResponse = deps.querier.query_wasm_smart(
        token,
        &Cw20QueryMsg::Balance {
            address: account.to_string(),
        },
    )?;
    Ok(response.balance)
}

fn sync_free_balances(deps: Deps, env: &Env, bot: &mut Bot) -> Result<[Uint128; 2], ContractError> {
    let mut credited = [Uint128::zero(), Uint128::zero()];
    for (index, credited_amount) in credited.iter_mut().enumerate() {
        let actual = query_token_balance(deps, &bot.asset_tokens[index], &env.contract.address)?;
        if actual < bot.free_balances[index] {
            return Err(ContractError::UnsupportedTokenBehavior);
        }
        *credited_amount = actual.checked_sub(bot.free_balances[index])?;
        bot.free_balances[index] = actual;
    }
    Ok(credited)
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

fn check_solvency(deps: Deps, env: &Env, bot_id: u64) -> StdResult<SolvencyResponse> {
    let bot = BOTS.load(deps.storage, bot_id)?;
    let mut expected = bot.free_balances;
    let mut on_chain_escrow = [Uint128::zero(), Uint128::zero()];
    let mut active_escrow_orders = 0u32;
    let mut parked_refund_orders = 0u32;
    let mut executed_orders = 0u32;
    let mut cancelled_orders = 0u32;
    let mut unverifiable_orders = 0u32;
    let mut warnings = Vec::new();
    for item in ORDERS
        .prefix(bot_id)
        .range(deps.storage, None, None, Order::Ascending)
    {
        let (order_id, order) = item?;
        let token_index = match order.side {
            LimitOrderSide::Ask => 0,
            LimitOrderSide::Bid => 1,
        };
        expected[token_index] = expected[token_index].checked_add(order.remaining)?;
        match deps.querier.query_wasm_smart::<LimitOrderResponse>(
            &bot.pair,
            &PairQueryMsg::LimitOrder { order_id },
        ) {
            Ok(on_chain)
                if on_chain.order_id == order_id
                    && on_chain.owner == env.contract.address
                    && on_chain.side == order.side
                    && on_chain.price == order.price
                    && on_chain.remaining <= order.remaining =>
            {
                on_chain_escrow[token_index] =
                    on_chain_escrow[token_index].checked_add(on_chain.remaining)?;
                active_escrow_orders += 1;
            }
            Ok(_) => {
                unverifiable_orders += 1;
                warnings.push(format!("order {order_id} active escrow fields are invalid"));
            }
            Err(_) => match deps
                .querier
                .query_wasm_smart::<Option<ExpiredLimitRefundResponse>>(
                    &bot.pair,
                    &PairQueryMsg::ExpiredLimitRefund { order_id },
                ) {
                Ok(Some(refund))
                    if refund.order_id == order_id
                        && refund.owner == env.contract.address
                        && refund.side == order.side
                        && refund.remaining <= order.remaining =>
                {
                    on_chain_escrow[token_index] =
                        on_chain_escrow[token_index].checked_add(refund.remaining)?;
                    parked_refund_orders += 1;
                }
                Ok(Some(_)) => {
                    unverifiable_orders += 1;
                    warnings.push(format!("order {order_id} parked refund fields are invalid"));
                }
                Ok(None) => {
                    if CANCELLED_ORDERS.has(deps.storage, (bot_id, order_id)) {
                        // Cancelled via the vault: the escrow was returned when
                        // the cancel was confirmed and the order was re-tracked.
                        cancelled_orders += 1;
                    } else {
                        // Never cancelled: the order fully executed and its fill
                        // proceeds are already part of the vault balance.
                        executed_orders += 1;
                    }
                }
                Err(_) => {
                    unverifiable_orders += 1;
                    warnings.push(format!("order {order_id} escrow could not be verified"));
                }
            },
        }
    }
    let mut actual = [Uint128::zero(), Uint128::zero()];
    for (index, token) in bot.asset_tokens.iter().enumerate() {
        actual[index] = query_token_balance(deps, token, &env.contract.address)?
            .checked_add(on_chain_escrow[index])?;
    }
    Ok(SolvencyResponse {
        token_0_expected: expected[0],
        token_0_actual: actual[0],
        token_1_expected: expected[1],
        token_1_actual: actual[1],
        active_escrow_orders,
        parked_refund_orders,
        executed_orders,
        cancelled_orders,
        unverifiable_orders,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};
    use cosmwasm_std::{
        coin, from_json, ContractInfoResponse, ContractResult, Reply, ReplyOn, SubMsgResponse,
        SubMsgResult, SystemResult, WasmQuery,
    };

    fn contract_info(code_id: u64) -> ContractInfoResponse {
        let mut response = ContractInfoResponse::default();
        response.code_id = code_id;
        response
    }

    fn settle_replies(deps: &mut DepsMut, response: Response, ok: bool) -> Vec<u64> {
        let mut settled = vec![];
        for sub in response
            .messages
            .iter()
            .filter(|sub| sub.reply_on == ReplyOn::Always)
        {
            let result = if ok {
                SubMsgResult::Ok(SubMsgResponse {
                    events: vec![],
                    data: None,
                })
            } else {
                SubMsgResult::Err("mock pair failed".into())
            };
            reply(deps.branch(), mock_env(), Reply { id: sub.id, result }).unwrap();
            settled.push(sub.id);
        }
        settled
    }

    fn instantiate_default(deps: DepsMut) {
        instantiate(
            deps,
            mock_env(),
            mock_info("creator", &[]),
            InstantiateMsg {
                admin: "admin".into(),
                owner: "alice".into(),
                keeper: "keeper".into(),
                factory: "factory".into(),
                gas_denom: "uluna".into(),
                keeper_reward: Uint128::new(20),
                minimum_gas_reserve: Uint128::new(100),
                order_timeout_seconds: 86_400,
                max_grid_count: 20,
                max_orders_per_reconcile: 10,
                max_active_orders_per_bot: 40,
                fee_registry: None,
                fee_collector: None,
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
                let query: Cw20QueryMsg = from_json(msg).unwrap();
                let response = match query {
                    Cw20QueryMsg::TokenInfo {} => to_json_binary(&TokenInfoResponse {
                        name: contract_addr.clone(),
                        symbol: "TOK".into(),
                        decimals: 6,
                        total_supply: Uint128::new(1_000_000),
                    })
                    .unwrap(),
                    Cw20QueryMsg::Balance { .. } => to_json_binary(&BalanceResponse {
                        balance: if contract_addr == "token_a" {
                            Uint128::new(1_000)
                        } else {
                            Uint128::new(4_000)
                        },
                    })
                    .unwrap(),
                    _ => {
                        return SystemResult::Ok(ContractResult::Err(
                            "unsupported CW20 query".into(),
                        ))
                    }
                };
                SystemResult::Ok(ContractResult::Ok(response))
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
            WasmQuery::ContractInfo { contract_addr } if contract_addr == "pair" => {
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&contract_info(7)).unwrap(),
                ))
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
    fn creates_only_bot_one_and_rejects_a_second_bot() {
        let mut deps = mock_dependencies();
        install_pair_querier(&mut deps);
        instantiate_default(deps.as_mut());
        create_bot(deps.as_mut(), "alice");
        let error = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("bob", &[coin(200, "uluna")]),
            ExecuteMsg::CreateBot {
                pair: "pair".into(),
                lower_price: Decimal::one(),
                upper_price: Decimal::from_ratio(3u128, 1u128),
                grid_count: 5,
            },
        )
        .unwrap_err();

        let alice = BOTS.load(&deps.storage, 1).unwrap();
        assert_eq!(error, ContractError::BotAlreadyExists);
        assert_eq!(alice.owner, Addr::unchecked("alice"));
        assert_eq!(alice.gas_credit, Uint128::new(200));
        assert_eq!(alice.reference_price, Decimal::percent(200));
        assert!(!BOTS.has(&deps.storage, 2));

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
    fn deposit_share_conversion_reports_overflow_without_panicking() {
        let minimum_price = Decimal::from_atomics(Uint128::one(), 18).unwrap();
        assert_eq!(
            deposit_shares(Uint128::MAX, 1, minimum_price).unwrap_err(),
            ContractError::ArithmeticOutOfRange
        );
        assert_eq!(
            deposit_shares(Uint128::zero(), 1, Decimal::one()).unwrap(),
            Uint128::zero()
        );
        assert_eq!(
            deposit_shares(Uint128::new(25), 1, Decimal::percent(50)).unwrap(),
            Uint128::new(50)
        );
    }

    #[test]
    fn migration_accepts_live_vault_without_inventory_gate() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        BOTS.save(deps.as_mut().storage, BOT_ID, &test_bot("alice", 0))
            .unwrap();
        ORDERS
            .save(
                deps.as_mut().storage,
                (BOT_ID, 77),
                &GridOrder {
                    rung_index: 3,
                    side: LimitOrderSide::Ask,
                    price: Decimal::percent(250),
                    remaining: Uint128::new(10),
                },
            )
            .unwrap();
        set_contract_version(deps.as_mut().storage, CONTRACT_NAME, "0.1.0").unwrap();
        migrate(deps.as_mut(), mock_env(), MigrateMsg {}).unwrap();
        assert!(ORDERS.has(&deps.storage, (BOT_ID, 77)));
        assert_eq!(
            BOTS.load(&deps.storage, BOT_ID).unwrap().owner,
            Addr::unchecked("alice")
        );
        assert_eq!(BOTS.load(&deps.storage, BOT_ID).unwrap().active_orders, 0);
    }

    #[test]
    fn migration_is_repeat_safe() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        BOTS.save(deps.as_mut().storage, BOT_ID, &test_bot("alice", 0))
            .unwrap();
        set_contract_version(deps.as_mut().storage, CONTRACT_NAME, "0.1.0").unwrap();
        migrate(deps.as_mut(), mock_env(), MigrateMsg {}).unwrap();
        assert_eq!(
            migrate(deps.as_mut(), mock_env(), MigrateMsg {}).unwrap_err(),
            ContractError::UnsupportedMigrationSource
        );
    }

    #[test]
    fn migration_rejects_wrong_malformed_same_and_newer_sources_without_mutation() {
        for (contract, version) in [
            ("wrong-contract", "0.1.0"),
            (CONTRACT_NAME, "not-semver"),
            (CONTRACT_NAME, "0.0.9"),
            (CONTRACT_NAME, CONTRACT_VERSION),
            (CONTRACT_NAME, "99.0.0"),
        ] {
            let mut deps = mock_dependencies();
            instantiate_default(deps.as_mut());
            set_contract_version(deps.as_mut().storage, contract, version).unwrap();
            assert_eq!(
                migrate(deps.as_mut(), mock_env(), MigrateMsg {}).unwrap_err(),
                ContractError::UnsupportedMigrationSource
            );
        }
    }

    #[test]
    fn empty_pre_bot_migration_remains_unlocked_and_can_create_bot() {
        let mut deps = mock_dependencies();
        install_pair_querier(&mut deps);
        instantiate_default(deps.as_mut());
        set_contract_version(deps.as_mut().storage, CONTRACT_NAME, "0.1.0").unwrap();
        migrate(deps.as_mut(), mock_env(), MigrateMsg {}).unwrap();
        create_bot(deps.as_mut(), "alice");
        assert!(BOTS.has(&deps.storage, BOT_ID));
    }

    #[test]
    fn owner_can_recover_a_positively_verified_forgotten_active_order() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        BOTS.save(deps.as_mut().storage, BOT_ID, &test_bot("alice", 0))
            .unwrap();
        RUNGS
            .save(
                deps.as_mut().storage,
                (BOT_ID, 3),
                &Rung {
                    price: Decimal::percent(250),
                    side: Some(LimitOrderSide::Ask),
                },
            )
            .unwrap();
        deps.querier.update_wasm(|query| match query {
            WasmQuery::ContractInfo { contract_addr } if contract_addr == "pair" => {
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&contract_info(7)).unwrap(),
                ))
            }
            WasmQuery::Smart { contract_addr, msg } if contract_addr == "pair" => {
                match from_json::<PairQueryMsg>(msg).unwrap() {
                    PairQueryMsg::LimitOrder { order_id: 77 } => {
                        SystemResult::Ok(ContractResult::Ok(
                            to_json_binary(&LimitOrderResponse {
                                order_id: 77,
                                owner: mock_env().contract.address.to_string(),
                                side: LimitOrderSide::Ask,
                                price: Decimal::percent(250),
                                remaining: Uint128::new(90),
                                expires_at: None,
                                prev: None,
                                next: None,
                            })
                            .unwrap(),
                        ))
                    }
                    _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
                }
            }
            _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
        });
        execute_recover_order(
            deps.as_mut(),
            mock_env(),
            mock_info("alice", &[]),
            BOT_ID,
            77,
            3,
        )
        .unwrap();
        assert_eq!(
            ORDERS.load(&deps.storage, (BOT_ID, 77)).unwrap().remaining,
            Uint128::new(90)
        );
        assert_eq!(BOTS.load(&deps.storage, BOT_ID).unwrap().active_orders, 1);
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
                pair_code_id: 7,
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
            mock_env(),
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
            mock_env(),
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
            mock_env(),
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
    fn withdrawal_is_isolated_by_contract_instance() {
        let mut alice_deps = mock_dependencies();
        let mut bob_deps = mock_dependencies();
        for (deps, owner) in [(&mut alice_deps, "alice"), (&mut bob_deps, "bob")] {
            instantiate_default(deps.as_mut());
            BOTS.save(
                deps.as_mut().storage,
                BOT_ID,
                &Bot {
                    owner: Addr::unchecked(owner),
                    pair: Addr::unchecked("pair"),
                    pair_code_id: 7,
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
                    (BOT_ID, &Addr::unchecked(owner)),
                    &Uint128::new(1_000),
                )
                .unwrap();
        }

        let response = execute_withdraw(
            alice_deps.as_mut(),
            mock_info("alice", &[]),
            1,
            Uint128::new(500),
            None,
        )
        .unwrap();
        assert_eq!(response.messages.len(), 2);
        assert_eq!(
            BOTS.load(&alice_deps.storage, 1).unwrap().free_balances,
            [Uint128::new(500), Uint128::new(1_000)]
        );
        assert_eq!(
            BOTS.load(&bob_deps.storage, 1).unwrap().free_balances,
            [Uint128::new(1_000), Uint128::new(2_000)]
        );
        assert_eq!(
            SHARES
                .load(&alice_deps.storage, (1, &Addr::unchecked("alice")))
                .unwrap(),
            Uint128::new(500)
        );
        assert_eq!(
            SHARES
                .load(&bob_deps.storage, (1, &Addr::unchecked("bob")))
                .unwrap(),
            Uint128::new(1_000)
        );
    }

    #[test]
    fn admin_transfer_pause_and_exit_are_authorized() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        BOTS.save(deps.as_mut().storage, BOT_ID, &test_bot("alice", 0))
            .unwrap();

        let error = execute_pause(deps.as_mut(), mock_info("alice", &[])).unwrap_err();
        assert_eq!(error, ContractError::Unauthorized);
        execute_transfer_admin(deps.as_mut(), mock_info("admin", &[]), "next_admin".into())
            .unwrap();
        let error = execute_accept_admin(deps.as_mut(), mock_info("alice", &[])).unwrap_err();
        assert_eq!(error, ContractError::Unauthorized);
        execute_accept_admin(deps.as_mut(), mock_info("next_admin", &[])).unwrap();
        execute_pause(deps.as_mut(), mock_info("next_admin", &[])).unwrap();
        assert_eq!(VAULT_MODE.load(&deps.storage).unwrap(), VaultMode::Paused);
        let error = execute_enter_exit(deps.as_mut(), mock_info("bob", &[]), BOT_ID).unwrap_err();
        assert_eq!(error, ContractError::Unauthorized);
        execute_enter_exit(deps.as_mut(), mock_info("alice", &[]), BOT_ID).unwrap();
        assert_eq!(VAULT_MODE.load(&deps.storage).unwrap(), VaultMode::Exit);
        assert_eq!(
            execute_resume(deps.as_mut(), mock_info("next_admin", &[])).unwrap_err(),
            ContractError::InvalidMode
        );
    }

    #[test]
    fn emergency_exit_retains_all_orders_when_one_status_is_indeterminate() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        let mut bot = test_bot("alice", 3);
        bot.total_shares = Uint128::new(1_000);
        bot.free_balances = [Uint128::new(999_999), Uint128::new(999_999)];
        BOTS.save(deps.as_mut().storage, BOT_ID, &bot).unwrap();
        SHARES
            .save(
                deps.as_mut().storage,
                (BOT_ID, &Addr::unchecked("alice")),
                &Uint128::new(1_000),
            )
            .unwrap();
        for order_id in [77, 78, 79] {
            ORDERS
                .save(
                    deps.as_mut().storage,
                    (BOT_ID, order_id),
                    &GridOrder {
                        rung_index: 3,
                        side: LimitOrderSide::Ask,
                        price: Decimal::percent(250),
                        remaining: Uint128::new(100),
                    },
                )
                .unwrap();
        }
        deps.querier.update_wasm(|query| match query {
            WasmQuery::Smart { contract_addr, msg } if contract_addr == "pair" => {
                match from_json::<PairQueryMsg>(msg).unwrap() {
                    PairQueryMsg::LimitOrder { order_id: 77 } => {
                        SystemResult::Ok(ContractResult::Ok(
                            to_json_binary(&LimitOrderResponse {
                                order_id: 77,
                                owner: mock_env().contract.address.to_string(),
                                side: LimitOrderSide::Ask,
                                price: Decimal::percent(250),
                                remaining: Uint128::new(25),
                                expires_at: None,
                                prev: None,
                                next: None,
                            })
                            .unwrap(),
                        ))
                    }
                    PairQueryMsg::LimitOrder { .. } => {
                        SystemResult::Ok(ContractResult::Err("absent".into()))
                    }
                    PairQueryMsg::ExpiredLimitRefund { order_id: 79 } => {
                        SystemResult::Ok(ContractResult::Ok(
                            to_json_binary(&Some(ExpiredLimitRefundResponse {
                                order_id: 79,
                                owner: mock_env().contract.address.to_string(),
                                side: LimitOrderSide::Ask,
                                remaining: Uint128::new(40),
                                expires_at: Some(1),
                            }))
                            .unwrap(),
                        ))
                    }
                    PairQueryMsg::ExpiredLimitRefund { .. } => {
                        SystemResult::Ok(ContractResult::Ok(
                            to_json_binary(&Option::<ExpiredLimitRefundResponse>::None).unwrap(),
                        ))
                    }
                    _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
                }
            }
            WasmQuery::Smart { contract_addr, msg }
                if contract_addr == "token_a" || contract_addr == "token_b" =>
            {
                let _: Cw20QueryMsg = from_json(msg).unwrap();
                let balance = if contract_addr == "token_a" { 123 } else { 456 };
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&BalanceResponse {
                        balance: Uint128::new(balance),
                    })
                    .unwrap(),
                ))
            }
            WasmQuery::ContractInfo { contract_addr } if contract_addr == "pair" => {
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&contract_info(7)).unwrap(),
                ))
            }
            _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
        });

        execute_enter_exit(deps.as_mut(), mock_info("alice", &[]), BOT_ID).unwrap();
        assert_eq!(
            execute_emergency_cancel(deps.as_mut(), mock_env(), mock_info("keeper", &[]), BOT_ID,)
                .unwrap_err(),
            ContractError::Unauthorized
        );
        let error =
            execute_emergency_cancel(deps.as_mut(), mock_env(), mock_info("alice", &[]), BOT_ID)
                .unwrap_err();
        assert_eq!(error, ContractError::OrderStatusUnverifiable);
        assert!(ORDERS.has(&deps.storage, (BOT_ID, 77)));
        assert!(ORDERS.has(&deps.storage, (BOT_ID, 78)));
        assert!(ORDERS.has(&deps.storage, (BOT_ID, 79)));
        assert_eq!(BOTS.load(&deps.storage, BOT_ID).unwrap().active_orders, 3);
        assert_eq!(
            execute_emergency_withdraw(
                deps.as_mut(),
                mock_env(),
                mock_info("alice", &[]),
                BOT_ID,
                None,
            )
            .unwrap_err(),
            ContractError::ExitOrdersRemain
        );
    }

    fn test_bot(owner: &str, active_orders: u32) -> Bot {
        Bot {
            owner: Addr::unchecked(owner),
            pair: Addr::unchecked("pair"),
            pair_code_id: 7,
            asset_tokens: [Addr::unchecked("token_a"), Addr::unchecked("token_b")],
            lower_price: Decimal::one(),
            upper_price: Decimal::percent(300),
            grid_count: 5,
            reference_price: Decimal::percent(200),
            free_balances: [Uint128::zero(), Uint128::zero()],
            total_shares: Uint128::zero(),
            gas_credit: Uint128::new(200),
            active_orders,
            pair_batch_limit: 20,
        }
    }

    #[test]
    fn partial_fill_credits_only_the_observed_vault_balance() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        let bot = Bot {
            owner: Addr::unchecked("alice"),
            pair: Addr::unchecked("pair"),
            pair_code_id: 7,
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
                        side: match index.cmp(&2) {
                            CmpOrdering::Less => Some(LimitOrderSide::Bid),
                            CmpOrdering::Greater => Some(LimitOrderSide::Ask),
                            CmpOrdering::Equal => None,
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
            WasmQuery::Smart { contract_addr, msg } if contract_addr == "pair" => {
                match from_json::<PairQueryMsg>(msg).unwrap() {
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
            WasmQuery::Smart { contract_addr, msg }
                if contract_addr == "token_a" || contract_addr == "token_b" =>
            {
                let _: Cw20QueryMsg = from_json(msg).unwrap();
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&BalanceResponse {
                        balance: if contract_addr == "token_a" {
                            Uint128::one()
                        } else {
                            Uint128::zero()
                        },
                    })
                    .unwrap(),
                ))
            }
            WasmQuery::ContractInfo { contract_addr } if contract_addr == "pair" => {
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&contract_info(7)).unwrap(),
                ))
            }
            _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
        });

        let response = execute_reconcile(
            deps.as_mut(),
            mock_env(),
            mock_info("permissionless_caller", &[]),
            1,
            vec![77],
        )
        .unwrap();
        assert_eq!(
            ORDERS.load(&deps.storage, (1, 77)).unwrap().remaining,
            Uint128::new(99)
        );
        assert_eq!(
            BOTS.load(&deps.storage, 1).unwrap().free_balances,
            [Uint128::one(), Uint128::zero()]
        );
        assert!(PLACEMENTS.is_empty(&deps.storage));
        assert_eq!(
            BOTS.load(&deps.storage, 1).unwrap().gas_credit,
            Uint128::new(200)
        );
        assert!(response.messages.is_empty());

        let repeated = execute_reconcile(
            deps.as_mut(),
            mock_env(),
            mock_info("another_caller", &[]),
            1,
            vec![77],
        )
        .unwrap_err();
        assert_eq!(repeated, ContractError::NothingToReconcile);
    }

    #[test]
    fn old_keeper_report_cannot_deserialize_or_invent_output() {
        let old = br#"{"reconcile":{"bot_id":1,"reports":[{"pair":"pair","order_id":7,"input_amount":"1","output_amount":"999999","fill_count":1}]}}"#;
        assert!(from_json::<ExecuteMsg>(old).is_err());
    }

    #[test]
    fn rejects_deposit_when_cw20_balance_delta_is_not_exact() {
        let mut deps = mock_dependencies();
        install_pair_querier(&mut deps);
        instantiate_default(deps.as_mut());
        create_bot(deps.as_mut(), "alice");
        deps.querier.update_wasm(|query| match query {
            WasmQuery::Smart { contract_addr, msg } if contract_addr == "token_a" => {
                let _: Cw20QueryMsg = from_json(msg).unwrap();
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&BalanceResponse {
                        balance: Uint128::new(99),
                    })
                    .unwrap(),
                ))
            }
            WasmQuery::ContractInfo { contract_addr } if contract_addr == "pair" => {
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&contract_info(7)).unwrap(),
                ))
            }
            _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
        });

        let error = execute_receive(
            deps.as_mut(),
            mock_env(),
            mock_info("token_a", &[]),
            Cw20ReceiveMsg {
                sender: "alice".into(),
                amount: Uint128::new(100),
                msg: to_json_binary(&ReceiveMsg::Deposit { bot_id: 1 }).unwrap(),
            },
        )
        .unwrap_err();
        assert_eq!(error, ContractError::UnsupportedTokenBehavior);
        assert_eq!(
            BOTS.load(&deps.storage, 1).unwrap().free_balances[0],
            Uint128::zero()
        );
    }

    #[test]
    fn parked_refund_is_claimed_without_indexed_fill_history() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        BOTS.save(deps.as_mut().storage, 1, &test_bot("alice", 1))
            .unwrap();
        ORDERS
            .save(
                deps.as_mut().storage,
                (1, 77),
                &GridOrder {
                    rung_index: 3,
                    side: LimitOrderSide::Ask,
                    price: Decimal::percent(250),
                    remaining: Uint128::new(100),
                },
            )
            .unwrap();
        deps.querier.update_wasm(|query| match query {
            WasmQuery::Smart { contract_addr, msg } if contract_addr == "pair" => {
                match from_json::<PairQueryMsg>(msg).unwrap() {
                    PairQueryMsg::LimitOrder { .. } => {
                        SystemResult::Ok(ContractResult::Err("not found".into()))
                    }
                    PairQueryMsg::ExpiredLimitRefund { order_id } => {
                        SystemResult::Ok(ContractResult::Ok(
                            to_json_binary(&Some(ExpiredLimitRefundResponse {
                                order_id,
                                owner: mock_env().contract.address.to_string(),
                                side: LimitOrderSide::Ask,
                                remaining: Uint128::new(40),
                                expires_at: Some(1),
                            }))
                            .unwrap(),
                        ))
                    }
                    PairQueryMsg::LimitOrderConfig {} => SystemResult::Ok(ContractResult::Ok(
                        to_json_binary(&LimitOrderConfigResponse {
                            max_batch_rungs: 20,
                        })
                        .unwrap(),
                    )),
                    _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
                }
            }
            WasmQuery::Smart { contract_addr, msg }
                if contract_addr == "token_a" || contract_addr == "token_b" =>
            {
                let _: Cw20QueryMsg = from_json(msg).unwrap();
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&BalanceResponse {
                        balance: Uint128::zero(),
                    })
                    .unwrap(),
                ))
            }
            WasmQuery::ContractInfo { contract_addr } if contract_addr == "pair" => {
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&contract_info(7)).unwrap(),
                ))
            }
            _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
        });

        let response = execute_reconcile(
            deps.as_mut(),
            mock_env(),
            mock_info("permissionless", &[]),
            1,
            vec![77],
        )
        .unwrap();
        assert_eq!(response.messages.len(), 1);
        assert_eq!(
            BOTS.load(&deps.storage, 1).unwrap().free_balances,
            [Uint128::zero(), Uint128::zero()]
        );
        assert!(ORDERS.has(&deps.storage, (1, 77)));
        settle_replies(&mut deps.as_mut(), response, true);
        assert_eq!(
            BOTS.load(&deps.storage, 1).unwrap().free_balances,
            [Uint128::new(40), Uint128::zero()]
        );
        assert!(!ORDERS.has(&deps.storage, (1, 77)));
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
                pair_code_id: 7,
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

    #[test]
    fn solvency_query_sums_vault_and_pair_escrow() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        let mut bot = test_bot("alice", 2);
        bot.free_balances = [Uint128::new(100), Uint128::new(200)];
        BOTS.save(deps.as_mut().storage, 1, &bot).unwrap();
        for (order_id, side, price, remaining) in [
            (7, LimitOrderSide::Ask, 250u64, 40u128),
            (8, LimitOrderSide::Bid, 150u64, 60u128),
        ] {
            ORDERS
                .save(
                    deps.as_mut().storage,
                    (1, order_id),
                    &GridOrder {
                        rung_index: 3,
                        side,
                        price: Decimal::percent(price),
                        remaining: Uint128::new(remaining),
                    },
                )
                .unwrap();
        }
        deps.querier.update_wasm(|query| match query {
            WasmQuery::Smart { contract_addr, msg } if contract_addr == "pair" => {
                match from_json::<PairQueryMsg>(msg).unwrap() {
                    PairQueryMsg::LimitOrder { order_id } => SystemResult::Ok(ContractResult::Ok(
                        to_json_binary(&LimitOrderResponse {
                            order_id,
                            owner: mock_env().contract.address.to_string(),
                            side: if order_id == 7 {
                                LimitOrderSide::Ask
                            } else {
                                LimitOrderSide::Bid
                            },
                            price: Decimal::percent(if order_id == 7 { 250 } else { 150 }),
                            remaining: Uint128::new(if order_id == 7 { 35 } else { 55 }),
                            expires_at: None,
                            prev: None,
                            next: None,
                        })
                        .unwrap(),
                    )),
                    _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
                }
            }
            WasmQuery::Smart { contract_addr, msg }
                if contract_addr == "token_a" || contract_addr == "token_b" =>
            {
                let _: Cw20QueryMsg = from_json(msg).unwrap();
                let balance = if contract_addr == "token_a" { 105 } else { 205 };
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&BalanceResponse {
                        balance: Uint128::new(balance),
                    })
                    .unwrap(),
                ))
            }
            _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
        });

        let response: SolvencyResponse =
            from_json(query(deps.as_ref(), mock_env(), QueryMsg::Solvency { bot_id: 1 }).unwrap())
                .unwrap();
        assert_eq!(response.token_0_expected, Uint128::new(140));
        assert_eq!(response.token_0_actual, Uint128::new(140));
        assert_eq!(response.token_1_expected, Uint128::new(260));
        assert_eq!(response.token_1_actual, Uint128::new(260));
        assert_eq!(response.active_escrow_orders, 2);
        assert_eq!(response.parked_refund_orders, 0);
        assert!(response.warnings.is_empty());
    }

    #[test]
    fn solvency_query_warns_when_order_escrow_cannot_be_verified() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        let mut bot = test_bot("alice", 1);
        bot.free_balances = [Uint128::new(100), Uint128::zero()];
        BOTS.save(deps.as_mut().storage, 1, &bot).unwrap();
        ORDERS
            .save(
                deps.as_mut().storage,
                (1, 7),
                &GridOrder {
                    rung_index: 3,
                    side: LimitOrderSide::Ask,
                    price: Decimal::percent(250),
                    remaining: Uint128::new(40),
                },
            )
            .unwrap();
        deps.querier.update_wasm(|query| match query {
            WasmQuery::Smart { contract_addr, msg } if contract_addr == "pair" => {
                let _: PairQueryMsg = from_json(msg).unwrap();
                SystemResult::Ok(ContractResult::Err("not found".into()))
            }
            WasmQuery::Smart { contract_addr, msg }
                if contract_addr == "token_a" || contract_addr == "token_b" =>
            {
                let _: Cw20QueryMsg = from_json(msg).unwrap();
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&BalanceResponse {
                        balance: Uint128::zero(),
                    })
                    .unwrap(),
                ))
            }
            _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
        });

        let response: SolvencyResponse =
            from_json(query(deps.as_ref(), mock_env(), QueryMsg::Solvency { bot_id: 1 }).unwrap())
                .unwrap();
        assert_eq!(response.token_0_expected, Uint128::new(140));
        assert_eq!(response.token_0_actual, Uint128::zero());
        assert_eq!(response.unverifiable_orders, 1);
        assert_eq!(response.warnings.len(), 1);
        assert!(response.warnings[0].contains("order 7"));
    }

    #[test]
    fn solvency_query_includes_valid_parked_refund() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        let mut bot = test_bot("alice", 1);
        bot.free_balances = [Uint128::new(100), Uint128::zero()];
        BOTS.save(deps.as_mut().storage, 1, &bot).unwrap();
        ORDERS
            .save(
                deps.as_mut().storage,
                (1, 7),
                &GridOrder {
                    rung_index: 3,
                    side: LimitOrderSide::Ask,
                    price: Decimal::percent(250),
                    remaining: Uint128::new(40),
                },
            )
            .unwrap();
        deps.querier.update_wasm(|query| match query {
            WasmQuery::Smart { contract_addr, msg } if contract_addr == "pair" => {
                match from_json::<PairQueryMsg>(msg).unwrap() {
                    PairQueryMsg::LimitOrder { .. } => {
                        SystemResult::Ok(ContractResult::Err("not active".into()))
                    }
                    PairQueryMsg::ExpiredLimitRefund { order_id } => {
                        SystemResult::Ok(ContractResult::Ok(
                            to_json_binary(&Some(ExpiredLimitRefundResponse {
                                order_id,
                                owner: mock_env().contract.address.to_string(),
                                side: LimitOrderSide::Ask,
                                remaining: Uint128::new(35),
                                expires_at: Some(100),
                            }))
                            .unwrap(),
                        ))
                    }
                    _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
                }
            }
            WasmQuery::Smart { contract_addr, msg }
                if contract_addr == "token_a" || contract_addr == "token_b" =>
            {
                let _: Cw20QueryMsg = from_json(msg).unwrap();
                let balance = if contract_addr == "token_a" { 105 } else { 0 };
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&BalanceResponse {
                        balance: Uint128::new(balance),
                    })
                    .unwrap(),
                ))
            }
            _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
        });

        let response: SolvencyResponse =
            from_json(query(deps.as_ref(), mock_env(), QueryMsg::Solvency { bot_id: 1 }).unwrap())
                .unwrap();
        assert_eq!(response.token_0_expected, Uint128::new(140));
        assert_eq!(response.token_0_actual, Uint128::new(140));
        assert_eq!(response.active_escrow_orders, 0);
        assert_eq!(response.parked_refund_orders, 1);
        assert_eq!(response.executed_orders, 0);
        assert_eq!(response.cancelled_orders, 0);
        assert_eq!(response.unverifiable_orders, 0);
        assert!(response.warnings.is_empty());
    }

    #[test]
    fn reconcile_rejects_mismatched_pair_code() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        BOTS.save(deps.as_mut().storage, 1, &test_bot("alice", 1))
            .unwrap();
        ORDERS
            .save(
                deps.as_mut().storage,
                (1, 77),
                &GridOrder {
                    rung_index: 3,
                    side: LimitOrderSide::Ask,
                    price: Decimal::percent(250),
                    remaining: Uint128::new(100),
                },
            )
            .unwrap();
        deps.querier.update_wasm(|query| match query {
            WasmQuery::ContractInfo { contract_addr } if contract_addr == "pair" => {
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&contract_info(9)).unwrap(),
                ))
            }
            _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
        });

        let error = execute_reconcile(
            deps.as_mut(),
            mock_env(),
            mock_info("permissionless", &[]),
            1,
            vec![77],
        )
        .unwrap_err();
        assert_eq!(error, ContractError::PairCodeMismatch);
    }

    #[test]
    fn update_pair_code_is_admin_only() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        BOTS.save(deps.as_mut().storage, 1, &test_bot("alice", 0))
            .unwrap();

        let error =
            execute_update_pair_code(deps.as_mut(), mock_info("alice", &[]), 1, 8).unwrap_err();
        assert_eq!(error, ContractError::Unauthorized);
        execute_update_pair_code(deps.as_mut(), mock_info("admin", &[]), 1, 8).unwrap();
        assert_eq!(BOTS.load(&deps.storage, 1).unwrap().pair_code_id, 8);
    }

    fn install_cancel_all_querier(
        deps: &mut cosmwasm_std::OwnedDeps<
            cosmwasm_std::MemoryStorage,
            cosmwasm_std::testing::MockApi,
            cosmwasm_std::testing::MockQuerier,
        >,
    ) {
        deps.querier.update_wasm(|query| match query {
            WasmQuery::Smart { contract_addr, msg } if contract_addr == "pair" => {
                match from_json::<PairQueryMsg>(msg).unwrap() {
                    PairQueryMsg::LimitOrder { order_id } => SystemResult::Ok(ContractResult::Ok(
                        to_json_binary(&LimitOrderResponse {
                            order_id,
                            owner: mock_env().contract.address.to_string(),
                            side: LimitOrderSide::Ask,
                            price: Decimal::percent(250),
                            remaining: Uint128::new(100),
                            expires_at: None,
                            prev: None,
                            next: None,
                        })
                        .unwrap(),
                    )),
                    PairQueryMsg::LimitOrderConfig {} => SystemResult::Ok(ContractResult::Ok(
                        to_json_binary(&LimitOrderConfigResponse {
                            max_batch_rungs: 20,
                        })
                        .unwrap(),
                    )),
                    _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
                }
            }
            WasmQuery::ContractInfo { contract_addr } if contract_addr == "pair" => {
                SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&contract_info(7)).unwrap(),
                ))
            }
            _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
        });
    }

    #[test]
    fn cancelled_page_failure_reverts_accounting_and_allows_retry() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        BOTS.save(deps.as_mut().storage, 1, &test_bot("alice", 1))
            .unwrap();
        ORDERS
            .save(
                deps.as_mut().storage,
                (1, 77),
                &GridOrder {
                    rung_index: 3,
                    side: LimitOrderSide::Ask,
                    price: Decimal::percent(250),
                    remaining: Uint128::new(100),
                },
            )
            .unwrap();
        install_cancel_all_querier(&mut deps);

        let cancel =
            execute_cancel_all(deps.as_mut(), mock_env(), mock_info("alice", &[]), 1).unwrap();
        assert_eq!(cancel.messages.len(), 1);
        let failed = reply(
            deps.as_mut(),
            mock_env(),
            Reply {
                id: cancel.messages[0].id,
                result: SubMsgResult::Err("pair reverted".into()),
            },
        )
        .unwrap();
        assert!(failed.attributes.iter().any(|attribute| {
            attribute.key == "action" && attribute.value == "reverted_grid_page"
        }));
        assert!(failed
            .attributes
            .iter()
            .any(|attribute| attribute.key == "reason" && attribute.value == "pair reverted"));
        assert!(PENDING_PAGES
            .keys(&deps.storage, None, None, Order::Ascending)
            .next()
            .is_none());
        let bot = BOTS.load(&deps.storage, 1).unwrap();
        assert_eq!(bot.active_orders, 1);
        assert_eq!(bot.free_balances, [Uint128::zero(), Uint128::zero()]);
        assert!(ORDERS.has(&deps.storage, (1, 77)));

        let retry =
            execute_cancel_all(deps.as_mut(), mock_env(), mock_info("alice", &[]), 1).unwrap();
        settle_replies(&mut deps.as_mut(), retry, true);
        assert!(ORDERS.prefix(1).is_empty(&deps.storage));
        let bot = BOTS.load(&deps.storage, 1).unwrap();
        assert_eq!(bot.active_orders, 0);
        assert_eq!(bot.free_balances, [Uint128::new(100), Uint128::zero()]);
    }

    #[test]
    fn create_bot_rejects_token_outside_allowlist() {
        let mut deps = mock_dependencies();
        install_pair_querier(&mut deps);
        instantiate_default(deps.as_mut());
        execute_add_allowed_token(deps.as_mut(), mock_info("admin", &[]), "token_a".into())
            .unwrap();

        let error = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("alice", &[coin(200, "uluna")]),
            ExecuteMsg::CreateBot {
                pair: "pair".into(),
                lower_price: Decimal::one(),
                upper_price: Decimal::from_ratio(3u128, 1u128),
                grid_count: 5,
            },
        )
        .unwrap_err();
        assert_eq!(error, ContractError::TokenNotAllowed);
        assert!(!BOTS.has(&deps.storage, 1));

        execute_add_allowed_token(deps.as_mut(), mock_info("admin", &[]), "token_b".into())
            .unwrap();
        create_bot(deps.as_mut(), "alice");
        assert!(BOTS.has(&deps.storage, 1));
    }

    #[test]
    fn first_pair_seeds_fail_closed_token_policy() {
        let mut deps = mock_dependencies();
        install_pair_querier(&mut deps);
        instantiate_default(deps.as_mut());
        assert!(!TOKEN_POLICY_ENABLED.load(&deps.storage).unwrap());

        create_bot(deps.as_mut(), "alice");
        assert!(TOKEN_POLICY_ENABLED.load(&deps.storage).unwrap());
        assert!(ALLOWED_TOKENS.has(&deps.storage, &Addr::unchecked("token_a")));
        assert!(ALLOWED_TOKENS.has(&deps.storage, &Addr::unchecked("token_b")));

        execute_remove_allowed_token(deps.as_mut(), mock_info("admin", &[]), "token_a".into())
            .unwrap();
        execute_remove_allowed_token(deps.as_mut(), mock_info("admin", &[]), "token_b".into())
            .unwrap();
        assert_eq!(
            require_token_available(deps.as_ref(), &Addr::unchecked("token_a")),
            Err(ContractError::TokenNotAllowed)
        );
    }

    #[test]
    fn quarantined_token_blocks_create_bot_and_deposit() {
        let mut deps = mock_dependencies();
        install_pair_querier(&mut deps);
        instantiate_default(deps.as_mut());
        execute_add_allowed_token(deps.as_mut(), mock_info("admin", &[]), "token_a".into())
            .unwrap();
        execute_add_allowed_token(deps.as_mut(), mock_info("admin", &[]), "token_b".into())
            .unwrap();
        execute_quarantine_token(deps.as_mut(), mock_info("admin", &[]), "token_a".into()).unwrap();

        let error = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("alice", &[coin(200, "uluna")]),
            ExecuteMsg::CreateBot {
                pair: "pair".into(),
                lower_price: Decimal::one(),
                upper_price: Decimal::from_ratio(3u128, 1u128),
                grid_count: 5,
            },
        )
        .unwrap_err();
        assert_eq!(error, ContractError::TokenQuarantined);

        execute_unquarantine_token(deps.as_mut(), mock_info("admin", &[]), "token_a".into())
            .unwrap();
        create_bot(deps.as_mut(), "alice");
        execute_quarantine_token(deps.as_mut(), mock_info("admin", &[]), "token_a".into()).unwrap();

        let error = execute_receive(
            deps.as_mut(),
            mock_env(),
            mock_info("token_a", &[]),
            Cw20ReceiveMsg {
                sender: "alice".into(),
                amount: Uint128::new(100),
                msg: to_json_binary(&ReceiveMsg::Deposit { bot_id: 1 }).unwrap(),
            },
        )
        .unwrap_err();
        assert_eq!(error, ContractError::TokenQuarantined);
        assert_eq!(
            BOTS.load(&deps.storage, 1).unwrap().free_balances[0],
            Uint128::zero()
        );
    }

    #[test]
    fn token_policy_admin_is_restricted_and_query_reflects_changes() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());

        let error =
            execute_add_allowed_token(deps.as_mut(), mock_info("alice", &[]), "token_a".into())
                .unwrap_err();
        assert_eq!(error, ContractError::Unauthorized);

        execute_add_allowed_token(deps.as_mut(), mock_info("admin", &[]), "token_a".into())
            .unwrap();
        execute_quarantine_token(deps.as_mut(), mock_info("admin", &[]), "token_b".into()).unwrap();

        let response: TokenPolicyResponse =
            from_json(query(deps.as_ref(), mock_env(), QueryMsg::TokenPolicy {}).unwrap()).unwrap();
        assert_eq!(response.allowed_tokens, vec!["token_a".to_string()]);
        assert_eq!(response.quarantined_tokens, vec!["token_b".to_string()]);

        execute_remove_allowed_token(deps.as_mut(), mock_info("admin", &[]), "token_a".into())
            .unwrap();
        execute_unquarantine_token(deps.as_mut(), mock_info("admin", &[]), "token_b".into())
            .unwrap();
        let response: TokenPolicyResponse =
            from_json(query(deps.as_ref(), mock_env(), QueryMsg::TokenPolicy {}).unwrap()).unwrap();
        assert!(response.allowed_tokens.is_empty());
        assert!(response.quarantined_tokens.is_empty());
    }
}
