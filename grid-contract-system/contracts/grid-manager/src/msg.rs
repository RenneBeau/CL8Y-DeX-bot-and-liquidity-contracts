use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Decimal, Uint128};
use cw20::Cw20ReceiveMsg;

#[cw_serde]
pub struct InstantiateMsg {
    pub admin: String,
    pub keeper: String,
    pub factory: String,
    pub gas_denom: String,
    pub keeper_reward: Uint128,
    pub minimum_gas_reserve: Uint128,
    pub max_grid_count: u32,
    pub max_orders_per_reconcile: u32,
    pub max_active_orders_per_bot: u32,
}

#[cw_serde]
pub enum ExecuteMsg {
    CreateBot {
        pair: String,
        lower_price: Decimal,
        upper_price: Decimal,
        grid_count: u32,
    },
    Receive(Cw20ReceiveMsg),
    FundGas {
        bot_id: u64,
    },
    WithdrawGas {
        bot_id: u64,
        amount: Uint128,
        recipient: Option<String>,
    },
    Allocate {
        bot_id: u64,
    },
    Reconcile {
        bot_id: u64,
        reports: Vec<OrderFillReport>,
    },
    CancelAll {
        bot_id: u64,
    },
    Withdraw {
        bot_id: u64,
        shares: Uint128,
        recipient: Option<String>,
    },
    UpdateKeeper {
        keeper: String,
    },
}

#[cw_serde]
pub enum ReceiveMsg {
    Deposit { bot_id: u64 },
}

#[cw_serde]
pub enum LimitOrderSide {
    Bid,
    Ask,
}

#[cw_serde]
pub struct LimitOrderPlacementItem {
    pub price: Decimal,
    pub amount: Uint128,
    pub max_adjust_steps: u32,
    pub expires_at: Option<u64>,
    pub hint_after_order_id: Option<u64>,
}

#[cw_serde]
pub struct OrderFillReport {
    pub order_id: u64,
    /// Aggregate escrow consumed since the previous reconciliation.
    pub input_amount: Uint128,
    /// Exact aggregate maker output from indexed CL8Y fill events.
    pub output_amount: Uint128,
    pub fill_count: u32,
}

#[cw_serde]
pub enum PairCw20HookMsg {
    PlaceLimitOrderBatch {
        side: LimitOrderSide,
        orders: Vec<LimitOrderPlacementItem>,
    },
}

#[cw_serde]
pub enum PairExecuteMsg {
    CancelLimitOrders { order_ids: Vec<u64> },
    ClaimExpiredLimitOrders { order_ids: Vec<u64> },
}

#[cw_serde]
pub enum PairQueryMsg {
    Pair {},
    Pool {},
    LimitOrder { order_id: u64 },
    ExpiredLimitRefund { order_id: u64 },
    LimitOrderConfig {},
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
pub struct LimitOrderResponse {
    pub order_id: u64,
    pub owner: String,
    pub side: LimitOrderSide,
    pub price: Decimal,
    pub remaining: Uint128,
    pub expires_at: Option<u64>,
    pub prev: Option<u64>,
    pub next: Option<u64>,
}

#[cw_serde]
pub struct ExpiredLimitRefundResponse {
    pub order_id: u64,
    pub owner: String,
    pub side: LimitOrderSide,
    pub remaining: Uint128,
    pub expires_at: Option<u64>,
}

#[cw_serde]
pub struct LimitOrderConfigResponse {
    pub max_batch_rungs: u32,
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},
    #[returns(BotResponse)]
    Bot { bot_id: u64 },
    #[returns(Vec<RungResponse>)]
    Rungs { bot_id: u64 },
    #[returns(Vec<OrderResponse>)]
    Orders { bot_id: u64 },
    #[returns(ShareResponse)]
    Shares { bot_id: u64, address: String },
}

#[cw_serde]
pub struct ConfigResponse {
    pub admin: String,
    pub keeper: String,
    pub factory: String,
    pub gas_denom: String,
    pub keeper_reward: Uint128,
    pub minimum_gas_reserve: Uint128,
    pub max_grid_count: u32,
    pub max_orders_per_reconcile: u32,
    pub max_active_orders_per_bot: u32,
}

#[cw_serde]
pub struct BotResponse {
    pub bot_id: u64,
    pub owner: String,
    pub pair: String,
    pub asset_tokens: [String; 2],
    pub lower_price: Decimal,
    pub upper_price: Decimal,
    pub grid_count: u32,
    pub reference_price: Decimal,
    pub free_balances: [Uint128; 2],
    pub total_shares: Uint128,
    pub gas_credit: Uint128,
    pub active_orders: u32,
    pub pair_batch_limit: u32,
}

#[cw_serde]
pub struct RungResponse {
    pub index: u32,
    pub price: Decimal,
    pub side: Option<LimitOrderSide>,
}

#[cw_serde]
pub struct OrderResponse {
    pub order_id: u64,
    pub rung_index: u32,
    pub side: LimitOrderSide,
    pub price: Decimal,
    pub remaining: Uint128,
}

#[cw_serde]
pub struct ShareResponse {
    pub shares: Uint128,
}
