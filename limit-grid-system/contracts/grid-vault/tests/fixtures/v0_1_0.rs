use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Uint128};

pub const CONTRACT_INFO: &[u8] = include_bytes!("v0_1_0_contract_info.json");
pub const CONFIG: &[u8] = include_bytes!("v0_1_0_config.json");
pub const VAULT_MODE: &[u8] = include_bytes!("v0_1_0_vault_mode.json");

// Copied from grid-vault 0.1.0 at commit
// 921859b7bcfa9e6e80ffb78672474874332dd03d. Do not replace with current state types.
#[cw_serde]
pub struct LegacyConfig {
    pub admin: Addr,
    pub owner: Addr,
    pub pending_admin: Option<Addr>,
    pub keeper: Addr,
    pub factory: Addr,
    pub gas_denom: String,
    pub keeper_reward: Uint128,
    pub minimum_gas_reserve: Uint128,
    pub order_timeout_seconds: u64,
    pub max_grid_count: u32,
    pub max_orders_per_reconcile: u32,
    pub max_active_orders_per_bot: u32,
    pub fee_registry: Option<Addr>,
    pub fee_collector: Option<Addr>,
}

#[cw_serde]
pub enum LegacyVaultMode {
    Active,
    Paused,
    Exit,
}
