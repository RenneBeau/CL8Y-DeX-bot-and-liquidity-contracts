use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Uint128};
use cw_storage_plus::Item;

#[cw_serde]
pub struct Config {
    pub admin: Addr,
    pub keeper: Addr,
    pub liquidity_contract: Option<Addr>,
    pub proxy: Addr,
    pub pair: Addr,
    pub asset_tokens: [Addr; 2],
    pub decimals: u8,
    pub twap_window_seconds: u32,
    pub rebalance_threshold_bps: u16,
    pub allocation_tolerance_bps: u16,
    pub max_trade_bps: u16,
    pub max_execution_deviation_bps: u16,
    pub quote_slippage_bps: u16,
    pub max_spread: Decimal,
    pub reference_price: Decimal,
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
