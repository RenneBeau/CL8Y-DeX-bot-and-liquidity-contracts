use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Decimal, Uint128};
use cw20::Cw20ReceiveMsg;

#[cw_serde]
pub struct InstantiateMsg {
    pub admin: String,
    pub factory: String,
    pub pair: String,
    pub pair_code_id: u64,
    pub twap_window_seconds: u32,
    pub grid_count: u32,
    pub lower_price: Decimal,
    pub upper_price: Decimal,
    pub allocation_tolerance_bps: Option<u16>,
    pub max_trade_bps: Option<u16>,
    pub max_execution_deviation_bps: Option<u16>,
    pub quote_slippage_bps: Option<u16>,
    pub max_spot_twap_deviation_bps: Option<u16>,
    pub max_trade_pool_bps: Option<u16>,
    pub max_spread: Option<Decimal>,
    pub fee_registry: Option<String>,
    pub fee_collector: Option<String>,
    pub proxy: Option<String>,
}

#[cw_serde]
pub enum ExecuteMsg {
    Receive(Cw20ReceiveMsg),
    Withdraw {
        shares: Uint128,
        recipient: Option<String>,
    },
    Rebalance {
        deadline: u64,
    },
    UpdateConfig {
        grid_count: Option<u32>,
        lower_price: Option<Decimal>,
        upper_price: Option<Decimal>,
        allocation_tolerance_bps: Option<u16>,
        max_trade_bps: Option<u16>,
        max_execution_deviation_bps: Option<u16>,
        quote_slippage_bps: Option<u16>,
        max_spot_twap_deviation_bps: Option<u16>,
        max_trade_pool_bps: Option<u16>,
        max_spread: Option<Decimal>,
        fee_registry: Option<String>,
        fee_collector: Option<String>,
        proxy: Option<String>,
    },
    TransferAdmin {
        admin: String,
    },
    AcceptAdmin {},
    Pause {},
    Resume {},
    RedeemShares {
        bot_id: u64,
        recipient: Option<String>,
    },
}

#[cw_serde]
pub enum ReceiveMsg {
    Deposit {},
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
}

/// Hook sent to a shared swap-proxy when the vault routes its rebalance swap
/// through a single, whitelistable provider. Mirrors the rebalancer's
/// `SwapProxyHookMsg` (`rebalancer-system/packages/bot-types`).
#[cw_serde]
pub enum SwapProxyHookMsg {
    Swap {
        pair: String,
        min_return: Uint128,
        max_spread: Decimal,
        deadline: u64,
    },
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
pub enum FactoryQueryMsg {
    Pair { asset_infos: [AssetInfo; 2] },
}

#[cw_serde]
pub struct PairResponse {
    pub pair: PairInfo,
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
    pub contract_addr: String,
    pub liquidity_token: String,
}

#[cw_serde]
pub struct PoolResponse {
    pub assets: [Asset; 2],
    pub total_share: Uint128,
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},
    #[returns(GridStatusResponse)]
    GridStatus {},
    #[returns(SharesResponse)]
    Shares { bot_id: u64, address: String },
    #[returns(VaultResponse)]
    Vault {},
}

#[cw_serde]
pub struct ConfigResponse {
    pub admin: String,
    pub pending_admin: Option<String>,
    pub factory: String,
    pub pair: String,
    pub pair_code_id: u64,
    pub asset_tokens: [String; 2],
    pub decimals: u8,
    pub twap_window_seconds: u32,
    pub grid_count: u32,
    pub lower_price: Decimal,
    pub upper_price: Decimal,
    pub allocation_tolerance_bps: u16,
    pub max_trade_bps: u16,
    pub max_execution_deviation_bps: u16,
    pub quote_slippage_bps: u16,
    pub max_spot_twap_deviation_bps: u16,
    pub max_trade_pool_bps: u16,
    pub max_spread: Decimal,
    pub reference_price: Decimal,
    pub last_cell: u32,
    pub paused: bool,
    pub fee_registry: Option<String>,
    pub fee_collector: Option<String>,
    pub proxy: Option<String>,
}

#[cw_serde]
pub struct GridStatusResponse {
    pub current_cell: u32,
    pub target_weight_bps: u16,
    pub allocation_deviation_bps: u16,
    pub should_rebalance: bool,
    pub captured_twap: Decimal,
    pub balances: [Uint128; 2],
    pub offer_token: Option<String>,
    pub amount: Option<Uint128>,
    pub min_return: Option<Uint128>,
    pub pending_swap: bool,
}

#[cw_serde]
pub struct SharesResponse {
    pub shares: Uint128,
}

#[cw_serde]
pub struct VaultResponse {
    pub balances: [Uint128; 2],
    pub total_shares: Uint128,
    pub value_in_token_1: Uint128,
}

#[cw_serde]
pub struct MigrateMsg {}
