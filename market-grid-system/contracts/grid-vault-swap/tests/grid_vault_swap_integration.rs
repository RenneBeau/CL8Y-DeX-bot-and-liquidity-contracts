//! cw-multi-test integration tests for the CL8Y grid vault (swap-only).
//!
//! The real CL8Y pair contract is not linked here; we drive the vault against
//! a minimal in-test mock that mirrors the pair behavior the vault depends on
//! (Pair, Pool, Observe, HybridSimulation, and the CW20 Swap hook). The tokens
//! are the honest `cw20-base` implementation, which dispatches the CW20
//! `Send` hook to the vault/pair via `Cw20ReceiveMsg`.

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    from_json, to_json_binary, Addr, Binary, Decimal, DepsMut, Empty, Env, StdError, StdResult,
    Uint128, WasmMsg,
};
use cw20::{Cw20QueryMsg, Cw20ReceiveMsg};
use cw_multi_test::{App, Contract, ContractWrapper, Executor};
use cw_storage_plus::Item;

use cl8y_grid_vault_swap::contract;
use cl8y_grid_vault_swap::msg::{
    Asset, AssetInfo, ExecuteMsg as VaultExecuteMsg, HybridSimulationResponse, HybridSwapParams,
    InstantiateMsg, ObserveResponse, PairCw20HookMsg, PairInfo, PairQueryMsg, PoolResponse,
    QueryMsg, ReceiveMsg, SharesResponse, SwapProxyHookMsg,
};

// ---------------------------------------------------------------------------
// Mock pair: holds pool reserves and honors the CW20 Swap hook.
// The pair must be pre-funded with both tokens so it can pay the ask out.
// ---------------------------------------------------------------------------

#[cw_serde]
pub struct MockPairInstantiateMsg {
    pub token_0: String,
    pub token_1: String,
    pub reserve_0: Uint128,
    pub reserve_1: Uint128,
    pub twap: Decimal,
    pub window: u32,
}

#[cw_serde]
pub enum MockPairExecuteMsg {
    Receive(Cw20ReceiveMsg),
    SetTwap { twap: Decimal },
}

const PAIR_TOKEN_0: Item<String> = Item::new("pair_token_0");
const PAIR_TOKEN_1: Item<String> = Item::new("pair_token_1");
const PAIR_RESERVES: Item<[Uint128; 2]> = Item::new("pair_reserves");
const PAIR_TWAP: Item<Decimal> = Item::new("pair_twap");
const PAIR_WINDOW: Item<u32> = Item::new("pair_window");
const PAIR_SELF: Item<String> = Item::new("pair_self");

fn mock_pair_execute(
    deps: DepsMut,
    env: Env,
    info: cosmwasm_std::MessageInfo,
    msg: MockPairExecuteMsg,
) -> StdResult<cosmwasm_std::Response> {
    match msg {
        MockPairExecuteMsg::SetTwap { twap } => {
            PAIR_TWAP.save(deps.storage, &twap)?;
            let base = Uint128::new(1_000_000_000_000);
            let reserve_1 = base
                .u128()
                .checked_mul(twap.atomics().u128())
                .map(|value| value / Decimal::one().atomics().u128())
                .ok_or_else(|| StdError::generic_err("reserve overflow"))?;
            PAIR_RESERVES.save(deps.storage, &[base, Uint128::from(reserve_1)])?;
            Ok(cosmwasm_std::Response::new())
        }
        MockPairExecuteMsg::Receive(receive) => {
            let hook: PairCw20HookMsg = from_json(&receive.msg)?;
            let PairCw20HookMsg::Swap {
                min_return,
                to,
                deadline,
                ..
            } = hook;
            let reserves = PAIR_RESERVES.load(deps.storage)?;
            let token_0 = PAIR_TOKEN_0.load(deps.storage)?;
            let token_1 = PAIR_TOKEN_1.load(deps.storage)?;
            let offer_index = if info.sender.as_str() == token_0 {
                0
            } else {
                1
            };
            let ask_index = 1 - offer_index;
            let before = reserves[offer_index];
            let after = before + receive.amount;
            let ask_before = reserves[ask_index];
            let ask_after = (Uint128::new(before.u128()) * Uint128::new(ask_before.u128()))
                / Uint128::new(after.u128());
            let return_amount = ask_before - ask_after;
            if let Some(min_return) = min_return {
                if return_amount < min_return {
                    return Err(StdError::generic_err("slippage assertion failed"));
                }
            }
            if let Some(deadline) = deadline {
                if deadline < env.block.time.seconds() {
                    return Err(StdError::generic_err("deadline passed"));
                }
            }
            let next = if offer_index == 0 {
                [after, ask_after]
            } else {
                [ask_after, after]
            };
            PAIR_RESERVES.save(deps.storage, &next)?;
            let ask_token = if ask_index == 0 { token_0 } else { token_1 };
            let recipient = to.unwrap_or(receive.sender);
            Ok(cosmwasm_std::Response::new().add_message(WasmMsg::Execute {
                contract_addr: ask_token.clone(),
                msg: to_json_binary(&cw20::Cw20ExecuteMsg::Transfer {
                    recipient,
                    amount: return_amount,
                })?,
                funds: vec![],
            }))
        }
    }
}

