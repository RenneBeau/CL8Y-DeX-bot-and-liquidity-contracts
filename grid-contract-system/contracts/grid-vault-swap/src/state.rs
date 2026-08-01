use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Uint128};
use cw_storage_plus::{Item, Map};

#[cw_serde]
pub struct Config {
    pub admin: Addr,
    pub pending_admin: Option<Addr>,
    pub pair: Addr,
    pub asset_tokens: [Addr; 2],
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
}

#[cw_serde]
pub struct PendingSwap {
    pub captured_twap: Decimal,
    pub balances: [Uint128; 2],
    pub pre_deviation_bps: u16,
    pub offer_index: u8,
    pub amount: Uint128,
    pub min_return: Uint128,
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const PENDING_SWAP: Item<PendingSwap> = Item::new("pending_swap");
pub const SHARES: Map<&str, Uint128> = Map::new("shares");
pub const TOTAL_SHARES: Item<Uint128> = Item::new("total_shares");
pub const PAUSED: Item<bool> = Item::new("paused");
