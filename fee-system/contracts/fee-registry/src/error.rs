use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Tier {tier_id} already exists")]
    TierAlreadyExists { tier_id: u8 },

    #[error("Tier {tier_id} not found")]
    TierNotFound { tier_id: u8 },

    #[error("Invalid discount_bps: {value} exceeds maximum of 10000")]
    InvalidDiscountBps { value: u16 },

    #[error("Tier ID {tier_id} is reserved for governance-only assignment")]
    ReservedTierId { tier_id: u8 },

    #[error("Effective fee underflow")]
    FeeUnderflow,
}
