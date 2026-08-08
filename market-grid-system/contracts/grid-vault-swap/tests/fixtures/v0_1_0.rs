use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal};

pub const CONTRACT_INFO: &[u8] = include_bytes!("v0_1_0_contract_info.json");
pub const CONFIG: &[u8] = include_bytes!("v0_1_0_config.json");

// Copied from grid-vault-swap 0.1.0 at commit
// 921859b7bcfa9e6e80ffb78672474874332dd03d. Do not replace with current state types.
#[cw_serde]
pub struct LegacyConfig {
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
    pub fee_registry: Option<Addr>,
    pub fee_collector: Option<Addr>,
    pub proxy: Option<Addr>,
}
