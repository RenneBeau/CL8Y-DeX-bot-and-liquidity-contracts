use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;
use cw_storage_plus::{Item, Map};

#[cw_serde]
pub struct Config {
    pub governance: Addr,
    pub registry: Addr,
    pub keeper: Addr,
    pub treasury: Addr,
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const VAULT_SHARES: Map<(&Addr, u64), u128> = Map::new("vault_shares");