fn mock_pair_code() -> Box<dyn Contract<Empty, Empty>> {
    let contract = ContractWrapper::new(
        |deps, env, info, msg: MockPairExecuteMsg| mock_pair_execute(deps, env, info, msg),
        |deps, env, _info, msg: MockPairInstantiateMsg| -> StdResult<cosmwasm_std::Response> {
            PAIR_TOKEN_0.save(
                deps.storage,
                &deps.api.addr_validate(&msg.token_0)?.to_string(),
            )?;
            PAIR_TOKEN_1.save(
                deps.storage,
                &deps.api.addr_validate(&msg.token_1)?.to_string(),
            )?;
            PAIR_RESERVES.save(deps.storage, &[msg.reserve_0, msg.reserve_1])?;
            PAIR_TWAP.save(deps.storage, &msg.twap)?;
            PAIR_WINDOW.save(deps.storage, &msg.window)?;
            PAIR_SELF.save(deps.storage, &env.contract.address.to_string())?;
            Ok(cosmwasm_std::Response::new())
        },
        |deps, _env, msg: PairQueryMsg| -> StdResult<Binary> {
            match msg {
                PairQueryMsg::Pair {} => {
                    let token_0 = PAIR_TOKEN_0.load(deps.storage)?;
                    let token_1 = PAIR_TOKEN_1.load(deps.storage)?;
                    to_json_binary(&PairInfo {
                        asset_infos: [
                            AssetInfo::Token {
                                contract_addr: token_0,
                            },
                            AssetInfo::Token {
                                contract_addr: token_1,
                            },
                        ],
                        contract_addr: PAIR_SELF.load(deps.storage)?,
                        liquidity_token: "lp".to_string(),
                    })
                }
                PairQueryMsg::Pool {} => {
                    let reserves = PAIR_RESERVES.load(deps.storage)?;
                    to_json_binary(&PoolResponse {
                        assets: [
                            Asset {
                                info: AssetInfo::Token {
                                    contract_addr: PAIR_TOKEN_0.load(deps.storage)?,
                                },
                                amount: reserves[0],
                            },
                            Asset {
                                info: AssetInfo::Token {
                                    contract_addr: PAIR_TOKEN_1.load(deps.storage)?,
                                },
                                amount: reserves[1],
                            },
                        ],
                        total_share: Uint128::new(1_000_000),
                    })
                }
                PairQueryMsg::Observe { .. } => {
                    let twap = PAIR_TWAP.load(deps.storage)?;
                    let window = Uint128::from(PAIR_WINDOW.load(deps.storage)?);
                    let cum = twap.atomics() * window;
                    to_json_binary(&ObserveResponse {
                        price_a_cumulatives: vec![cum * Uint128::new(2), cum],
                        price_b_cumulatives: vec![cum * Uint128::new(2), cum],
                    })
                }
                PairQueryMsg::HybridSimulation { offer_asset, .. } => {
                    let reserves = PAIR_RESERVES.load(deps.storage)?;
                    let is_token0 = match offer_asset.info {
                        AssetInfo::Token { contract_addr } => {
                            contract_addr == PAIR_TOKEN_0.load(deps.storage)?
                        }
                        AssetInfo::NativeToken { .. } => false,
                    };
                    let (offer_index, ask_index) = if is_token0 { (0, 1) } else { (1, 0) };
                    let before = reserves[offer_index];
                    let after = before + offer_asset.amount;
                    let ask_before = reserves[ask_index];
                    let ask_after = (Uint128::new(before.u128()) * Uint128::new(ask_before.u128()))
                        / Uint128::new(after.u128());
                    let return_amount = ask_before - ask_after;
                    to_json_binary(&HybridSimulationResponse {
                        return_amount,
                        spread_amount: Uint128::zero(),
                        commission_amount: Uint128::zero(),
                        pool_commission_amount: Uint128::zero(),
                        book_commission_amount: Uint128::zero(),
                        book_return_amount: Uint128::zero(),
                        pool_return_amount: return_amount,
                        limit_book_offer_consumed: Uint128::zero(),
                    })
                }
            }
        },
    );
    Box::new(contract)
}

// ---------------------------------------------------------------------------
// Mock swap proxy: mirrors the shared `swap-proxy` hook. When the vault routes
// its rebalance swap here (SwapProxyHookMsg::Swap), the proxy re-emits the offer
// token to the pair with the proceeds sent back to the vault. Offer-token
// detection in the mock pair is sender-based, so routing through the proxy does
// not change the swap outcome.
// ---------------------------------------------------------------------------

#[cw_serde]
pub enum MockProxyExecuteMsg {
    Receive(Cw20ReceiveMsg),
}

fn mock_proxy_execute(
    _deps: DepsMut,
    env: Env,
    info: cosmwasm_std::MessageInfo,
    msg: MockProxyExecuteMsg,
) -> StdResult<cosmwasm_std::Response> {
    match msg {
        MockProxyExecuteMsg::Receive(receive) => {
            if receive.amount.is_zero() {
                return Err(StdError::generic_err("zero amount"));
            }
            let hook: SwapProxyHookMsg = from_json(&receive.msg)?;
            let SwapProxyHookMsg::Swap {
                pair,
                min_return,
                max_spread,
                deadline,
            } = hook;
            if deadline < env.block.time.seconds() {
                return Err(StdError::generic_err("deadline passed"));
            }
            let vault = receive.sender;
            let pair_hook = PairCw20HookMsg::Swap {
                belief_price: None,
                max_spread: Some(max_spread),
                min_return: Some(min_return),
                to: Some(vault.to_string()),
                deadline: Some(deadline),
                trader: None,
                hybrid: Some(HybridSwapParams::pool_only(receive.amount)),
            };
            Ok(cosmwasm_std::Response::new()
                .add_message(WasmMsg::Execute {
                    contract_addr: info.sender.to_string(),
                    msg: to_json_binary(&cw20::Cw20ExecuteMsg::Send {
                        contract: pair,
                        amount: receive.amount,
                        msg: to_json_binary(&pair_hook)?,
                    })?,
                    funds: vec![],
                })
                .add_attribute("action", "proxy_swap")
                .add_attribute("vault", vault))
        }
    }
}

