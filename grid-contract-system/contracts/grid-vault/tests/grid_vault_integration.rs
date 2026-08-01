//! cw-multi-test integration tests for the CL8Y grid vault.
//!
//! The real CL8Y pair/factory contracts are not linked here; we drive the vault
//! against minimal in-test mocks that faithfully mirror the pair/order-book
//! behavior the vault depends on (place batch, cancel, claim expired, limit
//! order book, pool/factory queries). A malicious CW20 mock lets us exercise
//! the vault's handling of lying `Balance` queries.

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    coin, from_json, to_json_binary, Addr, Binary, Decimal, Empty, StdError, StdResult, SubMsg,
    Uint128, WasmMsg,
};
use cw20::{BalanceResponse, Cw20ExecuteMsg, Cw20ReceiveMsg, TokenInfoResponse};
use cw_multi_test::{App, Contract, ContractWrapper, Executor};
use cw_storage_plus::{Item, Map};

use cl8y_grid_manager::msg::{
    ExecuteMsg as ManagerExecuteMsg, InstantiateMsg as ManagerInstantiateMsg,
    QueryMsg as ManagerQueryMsg,
};
use cl8y_grid_vault::msg::{
    Asset, AssetInfo, ExecuteMsg as VaultExecuteMsg, ExpiredLimitRefundResponse, FactoryQueryMsg,
    InstantiateMsg as VaultInstantiateMsg, LimitOrderConfigResponse, LimitOrderResponse,
    LimitOrderSide, PairCw20HookMsg, PairInfo, PairQueryMsg, PoolResponse,
    QueryMsg as VaultQueryMsg, ReceiveMsg, TokenPolicyResponse,
};

// ---------------------------------------------------------------------------
// Mock factory
// ---------------------------------------------------------------------------

#[cw_serde]
pub struct MockFactoryInstantiateMsg {
    pub pair: String,
}

const FACTORY_PAIR: Item<String> = Item::new("mock_factory_pair");

fn mock_factory_code() -> Box<dyn Contract<Empty, Empty>> {
    let contract = ContractWrapper::new(
        |_deps, _env, _info, _msg: Empty| -> StdResult<cosmwasm_std::Response> {
            Err(StdError::generic_err("factory has no execute"))
        },
        |deps, _env, _info, msg: MockFactoryInstantiateMsg| -> StdResult<cosmwasm_std::Response> {
            FACTORY_PAIR.save(
                deps.storage,
                &deps.api.addr_validate(&msg.pair)?.to_string(),
            )?;
            Ok(cosmwasm_std::Response::new())
        },
        |deps, _env, msg: FactoryQueryMsg| -> StdResult<Binary> {
            let pair = FACTORY_PAIR.load(deps.storage)?;
            let response = match msg {
                FactoryQueryMsg::Pair { asset_infos } => cl8y_grid_vault::msg::PairResponse {
                    pair: PairInfo {
                        asset_infos,
                        contract_addr: pair,
                        liquidity_token: "mock-lp".to_string(),
                    },
                },
            };
            to_json_binary(&response)
        },
    );
    Box::new(contract)
}

// ---------------------------------------------------------------------------
// Mock CL8Y pair
// ---------------------------------------------------------------------------

#[cw_serde]
pub struct MockPairInstantiateMsg {
    pub token_0: String,
    pub token_1: String,
    pub reserve_0: Uint128,
    pub reserve_1: Uint128,
    pub max_batch_rungs: u32,
}

#[cw_serde]
pub enum MockPairExecuteMsg {
    Receive(Cw20ReceiveMsg),
    CancelLimitOrders {
        order_ids: Vec<u64>,
    },
    ClaimExpiredLimitOrders {
        order_ids: Vec<u64>,
    },
    Expire {
        order_id: u64,
    },
    SetPaused {
        paused: bool,
    },
    SetLimitOrderError {
        order_id: u64,
        error: Option<String>,
    },
}

/// Fill hook sent by the taker to the pair via a CW20 `Send` of the input
/// token. The pair passes the output token through to the order owner,
/// mirroring how a real match settles (taker's payment funds the payout).
#[cw_serde]
pub struct MockFillHookMsg {
    pub order_id: u64,
    pub fill_amount: Uint128,
    pub output_amount: Uint128,
}

#[cw_serde]
pub struct MockOrder {
    pub owner: String,
    pub side: LimitOrderSide,
    pub price: Decimal,
    pub remaining: Uint128,
    pub expires_at: Option<u64>,
}

const PAIR_TOKEN_0: Item<String> = Item::new("mock_pair_token_0");
const PAIR_TOKEN_1: Item<String> = Item::new("mock_pair_token_1");
const PAIR_RESERVE_0: Item<Uint128> = Item::new("mock_pair_reserve_0");
const PAIR_RESERVE_1: Item<Uint128> = Item::new("mock_pair_reserve_1");
const PAIR_MAX_BATCH: Item<u32> = Item::new("mock_pair_max_batch");
const PAIR_PAUSED: Item<bool> = Item::new("mock_pair_paused");
const PAIR_NEXT_ORDER: Item<u64> = Item::new("mock_pair_next_order");
const PAIR_ORDERS: Map<u64, MockOrder> = Map::new("mock_pair_orders");
const PAIR_PARKED: Map<u64, MockOrder> = Map::new("mock_pair_parked");
const PAIR_QUERY_ERRORS: Map<u64, String> = Map::new("mock_pair_query_errors");

fn mock_pair_code() -> Box<dyn Contract<Empty, Empty>> {
    let contract = ContractWrapper::new(
        |deps, _env, info, msg: MockPairExecuteMsg| -> StdResult<cosmwasm_std::Response> {
            mock_pair_execute(deps, info, msg)
        },
        |deps, _env, _info, msg: MockPairInstantiateMsg| -> StdResult<cosmwasm_std::Response> {
            PAIR_TOKEN_0.save(
                deps.storage,
                &deps.api.addr_validate(&msg.token_0)?.to_string(),
            )?;
            PAIR_TOKEN_1.save(
                deps.storage,
                &deps.api.addr_validate(&msg.token_1)?.to_string(),
            )?;
            PAIR_RESERVE_0.save(deps.storage, &msg.reserve_0)?;
            PAIR_RESERVE_1.save(deps.storage, &msg.reserve_1)?;
            PAIR_MAX_BATCH.save(deps.storage, &msg.max_batch_rungs.max(1))?;
            PAIR_PAUSED.save(deps.storage, &false)?;
            PAIR_NEXT_ORDER.save(deps.storage, &1)?;
            Ok(cosmwasm_std::Response::new())
        },
        |deps, env, msg: PairQueryMsg| -> StdResult<Binary> { mock_pair_query(deps, env, msg) },
    );
    Box::new(contract)
}

fn escrow_token(storage: &dyn cosmwasm_std::Storage, side: &LimitOrderSide) -> StdResult<String> {
    match side {
        LimitOrderSide::Ask => PAIR_TOKEN_0.load(storage),
        LimitOrderSide::Bid => PAIR_TOKEN_1.load(storage),
    }
}

fn cw20_transfer(token: String, recipient: String, amount: Uint128) -> WasmMsg {
    WasmMsg::Execute {
        contract_addr: token,
        msg: to_json_binary(&Cw20ExecuteMsg::Transfer { recipient, amount })
            .expect("serialize transfer"),
        funds: vec![],
    }
}

