use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Uint128};
use cw_storage_plus::{Item, Map};

use crate::msg::LimitOrderSide;

#[cw_serde]
pub struct Config {
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
}

#[cw_serde]
pub enum VaultMode {
    Active,
    Paused,
    Exit,
}

#[cw_serde]
pub struct Bot {
    pub owner: Addr,
    pub pair: Addr,
    pub pair_code_id: u64,
    pub asset_tokens: [Addr; 2],
    pub lower_price: Decimal,
    pub upper_price: Decimal,
    pub grid_count: u32,
    pub reference_price: Decimal,
    pub free_balances: [Uint128; 2],
    pub total_shares: Uint128,
    pub gas_credit: Uint128,
    pub active_orders: u32,
    pub pair_batch_limit: u32,
}

#[cw_serde]
pub struct Rung {
    pub price: Decimal,
    pub side: Option<LimitOrderSide>,
}

#[cw_serde]
pub struct GridOrder {
    pub rung_index: u32,
    pub side: LimitOrderSide,
    pub price: Decimal,
    pub remaining: Uint128,
}

#[cw_serde]
pub struct PlacementPlan {
    pub bot_id: u64,
    pub side: LimitOrderSide,
    pub rungs: Vec<u32>,
    pub gross_amounts: Vec<Uint128>,
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const VAULT_MODE: Item<VaultMode> = Item::new("vault_mode");
pub const NEXT_BOT_ID: Item<u64> = Item::new("next_bot_id");
pub const NEXT_REPLY_ID: Item<u64> = Item::new("next_reply_id");
pub const BOTS: Map<u64, Bot> = Map::new("bots");
pub const RUNGS: Map<(u64, u32), Rung> = Map::new("rungs");
pub const ORDERS: Map<(u64, u64), GridOrder> = Map::new("orders");
pub const SHARES: Map<(u64, &Addr), Uint128> = Map::new("shares");
pub const PLACEMENTS: Map<u64, PlacementPlan> = Map::new("placements");
pub const ALLOWED_TOKENS: Map<&Addr, ()> = Map::new("allowed_tokens");
pub const TOKEN_POLICY_ENABLED: Item<bool> = Item::new("token_policy_enabled");
pub const QUARANTINE: Map<&Addr, ()> = Map::new("quarantine");

#[cw_serde]
pub enum PageKind {
    Cancel,
    Claim,
}

#[cw_serde]
pub struct PendingPageEntry {
    pub order_id: u64,
    pub token_index: u8,
    pub refund: Uint128,
}

#[cw_serde]
pub struct PendingPage {
    pub bot_id: u64,
    pub kind: PageKind,
    pub entries: Vec<PendingPageEntry>,
}

pub const PENDING_PAGES: Map<u64, PendingPage> = Map::new("pending_pages");
