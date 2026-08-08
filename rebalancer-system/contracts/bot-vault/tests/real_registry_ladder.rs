#![cfg_attr(feature = "mainnet", allow(dead_code, unused_imports))]

use bot_types::{VaultBalancesResponse, VaultExecuteMsg, VaultQueryMsg, WithdrawalType};
use cl8y_dex::{
    Asset, AssetInfo, FactoryQueryMsg, HybridSimulationResponse, ObserveResponse, PairCw20HookMsg,
    PairInfo, PairQueryMsg, PairResponse, PoolResponse,
};
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    from_json, to_json_binary, Addr, Binary, Decimal, Deps, DepsMut, Empty, Env, MessageInfo,
    Response, StdError, StdResult, Uint128, WasmMsg,
};
use cw20::{BalanceResponse, Cw20ExecuteMsg, Cw20QueryMsg, Cw20ReceiveMsg, TokenInfoResponse};
use cw_multi_test::{App, AppResponse, Contract, ContractWrapper, Executor};
use cw_storage_plus::Item;

const CL8Y_LADDER: [(u8, u128, u16); 9] = [
    (1, ONE_CL8Y, 250),
    (2, ONE_CL8Y * 5, 1_000),
    (3, ONE_CL8Y * 20, 2_000),
    (4, ONE_CL8Y * 75, 3_500),
    (5, ONE_CL8Y * 200, 5_000),
    (6, ONE_CL8Y * 500, 6_000),
    (7, ONE_CL8Y * 1_500, 7_500),
    (8, ONE_CL8Y * 3_500, 8_500),
    (9, ONE_CL8Y * 7_500, 9_500),
];

const ONE_CL8Y: u128 = 1_000_000_000_000_000_000;
const BASE_FEE_BPS: u16 = 180;
const ASSET_RESERVE: u128 = 10_000_000;
const DEPOSIT_PER_ASSET: u128 = 1_000_000;
const REBALANCE_DONATION: u128 = 500_000;
const REBALANCE_FILL: u128 = 250_000;
const TWAP_WINDOW: u32 = 60;

#[cw_serde]
struct RealEffectiveFee {
    fee_bps: u16,
    discount_bps: u16,
    tier_id: Option<u8>,
    holding: Option<Uint128>,
    source: String,
}

fn cw20_code() -> Box<dyn Contract<Empty, Empty>> {
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

fn real_registry_code() -> Box<dyn Contract<Empty, Empty>> {
    let contract = ContractWrapper::new(
        cl8y_fee_registry::contract::execute,
        cl8y_fee_registry::contract::instantiate,
        cl8y_fee_registry::contract::query,
    )
    .with_migrate(cl8y_fee_registry::contract::migrate);
    Box::new(contract)
}

fn vault_code() -> Box<dyn Contract<Empty, Empty>> {
    Box::new(
        ContractWrapper::new(
            cl8y_bot_vault::contract::execute,
            cl8y_bot_vault::contract::instantiate,
            cl8y_bot_vault::contract::query,
        )
        .with_reply(cl8y_bot_vault::contract::reply)
        .with_migrate(cl8y_bot_vault::contract::migrate),
    )
}

fn liquidity_code() -> Box<dyn Contract<Empty, Empty>> {
    Box::new(
        ContractWrapper::new(
            cl8y_bot_liquidity::contract::execute,
            cl8y_bot_liquidity::contract::instantiate,
            cl8y_bot_liquidity::contract::query,
        )
        .with_reply(cl8y_bot_liquidity::contract::reply)
        .with_migrate(cl8y_bot_liquidity::contract::migrate),
    )
}

fn proxy_code() -> Box<dyn Contract<Empty, Empty>> {
    Box::new(
        ContractWrapper::new(
            cl8y_swap_proxy::contract::execute,
            cl8y_swap_proxy::contract::instantiate,
            cl8y_swap_proxy::contract::query,
        )
        .with_migrate(cl8y_swap_proxy::contract::migrate),
    )
}

#[cw_serde]
struct PairInstantiateMsg {
    asset_tokens: [String; 2],
}

#[cw_serde]
enum PairExecuteMsg {
    Receive(Cw20ReceiveMsg),
}

const PAIR_ASSETS: Item<[Addr; 2]> = Item::new("assets");

fn pair_instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: PairInstantiateMsg,
) -> StdResult<Response> {
    PAIR_ASSETS.save(
        deps.storage,
        &[
            deps.api.addr_validate(&msg.asset_tokens[0])?,
            deps.api.addr_validate(&msg.asset_tokens[1])?,
        ],
    )?;
    Ok(Response::new())
}

