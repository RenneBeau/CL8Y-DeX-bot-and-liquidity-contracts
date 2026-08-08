use cosmwasm_schema::cw_serde;
use cosmwasm_std::Decimal;

#[cw_serde]
pub struct InstantiateMsg {
    pub admin: String,
    pub keeper: String,
    pub proxy: String,
    pub factory: String,
    pub pair: String,
    pub pair_code_id: u64,
    pub liquidity_code_id: u64,
    pub twap_window_seconds: u32,
    pub rebalance_threshold_bps: Option<u16>,
    pub allocation_tolerance_bps: Option<u16>,
    pub max_trade_bps: Option<u16>,
    pub max_execution_deviation_bps: Option<u16>,
    pub quote_slippage_bps: Option<u16>,
    pub max_spot_twap_deviation_bps: Option<u16>,
    pub max_trade_pool_bps: Option<u16>,
    pub max_spread: Option<Decimal>,
    pub fee_registry: Option<String>,
    pub fee_collector: Option<String>,
}

#[cw_serde]
pub struct MigrateMsg {
    pub liquidity_code_id: u64,
}