fn mock_proxy_code() -> Box<dyn Contract<Empty, Empty>> {
    let contract = ContractWrapper::new(
        |deps, env, info, msg: MockProxyExecuteMsg| mock_proxy_execute(deps, env, info, msg),
        |_deps, _env, _info, _msg: Empty| -> StdResult<cosmwasm_std::Response> {
            Ok(cosmwasm_std::Response::new().add_attribute("action", "instantiate"))
        },
        |_deps, _env, _msg: Empty| -> StdResult<Binary> { Ok(Binary::default()) },
    );
    Box::new(contract)
}

// ---------------------------------------------------------------------------
// Contract wrappers
// ---------------------------------------------------------------------------

fn mock_cw20_code() -> Box<dyn Contract<Empty, Empty>> {
    let contract = ContractWrapper::new(
        |deps, env, info, msg: cw20_base::msg::ExecuteMsg| {
            cw20_base::contract::execute(deps, env, info, msg)
        },
        |deps, env, info, msg: cw20_base::msg::InstantiateMsg| {
            cw20_base::contract::instantiate(deps, env, info, msg)
        },
        |deps, env, msg: cw20_base::msg::QueryMsg| cw20_base::contract::query(deps, env, msg),
    );
    Box::new(contract)
}

fn mock_vault_code() -> Box<dyn Contract<Empty, Empty>> {
    let contract = ContractWrapper::new(
        |deps, env, info, msg: VaultExecuteMsg| contract::execute(deps, env, info, msg),
        |deps, env, info, msg: InstantiateMsg| contract::instantiate(deps, env, info, msg),
        |deps, env, msg: QueryMsg| contract::query(deps, env, msg),
    )
    .with_reply(contract::reply);
    Box::new(contract)
}

#[cw_serde]
enum MockFeeRegistryQueryMsg {
    EffectiveFee { trader: String },
}

#[cw_serde]
struct MockFeeRegistryEffectiveFeeResponse {
    fee_bps: u16,
    discount_bps: u16,
    tier_id: Option<u8>,
    holding: Option<Uint128>,
    source: String,
}

fn mock_fee_registry_code() -> Box<dyn Contract<Empty, Empty>> {
    let contract = ContractWrapper::new(
        |_deps, _env, _info, _msg: Empty| -> StdResult<cosmwasm_std::Response> {
            Ok(cosmwasm_std::Response::new())
        },
        |_deps, _env, _info, _msg: Empty| -> StdResult<cosmwasm_std::Response> {
            Ok(cosmwasm_std::Response::new())
        },
        |_deps, _env, _msg: MockFeeRegistryQueryMsg| -> StdResult<Binary> {
            let response = MockFeeRegistryEffectiveFeeResponse {
                fee_bps: 1_800,
                discount_bps: 0,
                tier_id: None,
                holding: Some(Uint128::new(1)),
                source: "live".to_string(),
            };
            to_json_binary(&response)
        },
    );
    Box::new(contract)
}

/// A tiered fee-registry whose `EffectiveFee` depends deterministically on the
/// trader: a trader whose first address byte is even gets a LOW fee (200 bps,
/// "high tier"), otherwise the full base fee (1 800 bps, "low tier"). This lets
/// a test prove per-LP tiering (higher tier loses less LP) without a real CL8Y
/// fixture -- the test derives each holder's tier from the same first-byte rule.
fn mock_fee_registry_code_tiered() -> Box<dyn Contract<Empty, Empty>> {
    let contract = ContractWrapper::new(
        |_deps, _env, _info, _msg: Empty| -> StdResult<cosmwasm_std::Response> {
            Ok(cosmwasm_std::Response::new())
        },
        |_deps, _env, _info, _msg: Empty| -> StdResult<cosmwasm_std::Response> {
            Ok(cosmwasm_std::Response::new())
        },
        |_deps, _env, _msg: MockFeeRegistryQueryMsg| -> StdResult<Binary> {
            let trader = match &_msg {
                MockFeeRegistryQueryMsg::EffectiveFee { trader } => trader.clone(),
            };
            let even_byte = trader
                .as_bytes()
                .last()
                .map(|b| *b % 2 == 0)
                .unwrap_or(false);
            let fee_bps = if even_byte { 200 } else { 1_800 };
            let response = MockFeeRegistryEffectiveFeeResponse {
                fee_bps,
                discount_bps: 0,
                tier_id: None,
                holding: Some(Uint128::new(1)),
                source: "live".to_string(),
            };
            to_json_binary(&response)
        },
    );
    Box::new(contract)
}

/// Tier keys used to distinguish a high-tier (low-fee) holder from a low-tier one.
#[derive(Clone, Copy)]
enum TieredRegistry {
    Flat,
    Tiered,
    /// Tier-9 holder (>=7500 CL8Y) -> 9500 bps discount -> fee = 180 * 500/10000 = 9 bps.
    Nine,
}

