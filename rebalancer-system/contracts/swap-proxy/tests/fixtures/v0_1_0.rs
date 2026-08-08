use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;

pub const CONTRACT_INFO: &[u8] = include_bytes!("v0_1_0_contract_info.json");
pub const CONFIG: &[u8] = include_bytes!("v0_1_0_config.json");
pub const ROUTE: &[u8] = include_bytes!("v0_1_0_route.json");
pub const ROUTE_KEY: &[u8] = b"\0\x06routesvault";

// Copied from swap-proxy 0.1.0 at commit
// 921859b7bcfa9e6e80ffb78672474874332dd03d. Do not replace with current state types.
#[cw_serde]
pub struct LegacyConfig {
    pub admin: Addr,
}

#[cw_serde]
pub struct LegacyRoute {
    pub pair: Addr,
    pub asset_tokens: [Addr; 2],
}
