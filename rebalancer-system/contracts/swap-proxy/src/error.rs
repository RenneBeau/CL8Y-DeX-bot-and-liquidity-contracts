use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),
    #[error("unauthorized")]
    Unauthorized,
    #[error("invalid CL8Y pair or vault configuration")]
    InvalidRoute,
    #[error("vault is not registered")]
    UnregisteredVault,
    #[error("unsupported offer token")]
    UnsupportedToken,
    #[error("amount must be greater than zero")]
    ZeroAmount,
    #[error("deadline has expired")]
    Expired,
    #[error("maximum spread cannot exceed 10%")]
    ExcessiveSpread,
}
