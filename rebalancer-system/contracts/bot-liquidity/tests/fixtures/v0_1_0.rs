use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Uint128};

pub const CONTRACT_INFO: &[u8] = include_bytes!("v0_1_0_contract_info.json");
pub const CONFIG: &[u8] = include_bytes!("v0_1_0_config.json");

// Copied from bot-liquidity 0.1.0 at commit
// 348090fd66159052d19b64825bd810bf2ba94b76. Do not replace with current state types.
#[cw_serde]
pub struct LegacyConfig {
    pub vault: Addr,
    pub asset_tokens: [Addr; 2],
    pub minimum_initial_deposit: Uint128,
}
