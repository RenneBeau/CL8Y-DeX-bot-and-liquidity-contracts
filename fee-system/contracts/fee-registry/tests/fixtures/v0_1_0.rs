use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;

pub const CONTRACT_INFO: &[u8] = include_bytes!("v0_1_0_contract_info.json");
pub const CONFIG: &[u8] = include_bytes!("v0_1_0_config.json");

// Copied from the initial fee-registry 0.1.0 at commit
// 2a761204effff670421be81b6e7d81479f57e263. Do not replace with current state types.
#[cw_serde]
pub struct LegacyConfig {
    pub governance: Addr,
    pub cl8y: Addr,
    pub treasury: Addr,
    pub fee_collector: Addr,
    pub base_fee_bps: u16,
    pub ladder_version: u32,
}