fn pair_execute(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: PairExecuteMsg,
) -> StdResult<Response> {
    let PairExecuteMsg::Receive(receive) = msg;
    let assets = PAIR_ASSETS.load(deps.storage)?;
    let offer_index = assets
        .iter()
        .position(|token| token == info.sender)
        .ok_or_else(|| StdError::generic_err("unsupported offer token"))?;
    let hook: PairCw20HookMsg = from_json(receive.msg)?;
    let PairCw20HookMsg::Swap { min_return, to, .. } = hook else {
        return Err(StdError::generic_err("unsupported pair hook"));
    };
    if receive.amount < min_return.unwrap_or_default() {
        return Err(StdError::generic_err("minimum return not met"));
    }
    Ok(Response::new()
        .add_message(WasmMsg::Execute {
            contract_addr: assets[1 - offer_index].to_string(),
            msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
                recipient: to.unwrap_or(receive.sender),
                amount: receive.amount,
            })?,
            funds: vec![],
        })
        .add_attribute("action", "mock_pair_swap")
        .add_attribute("return_amount", receive.amount))
}

fn pair_query(deps: Deps, env: Env, msg: PairQueryMsg) -> StdResult<Binary> {
    let assets = PAIR_ASSETS.load(deps.storage)?;
    let asset_infos = assets.clone().map(|token| AssetInfo::Token {
        contract_addr: token.to_string(),
    });
    match msg {
        PairQueryMsg::Pair {} => to_json_binary(&PairInfo {
            asset_infos,
            contract_addr: env.contract.address.clone(),
            liquidity_token: env.contract.address,
        }),
        PairQueryMsg::Pool {} => {
            let balances = assets
                .iter()
                .map(|token| -> StdResult<Uint128> {
                    let response: BalanceResponse = deps.querier.query_wasm_smart(
                        token,
                        &Cw20QueryMsg::Balance {
                            address: env.contract.address.to_string(),
                        },
                    )?;
                    Ok(response.balance)
                })
                .collect::<StdResult<Vec<_>>>()?;
            to_json_binary(&PoolResponse {
                assets: [
                    Asset {
                        info: asset_infos[0].clone(),
                        amount: balances[0],
                    },
                    Asset {
                        info: asset_infos[1].clone(),
                        amount: balances[1],
                    },
                ],
                total_share: Uint128::new(ASSET_RESERVE * 2),
            })
        }
        PairQueryMsg::HybridSimulation { offer_asset, .. } => {
            to_json_binary(&HybridSimulationResponse {
                return_amount: offer_asset.amount,
                spread_amount: Uint128::zero(),
                commission_amount: Uint128::zero(),
                pool_commission_amount: Uint128::zero(),
                book_commission_amount: Uint128::zero(),
                book_return_amount: Uint128::zero(),
                pool_return_amount: offer_asset.amount,
                limit_book_offer_consumed: Uint128::zero(),
            })
        }
        PairQueryMsg::Observe { seconds_ago } => {
            if seconds_ago != [0, TWAP_WINDOW] {
                return Err(StdError::generic_err("unexpected TWAP window"));
            }
            to_json_binary(&ObserveResponse {
                price_a_cumulatives: vec![
                    Uint128::new(u128::from(TWAP_WINDOW) * Decimal::one().atomics().u128()),
                    Uint128::zero(),
                ],
                price_b_cumulatives: vec![
                    Uint128::new(u128::from(TWAP_WINDOW) * Decimal::one().atomics().u128()),
                    Uint128::zero(),
                ],
            })
        }
    }
}

fn pair_code() -> Box<dyn Contract<Empty, Empty>> {
    Box::new(ContractWrapper::new(
        pair_execute,
        pair_instantiate,
        pair_query,
    ))
}

