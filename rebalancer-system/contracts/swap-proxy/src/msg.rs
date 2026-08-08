use cosmwasm_schema::{cw_serde, QueryResponses};
use cw20::Cw20ReceiveMsg;

#[cw_serde]
pub struct InstantiateMsg {
    pub admin: String,
}

#[cw_serde]
pub struct MigrateMsg {}

#[cw_serde]
pub enum ExecuteMsg {
    Receive(Cw20ReceiveMsg),
    RegisterVault { vault: String, pair: String },
    RemoveVault { vault: String },
    TransferAdmin { admin: String },
    AcceptAdmin {},
    CancelAdminTransfer {},
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},
    #[returns(RouteResponse)]
    Route { vault: String },
}

#[cw_serde]
pub struct ConfigResponse {
    pub admin: String,
    pub pending_admin: Option<String>,
}

#[cw_serde]
pub struct RouteResponse {
    pub vault: String,
    pub pair: String,
    pub pair_code_id: u64,
    pub asset_tokens: [String; 2],
}
