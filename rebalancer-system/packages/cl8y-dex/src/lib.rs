use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Uint128};

#[cw_serde]
pub enum AssetInfo {
    Token { contract_addr: String },
    NativeToken { denom: String },
}

#[cw_serde]
pub struct Asset {
    pub info: AssetInfo,
    pub amount: Uint128,
}

#[cw_serde]
pub struct PairInfo {
    pub asset_infos: [AssetInfo; 2],
    pub contract_addr: Addr,
    pub liquidity_token: Addr,
}

#[cw_serde]
pub enum FactoryQueryMsg {
    Pair { asset_infos: [AssetInfo; 2] },
}

#[cw_serde]
pub struct PairResponse {
    pub pair: PairInfo,
}

#[cw_serde]
pub struct PoolResponse {
    pub assets: [Asset; 2],
    pub total_share: Uint128,
}

#[cw_serde]
pub struct HybridSwapParams {
    pub pool_input: Uint128,
    pub book_input: Uint128,
    pub max_maker_fills: u32,
    pub book_start_hint: Option<u64>,
}

impl HybridSwapParams {
    pub fn pool_only(amount: Uint128) -> Self {
        Self {
            pool_input: amount,
            book_input: Uint128::zero(),
            max_maker_fills: 1,
            book_start_hint: None,
        }
    }
}

#[cw_serde]
pub enum PairQueryMsg {
    Pair {},
    Pool {},
    HybridSimulation {
        offer_asset: Asset,
        hybrid: HybridSwapParams,
        trader: Option<String>,
        sender: Option<String>,
        belief_price: Option<Decimal>,
    },
    Observe {
        seconds_ago: Vec<u32>,
    },
}

#[cw_serde]
pub enum PairExecuteMsg {
    ProvideLiquidity {
        assets: [Asset; 2],
        slippage_tolerance: Option<Decimal>,
        receiver: Option<String>,
        deadline: Option<u64>,
    },
}

#[cw_serde]
pub enum PairCw20HookMsg {
    Swap {
        belief_price: Option<Decimal>,
        max_spread: Option<Decimal>,
        min_return: Option<Uint128>,
        to: Option<String>,
        deadline: Option<u64>,
        trader: Option<String>,
        hybrid: Option<HybridSwapParams>,
    },
    WithdrawLiquidity {
        min_assets: Option<[Uint128; 2]>,
    },
}

#[cw_serde]
pub struct HybridSimulationResponse {
    pub return_amount: Uint128,
    pub spread_amount: Uint128,
    pub commission_amount: Uint128,
    pub pool_commission_amount: Uint128,
    pub book_commission_amount: Uint128,
    pub book_return_amount: Uint128,
    pub pool_return_amount: Uint128,
    pub limit_book_offer_consumed: Uint128,
}

#[cw_serde]
pub struct ObserveResponse {
    pub price_a_cumulatives: Vec<Uint128>,
    pub price_b_cumulatives: Vec<Uint128>,
}
