use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Uint128};
use cw_storage_plus::Item;

#[cw_serde]
pub struct Config {
    pub vault: Addr,
    pub asset_tokens: [Addr; 2],
    pub minimum_initial_deposit: Uint128,
}

#[cw_serde]
pub enum PendingOperation {
    Deposit {
        depositor: Addr,
        pre_balances: [Uint128; 2],
        pre_supply: Uint128,
        price: Decimal,
        min_shares: Uint128,
    },
    WithdrawSingle {
        recipient: Addr,
        payout_token: Addr,
        base_amount: Uint128,
        pre_payout_balance: Uint128,
        min_amount: Uint128,
    },
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const PENDING: Item<PendingOperation> = Item::new("pending");
