use cosmwasm_std::StdError;
use cw_utils::ParseReplyError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),
    #[error("{0}")]
    ParseReply(#[from] ParseReplyError),
    #[error("unauthorized")]
    Unauthorized,
    #[error("unexpected native funds")]
    UnexpectedFunds,
    #[error("invalid manager configuration")]
    InvalidConfig,
    #[error("fee registry and fee collector must be configured together")]
    InvalidFeeConfig,
    #[error("unknown vault instantiation reply")]
    UnknownReply,
}
