use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Empty, Uint128};
use cw_multi_test::{App, Contract, ContractWrapper, Executor};

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
const BASE_FEE_BPS: u16 = 1_800;

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
