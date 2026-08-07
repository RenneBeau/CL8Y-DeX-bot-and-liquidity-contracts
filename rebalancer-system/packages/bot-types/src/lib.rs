use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Decimal, Uint128};

#[cw_serde]
pub struct SwapParams {
    pub offer_token: String,
    pub amount: Uint128,
    pub min_return: Uint128,
    pub max_spread: Decimal,
    pub deadline: u64,
}

#[cw_serde]
pub struct AuthorizedTransfer {
    pub token: String,
    pub amount: Uint128,
    pub recipient: String,
}

#[cw_serde]
pub struct LiquidityAuthorizationResponse {
    pub swap: Option<SwapParams>,
    pub transfers: Vec<AuthorizedTransfer>,
    pub finalize: bool,
}

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
pub enum VaultExecuteMsg {
    SetLiquidityContract {
        liquidity_contract: String,
    },
    LiquiditySwap {
        params: SwapParams,
    },
    TransferTo {
        token: String,
        amount: Uint128,
        recipient: String,
    },
    FinalizeLiquidityOperation {},
    Rebalance {
        deadline: u64,
    },
    SyncReference {},
    UpdateKeeper {
        keeper: String,
    },
    UpdateThresholds {
        rebalance_threshold_bps: Option<u16>,
        allocation_tolerance_bps: Option<u16>,
        max_trade_bps: Option<u16>,
        max_execution_deviation_bps: Option<u16>,
        quote_slippage_bps: Option<u16>,
        max_spot_twap_deviation_bps: Option<u16>,
        max_trade_pool_bps: Option<u16>,
        max_spread: Option<Decimal>,
        twap_window_seconds: Option<u32>,
    },
    UpdateFeeConfig {
        fee_registry: Option<String>,
        fee_collector: Option<String>,
    },
    TransferAdmin {
        admin: String,
    },
    AcceptAdmin {},
    CancelAdminTransfer {},
    RevokeLiquidityContract {},
    Pause {},
    Resume {},
}

#[cw_serde]
pub enum VaultQueryMsg {
    Config {},
    Balances {},
    Price {},
    RebalanceStatus {},
    RebalancePlan {},
    Shares { bot_id: u64, address: String },
}

#[cw_serde]
pub struct VaultConfigResponse {
    pub admin: String,
    pub pending_admin: Option<String>,
    pub keeper: String,
    pub liquidity_contract: Option<String>,
    pub liquidity_code_id: Option<u64>,
    pub paused: bool,
    pub proxy: String,
    pub pair: String,
    pub asset_tokens: [String; 2],
    pub decimals: u8,
    pub twap_window_seconds: u32,
    pub rebalance_threshold_bps: u16,
    pub allocation_tolerance_bps: u16,
    pub max_trade_bps: u16,
    pub max_execution_deviation_bps: u16,
    pub quote_slippage_bps: u16,
    pub max_spot_twap_deviation_bps: u16,
    pub max_trade_pool_bps: u16,
    pub max_spread: Decimal,
    pub fee_registry: Option<String>,
    pub fee_collector: Option<String>,
}

#[cw_serde]
pub struct VaultBalancesResponse {
    pub balances: [Uint128; 2],
}

#[cw_serde]
pub struct VaultFeeSharesResponse {
    pub shares: Uint128,
}

#[cw_serde]
pub struct VaultPriceResponse {
    pub token1_per_token0: Decimal,
}

#[cw_serde]
pub struct RebalanceStatusResponse {
    pub should_rebalance: bool,
    pub price_deviation_bps: u16,
    pub allocation_deviation_bps: u16,
    pub reference_price: Decimal,
    pub current_price: Decimal,
}

#[cw_serde]
pub struct RebalancePlanResponse {
    pub should_rebalance: bool,
    pub captured_twap: Decimal,
    pub balances: [Uint128; 2],
    pub price_deviation_bps: u16,
    pub allocation_deviation_bps: u16,
    pub reference_price: Decimal,
    pub offer_token: Option<String>,
    pub amount: Option<Uint128>,
    pub min_return: Option<Uint128>,
    pub max_spread: Decimal,
}

#[cw_serde]
pub enum WithdrawalType {
    ProRata {
        min_assets: [Uint128; 2],
    },
    Token0 {
        min_amount: Uint128,
        swap: SwapParams,
    },
    Token1 {
        min_amount: Uint128,
        swap: SwapParams,
    },
}
