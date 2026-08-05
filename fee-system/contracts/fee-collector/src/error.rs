use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Vault {vault} reported {shares} shares for the collector; nothing to collect")]
    NoEntitlement { vault: String, shares: u128 },
}