fn mock_pair_execute(
    deps: cosmwasm_std::DepsMut,
    info: cosmwasm_std::MessageInfo,
    msg: MockPairExecuteMsg,
) -> Result<cosmwasm_std::Response, StdError> {
    match msg {
        MockPairExecuteMsg::Receive(receive) => {
            if PAIR_PAUSED.load(deps.storage)? {
                return Err(StdError::generic_err("pair paused"));
            }
            // Taker-sent fill: the received input token funds the payout to the
            // order owner (the vault).
            if let Ok(fill) = from_json::<MockFillHookMsg>(&receive.msg) {
                let mut order = PAIR_ORDERS.load(deps.storage, fill.order_id)?;
                if fill.fill_amount > order.remaining {
                    return Err(StdError::generic_err("fill exceeds remaining"));
                }
                let expected_input = match order.side {
                    LimitOrderSide::Ask => PAIR_TOKEN_1.load(deps.storage)?,
                    LimitOrderSide::Bid => PAIR_TOKEN_0.load(deps.storage)?,
                };
                if info.sender != expected_input {
                    return Err(StdError::generic_err("fill input token mismatch"));
                }
                order.remaining = order.remaining.checked_sub(fill.fill_amount)?;
                if order.remaining.is_zero() {
                    PAIR_ORDERS.remove(deps.storage, fill.order_id);
                } else {
                    PAIR_ORDERS.save(deps.storage, fill.order_id, &order)?;
                }
                let output_token = match order.side {
                    LimitOrderSide::Ask => PAIR_TOKEN_1.load(deps.storage)?,
                    LimitOrderSide::Bid => PAIR_TOKEN_0.load(deps.storage)?,
                };
                let message = cw20_transfer(output_token, order.owner, fill.output_amount);
                return Ok(cosmwasm_std::Response::new().add_message(message));
            }
            let hook: PairCw20HookMsg = from_json(receive.msg)?;
            let token_0 = PAIR_TOKEN_0.load(deps.storage)?;
            let token_1 = PAIR_TOKEN_1.load(deps.storage)?;
            let sender = info.sender.to_string();
            let PairCw20HookMsg::PlaceLimitOrderBatch { side, orders } = hook;
            match &side {
                LimitOrderSide::Ask => {
                    if sender != token_0 {
                        return Err(StdError::generic_err("ask must escrow token_0"));
                    }
                }
                LimitOrderSide::Bid => {
                    if sender != token_1 {
                        return Err(StdError::generic_err("bid must escrow token_1"));
                    }
                }
            }
            let mut next = PAIR_NEXT_ORDER.load(deps.storage)?;
            let mut response = cosmwasm_std::Response::new();
            for order in orders {
                PAIR_ORDERS.save(
                    deps.storage,
                    next,
                    &MockOrder {
                        owner: receive.sender.clone(),
                        side: side.clone(),
                        price: order.price,
                        remaining: order.amount,
                        expires_at: order.expires_at,
                    },
                )?;
                response = response.add_attribute("limit_order_placed", next.to_string());
                next += 1;
            }
            PAIR_NEXT_ORDER.save(deps.storage, &next)?;
            Ok(response)
        }
        MockPairExecuteMsg::CancelLimitOrders { order_ids } => {
            if PAIR_PAUSED.load(deps.storage)? {
                return Err(StdError::generic_err("pair paused"));
            }
            let mut response = cosmwasm_std::Response::new();
            for id in order_ids {
                let order = PAIR_ORDERS.load(deps.storage, id)?;
                let token = escrow_token(deps.storage, &order.side)?;
                response = response.add_message(cw20_transfer(token, order.owner, order.remaining));
                PAIR_ORDERS.remove(deps.storage, id);
            }
            Ok(response)
        }
        MockPairExecuteMsg::ClaimExpiredLimitOrders { order_ids } => {
            if PAIR_PAUSED.load(deps.storage)? {
                return Err(StdError::generic_err("pair paused"));
            }
            let mut response = cosmwasm_std::Response::new();
            for id in order_ids {
                let order = PAIR_PARKED.load(deps.storage, id)?;
                let token = escrow_token(deps.storage, &order.side)?;
                response = response.add_message(cw20_transfer(token, order.owner, order.remaining));
                PAIR_PARKED.remove(deps.storage, id);
            }
            Ok(response)
        }
        MockPairExecuteMsg::Expire { order_id } => {
            let order = PAIR_ORDERS.load(deps.storage, order_id)?;
            PAIR_ORDERS.remove(deps.storage, order_id);
            PAIR_PARKED.save(deps.storage, order_id, &order)?;
            Ok(cosmwasm_std::Response::new())
        }
        MockPairExecuteMsg::SetPaused { paused } => {
            PAIR_PAUSED.save(deps.storage, &paused)?;
            Ok(cosmwasm_std::Response::new())
        }
        MockPairExecuteMsg::SetLimitOrderError { order_id, error } => {
            match error {
                Some(error) => PAIR_QUERY_ERRORS.save(deps.storage, order_id, &error)?,
                None => PAIR_QUERY_ERRORS.remove(deps.storage, order_id),
            }
            Ok(cosmwasm_std::Response::new())
        }
    }
}

fn mock_pair_query(
    deps: cosmwasm_std::Deps,
    env: cosmwasm_std::Env,
    msg: PairQueryMsg,
) -> Result<Binary, StdError> {
    match msg {
        PairQueryMsg::Pair {} => to_json_binary(&PairInfo {
            asset_infos: [
                AssetInfo::Token {
                    contract_addr: PAIR_TOKEN_0.load(deps.storage)?,
                },
                AssetInfo::Token {
                    contract_addr: PAIR_TOKEN_1.load(deps.storage)?,
                },
            ],
            contract_addr: env.contract.address.to_string(),
            liquidity_token: "mock-lp".to_string(),
        }),
        PairQueryMsg::Pool {} => to_json_binary(&PoolResponse {
            assets: [
                Asset {
                    info: AssetInfo::Token {
                        contract_addr: PAIR_TOKEN_0.load(deps.storage)?,
                    },
                    amount: PAIR_RESERVE_0.load(deps.storage)?,
                },
                Asset {
                    info: AssetInfo::Token {
                        contract_addr: PAIR_TOKEN_1.load(deps.storage)?,
                    },
                    amount: PAIR_RESERVE_1.load(deps.storage)?,
                },
            ],
            total_share: Uint128::new(1),
        }),
        PairQueryMsg::LimitOrderConfig {} => to_json_binary(&LimitOrderConfigResponse {
            max_batch_rungs: PAIR_MAX_BATCH.load(deps.storage)?,
        }),
        PairQueryMsg::LimitOrder { order_id } => {
            if let Some(error) = PAIR_QUERY_ERRORS.may_load(deps.storage, order_id)? {
                return Err(StdError::generic_err(error));
            }
            let order = PAIR_ORDERS.load(deps.storage, order_id)?;
            to_json_binary(&LimitOrderResponse {
                order_id,
                owner: order.owner,
                side: order.side,
                price: order.price,
                remaining: order.remaining,
                expires_at: order.expires_at,
                prev: None,
                next: None,
            })
        }
        PairQueryMsg::ExpiredLimitRefund { order_id } => {
            let refund = PAIR_PARKED.may_load(deps.storage, order_id)?.map(|order| {
                ExpiredLimitRefundResponse {
                    order_id,
                    owner: order.owner,
                    side: order.side,
                    remaining: order.remaining,
                    expires_at: order.expires_at,
                }
            });
            to_json_binary(&refund)
        }
    }
}

