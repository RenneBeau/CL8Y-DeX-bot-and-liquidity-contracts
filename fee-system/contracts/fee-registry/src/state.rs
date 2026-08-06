use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Uint128};
use cw_storage_plus::{Item, Map};

/// Registry configuration.
///
/// `base_fee_bps` is the protocol's flat per-fill fee before the CL8Y tier
/// discount. `cl8y` is the CL8Y (`cl8y-cb`) CW20 token whose holding determines
/// the discount; the registry is the only reader of that balance.
#[cw_serde]
pub struct Config {
    pub governance: Addr,
    pub cl8y: Addr,
    pub treasury: Addr,
    pub fee_collector: Addr,
    pub base_fee_bps: u16,
    /// Monotonic version of the tier ladder; bumped on any tier mutation so a
    /// saved holding maps to an auditable ladder revision.
    pub ladder_version: u32,
}

/// One discount tier of the historised CL8Y DEX ladder.
///
/// A holder's discount is the highest `discount_bps` among non-`governance_only`
/// tiers whose `min_cl8y_balance` is met. `governance_only` tiers (0 = market
/// makers, 255 = blacklist) are never auto-applied to a balance.
#[cw_serde]
pub struct Tier {
    pub min_cl8y_balance: Uint128,
    pub discount_bps: u16,
    pub governance_only: bool,
}

/// Last-known-good CL8Y holding, written from a *successful* live query and used
/// as fallback when a live query fails.
#[cw_serde]
pub struct Holding {
    pub amount: Uint128,
    pub at_height: u64,
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const TIERS: Map<u8, Tier> = Map::new("tiers");
pub const HOLDINGS: Map<&Addr, Holding> = Map::new("holdings");