#[cw_serde]
struct FactoryInstantiateMsg {
    pair: String,
    asset_tokens: [String; 2],
}

const FACTORY_PAIR: Item<PairInfo> = Item::new("pair");

fn factory_instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: FactoryInstantiateMsg,
) -> StdResult<Response> {
    let pair = deps.api.addr_validate(&msg.pair)?;
    FACTORY_PAIR.save(
        deps.storage,
        &PairInfo {
            asset_infos: msg
                .asset_tokens
                .map(|contract_addr| AssetInfo::Token { contract_addr }),
            contract_addr: pair.clone(),
            liquidity_token: pair,
        },
    )?;
    Ok(Response::new())
}

fn factory_query(deps: Deps, _env: Env, msg: FactoryQueryMsg) -> StdResult<Binary> {
    let FactoryQueryMsg::Pair { asset_infos } = msg;
    let pair = FACTORY_PAIR.load(deps.storage)?;
    if pair.asset_infos != asset_infos {
        return Err(StdError::not_found("pair"));
    }
    to_json_binary(&PairResponse { pair })
}

fn factory_code() -> Box<dyn Contract<Empty, Empty>> {
    Box::new(ContractWrapper::new(
        |_deps, _env, _info, _msg: Empty| -> StdResult<Response> { Ok(Response::new()) },
        factory_instantiate,
        factory_query,
    ))
}

fn real_registry_app(cl8y_balance: Uint128) -> (App, Addr, Addr, Addr) {
    let mut app = App::default();
    let minter = app.api().addr_make("cl8y-minter");
    let trader = app.api().addr_make("fee-subject");

    let cl_code = app.store_code(cw20_code());
    let cl8y = app
        .instantiate_contract(
            cl_code,
            minter.clone(),
            &cw20_base::msg::InstantiateMsg {
                name: "CL8Y".to_string(),
                symbol: "CLY".to_string(),
                decimals: 18,
                initial_balances: vec![cw20::Cw20Coin {
                    address: trader.to_string(),
                    amount: cl8y_balance,
                }],
                mint: Some(cw20::MinterResponse {
                    minter: minter.to_string(),
                    cap: None,
                }),
                marketing: None,
            },
            &[],
            "cl8y",
            None,
        )
        .unwrap();

    let governance = app.api().addr_make("governance");
    let registry_code = app.store_code(real_registry_code());
    let registry = app
        .instantiate_contract(
            registry_code,
            governance.clone(),
            &cl8y_fee_registry::msg::InstantiateMsg {
                governance: governance.to_string(),
                cl8y: cl8y.to_string(),
                treasury: app.api().addr_make("treasury").to_string(),
                fee_collector: app.api().addr_make("collector").to_string(),
                base_fee_bps: BASE_FEE_BPS,
            },
            &[],
            "fee-registry",
            None,
        )
        .unwrap();

    (app, registry, cl8y, trader)
}

fn real_effective_fee(app: &App, registry: &Addr, trader: &Addr) -> RealEffectiveFee {
    app.wrap()
        .query_wasm_smart(
            registry,
            &cl8y_fee_registry::msg::QueryMsg::EffectiveFee {
                trader: trader.to_string(),
            },
        )
        .unwrap()
}

struct FullFlowMarket {
    app: App,
    keeper: Addr,
    collector: Addr,
    vault: Addr,
    liquidity: Addr,
    asset_tokens: [Addr; 2],
}

