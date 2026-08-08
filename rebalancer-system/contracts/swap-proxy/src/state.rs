use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;
use cw_storage_plus::{Item, Map};

#[cw_serde]
pub struct Config {
    pub admin: Addr,
}

#[cw_serde]
pub struct Route {
    pub pair: Addr,
    pub asset_tokens: [Addr; 2],
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const ROUTES: Map<&Addr, Route> = Map::new("routes");
pub const PAIR_VAULTS: Map<&Addr, Addr> = Map::new("pair_vaults");
pub const PENDING_ADMIN: Item<Addr> = Item::new("pending_admin");
