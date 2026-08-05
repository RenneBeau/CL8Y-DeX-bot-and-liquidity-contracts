use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::Uint128;

#[cw_serde]
pub struct InstantiateMsg {
    pub governance: String,
    pub cl8y: String,
    pub treasury: String,
    pub fee_collector: String,
    pub base_fee_bps: u16,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Re-read the live CL8Y balance of `trader` and persist it as the saved
    /// holding. Permissionless (only ever reads an on-chain balance), but the
    /// vaults/keeper should call it so `EffectiveFee` always has a value to
    /// read back. On a CL8Y read failure the previous holding is kept.
    RefreshHolding { trader: String },
    AddTier {
        tier_id: u8,
        min_cl8y_balance: Uint128,
        discount_bps: u16,
        governance_only: bool,
    },
    UpdateTier {
        tier_id: u8,
        min_cl8y_balance: Option<Uint128>,
        discount_bps: Option<u16>,
        governance_only: Option<bool>,
    },
    RemoveTier {
        tier_id: u8,
    },
    UpdateConfig {
        governance: Option<String>,
        cl8y: Option<String>,
        treasury: Option<String>,
        fee_collector: Option<String>,
        base_fee_bps: Option<u16>,
    },
}

/// Where an effective fee came from: the live CL8Y balance, the saved holding
/// (live query failed), or the lowest tier (no data at all).
#[cw_serde]
pub enum TierSource {
    Live,
    Cached,
    Lowest,
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},
    #[returns(EffectiveFeeResponse)]
    EffectiveFee { trader: String },
    #[returns(HoldingResponse)]
    Holding { trader: String },
    #[returns(Vec<TierEntry>)]
    Tiers {},
    #[returns(TierEntry)]
    Tier { tier_id: u8 },
}

#[cw_serde]
pub struct ConfigResponse {
    pub governance: String,
    pub cl8y: String,
    pub treasury: String,
    pub fee_collector: String,
    pub base_fee_bps: u16,
    pub ladder_version: u32,
}

#[cw_serde]
pub struct EffectiveFeeResponse {
    pub fee_bps: u16,
    pub discount_bps: u16,
    pub tier_id: Option<u8>,
    pub holding: Option<Uint128>,
    pub source: TierSource,
}

#[cw_serde]
pub struct HoldingResponse {
    pub holding: Option<Uint128>,
    pub at_height: Option<u64>,
}

#[cw_serde]
pub struct TierEntry {
    pub tier_id: u8,
    pub min_cl8y_balance: Uint128,
    pub discount_bps: u16,
    pub governance_only: bool,
}

#[cw_serde]
pub struct MigrateMsg {}