impl FullFlowMarket {
    fn new(cl8y_balance: Uint128) -> Self {
        let mut app = App::default();
        let admin = app.api().addr_make("vault-admin");
        let keeper = app.api().addr_make("keeper");
        let collector = app.api().addr_make("fee-collector");
        let governance = app.api().addr_make("governance");

        let cw20_id = app.store_code(cw20_code());
        let pair_id = app.store_code(pair_code());
        let factory_id = app.store_code(factory_code());
        let proxy_id = app.store_code(proxy_code());
        let vault_id = app.store_code(vault_code());
        let liquidity_id = app.store_code(liquidity_code());
        let registry_id = app.store_code(real_registry_code());

        let instantiate_token = |app: &mut App, name: &str, symbol: &str, decimals: u8| {
            app.instantiate_contract(
                cw20_id,
                admin.clone(),
                &cw20_base::msg::InstantiateMsg {
                    name: name.to_string(),
                    symbol: symbol.to_string(),
                    decimals,
                    initial_balances: vec![],
                    mint: Some(cw20::MinterResponse {
                        minter: admin.to_string(),
                        cap: None,
                    }),
                    marketing: None,
                },
                &[],
                name,
                None,
            )
            .unwrap()
        };
        let asset_tokens = [
            instantiate_token(&mut app, "Asset Zero", "ZERO", 6),
            instantiate_token(&mut app, "Asset One", "ONE", 6),
        ];
        let cl8y = app
            .instantiate_contract(
                cw20_id,
                admin.clone(),
                &cw20_base::msg::InstantiateMsg {
                    name: "CL8Y".to_string(),
                    symbol: "CLY".to_string(),
                    decimals: 18,
                    initial_balances: vec![cw20::Cw20Coin {
                        address: admin.to_string(),
                        amount: cl8y_balance,
                    }],
                    mint: Some(cw20::MinterResponse {
                        minter: admin.to_string(),
                        cap: None,
                    }),
                    marketing: None,
                },
                &[],
                "cl8y",
                None,
            )
            .unwrap();

        let pair = app
            .instantiate_contract(
                pair_id,
                governance.clone(),
                &PairInstantiateMsg {
                    asset_tokens: asset_tokens.clone().map(|addr| addr.to_string()),
                },
                &[],
                "dex-pair",
                None,
            )
            .unwrap();
        for token in &asset_tokens {
            app.execute_contract(
                admin.clone(),
                token.clone(),
                &Cw20ExecuteMsg::Mint {
                    recipient: pair.to_string(),
                    amount: Uint128::new(ASSET_RESERVE),
                },
                &[],
            )
            .unwrap();
        }
        let factory = app
            .instantiate_contract(
                factory_id,
                governance.clone(),
                &FactoryInstantiateMsg {
                    pair: pair.to_string(),
                    asset_tokens: asset_tokens.clone().map(|addr| addr.to_string()),
                },
                &[],
                "dex-factory",
                None,
            )
            .unwrap();
        let proxy = app
            .instantiate_contract(
                proxy_id,
                governance.clone(),
                &cl8y_swap_proxy::msg::InstantiateMsg {
                    admin: governance.to_string(),
                },
                &[],
                "swap-proxy",
                None,
            )
            .unwrap();
        let registry = app
            .instantiate_contract(
                registry_id,
                governance.clone(),
                &cl8y_fee_registry::msg::InstantiateMsg {
                    governance: governance.to_string(),
                    cl8y: cl8y.to_string(),
                    treasury: app.api().addr_make("treasury").to_string(),
                    fee_collector: collector.to_string(),
                    base_fee_bps: BASE_FEE_BPS,
                },
                &[],
                "fee-registry",
                None,
            )
            .unwrap();
        let vault = app
            .instantiate_contract(
                vault_id,
                admin.clone(),
                &cl8y_bot_vault::msg::InstantiateMsg {
                    admin: admin.to_string(),
                    keeper: keeper.to_string(),
                    proxy: proxy.to_string(),
                    factory: factory.to_string(),
                    pair: pair.to_string(),
                    pair_code_id: pair_id,
                    liquidity_code_id: liquidity_id,
                    twap_window_seconds: TWAP_WINDOW,
                    rebalance_threshold_bps: Some(500),
                    allocation_tolerance_bps: Some(100),
                    max_trade_bps: Some(5_000),
                    max_execution_deviation_bps: Some(500),
                    quote_slippage_bps: Some(200),
                    max_spot_twap_deviation_bps: Some(500),
                    max_trade_pool_bps: Some(2_000),
                    max_spread: Some(Decimal::percent(5)),
                    fee_registry: Some(registry.to_string()),
                    fee_collector: Some(collector.to_string()),
                },
                &[],
                "bot-vault",
                None,
            )
            .unwrap();
        app.execute_contract(
            governance.clone(),
            proxy.clone(),
            &cl8y_swap_proxy::msg::ExecuteMsg::RegisterVault {
                vault: vault.to_string(),
                pair: pair.to_string(),
            },
            &[],
        )
        .unwrap();
        let route: cl8y_swap_proxy::msg::RouteResponse = app
            .wrap()
            .query_wasm_smart(
                &proxy,
                &cl8y_swap_proxy::msg::QueryMsg::Route {
                    vault: vault.to_string(),
                },
            )
            .unwrap();
        assert_eq!(route.pair, pair);
        assert_eq!(route.pair_code_id, pair_id);

        let liquidity = app
            .instantiate_contract(
                liquidity_id,
                admin.clone(),
                &cl8y_bot_liquidity::msg::InstantiateMsg {
                    admin: admin.to_string(),
                    vault: vault.to_string(),
                    name: "CL8Y Bot Liquidity".to_string(),
                    symbol: "BOT-LP".to_string(),
                    decimals: 6,
                    minimum_initial_deposit: Uint128::new(10_000),
                    marketing: None,
                },
                &[],
                "bot-liquidity",
                None,
            )
            .unwrap();
        app.execute_contract(
            admin.clone(),
            vault.clone(),
            &VaultExecuteMsg::SetLiquidityContract {
                liquidity_contract: liquidity.to_string(),
            },
            &[],
        )
        .unwrap();

        for token in &asset_tokens {
            app.execute_contract(
                admin.clone(),
                token.clone(),
                &Cw20ExecuteMsg::Mint {
                    recipient: admin.to_string(),
                    amount: Uint128::new(DEPOSIT_PER_ASSET + REBALANCE_DONATION),
                },
                &[],
            )
            .unwrap();
            app.execute_contract(
                admin.clone(),
                token.clone(),
                &Cw20ExecuteMsg::IncreaseAllowance {
                    spender: liquidity.to_string(),
                    amount: Uint128::new(DEPOSIT_PER_ASSET),
                    expires: None,
                },
                &[],
            )
            .unwrap();
        }
        let deadline = app.block_info().time.seconds() + 60;
        app.execute_contract(
            admin.clone(),
            liquidity.clone(),
            &cl8y_bot_liquidity::msg::ExecuteMsg::Deposit {
                amounts: [
                    Uint128::new(DEPOSIT_PER_ASSET),
                    Uint128::new(DEPOSIT_PER_ASSET),
                ],
                min_shares: Uint128::new(1),
                deadline,
                swap: None,
            },
            &[],
        )
        .unwrap();
        app.execute_contract(
            admin.clone(),
            asset_tokens[0].clone(),
            &Cw20ExecuteMsg::Transfer {
                recipient: vault.to_string(),
                amount: Uint128::new(REBALANCE_DONATION),
            },
            &[],
        )
        .unwrap();

        Self {
            app,
            keeper,
            collector,
            vault,
            liquidity,
            asset_tokens,
        }
    }

