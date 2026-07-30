use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::Uint128;

#[cw_serde]
pub struct InstantiateMsg {
    pub admin: String,
    pub keeper: String,
    pub dex_factory: String,
    pub vault_code_id: u64,
    pub gas_denom: String,
    pub keeper_reward: Uint128,
    pub minimum_gas_reserve: Uint128,
    pub order_timeout_seconds: u64,
    pub max_grid_count: u32,
    pub max_orders_per_reconcile: u32,
    pub max_active_orders_per_vault: u32,
}

#[cw_serde]
pub enum ExecuteMsg {
    CreateVault {
        label: Option<String>,
    },
    UpdateConfig {
        keeper: Option<String>,
        vault_code_id: Option<u64>,
    },
    TransferAdmin {
        admin: String,
    },
    AcceptAdmin {},
}

#[cw_serde]
pub struct VaultInstantiateMsg {
    pub admin: String,
    pub owner: String,
    pub keeper: String,
    pub factory: String,
    pub gas_denom: String,
    pub keeper_reward: Uint128,
    pub minimum_gas_reserve: Uint128,
    pub order_timeout_seconds: u64,
    pub max_grid_count: u32,
    pub max_orders_per_reconcile: u32,
    pub max_active_orders_per_bot: u32,
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},
    #[returns(Option<VaultResponse>)]
    Vault { vault_id: u64 },
    #[returns(Vec<VaultResponse>)]
    VaultsByOwner { owner: String },
}

#[cw_serde]
pub struct ConfigResponse {
    pub admin: String,
    pub pending_admin: Option<String>,
    pub keeper: String,
    pub dex_factory: String,
    pub vault_code_id: u64,
    pub gas_denom: String,
    pub keeper_reward: Uint128,
    pub minimum_gas_reserve: Uint128,
    pub order_timeout_seconds: u64,
    pub max_grid_count: u32,
    pub max_orders_per_reconcile: u32,
    pub max_active_orders_per_vault: u32,
}

#[cw_serde]
pub struct VaultResponse {
    pub vault_id: u64,
    pub owner: String,
    pub address: String,
}
