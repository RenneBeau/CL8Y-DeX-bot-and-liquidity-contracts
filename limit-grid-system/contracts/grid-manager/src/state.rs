use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Uint128};
use cw_storage_plus::{Item, Map};

#[cw_serde]
pub struct Config {
    pub admin: Addr,
    pub pending_admin: Option<Addr>,
    pub keeper: Addr,
    pub dex_factory: Addr,
    pub vault_code_id: u64,
    pub gas_denom: String,
    pub keeper_reward: Uint128,
    pub minimum_gas_reserve: Uint128,
    pub order_timeout_seconds: u64,
    pub max_grid_count: u32,
    pub max_orders_per_reconcile: u32,
    pub max_active_orders_per_vault: u32,
    #[serde(default)]
    pub fee_registry: Option<Addr>,
    #[serde(default)]
    pub fee_collector: Option<Addr>,
}

#[cw_serde]
pub struct PendingVault {
    pub vault_id: u64,
    pub owner: Addr,
}

#[cw_serde]
pub struct Vault {
    pub owner: Addr,
    pub address: Addr,
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const NEXT_VAULT_ID: Item<u64> = Item::new("next_vault_id");
pub const PENDING_VAULTS: Map<u64, PendingVault> = Map::new("pending_vaults");
pub const VAULTS: Map<u64, Vault> = Map::new("vaults");
pub const OWNER_VAULTS: Map<(&Addr, u64), Addr> = Map::new("owner_vaults");
