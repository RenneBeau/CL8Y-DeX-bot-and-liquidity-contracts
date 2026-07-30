use cosmwasm_schema::cw_serde;

#[cw_serde]
pub struct InstantiateMsg {
    pub admin: String,
    pub keeper: String,
    pub proxy: String,
    pub pair: String,
    pub twap_window_seconds: u32,
    pub rebalance_threshold_bps: Option<u16>,
    pub allocation_tolerance_bps: Option<u16>,
}
