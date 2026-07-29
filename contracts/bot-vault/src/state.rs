use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal};
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
    pub reference_price: Decimal,
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const PENDING_REBALANCE_ALLOCATION: Item<u16> = Item::new("pending_rebalance_allocation");
