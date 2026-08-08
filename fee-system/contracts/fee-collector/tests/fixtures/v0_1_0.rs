use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;

pub const CONTRACT_INFO: &[u8] = include_bytes!("v0_1_0_contract_info.json");
pub const CONFIG: &[u8] = include_bytes!("v0_1_0_config.json");

// Copied from the initial fee-collector 0.1.0 at commit
// 54146c85a8fc0f75f10fd02558bbdeeb57fce0ea. Do not replace with current state types.
#[cw_serde]
pub struct LegacyConfig {
    pub governance: Addr,
    pub registry: Addr,
    pub keeper: Addr,
    pub treasury: Addr,
}