    fn cw20_balance(&self, token: &Addr, owner: &Addr) -> Uint128 {
        let response: BalanceResponse = self
            .app
            .wrap()
            .query_wasm_smart(
                token,
                &Cw20QueryMsg::Balance {
                    address: owner.to_string(),
                },
            )
            .unwrap();
        response.balance
    }

    fn token_info(&self) -> TokenInfoResponse {
        self.app
            .wrap()
            .query_wasm_smart(&self.liquidity, &Cw20QueryMsg::TokenInfo {})
            .unwrap()
    }
}

fn attribute<'a>(response: &'a AppResponse, key: &str) -> &'a str {
    response
        .events
        .iter()
        .flat_map(|event| &event.attributes)
        .find(|attribute| attribute.key == key)
        .unwrap_or_else(|| panic!("missing response attribute {key}"))
        .value
        .as_str()
}

#[test]
fn real_registry_detects_every_ladder_tier_for_the_bot_admin_model() {
    for (tier_id, min_cl8y, discount_bps) in CL8Y_LADDER {
        for (label, balance) in [
            ("exact boundary", Uint128::new(min_cl8y)),
            ("one wei above", Uint128::new(min_cl8y + 1)),
        ] {
            let (app, registry, _cl8y, trader) = real_registry_app(balance);
            let fee = real_effective_fee(&app, &registry, &trader);
            let expected_bps =
                ((BASE_FEE_BPS as u32 * (10_000 - discount_bps) as u32) / 10_000) as u16;
            assert_eq!(
                fee.fee_bps, expected_bps,
                "tier {tier_id} @ {label}: expected {expected_bps} bps, got {}",
                fee.fee_bps
            );
            assert_eq!(fee.tier_id, Some(tier_id));
            assert_eq!(fee.source, "live");
            assert_eq!(fee.holding, Some(balance));
        }
    }
}

