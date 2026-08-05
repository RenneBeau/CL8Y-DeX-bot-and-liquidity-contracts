use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::Uint128;

#[cw_serde]
pub struct InstantiateMsg {
    pub governance: String,
    pub registry: String,
    pub keeper: String,
    pub treasury: String,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Keeper-only. Trusts `registry` to compute the fee; mints the collector's
    /// share of the vault's LP for (vault, bot_id) at the current fee rate.
    Collect { vault: String, bot_id: u64 },
    UpdateConfig {
        governance: Option<String>,
        registry: Option<String>,
        keeper: Option<String>,
        treasury: Option<String>,
    },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},
    #[returns(VaultSharesResponse)]
    VaultShares { vault: String, bot_id: u64 },
}

#[cw_serde]
pub struct ConfigResponse {
    pub governance: String,
    pub registry: String,
    pub keeper: String,
    pub treasury: String,
}

#[cw_serde]
pub struct VaultSharesResponse {
    pub vault: String,
    pub bot_id: u64,
    pub shares: Uint128,
}

#[cw_serde]
pub struct MigrateMsg {}