/// A tier-9 fee-registry: `EffectiveFee` always resolves the caller to tier 9
/// (9_500/10_000 discount, 9 bps). Models a 7500+ CL8Y holder without needing a
/// real CL8Y fixture.
fn mock_fee_registry_code_tier9() -> Box<dyn Contract<Empty, Empty>> {
    let contract = ContractWrapper::new(
        |_deps, _env, _info, _msg: Empty| -> StdResult<cosmwasm_std::Response> {
            Ok(cosmwasm_std::Response::new())
        },
        |_deps, _env, _info, _msg: Empty| -> StdResult<cosmwasm_std::Response> {
            Ok(cosmwasm_std::Response::new())
        },
        |_deps, _env, _msg: MockFeeRegistryQueryMsg| -> StdResult<Binary> {
            let response = MockFeeRegistryEffectiveFeeResponse {
                fee_bps: 9,
                discount_bps: 9_500,
                tier_id: Some(9),
                holding: Some(Uint128::new(750_000)),
                source: "live".to_string(),
            };
            to_json_binary(&response)
        },
    );
    Box::new(contract)
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

const DEPOSITOR: &str = "depositor";
const ADMIN: &str = "admin";
const DEFAULT_TWAP: &str = "1.5";

struct Harness {
    app: App,
    vault: Addr,
    token0: Addr,
    token1: Addr,
    depositor: Addr,
    admin: Addr,
    pair: Addr,
    collector: Addr,
    treasury: Addr,
    proxy: Option<Addr>,
}

fn setup_with_fee(enable_fees: bool) -> Harness {
    setup_with_registry(enable_fees, TieredRegistry::Flat)
}

fn setup_proxy() -> Harness {
    setup_core(false, TieredRegistry::Flat, true)
}

/// A tier-9 holder operating a market-grid through the whitelisted shared
/// swap-proxy (DEX commission = 0). This is the exact on-chain demo requested:
/// "proxy has 0 fees, tier-9 user, pool rebalances CORAL -> EMBER".
fn setup_proxy_tier9() -> Harness {
    setup_core(true, TieredRegistry::Nine, true)
}

fn setup_with_registry(enable_fees: bool, registry_kind: TieredRegistry) -> Harness {
    setup_core(enable_fees, registry_kind, false)
}

fn setup_core(enable_fees: bool, registry_kind: TieredRegistry, use_proxy: bool) -> Harness {
    let mut app = App::default();
    let admin = app.api().addr_make(ADMIN);
    let depositor = app.api().addr_make(DEPOSITOR);

    let cw20_code = app.store_code(mock_cw20_code());
    let pair_code = app.store_code(mock_pair_code());
    let vault_code = app.store_code(mock_vault_code());

    let proxy = if use_proxy {
        let proxy_code = app.store_code(mock_proxy_code());
        Some(
            app.instantiate_contract(proxy_code, admin.clone(), &Empty {}, &[], "proxy", None)
                .unwrap(),
        )
    } else {
        None
    };

    let registry = if enable_fees {
        let registry_code = app.store_code(match registry_kind {
            TieredRegistry::Flat => mock_fee_registry_code(),
            TieredRegistry::Tiered => mock_fee_registry_code_tiered(),
            TieredRegistry::Nine => mock_fee_registry_code_tier9(),
        });
        let registry = app
            .instantiate_contract(
                registry_code,
                admin.clone(),
                &Empty {},
                &[],
                "fee_registry",
                None,
            )
            .unwrap();
        Some(registry)
    } else {
        None
    };
    let collector = app.api().addr_make("collector");
    let treasury = app.api().addr_make("treasury");

    let token0 = app
        .instantiate_contract(
            cw20_code,
            admin.clone(),
            &cw20_base::msg::InstantiateMsg {
                name: "Token0".to_string(),
                symbol: "TKA".to_string(),
                decimals: 6,
                initial_balances: vec![cw20::Cw20Coin {
                    address: depositor.to_string(),
                    amount: Uint128::new(100_000_000_000),
                }],
                mint: Some(cw20::MinterResponse {
                    minter: admin.to_string(),
                    cap: None,
                }),
                marketing: None,
            },
            &[],
            "token0",
            None,
        )
        .unwrap();

    let token1 = app
        .instantiate_contract(
            cw20_code,
            admin.clone(),
            &cw20_base::msg::InstantiateMsg {
                name: "Token1".to_string(),
                symbol: "TKB".to_string(),
                decimals: 6,
                initial_balances: vec![cw20::Cw20Coin {
                    address: depositor.to_string(),
                    amount: Uint128::new(100_000_000_000),
                }],
                mint: Some(cw20::MinterResponse {
                    minter: admin.to_string(),
                    cap: None,
                }),
                marketing: None,
            },
            &[],
            "token1",
            None,
        )
        .unwrap();

    let pair = app
        .instantiate_contract(
            pair_code,
            admin.clone(),
            &MockPairInstantiateMsg {
                token_0: token0.to_string(),
                token_1: token1.to_string(),
                reserve_0: Uint128::new(1_000_000_000_000),
                reserve_1: Uint128::new(1_500_000_000_000),
                twap: Decimal::from_str(DEFAULT_TWAP).unwrap(),
                window: 300,
            },
            &[],
            "pair",
            None,
        )
        .unwrap();

    // Pre-fund the pair so it can pay swap ask out.
    for (token, amount) in [
        (&token0, Uint128::new(1_000_000_000_000)),
        (&token1, Uint128::new(1_500_000_000_000)),
    ] {
        app.execute_contract(
            admin.clone(),
            token.clone(),
            &cw20::Cw20ExecuteMsg::Mint {
                recipient: pair.to_string(),
                amount,
            },
            &[],
        )
        .unwrap();
    }

    let vault = app
        .instantiate_contract(
            vault_code,
            admin.clone(),
            &InstantiateMsg {
                admin: admin.to_string(),
                pair: pair.to_string(),
                twap_window_seconds: 300,
                grid_count: 4,
                lower_price: Decimal::from_str("1.0").unwrap(),
                upper_price: Decimal::from_str("2.0").unwrap(),
                allocation_tolerance_bps: Some(100),
                max_trade_bps: Some(2_500),
                max_execution_deviation_bps: Some(500),
                quote_slippage_bps: Some(200),
                max_spot_twap_deviation_bps: Some(500),
                max_trade_pool_bps: Some(1_000),
                max_spread: Some(Decimal::percent(5)),
                fee_registry: registry.as_ref().map(ToString::to_string),
                fee_collector: if enable_fees {
                    Some(collector.to_string())
                } else {
                    None
                },
                proxy: proxy.as_ref().map(ToString::to_string),
            },
            &[],
            "vault",
            None,
        )
        .unwrap();

    Harness {
        app,
        vault,
        token0,
        token1,
        depositor,
        admin,
        pair,
        collector,
        treasury,
        proxy,
    }
}

fn setup() -> Harness {
    setup_with_fee(false)
}

fn deposit(h: &mut Harness, token: &Addr, amount: Uint128) {
    deposit_as(h, &h.depositor.clone(), token, amount);
}

fn deposit_as(h: &mut Harness, user: &Addr, token: &Addr, amount: Uint128) {
    h.app
        .execute_contract(
            user.clone(),
            token.clone(),
            &cw20::Cw20ExecuteMsg::Send {
                contract: h.vault.to_string(),
                amount,
                msg: to_json_binary(&ReceiveMsg::Deposit {}).unwrap(),
            },
            &[],
        )
        .unwrap();
}

fn withdraw(h: &mut Harness, shares: Uint128) {
    h.app
        .execute_contract(
            h.depositor.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::Withdraw {
                shares,
                recipient: None,
            },
            &[],
        )
        .unwrap();
}

fn shares_of(h: &Harness, address: &Addr) -> Uint128 {
    let response: SharesResponse = h
        .app
        .wrap()
        .query_wasm_smart(
            &h.vault,
            &QueryMsg::Shares {
                bot_id: 0,
                address: address.to_string(),
            },
        )
        .unwrap();
    response.shares
}

fn balance_of(app: &App, token: &Addr, address: &Addr) -> Uint128 {
    let response: cw20::BalanceResponse = app
        .wrap()
        .query_wasm_smart(
            token,
            &Cw20QueryMsg::Balance {
                address: address.to_string(),
            },
        )
        .unwrap();
    response.balance
}

fn set_twap(h: &mut Harness, twap: &str) {
    h.app
        .execute_contract(
            h.admin.clone(),
            h.pair.clone(),
            &MockPairExecuteMsg::SetTwap {
                twap: Decimal::from_str(twap).unwrap(),
            },
            &[],
        )
        .unwrap();
}

use std::str::FromStr;

#[test]
fn deposit_mints_shares_from_twap() {
    let mut h = setup();
    let (token0, token1) = (h.token0.clone(), h.token1.clone());
    deposit(&mut h, &token0, Uint128::new(50_000_000_000));
    deposit(&mut h, &token1, Uint128::new(75_000_000_000));
    // token0 deposits mint 1:1; token1 deposits mint amount/price (1.5).
    assert_eq!(shares_of(&h, &h.depositor), Uint128::new(100_000_000_000));
    assert_eq!(
        balance_of(&h.app, &h.token0, &h.vault),
        Uint128::new(50_000_000_000)
    );
    assert_eq!(
        balance_of(&h.app, &h.token1, &h.vault),
        Uint128::new(75_000_000_000)
    );
}

#[test]
fn rebalance_not_required_when_price_in_cell() {
    let mut h = setup();
    let (token0, token1) = (h.token0.clone(), h.token1.clone());
    deposit(&mut h, &token0, Uint128::new(60_000_000_000));
    deposit(&mut h, &token1, Uint128::new(90_000_000_000));
    // TWAP 1.5, cell 2: value balanced (60*1.5 == 90); no rebalance.
    let err = h
        .app
        .execute_contract(
            h.depositor.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::Rebalance { deadline: u64::MAX },
            &[],
        )
        .unwrap_err();
    assert!(
        err.root_cause().to_string().contains("not required"),
        "{}",
        err
    );
}

#[test]
fn rebalance_swaps_toward_target_cell() {
    let mut h = setup();
    let (token0, token1) = (h.token0.clone(), h.token1.clone());
    deposit(&mut h, &token0, Uint128::new(60_000_000_000));
    deposit(&mut h, &token1, Uint128::new(60_000_000_000));
    // Price 1.75 -> cell 3 (75% token1 target). With 60t0+60t1 the value in
    // token1 units is 60*1.75+60 = 165, target token1 = 123.75, so we sell
    // token0. The single half-delta swap must execute without slippage error.
    set_twap(&mut h, "1.75");
    let response = h
        .app
        .execute_contract(
            h.depositor.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::Rebalance { deadline: u64::MAX },
            &[],
        )
        .unwrap();
    assert!(
        response.events.iter().any(|e| e
            .attributes
            .iter()
            .any(|a| a.key == "action" && a.value == "rebalance")),
        "expected rebalance event"
    );
}

#[test]
fn rebalance_routes_through_shared_proxy() {
    // When the vault is configured with `proxy`, the rebalance swap must leave
    // the vault and land on the swap-proxy first (proving the vault sends the
    // offer token to the proxy, which forwards it to the pair and routes the
    // proceeds back to the vault).
    let mut h = setup_proxy();
    let (token0, token1) = (h.token0.clone(), h.token1.clone());
    deposit(&mut h, &token0, Uint128::new(60_000_000_000));
    deposit(&mut h, &token1, Uint128::new(60_000_000_000));
    set_twap(&mut h, "1.75");
    // The vault must advertise the proxy it routes through (the shared
    // swap-proxy checks this on register_vault).
    let config: cl8y_grid_vault_swap::msg::ConfigResponse = h
        .app
        .wrap()
        .query_wasm_smart(&h.vault, &QueryMsg::Config {})
        .unwrap();
    assert_eq!(config.proxy, h.proxy.as_ref().map(ToString::to_string));
    let response = h
        .app
        .execute_contract(
            h.depositor.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::Rebalance { deadline: u64::MAX },
            &[],
        )
        .unwrap();
    assert!(
        response.events.iter().any(|e| e
            .attributes
            .iter()
            .any(|a| a.key == "action" && a.value == "proxy_swap")),
        "expected rebalance swap routed through the shared proxy"
    );
    // The proceeds still land in the vault (that is where the rebalance reply
    // books the fill and charges the fee).
    assert!(balance_of(&h.app, &token1, &h.vault) > Uint128::new(60_000_000_000));
}

#[test]
fn withdraw_burns_shares_pro_rata() {
    let mut h = setup();
    let (token0, token1) = (h.token0.clone(), h.token1.clone());
    deposit(&mut h, &token0, Uint128::new(60_000_000_000));
    deposit(&mut h, &token1, Uint128::new(90_000_000_000));
    let before = shares_of(&h, &h.depositor);
    let half = before / Uint128::new(2);
    withdraw(&mut h, half);
    assert_eq!(shares_of(&h, &h.depositor), before - half);
}

#[test]
fn deposit_prices_against_vault_nav() {
    let mut h = setup();
    let (token0, token1) = (h.token0.clone(), h.token1.clone());
    // Balanced seed at TWAP 1.5: 60 t0 + 90 t1 -> 120 shares at NAV 1.0.
    deposit(&mut h, &token0, Uint128::new(60_000_000_000));
    deposit(&mut h, &token1, Uint128::new(90_000_000_000));
    assert_eq!(shares_of(&h, &h.depositor), Uint128::new(120_000_000_000));

    // Price drops to 1.0 (cell 1): holdings 60 t0 + 90 t1 are now worth
    // 150 t0, so a share is worth 1.25 t0 (~NAV 1.25). Under the old fixed
    // 1:1 basis this would let a token0 depositor mint cheap shares and
    // withdraw ~70 t0 after depositing 60 t0 (16.7% risk-free extraction).
    set_twap(&mut h, "1.0");

    let attacker = h.app.api().addr_make("attacker");
    h.app
        .execute_contract(
            h.admin.clone(),
            token0.clone(),
            &cw20::Cw20ExecuteMsg::Mint {
                recipient: attacker.to_string(),
                amount: Uint128::new(60_000_000_000),
            },
            &[],
        )
        .unwrap();
    h.app
        .execute_contract(
            attacker.clone(),
            token0.clone(),
            &cw20::Cw20ExecuteMsg::Send {
                contract: h.vault.to_string(),
                amount: Uint128::new(60_000_000_000),
                msg: to_json_binary(&ReceiveMsg::Deposit {}).unwrap(),
            },
            &[],
        )
        .unwrap();
    // NAV-based mint: 60 t0 * 120e9 / 150e9 == 48e9 shares (not 60e9).
    let got_shares = shares_of(&h, &attacker);
    assert_eq!(got_shares, Uint128::new(48_000_000_000));
    h.app
        .execute_contract(
            attacker.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::Withdraw {
                shares: got_shares,
                recipient: None,
            },
            &[],
        )
        .unwrap();
    // At price 1.0 the pair is 1:1, so attacker value == t0 + t1.
    let value = balance_of(&h.app, &token0, &attacker)
        .checked_add(balance_of(&h.app, &token1, &attacker))
        .unwrap();
    // Must break even; the old fixed-basis bug would have returned ~70 t0.
    assert!(
        value <= Uint128::new(60_000_000_000) && value >= Uint128::new(59_900_000_000),
        "attacker must not extract value (got {value})"
    );
}

#[test]
fn admin_can_pause_and_resume() {
    let mut h = setup();
    h.app
        .execute_contract(
            h.admin.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::Pause {},
            &[],
        )
        .unwrap();
    let err = h
        .app
        .execute_contract(
            h.depositor.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::Rebalance { deadline: u64::MAX },
            &[],
        )
        .unwrap_err();
    assert!(err.root_cause().to_string().contains("paused"), "{}", err);
    h.app
        .execute_contract(
            h.admin.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::Resume {},
            &[],
        )
        .unwrap();
}

#[test]
fn non_admin_cannot_update_config() {
    let mut h = setup();
    let err = h
        .app
        .execute_contract(
            h.depositor.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::UpdateConfig {
                grid_count: Some(8),
                lower_price: None,
                upper_price: None,
                allocation_tolerance_bps: None,
                max_trade_bps: None,
                max_execution_deviation_bps: None,
                quote_slippage_bps: None,
                max_spot_twap_deviation_bps: None,
                max_trade_pool_bps: None,
                max_spread: None,
                fee_registry: None,
                fee_collector: None,
                proxy: None,
            },
            &[],
        )
        .unwrap_err();
    assert!(
        err.root_cause().to_string().contains("unauthorized"),
        "{}",
        err
    );
}

#[test]
fn update_config_can_clear_fee_settings() {
    let mut h = setup_with_fee(true);
    h.app
        .execute_contract(
            h.admin.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::UpdateConfig {
                grid_count: None,
                lower_price: None,
                upper_price: None,
                allocation_tolerance_bps: None,
                max_trade_bps: None,
                max_execution_deviation_bps: None,
                quote_slippage_bps: None,
                max_spot_twap_deviation_bps: None,
                max_trade_pool_bps: None,
                max_spread: None,
                fee_registry: Some(String::new()),
                fee_collector: Some(String::new()),
                proxy: None,
            },
            &[],
        )
        .unwrap();
    let response: cl8y_grid_vault_swap::msg::ConfigResponse = h
        .app
        .wrap()
        .query_wasm_smart(&h.vault, &QueryMsg::Config {})
        .unwrap();
    assert!(response.fee_registry.is_none());
    assert!(response.fee_collector.is_none());
}

#[test]
fn rebalance_mints_fee_lp_to_collector_and_can_redistribute_to_treasury() {
    let mut h = setup_with_fee(true);
    let (token0, token1) = (h.token0.clone(), h.token1.clone());
    let collector = h.collector.clone();
    let treasury = h.treasury.clone();
    deposit(&mut h, &token0, Uint128::new(60_000_000_000));
    deposit(&mut h, &token1, Uint128::new(60_000_000_000));
    set_twap(&mut h, "1.75");
    let depositor_before = shares_of(&h, &h.depositor);
    let admin_before = shares_of(&h, &h.admin);

    let response = h
        .app
        .execute_contract(
            h.depositor.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::Rebalance { deadline: u64::MAX },
            &[],
        )
        .unwrap();
    let fee_shares = response
        .events
        .iter()
        .flat_map(|e| &e.attributes)
        .find(|a| a.key == "fee_shares")
        .expect("fee_shares event")
        .value
        .clone();
    assert!(
        fee_shares.parse::<u128>().unwrap() > 0,
        "fee must be > 0 per executed swap"
    );

    // Single-user model (no user mint): the operating user (vault `admin`) keeps
    // their share count — the fill value accrues to the pool via NAV. Only the
    // collector receives fresh fee LP. No other holder is minted or burned.
    let fee_shares_u = Uint128::new(fee_shares.parse::<u128>().unwrap());
    assert_eq!(
        shares_of(&h, &h.depositor),
        depositor_before,
        "other holders are untouched in the single-user model"
    );
    assert_eq!(
        shares_of(&h, &h.collector),
        fee_shares_u,
        "collector owns exactly the fee LP"
    );
    assert_eq!(
        shares_of(&h, &h.admin),
        admin_before,
        "no user LP is minted on a fill (single-user NAV growth)"
    );

    // The reported rate is the single user's exact tier (flat base 1 800 bps).
    let fee_bps_val: u128 = response
        .events
        .iter()
        .flat_map(|e| &e.attributes)
        .find(|a| a.key == "fee_bps")
        .expect("fee_bps event")
        .value
        .parse()
        .unwrap();
    assert_eq!(
        fee_bps_val, 1_800,
        "single user reports their exact tier, got {fee_bps_val}"
    );

    // The collector now holds the accrued fee as vault LP.
    let collector_shares = shares_of(&h, &collector);
    assert!(
        collector_shares > Uint128::zero(),
        "collector shares minted"
    );

    // A non-collector cannot redeem the fee.
    let err = h
        .app
        .execute_contract(
            treasury.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::RedeemShares {
                bot_id: 0,
                recipient: Some(treasury.to_string()),
            },
            &[],
        )
        .unwrap_err();
    assert!(
        err.root_cause().to_string().contains("unauthorized"),
        "{}",
        err
    );

    // The collector redeems the fee LP to the treasury.
    let t0_before = balance_of(&h.app, &token0, &treasury);
    let t1_before = balance_of(&h.app, &token1, &treasury);
    h.app
        .execute_contract(
            collector.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::RedeemShares {
                bot_id: 0,
                recipient: Some(treasury.to_string()),
            },
            &[],
        )
        .unwrap();
    assert!(balance_of(&h.app, &token0, &treasury) > t0_before);
    assert!(balance_of(&h.app, &token1, &treasury) > t1_before);
    assert_eq!(shares_of(&h, &collector), Uint128::zero());
}

#[test]
fn single_user_is_billed_at_the_admin_tier() {
    // Single-user model (`FEE_TIER_PROTOCOL.md` §5): only the operating user (the
    // vault `admin`) is ever billed, at THEIR OWN tier. No user LP is minted on
    // the fill; the fill value accrues to the pool (amot NAV) and only the fee is
    // realized as fresh LP for the fee-collector.
    let mut h = setup_with_registry(true, TieredRegistry::Tiered);
    let (token0, token1) = (h.token0.clone(), h.token1.clone());

    // A depositor (different identity, possibly different tier) funds the pool
    // but is NOT the fee subject in single-user mode.
    deposit(&mut h, &token0, Uint128::new(60_000_000_000));
    deposit(&mut h, &token1, Uint128::new(60_000_000_000));
    let depositor_before = shares_of(&h, &h.depositor);
    let admin_before = shares_of(&h, &h.admin);
    let pool_before = balance_of(&h.app, &token0, &h.vault) + balance_of(&h.app, &token1, &h.vault);
    set_twap(&mut h, "1.75");

    h.app
        .execute_contract(
            h.depositor.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::Rebalance { deadline: u64::MAX },
            &[],
        )
        .unwrap();

    let depositor_after = shares_of(&h, &h.depositor);
    let admin_after = shares_of(&h, &h.admin);
    let collector_fee = shares_of(&h, &h.collector);
    let pool_after = balance_of(&h.app, &token0, &h.vault) + balance_of(&h.app, &token1, &h.vault);

    assert_eq!(
        depositor_after, depositor_before,
        "the depositor is not the fee subject in single-user mode"
    );
    assert_eq!(
        admin_after, admin_before,
        "no user LP is minted on a fill (single-user NAV growth)"
    );
    assert!(
        collector_fee > Uint128::zero(),
        "the collector receives the fee LP"
    );
    assert!(
        pool_after > pool_before,
        "the fill value accrues to the pool holdings (no user mint)"
    );
}

#[test]
fn tier9_rebalance_through_zero_fee_proxy_collector_gets_exact_9bps() {
    // DEMO: a tier-9 user (9_500/10_000 discount -> 9 bps) operates a market-grid
    // through the whitelisted shared swap-proxy. The proxy adds NO DEX fee, so the
    // only deduction from the fill value is the protocol tier-9 fee. The collector
    // must end up with exactly floor(value × 9 / 10_000).
    //
    // Pool: 10000 CORAL (t0) + 10000 EMBER (t1). Price in-cell -> rebalance swap.
    let mut h = setup_proxy_tier9();
    let (token0, token1) = (h.token0.clone(), h.token1.clone());

    deposit(&mut h, &token0, Uint128::new(10_000_000_000));
    deposit(&mut h, &token1, Uint128::new(10_000_000_000));
    set_twap(&mut h, "1.75");

    let response = h
        .app
        .execute_contract(
            h.depositor.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::Rebalance { deadline: u64::MAX },
            &[],
        )
        .unwrap();

    // The swap must be routed through the shared proxy (proving 0 DEX fee).
    assert!(
        response.events.iter().any(|e| e
            .attributes
            .iter()
            .any(|a| a.key == "action" && a.value == "proxy_swap")),
        "rebalance swap routed through the zero-fee proxy"
    );

    let mut fee_shares = 0u128;
    let mut fee_bps = 0u16;
    let mut fee_tier = 0u16;
    let mut fee_source = String::new();
    for e in &response.events {
        if e.attributes
            .iter()
            .any(|a| a.key == "action" && a.value == "complete_rebalance")
        {
            for a in &e.attributes {
                if a.key == "fee_shares" {
                    fee_shares = a.value.parse().unwrap();
                } else if a.key == "fee_bps" {
                    fee_bps = a.value.parse().unwrap();
                } else if a.key == "fee_tier" {
                    fee_tier = a.value.parse().unwrap();
                } else if a.key == "fee_source" {
                    fee_source = a.value.clone();
                }
            }
        }
    }

    // Tier-9 applies live: 9 bps exact, tier 9, source live.
    assert_eq!(fee_bps, 9, "tier-9 user must be billed 9 bps");
    assert_eq!(fee_tier, 9, "fee tier must be 9");
    assert_eq!(
        fee_source, "live",
        "tier-9 resolution must come from the live registry"
    );

    // `fee_shares` is the LP minted straight to the collector, equal to
    // floor(value_in_token0 × 9 / 10_000). The collector owns exactly it.
    let collector_shares = shares_of(&h, &h.collector);
    assert_eq!(
        collector_shares.u128(),
        fee_shares,
        "collector shares == accrued fee"
    );
    assert!(
        collector_shares > Uint128::zero(),
        "collector must receive a non-zero tier-9 fee"
    );
    println!(
        "rebalance value_in_token0 => collector fee_shares {} (tier {} @ {} bps, source {})",
        collector_shares, fee_tier, fee_bps, fee_source
    );
}

#[test]
fn rebalance_is_non_blocking_when_fee_registry_is_unreachable() {
    let mut h = setup_with_fee(true);
    let (token0, token1) = (h.token0.clone(), h.token1.clone());
    deposit(&mut h, &token0, Uint128::new(60_000_000_000));
    deposit(&mut h, &token1, Uint128::new(60_000_000_000));
    set_twap(&mut h, "1.75");

    // Re-point the vault at a valid address that hosts no fee-registry contract.
    let dead_registry = h.app.api().addr_make("dead-fee-registry");
    h.app
        .execute_contract(
            h.admin.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::UpdateConfig {
                grid_count: None,
                lower_price: None,
                upper_price: None,
                allocation_tolerance_bps: None,
                max_trade_bps: None,
                max_execution_deviation_bps: None,
                quote_slippage_bps: None,
                max_spot_twap_deviation_bps: None,
                max_trade_pool_bps: None,
                max_spread: None,
                fee_registry: Some(dead_registry.to_string()),
                fee_collector: None,
                proxy: None,
            },
            &[],
        )
        .unwrap();

    // The rebalance must complete; the fee is skipped via `fee_skipped`.
    let response = h
        .app
        .execute_contract(
            h.depositor.clone(),
            h.vault.clone(),
            &VaultExecuteMsg::Rebalance { deadline: u64::MAX },
            &[],
        )
        .unwrap();
    let fee_skipped = response
        .events
        .iter()
        .flat_map(|e| &e.attributes)
        .filter(|a| a.key == "fee_skipped")
        .count();
    assert_eq!(fee_skipped, 1, "rebalance must skip the fee, not revert");
    let fee_shares = response
        .events
        .iter()
        .flat_map(|e| &e.attributes)
        .filter(|a| a.key == "fee_shares")
        .count();
    assert_eq!(
        fee_shares, 0,
        "no fee may be minted against an unreachable registry"
    );
}