// ---------------------------------------------------------------------------
// Malicious CW20 mock (lies about `Balance`)
// ---------------------------------------------------------------------------

#[cw_serde]
pub struct MockMaliciousInstantiateMsg {
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    pub initial_balances: Vec<cw20::Cw20Coin>,
    pub lie: Uint128,
}

#[cw_serde]
pub enum MockMaliciousExecuteMsg {
    Transfer {
        recipient: String,
        amount: Uint128,
    },
    Send {
        contract: String,
        amount: Uint128,
        msg: Binary,
    },
    Mint {
        recipient: String,
        amount: Uint128,
    },
}

#[cw_serde]
pub enum MockMaliciousQueryMsg {
    Balance { address: String },
    TokenInfo {},
}

const MAL_BALANCES: Map<&Addr, Uint128> = Map::new("mal_balances");
const MAL_TOTAL: Item<Uint128> = Item::new("mal_total");
const MAL_LIE: Item<Uint128> = Item::new("mal_lie");
const MAL_NAME: Item<String> = Item::new("mal_name");
const MAL_SYMBOL: Item<String> = Item::new("mal_symbol");
const MAL_DECIMALS: Item<u8> = Item::new("mal_decimals");

fn malicious_token_code() -> Box<dyn Contract<Empty, Empty>> {
    let contract = ContractWrapper::new(
        |deps, _env, info, msg: MockMaliciousExecuteMsg| -> StdResult<cosmwasm_std::Response> {
            match msg {
                MockMaliciousExecuteMsg::Transfer { recipient, amount } => {
                    let recipient = deps.api.addr_validate(&recipient)?;
                    let sender = MAL_BALANCES
                        .may_load(deps.storage, &info.sender)?
                        .unwrap_or_default()
                        .checked_sub(amount)?;
                    MAL_BALANCES.save(deps.storage, &info.sender, &sender)?;
                    let recipient_balance = MAL_BALANCES
                        .may_load(deps.storage, &recipient)?
                        .unwrap_or_default()
                        .checked_add(amount)?;
                    MAL_BALANCES.save(deps.storage, &recipient, &recipient_balance)?;
                    Ok(cosmwasm_std::Response::new())
                }
                MockMaliciousExecuteMsg::Send {
                    contract,
                    amount,
                    msg,
                } => {
                    let contract = deps.api.addr_validate(&contract)?;
                    let sender = MAL_BALANCES
                        .may_load(deps.storage, &info.sender)?
                        .unwrap_or_default()
                        .checked_sub(amount)?;
                    MAL_BALANCES.save(deps.storage, &info.sender, &sender)?;
                    let recipient_balance = MAL_BALANCES
                        .may_load(deps.storage, &contract)?
                        .unwrap_or_default()
                        .checked_add(amount)?;
                    MAL_BALANCES.save(deps.storage, &contract, &recipient_balance)?;
                    let receive = Cw20ReceiveMsg {
                        sender: info.sender.to_string(),
                        amount,
                        msg,
                    };
                    Ok(cosmwasm_std::Response::new().add_submessage(SubMsg::new(
                        WasmMsg::Execute {
                            contract_addr: contract.to_string(),
                            msg: to_json_binary(&VaultExecuteMsg::Receive(receive))?,
                            funds: vec![],
                        },
                    )))
                }
                MockMaliciousExecuteMsg::Mint { recipient, amount } => {
                    let recipient = deps.api.addr_validate(&recipient)?;
                    let balance = MAL_BALANCES
                        .may_load(deps.storage, &recipient)?
                        .unwrap_or_default()
                        .checked_add(amount)?;
                    MAL_BALANCES.save(deps.storage, &recipient, &balance)?;
                    let total = MAL_TOTAL.load(deps.storage)?.checked_add(amount)?;
                    MAL_TOTAL.save(deps.storage, &total)?;
                    Ok(cosmwasm_std::Response::new())
                }
            }
        },
        |deps,
         _env,
         _info,
         msg: MockMaliciousInstantiateMsg|
         -> StdResult<cosmwasm_std::Response> {
            MAL_NAME.save(deps.storage, &msg.name)?;
            MAL_SYMBOL.save(deps.storage, &msg.symbol)?;
            MAL_DECIMALS.save(deps.storage, &msg.decimals)?;
            MAL_LIE.save(deps.storage, &msg.lie)?;
            let mut total = Uint128::zero();
            for coin in msg.initial_balances {
                let addr = deps.api.addr_validate(&coin.address)?;
                MAL_BALANCES.save(deps.storage, &addr, &coin.amount)?;
                total = total.checked_add(coin.amount)?;
            }
            MAL_TOTAL.save(deps.storage, &total)?;
            Ok(cosmwasm_std::Response::new())
        },
        |deps, _env, msg: MockMaliciousQueryMsg| -> StdResult<Binary> {
            match msg {
                MockMaliciousQueryMsg::Balance { address } => {
                    let addr = deps.api.addr_validate(&address)?;
                    let real = MAL_BALANCES
                        .may_load(deps.storage, &addr)?
                        .unwrap_or_default();
                    let lie = MAL_LIE.load(deps.storage)?;
                    to_json_binary(&BalanceResponse {
                        balance: real.checked_add(lie)?,
                    })
                }
                MockMaliciousQueryMsg::TokenInfo {} => to_json_binary(&TokenInfoResponse {
                    name: MAL_NAME.load(deps.storage)?,
                    symbol: MAL_SYMBOL.load(deps.storage)?,
                    decimals: MAL_DECIMALS.load(deps.storage)?,
                    total_supply: MAL_TOTAL.load(deps.storage)?,
                }),
            }
        },
    );
    Box::new(contract)
}

// ---------------------------------------------------------------------------
// Vault / manager / cw20 contracts
// ---------------------------------------------------------------------------

fn vault_code() -> Box<dyn Contract<Empty, Empty>> {
    let contract = ContractWrapper::new(
        cl8y_grid_vault::contract::execute,
        cl8y_grid_vault::contract::instantiate,
        cl8y_grid_vault::contract::query,
    )
    .with_reply(cl8y_grid_vault::contract::reply);
    Box::new(contract)
}

fn manager_code() -> Box<dyn Contract<Empty, Empty>> {
    let contract = ContractWrapper::new(
        cl8y_grid_manager::contract::execute,
        cl8y_grid_manager::contract::instantiate,
        cl8y_grid_manager::contract::query,
    )
    .with_reply(cl8y_grid_manager::contract::reply);
    Box::new(contract)
}

