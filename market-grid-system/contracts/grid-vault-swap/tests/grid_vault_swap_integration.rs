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
    Asset, AssetInfo, ExecuteMsg as VaultExecuteMsg, HybridSimulationResponse, InstantiateMsg,
    ObserveResponse, PairCw20HookMsg, PairInfo, PairQueryMsg, PoolResponse, QueryMsg, ReceiveMsg,
    SharesResponse,
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
}

fn setup() -> Harness {
    let mut app = App::default();
    let admin = app.api().addr_make(ADMIN);
    let depositor = app.api().addr_make(DEPOSITOR);

    let cw20_code = app.store_code(mock_cw20_code());
    let pair_code = app.store_code(mock_pair_code());
    let vault_code = app.store_code(mock_vault_code());

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
    }
}

fn deposit(h: &mut Harness, token: &Addr, amount: Uint128) {
    h.app
        .execute_contract(
            h.depositor.clone(),
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
