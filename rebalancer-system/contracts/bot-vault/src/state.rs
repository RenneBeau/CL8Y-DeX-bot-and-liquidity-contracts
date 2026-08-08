use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Uint128};
use cw_storage_plus::{Item, Map};

#[cw_serde]
pub struct Config {
    pub admin: Addr,
    pub keeper: Addr,
    pub liquidity_contract: Option<Addr>,
    pub proxy: Addr,
    pub factory: Addr,
    pub pair: Addr,
    pub pair_code_id: u64,
    pub asset_tokens: [Addr; 2],
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
    pub reference_price: Decimal,
    pub fee_registry: Option<Addr>,
    pub fee_collector: Option<Addr>,
}

#[cw_serde]
pub struct CachedEffectiveFee {
    pub fee_bps: u16,
    pub tier_id: Option<u8>,
}

#[cw_serde]
pub struct PendingRebalance {
    pub captured_twap: Decimal,
    pub balances: [Uint128; 2],
    pub pre_deviation_bps: u16,
    pub offer_index: u8,
    pub amount: Uint128,
    pub min_return: Uint128,
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const PENDING_REBALANCE: Item<PendingRebalance> = Item::new("pending_rebalance");
pub const PENDING_ADMIN: Item<Addr> = Item::new("pending_admin");
pub const LIQUIDITY_CODE_ID: Item<u64> = Item::new("liquidity_code_id");
pub const PAUSED: Item<bool> = Item::new("paused");
pub const FEE_CACHE: Map<&Addr, CachedEffectiveFee> = Map::new("fee_cache");