fn cw20_code() -> Box<dyn Contract<Empty, Empty>> {
    let contract = ContractWrapper::new(
        cw20_base::contract::execute,
        cw20_base::contract::instantiate,
        cw20_base::contract::query,
    );
    Box::new(contract)
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

const GAS_DENOM: &str = "uluna";
const KEEPER_REWARD: u128 = 20;
const MIN_GAS_RESERVE: u128 = 100;
const ORDER_TIMEOUT_SECONDS: u64 = 3600;

struct Harness {
    app: App,
    vault: Addr,
    manager: Addr,
    pair: Addr,
    token_0: Addr,
    token_1: Addr,
    alice: Addr,
    keeper: Addr,
}

impl Harness {
    fn new() -> Self {
        Self::new_with_tokens(None)
    }

    fn new_with_malicious(lie: Uint128) -> Self {
        Self::new_with_tokens(Some(lie))
    }

    fn new_with_tokens(malicious_lie: Option<Uint128>) -> Self {
        let alice = Addr::unchecked("alice");
        let admin = Addr::unchecked("admin");
        let keeper = Addr::unchecked("keeper");
        let mut app = App::new(|router, _api, storage| {
            router
                .bank
                .init_balance(storage, &alice, vec![coin(1_000_000_000, GAS_DENOM)])
                .unwrap();
        });

        let cw20 = app.store_code(cw20_code());
        let malicious = app.store_code(malicious_token_code());
        let pair_code_id = app.store_code(mock_pair_code());
        let factory_code_id = app.store_code(mock_factory_code());

        let token_0 = app
            .instantiate_contract(
                cw20,
                alice.clone(),
                &cw20_base::msg::InstantiateMsg {
                    name: "Mock Token Zero".to_string(),
                    symbol: "MTZ".to_string(),
                    decimals: 6,
                    initial_balances: vec![cw20::Cw20Coin {
                        address: alice.to_string(),
                        amount: Uint128::new(1_000_000_000),
                    }],
                    mint: None,
                    marketing: None,
                },
                &[],
                "token-0",
                None,
            )
            .unwrap();

        let token_1 = app
            .instantiate_contract(
                cw20,
                alice.clone(),
                &cw20_base::msg::InstantiateMsg {
                    name: "Mock Token One".to_string(),
                    symbol: "MTO".to_string(),
                    decimals: 6,
                    initial_balances: vec![cw20::Cw20Coin {
                        address: alice.to_string(),
                        amount: Uint128::new(1_000_000_000),
                    }],
                    mint: None,
                    marketing: None,
                },
                &[],
                "token-1",
                None,
            )
            .unwrap();

        let token_0 = match malicious_lie {
            Some(lie) => app
                .instantiate_contract(
                    malicious,
                    alice.clone(),
                    &MockMaliciousInstantiateMsg {
                        name: "Malicious Token".to_string(),
                        symbol: "MAL".to_string(),
                        decimals: 6,
                        initial_balances: vec![cw20::Cw20Coin {
                            address: alice.to_string(),
                            amount: Uint128::new(1_000_000_000),
                        }],
                        lie,
                    },
                    &[],
                    "malicious-token",
                    None,
                )
                .unwrap(),
            None => token_0,
        };

        // Pair with reference price reserve_1 / reserve_0 = 2.0.
        let pair = app
            .instantiate_contract(
                pair_code_id,
                alice.clone(),
                &MockPairInstantiateMsg {
                    token_0: token_0.to_string(),
                    token_1: token_1.to_string(),
                    reserve_0: Uint128::new(1_000_000),
                    reserve_1: Uint128::new(2_000_000),
                    max_batch_rungs: 20,
                },
                &[],
                "mock-pair",
                None,
            )
            .unwrap();

        let factory = app
            .instantiate_contract(
                factory_code_id,
                alice.clone(),
                &MockFactoryInstantiateMsg {
                    pair: pair.to_string(),
                },
                &[],
                "mock-factory",
                None,
            )
            .unwrap();

        // Fund the taker so fills can pass tokens through the pair.
        let taker = Addr::unchecked("taker");
        for token in [&token_0, &token_1] {
            app.execute_contract(
                alice.clone(),
                token.clone(),
                &Cw20ExecuteMsg::Transfer {
                    recipient: taker.to_string(),
                    amount: Uint128::new(100_000_000),
                },
                &[],
            )
            .unwrap();
        }

        let vault_code_id = app.store_code(vault_code());
        let vault = app
            .instantiate_contract(
                vault_code_id,
                alice.clone(),
                &VaultInstantiateMsg {
                    admin: admin.to_string(),
                    owner: alice.to_string(),
                    keeper: keeper.to_string(),
                    factory: factory.to_string(),
                    gas_denom: GAS_DENOM.to_string(),
                    keeper_reward: Uint128::new(KEEPER_REWARD),
                    minimum_gas_reserve: Uint128::new(MIN_GAS_RESERVE),
                    order_timeout_seconds: ORDER_TIMEOUT_SECONDS,
                    max_grid_count: 12,
                    max_orders_per_reconcile: 20,
                    max_active_orders_per_bot: 20,
                },
                &[],
                "grid-vault",
                None,
            )
            .unwrap();

        let manager_code_id = app.store_code(manager_code());
        let manager = app
            .instantiate_contract(
                manager_code_id,
                alice.clone(),
                &ManagerInstantiateMsg {
                    admin: admin.to_string(),
                    keeper: keeper.to_string(),
                    dex_factory: factory.to_string(),
                    vault_code_id,
                    gas_denom: GAS_DENOM.to_string(),
                    keeper_reward: Uint128::new(KEEPER_REWARD),
                    minimum_gas_reserve: Uint128::new(MIN_GAS_RESERVE),
                    order_timeout_seconds: ORDER_TIMEOUT_SECONDS,
                    max_grid_count: 12,
                    max_orders_per_reconcile: 20,
                    max_active_orders_per_vault: 20,
                },
                &[],
                "grid-manager",
                None,
            )
            .unwrap();

        Harness {
            app,
            vault,
            manager,
            pair,
            token_0,
            token_1,
            alice,
            keeper,
        }
    }

    fn create_bot(&mut self) {
        self.app
            .execute_contract(
                self.alice.clone(),
                self.vault.clone(),
                &VaultExecuteMsg::CreateBot {
                    pair: self.pair.to_string(),
                    lower_price: Decimal::from_atomics(1u128, 0).unwrap(),
                    upper_price: Decimal::from_atomics(3u128, 0).unwrap(),
                    grid_count: 5,
                },
                &[coin(MIN_GAS_RESERVE + KEEPER_REWARD, GAS_DENOM)],
            )
            .unwrap();
    }

    fn deposit(&mut self, bot_id: u64, token: &Addr, amount: Uint128) {
        self.app
            .execute_contract(
                self.alice.clone(),
                token.clone(),
                &Cw20ExecuteMsg::Send {
                    contract: self.vault.to_string(),
                    amount,
                    msg: to_json_binary(&ReceiveMsg::Deposit { bot_id }).unwrap(),
                },
                &[],
            )
            .unwrap();
    }

    fn reconcile(&mut self, bot_id: u64, order_ids: Vec<u64>) {
        self.app
            .execute_contract(
                self.keeper.clone(),
                self.vault.clone(),
                &VaultExecuteMsg::Reconcile { bot_id, order_ids },
                &[],
            )
            .unwrap();
    }

    fn cancel_all(&mut self, bot_id: u64) {
        self.app
            .execute_contract(
                self.alice.clone(),
                self.vault.clone(),
                &VaultExecuteMsg::CancelAll { bot_id },
                &[],
            )
            .unwrap();
    }

    fn withdraw(&mut self, bot_id: u64, shares: Uint128) {
        self.app
            .execute_contract(
                self.alice.clone(),
                self.vault.clone(),
                &VaultExecuteMsg::Withdraw {
                    bot_id,
                    shares,
                    recipient: None,
                },
                &[],
            )
            .unwrap();
    }

    fn fill(&mut self, order_id: u64, fill_amount: Uint128, output_amount: Uint128) {
        let side = self
            .on_chain_order(order_id)
            .expect("order exists on the book")
            .side;
        let input_token = match side {
            LimitOrderSide::Ask => self.token_1.clone(),
            LimitOrderSide::Bid => self.token_0.clone(),
        };
        self.app
            .execute_contract(
                Addr::unchecked("taker"),
                input_token,
                &Cw20ExecuteMsg::Send {
                    contract: self.pair.to_string(),
                    amount: output_amount,
                    msg: to_json_binary(&MockFillHookMsg {
                        order_id,
                        fill_amount,
                        output_amount,
                    })
                    .unwrap(),
                },
                &[],
            )
            .unwrap();
    }

    fn expire(&mut self, order_id: u64) {
        self.app
            .execute_contract(
                Addr::unchecked("taker"),
                self.pair.clone(),
                &MockPairExecuteMsg::Expire { order_id },
                &[],
            )
            .unwrap();
    }

    fn set_paused(&mut self, paused: bool) {
        self.app
            .execute_contract(
                Addr::unchecked("taker"),
                self.pair.clone(),
                &MockPairExecuteMsg::SetPaused { paused },
                &[],
            )
            .unwrap();
    }

    fn set_limit_order_error(&mut self, order_id: u64, error: Option<String>) {
        self.app
            .execute_contract(
                Addr::unchecked("taker"),
                self.pair.clone(),
                &MockPairExecuteMsg::SetLimitOrderError { order_id, error },
                &[],
            )
            .unwrap();
    }

    fn orders(&self, bot_id: u64) -> Vec<cl8y_grid_vault::msg::OrderResponse> {
        self.app
            .wrap()
            .query_wasm_smart(&self.vault, &VaultQueryMsg::Orders { bot_id })
            .unwrap()
    }

    fn bot(&self, bot_id: u64) -> cl8y_grid_vault::msg::BotResponse {
        self.app
            .wrap()
            .query_wasm_smart(&self.vault, &VaultQueryMsg::Bot { bot_id })
            .unwrap()
    }

    fn balance_of(&self, token: &Addr, account: &Addr) -> Uint128 {
        let response: BalanceResponse = self
            .app
            .wrap()
            .query_wasm_smart(
                token,
                &cw20::Cw20QueryMsg::Balance {
                    address: account.to_string(),
                },
            )
            .unwrap();
        response.balance
    }

    fn on_chain_order(&self, order_id: u64) -> Option<LimitOrderResponse> {
        self.app
            .wrap()
            .query_wasm_smart(&self.pair, &PairQueryMsg::LimitOrder { order_id })
            .ok()
    }

    /// Sum of on-chain remaining amounts escrowing each token across the vault's
    /// tracked order ids. Expired (parked) orders are counted via their refund
    /// entry. Returns `[escrow_0, escrow_1]`.
    fn on_chain_escrow(&self, bot_id: u64) -> [Uint128; 2] {
        let mut escrow = [Uint128::zero(), Uint128::zero()];
        for order in self.orders(bot_id) {
            if let Some(on_chain) = self.on_chain_order(order.order_id) {
                let index = match on_chain.side {
                    LimitOrderSide::Ask => 0,
                    LimitOrderSide::Bid => 1,
                };
                escrow[index] = escrow[index].checked_add(on_chain.remaining).unwrap();
                continue;
            }
            let refund: Option<ExpiredLimitRefundResponse> = self
                .app
                .wrap()
                .query_wasm_smart(
                    &self.pair,
                    &PairQueryMsg::ExpiredLimitRefund {
                        order_id: order.order_id,
                    },
                )
                .unwrap();
            if let Some(refund) = refund {
                let index = match refund.side {
                    LimitOrderSide::Ask => 0,
                    LimitOrderSide::Bid => 1,
                };
                escrow[index] = escrow[index].checked_add(refund.remaining).unwrap();
            }
        }
        escrow
    }
}

// ---------------------------------------------------------------------------
// Scenario tests
// ---------------------------------------------------------------------------

#[test]
fn full_lifecycle_with_fill_reconcile_cancel_withdraw() {
    let mut h = Harness::new();
    h.create_bot();

    // token_0 -> Ask rungs (2 orders), token_1 -> Bid rungs (2 orders).
    h.deposit(1, &h.token_0.clone(), Uint128::new(1_000));
    h.deposit(1, &h.token_1.clone(), Uint128::new(2_000));

    let bot = h.bot(1);
    assert_eq!(bot.active_orders, 4);
    assert_eq!(bot.total_shares, Uint128::new(2_000));

    // Partially fill the first Ask order (escrows token_0).
    let orders = h.orders(1);
    let ask = orders
        .iter()
        .find(|order| order.side == LimitOrderSide::Ask)
        .unwrap();
    h.fill(ask.order_id, Uint128::new(200), Uint128::new(100));

    // Fully fill the first Bid order (escrows token_1 -> vault receives token_0).
    let bid = orders
        .iter()
        .find(|order| order.side == LimitOrderSide::Bid)
        .unwrap();
    h.fill(bid.order_id, Uint128::new(1_000), Uint128::new(500));

    // Reconcile as the keeper; both fills must be credited.
    let ids: Vec<u64> = h.orders(1).iter().map(|order| order.order_id).collect();
    h.reconcile(1, ids);

    let bot = h.bot(1);
    assert_eq!(bot.free_balances[0], Uint128::new(500));
    assert_eq!(bot.free_balances[1], Uint128::new(100));
    assert_eq!(bot.active_orders, 3);

    let orders = h.orders(1);
    assert_eq!(orders.len(), 3);
    let ask = orders
        .iter()
        .find(|order| order.side == LimitOrderSide::Ask)
        .unwrap();
    assert_eq!(ask.remaining, Uint128::new(300));

    // Cancel all -> refunds remaining escrow.
    h.cancel_all(1);
    let bot = h.bot(1);
    assert_eq!(bot.active_orders, 0);
    assert_eq!(bot.free_balances[0], Uint128::new(1_300));
    assert_eq!(bot.free_balances[1], Uint128::new(1_100));

    // Withdraw everything.
    h.withdraw(1, bot.total_shares);
    assert_eq!(
        h.balance_of(&h.token_0, &h.alice),
        Uint128::new(1_000_000_000 - 100_000_000 + 300)
    );
    assert_eq!(
        h.balance_of(&h.token_1, &h.alice),
        Uint128::new(1_000_000_000 - 100_000_000 - 900)
    );
}

#[test]
fn independent_bots_realize_exact_round_trip_spread_profit() {
    fn run_round_trip(h: &mut Harness, sold_base: Uint128) -> Uint128 {
        h.create_bot();
        h.deposit(1, &h.token_0.clone(), Uint128::new(1_000));
        h.deposit(1, &h.token_1.clone(), Uint128::new(2_000));
        let orders = h.orders(1);
        let ask = orders
            .iter()
            .find(|order| order.side == LimitOrderSide::Ask)
            .unwrap();
        let bid = orders
            .iter()
            .find(|order| order.side == LimitOrderSide::Bid)
            .unwrap();
        assert!(ask.price > bid.price);

        let quote_proceeds =
            sold_base.multiply_ratio(ask.price.atomics(), Decimal::one().atomics());
        let bought_base =
            quote_proceeds.multiply_ratio(Decimal::one().atomics(), bid.price.atomics());
        assert!(quote_proceeds <= bid.remaining);
        assert!(bought_base > sold_base);

        h.fill(ask.order_id, sold_base, quote_proceeds);
        h.fill(bid.order_id, quote_proceeds, bought_base);
        h.reconcile(1, vec![ask.order_id, bid.order_id]);
        h.cancel_all(1);

        let bot = h.bot(1);
        let profit = bought_base.checked_sub(sold_base).unwrap();
        assert_eq!(bot.free_balances[0], Uint128::new(1_000) + profit);
        assert_eq!(bot.free_balances[1], Uint128::new(2_000));
        profit
    }

    let mut first = Harness::new();
    let mut second = Harness::new();
    second.create_bot();
    second.deposit(1, &second.token_0.clone(), Uint128::new(1_000));
    second.deposit(1, &second.token_1.clone(), Uint128::new(2_000));
    let second_before = second.bot(1);
    let second_orders_before = second.orders(1);

    let first_profit = run_round_trip(&mut first, Uint128::new(200));
    assert_eq!(second.bot(1), second_before);
    assert_eq!(second.orders(1), second_orders_before);

    let orders = second.orders(1);
    let ask = orders
        .iter()
        .find(|order| order.side == LimitOrderSide::Ask)
        .unwrap();
    let bid = orders
        .iter()
        .find(|order| order.side == LimitOrderSide::Bid)
        .unwrap();
    let sold_base = Uint128::new(125);
    let quote_proceeds = sold_base.multiply_ratio(ask.price.atomics(), Decimal::one().atomics());
    let bought_base = quote_proceeds.multiply_ratio(Decimal::one().atomics(), bid.price.atomics());
    second.fill(ask.order_id, sold_base, quote_proceeds);
    second.fill(bid.order_id, quote_proceeds, bought_base);
    second.reconcile(1, vec![ask.order_id, bid.order_id]);
    second.cancel_all(1);
    let second_profit = bought_base.checked_sub(sold_base).unwrap();

    assert_eq!(
        second.bot(1).free_balances[0],
        Uint128::new(1_000) + second_profit
    );
    assert_eq!(second.bot(1).free_balances[1], Uint128::new(2_000));
    assert_eq!(first_profit, Uint128::new(300));
    assert_eq!(second_profit, Uint128::new(187));
}

#[test]
fn parking_expiry_and_dust_reconcile_claims_refund() {
    let mut h = Harness::new();
    h.create_bot();
    h.deposit(1, &h.token_0.clone(), Uint128::new(1_000));
    h.deposit(1, &h.token_1.clone(), Uint128::new(2_000));

    let orders = h.orders(1);
    assert_eq!(orders.len(), 4);

    // Expire one Ask and one Bid order (moves to the pair's parked queue).
    let ask = orders
        .iter()
        .find(|order| order.side == LimitOrderSide::Ask)
        .unwrap();
    let bid = orders
        .iter()
        .find(|order| order.side == LimitOrderSide::Bid)
        .unwrap();
    h.expire(ask.order_id);
    h.expire(bid.order_id);

    // Reconcile detects the active-book miss, reads ExpiredLimitRefund and
    // claims both parked refunds.
    let ids: Vec<u64> = h.orders(1).iter().map(|order| order.order_id).collect();
    h.reconcile(1, ids);

    let bot = h.bot(1);
    assert_eq!(bot.active_orders, 2);
    assert_eq!(bot.free_balances[0], Uint128::new(500));
    assert_eq!(bot.free_balances[1], Uint128::new(1_000));
    assert_eq!(h.orders(1).len(), 2);
    assert!(h.on_chain_order(ask.order_id).is_none());
    assert!(h.on_chain_order(bid.order_id).is_none());
}

#[test]
fn pair_pause_blocks_cancel_claim_and_deposit_until_resume() {
    let mut h = Harness::new();
    h.create_bot();
    h.deposit(1, &h.token_0.clone(), Uint128::new(1_000));
    h.deposit(1, &h.token_1.clone(), Uint128::new(2_000));

    let orders = h.orders(1);
    assert_eq!(orders.len(), 4);
    let ask = orders
        .iter()
        .find(|order| order.side == LimitOrderSide::Ask)
        .unwrap();

    // Pausing the pair must block cancel, new placements via deposit, and the
    // claim triggered by reconcile.
    h.set_paused(true);

    // 1. Cancel all -> the vault's cancel submessage is reverted by the paused
    //    pair. The outer tx succeeds but emits a `reverted_grid_page` event and
    //    leaves the accounting untouched.
    let response = h
        .app
        .execute_contract(
            h.alice.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::CancelAll { bot_id: 1 },
            &[],
        )
        .unwrap();
    assert!(response.events.iter().any(|event| {
        event
            .attributes
            .iter()
            .any(|attribute| attribute.key == "action" && attribute.value == "reverted_grid_page")
    }));
    assert_eq!(h.orders(1).len(), 4);
    let bot = h.bot(1);
    assert_eq!(bot.active_orders, 4);
    assert_eq!(bot.free_balances, [Uint128::zero(), Uint128::zero()]);

    // 2. Deposit -> the placement submessage hits the paused pair.
    let err = h
        .app
        .execute_contract(
            h.alice.clone(),
            h.token_0.clone(),
            &Cw20ExecuteMsg::Send {
                contract: h.vault.to_string(),
                amount: Uint128::new(1_000),
                msg: to_json_binary(&ReceiveMsg::Deposit { bot_id: 1 }).unwrap(),
            },
            &[],
        )
        .unwrap_err();
    assert!(err.root_cause().to_string().contains("pair paused"));

    // 3. Expire an order, then reconcile -> the refund claim hits the paused
    //    pair and the page reverts without crediting anything. A permissionless
    //    relayer (not the reimbursed keeper) reports, so no gas is consumed.
    h.expire(ask.order_id);
    let ids: Vec<u64> = h.orders(1).iter().map(|order| order.order_id).collect();
    let response = h
        .app
        .execute_contract(
            Addr::unchecked("relayer"),
            h.vault.clone(),
            &VaultExecuteMsg::Reconcile {
                bot_id: 1,
                order_ids: ids,
            },
            &[],
        )
        .unwrap();
    assert!(response.events.iter().any(|event| {
        event
            .attributes
            .iter()
            .any(|attribute| attribute.key == "action" && attribute.value == "reverted_grid_page")
    }));
    let bot = h.bot(1);
    assert_eq!(bot.active_orders, 4);
    assert_eq!(bot.free_balances, [Uint128::zero(), Uint128::zero()]);

    // Resume -> reconcile claims the parked refund, then everything works again.
    h.set_paused(false);
    let ids: Vec<u64> = h.orders(1).iter().map(|order| order.order_id).collect();
    h.reconcile(1, ids);
    h.cancel_all(1);
    let bot = h.bot(1);
    assert_eq!(bot.active_orders, 0);
    assert_eq!(bot.free_balances[0], Uint128::new(1_000));
    assert_eq!(bot.free_balances[1], Uint128::new(2_000));
}

#[test]
fn malicious_token_lying_balance_rejected_on_deposit() {
    let mut h = Harness::new_with_malicious(Uint128::new(100));
    h.create_bot();

    let err = h
        .app
        .execute_contract(
            h.alice.clone(),
            h.token_0.clone(),
            &Cw20ExecuteMsg::Send {
                contract: h.vault.to_string(),
                amount: Uint128::new(500),
                msg: to_json_binary(&ReceiveMsg::Deposit { bot_id: 1 }).unwrap(),
            },
            &[],
        )
        .unwrap_err();
    assert!(err
        .root_cause()
        .to_string()
        .contains("CW20 balance delta does not match"));

    // The vault never recorded the deposit.
    let bot = h.bot(1);
    assert_eq!(bot.free_balances[0], Uint128::zero());
    assert_eq!(bot.total_shares, Uint128::zero());
    assert_eq!(bot.active_orders, 0);
}

#[test]
fn unsolicited_transfer_can_be_synchronized_without_minting_shares() {
    let mut h = Harness::new();
    h.create_bot();
    h.app
        .execute_contract(
            h.alice.clone(),
            h.token_0.clone(),
            &Cw20ExecuteMsg::Transfer {
                recipient: h.vault.to_string(),
                amount: Uint128::new(100),
            },
            &[],
        )
        .unwrap();

    h.app
        .execute_contract(
            h.alice.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::SyncBalances { bot_id: 1 },
            &[],
        )
        .unwrap();
    let bot = h.bot(1);
    assert_eq!(bot.free_balances[0], Uint128::new(100));
    assert_eq!(bot.total_shares, Uint128::zero());

    h.deposit(1, &h.token_0.clone(), Uint128::new(500));
    assert!(!h.orders(1).is_empty());
}

#[test]
fn generic_pair_query_error_treated_as_terminal_without_panic() {
    let mut h = Harness::new();
    h.create_bot();
    h.deposit(1, &h.token_0.clone(), Uint128::new(1_000));

    // A tracked order whose LimitOrder query fails with a generic error.
    let orders = h.orders(1);
    let target = orders.first().unwrap();
    h.set_limit_order_error(target.order_id, Some("query reverted".to_string()));

    // Reconcile must not panic: the generic error is treated as a terminal
    // order with no refund (matching the pair-trust design).
    let ids: Vec<u64> = h.orders(1).iter().map(|order| order.order_id).collect();
    h.reconcile(1, ids);

    let bot = h.bot(1);
    assert_eq!(bot.active_orders, 1);
    assert_eq!(h.orders(1).len(), 1);
    assert_eq!(bot.free_balances[0], Uint128::zero());
}

#[test]
fn concurrent_fills_on_one_order_credited_once_on_reconcile() {
    let mut h = Harness::new();
    h.create_bot();
    h.deposit(1, &h.token_0.clone(), Uint128::new(1_000));

    let orders = h.orders(1);
    let target = orders.first().unwrap();
    assert_eq!(target.side, LimitOrderSide::Ask);

    // Three fills against the same order between reconciles, fully consuming it.
    h.fill(target.order_id, Uint128::new(100), Uint128::new(50));
    h.fill(target.order_id, Uint128::new(250), Uint128::new(125));
    h.fill(target.order_id, Uint128::new(150), Uint128::new(75));

    let ids: Vec<u64> = h.orders(1).iter().map(|order| order.order_id).collect();
    h.reconcile(1, ids);

    let bot = h.bot(1);
    assert_eq!(bot.free_balances[1], Uint128::new(250));
    assert_eq!(bot.active_orders, 1);
    let tracked = h.orders(1);
    let remaining = tracked.first().unwrap();
    assert_eq!(remaining.remaining, Uint128::new(500));
    assert_eq!(remaining.side, LimitOrderSide::Ask);
}

#[test]
fn manager_create_vault_then_full_vault_flow() {
    let mut h = Harness::new();

    // Alice creates a vault through the manager.
    h.app
        .execute_contract(
            h.alice.clone(),
            h.manager.clone(),
            &ManagerExecuteMsg::CreateVault { label: None },
            &[],
        )
        .unwrap();

    let vaults: Vec<cl8y_grid_manager::msg::VaultResponse> = h
        .app
        .wrap()
        .query_wasm_smart(
            &h.manager,
            &ManagerQueryMsg::VaultsByOwner {
                owner: h.alice.to_string(),
            },
        )
        .unwrap();
    assert_eq!(vaults.len(), 1);

    let standalone_vault = h.vault.clone();
    h.vault = Addr::unchecked(&vaults[0].address);

    h.create_bot();
    h.deposit(1, &h.token_0.clone(), Uint128::new(1_000));
    h.deposit(1, &h.token_1.clone(), Uint128::new(2_000));

    let bot = h.bot(1);
    assert_eq!(bot.active_orders, 4);

    let orders = h.orders(1);
    let ask = orders
        .iter()
        .find(|order| order.side == LimitOrderSide::Ask)
        .unwrap();
    h.fill(ask.order_id, Uint128::new(400), Uint128::new(200));

    let ids: Vec<u64> = h.orders(1).iter().map(|order| order.order_id).collect();
    h.reconcile(1, ids);

    let bot = h.bot(1);
    assert_eq!(bot.free_balances[1], Uint128::new(200));

    h.cancel_all(1);
    let bot = h.bot(1);
    assert_eq!(bot.active_orders, 0);
    assert_eq!(bot.free_balances[0], Uint128::new(600));
    assert_eq!(bot.free_balances[1], Uint128::new(2_200));

    h.withdraw(1, bot.total_shares);
    assert_eq!(
        h.balance_of(&h.token_0, &h.alice),
        Uint128::new(1_000_000_000 - 100_000_000 - 1_000 + 600)
    );
    assert_eq!(
        h.balance_of(&h.token_1, &h.alice),
        Uint128::new(1_000_000_000 - 100_000_000 - 2_000 + 2_200)
    );

    h.vault = standalone_vault;
}

// ---------------------------------------------------------------------------
// Property test: vault_balance + escrow_on_pair == total (per token)
// ---------------------------------------------------------------------------

/// Deterministic LCG so the random walk is reproducible.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

#[test]
fn property_conservation_holds_across_random_walk() {
    let mut h = Harness::new();
    h.create_bot();

    let mut rng = Lcg(0x9e3779b97f4a7c15);
    let mut deposited = [Uint128::zero(), Uint128::zero()];
    let mut withdrawn = [Uint128::zero(), Uint128::zero()];
    let mut fill_consumed = [Uint128::zero(), Uint128::zero()];
    let mut fill_output = [Uint128::zero(), Uint128::zero()];

    let reconcile_soft = |h: &mut Harness| {
        let bot = h.bot(1);
        if bot.gas_credit < Uint128::new(MIN_GAS_RESERVE + KEEPER_REWARD) {
            let _ = h.app.execute_contract(
                h.alice.clone(),
                h.vault.clone(),
                &VaultExecuteMsg::FundGas { bot_id: 1 },
                &[coin(500, GAS_DENOM)],
            );
        }
        let ids: Vec<u64> = h.orders(1).iter().map(|order| order.order_id).collect();
        if ids.is_empty() {
            return;
        }
        let _ = h.app.execute_contract(
            h.keeper.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::Reconcile {
                bot_id: 1,
                order_ids: ids,
            },
            &[],
        );
    };

    for step in 0..200u64 {
        let choice = rng.next() % 10;
        match choice {
            0..=1 => {
                // Deposit a random amount of a random token.
                reconcile_soft(&mut h);
                let token_index = (rng.next() % 2) as usize;
                let token = if token_index == 0 {
                    h.token_0.clone()
                } else {
                    h.token_1.clone()
                };
                let amount = Uint128::new(1 + (rng.next() as u128) % 400);
                if h.balance_of(&token, &h.alice) < amount {
                    continue;
                }
                let result = h.app.execute_contract(
                    h.alice.clone(),
                    token.clone(),
                    &Cw20ExecuteMsg::Send {
                        contract: h.vault.to_string(),
                        amount,
                        msg: to_json_binary(&ReceiveMsg::Deposit { bot_id: 1 }).unwrap(),
                    },
                    &[],
                );
                if result.is_ok() {
                    deposited[token_index] = deposited[token_index].checked_add(amount).unwrap();
                }
            }
            2 => {
                // Allocate any unallocated free balance.
                reconcile_soft(&mut h);
                let bot = h.bot(1);
                if bot.free_balances[0].is_zero() && bot.free_balances[1].is_zero() {
                    continue;
                }
                let _ = h.app.execute_contract(
                    h.alice.clone(),
                    h.vault.clone(),
                    &VaultExecuteMsg::Allocate { bot_id: 1 },
                    &[],
                );
            }
            3..=5 => {
                // Fill a random active tracked order.
                let orders = h.orders(1);
                if orders.is_empty() {
                    continue;
                }
                let order = &orders[(rng.next() % (orders.len() as u64)) as usize];
                let Some(on_chain) = h.on_chain_order(order.order_id) else {
                    continue;
                };
                if on_chain.remaining.is_zero() {
                    continue;
                }
                let max = on_chain.remaining.u128();
                let fill_amount = Uint128::new(1 + (rng.next() as u128) % max);
                let output_amount = fill_amount.multiply_ratio(1u128, 2u128);
                let (input_index, output_index) = match on_chain.side {
                    LimitOrderSide::Ask => (0, 1),
                    LimitOrderSide::Bid => (1, 0),
                };
                h.fill(order.order_id, fill_amount, output_amount);
                fill_consumed[input_index] =
                    fill_consumed[input_index].checked_add(fill_amount).unwrap();
                fill_output[output_index] = fill_output[output_index]
                    .checked_add(output_amount)
                    .unwrap();
            }
            6 => reconcile_soft(&mut h),
            7 => {
                // Expire a random tracked order still present on the active book.
                let orders = h.orders(1);
                if orders.is_empty() {
                    continue;
                }
                let order = &orders[(rng.next() % (orders.len() as u64)) as usize];
                if h.on_chain_order(order.order_id).is_none() {
                    continue;
                }
                h.expire(order.order_id);
            }
            8 => {
                // Cancel all (reconcile first so recorded == on-chain).
                reconcile_soft(&mut h);
                h.cancel_all(1);
            }
            9 => {
                // Withdraw a partial amount of shares (no active orders allowed).
                let bot = h.bot(1);
                if bot.active_orders != 0 || bot.total_shares.is_zero() {
                    continue;
                }
                let total = bot.total_shares;
                let shares = Uint128::new(1 + (rng.next() as u128) % total.u128());
                // Record the proportional withdrawal exactly (no active orders =>
                // free_balances == vault balances).
                for (index, entry) in withdrawn.iter_mut().enumerate() {
                    let amount = bot.free_balances[index].multiply_ratio(shares, total);
                    *entry = entry.checked_add(amount).unwrap();
                }
                h.withdraw(1, shares);
            }
            _ => {}
        }

        // Assert conservation after every step:
        //   vault_balance(token) + escrow_on_pair(token)
        //       == deposited - withdrawn + fill_output - fill_consumed
        let balance = [
            h.balance_of(&h.token_0, &h.vault),
            h.balance_of(&h.token_1, &h.vault),
        ];
        let escrow = h.on_chain_escrow(1);
        for index in 0..2 {
            let expected = deposited[index]
                .checked_add(fill_output[index])
                .unwrap()
                .checked_sub(fill_consumed[index])
                .unwrap()
                .checked_sub(withdrawn[index])
                .unwrap();
            let actual = balance[index].checked_add(escrow[index]).unwrap();
            assert_eq!(
                actual,
                expected,
                "conservation broken at step {step} for token {index}: \
                 balance[0]={} balance[1]={} escrow[0]={} escrow[1]={} \
                 deposited[0]={} deposited[1]={} withdrawn[0]={} withdrawn[1]={} \
                 fill_output[0]={} fill_output[1]={} fill_consumed[0]={} fill_consumed[1]={}",
                balance[0],
                balance[1],
                escrow[0],
                escrow[1],
                deposited[0],
                deposited[1],
                withdrawn[0],
                withdrawn[1],
                fill_output[0],
                fill_output[1],
                fill_consumed[0],
                fill_consumed[1],
            );
        }
    }
}

#[test]
fn allowlist_and_quarantine_block_usage_until_cleared() {
    let mut h = Harness::new();
    let admin = Addr::unchecked("admin");

    // 1. Admin allowlists only token_0 -> create_bot is rejected because token_1
    //    is not on the list.
    h.app
        .execute_contract(
            admin.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::AddAllowedToken {
                token: h.token_0.to_string(),
            },
            &[],
        )
        .unwrap();
    let err = h
        .app
        .execute_contract(
            h.alice.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::CreateBot {
                pair: h.pair.to_string(),
                lower_price: Decimal::from_atomics(1u128, 0).unwrap(),
                upper_price: Decimal::from_atomics(3u128, 0).unwrap(),
                grid_count: 5,
            },
            &[coin(MIN_GAS_RESERVE + KEEPER_REWARD, GAS_DENOM)],
        )
        .unwrap_err();
    assert!(err
        .root_cause()
        .to_string()
        .contains("not on the admin allowlist"));

    // 2. Allowlist token_1 -> create_bot now succeeds.
    h.app
        .execute_contract(
            admin.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::AddAllowedToken {
                token: h.token_1.to_string(),
            },
            &[],
        )
        .unwrap();
    h.create_bot();

    // 3. Quarantining token_0 blocks deposits of it.
    h.app
        .execute_contract(
            admin.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::QuarantineToken {
                token: h.token_0.to_string(),
            },
            &[],
        )
        .unwrap();
    let err = h
        .app
        .execute_contract(
            h.alice.clone(),
            h.token_0.clone(),
            &Cw20ExecuteMsg::Send {
                contract: h.vault.to_string(),
                amount: Uint128::new(1_000),
                msg: to_json_binary(&ReceiveMsg::Deposit { bot_id: 1 }).unwrap(),
            },
            &[],
        )
        .unwrap_err();
    assert!(err
        .root_cause()
        .to_string()
        .contains("quarantined by the admin"));

    // 4. TokenPolicy query reflects the quarantine.
    let policy: TokenPolicyResponse = h
        .app
        .wrap()
        .query_wasm_smart(&h.vault, &VaultQueryMsg::TokenPolicy {})
        .unwrap();
    assert_eq!(
        policy.allowed_tokens,
        vec![h.token_0.to_string(), h.token_1.to_string()]
    );
    assert_eq!(policy.quarantined_tokens, vec![h.token_0.to_string()]);

    // 5. Unquarantine -> the deposit lands and the pair assets are usable again.
    h.app
        .execute_contract(
            admin,
            h.vault.clone(),
            &VaultExecuteMsg::UnquarantineToken {
                token: h.token_0.to_string(),
            },
            &[],
        )
        .unwrap();
    h.deposit(1, &h.token_0.clone(), Uint128::new(1_000));
    h.deposit(1, &h.token_1.clone(), Uint128::new(2_000));
    assert_eq!(h.orders(1).len(), 4);
    let bot = h.bot(1);
    assert_eq!(bot.active_orders, 4);
}