#[test]
#[cfg(not(feature = "mainnet"))]
fn real_registry_drives_every_user_rate_through_rebalance_and_redemption() {
    let cases = std::iter::once((None, Uint128::zero(), BASE_FEE_BPS)).chain(
        CL8Y_LADDER.into_iter().map(|(tier, holding, discount)| {
            let fee_bps =
                ((u32::from(BASE_FEE_BPS) * u32::from(10_000 - discount)) / 10_000) as u16;
            (Some(tier), Uint128::new(holding), fee_bps)
        }),
    );

    for (expected_tier, cl8y_balance, expected_bps) in cases {
        let mut market = FullFlowMarket::new(cl8y_balance);
        let pre_fee_supply = market.token_info().total_supply;
        let response = market
            .app
            .execute_contract(
                market.keeper.clone(),
                market.vault.clone(),
                &VaultExecuteMsg::Rebalance {
                    deadline: market.app.block_info().time.seconds() + 60,
                },
                &[],
            )
            .unwrap();

        assert_eq!(attribute(&response, "fee_bps"), expected_bps.to_string());
        assert_eq!(
            attribute(&response, "fee_tier"),
            expected_tier
                .map(|tier| tier.to_string())
                .unwrap_or_default()
        );
        assert_eq!(attribute(&response, "fee_source"), "live");
        assert_eq!(attribute(&response, "fee_holders"), "1");

        let settled: VaultBalancesResponse = market
            .app
            .wrap()
            .query_wasm_smart(&market.vault, &VaultQueryMsg::Balances {})
            .unwrap();
        assert_eq!(
            settled.balances,
            [
                Uint128::new(DEPOSIT_PER_ASSET + REBALANCE_FILL),
                Uint128::new(DEPOSIT_PER_ASSET + REBALANCE_FILL),
            ]
        );
        let desired_fee = Uint128::new(REBALANCE_FILL).multiply_ratio(expected_bps, 10_000u16);
        let pre_fee_nav = settled.balances[0] + settled.balances[1];
        let expected_fee_shares = desired_fee.multiply_ratio(
            pre_fee_supply,
            pre_fee_nav.checked_sub(desired_fee).unwrap(),
        );
        assert_eq!(
            attribute(&response, "fee_shares"),
            expected_fee_shares.to_string()
        );
        assert_eq!(
            market.cw20_balance(&market.liquidity, &market.collector),
            expected_fee_shares
        );

        let collector_assets_before = market
            .asset_tokens
            .each_ref()
            .map(|token| market.cw20_balance(token, &market.collector));
        market
            .app
            .execute_contract(
                market.collector.clone(),
                market.liquidity.clone(),
                &cl8y_bot_liquidity::msg::ExecuteMsg::Withdraw {
                    shares: expected_fee_shares,
                    recipient: None,
                    deadline: market.app.block_info().time.seconds() + 60,
                    output: WithdrawalType::ProRata {
                        min_assets: [Uint128::zero(), Uint128::zero()],
                    },
                },
                &[],
            )
            .unwrap();
        let collector_assets_after = market
            .asset_tokens
            .each_ref()
            .map(|token| market.cw20_balance(token, &market.collector));
        let redeemed_nav = collector_assets_after[0]
            .checked_sub(collector_assets_before[0])
            .unwrap()
            + collector_assets_after[1]
                .checked_sub(collector_assets_before[1])
                .unwrap();
        assert!(
            redeemed_nav <= desired_fee,
            "tier {expected_tier:?}: redeemed NAV {redeemed_nav} exceeds desired fee {desired_fee}"
        );
        assert_eq!(
            market.cw20_balance(&market.liquidity, &market.collector),
            Uint128::zero()
        );
    }
}
