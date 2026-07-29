use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),
    #[error("unauthorized")]
    Unauthorized,
    #[error("invalid CL8Y pair")]
    InvalidPair,
    #[error("pool assets must use the same decimals")]
    DecimalMismatch,
    #[error("liquidity contract is already configured")]
    LiquidityAlreadyConfigured,
    #[error("liquidity contract is not configured")]
    LiquidityNotConfigured,
    #[error("unsupported token")]
    UnsupportedToken,
    #[error("amount must be greater than zero")]
    ZeroAmount,
    #[error("deadline has expired")]
    Expired,
    #[error("maximum spread cannot exceed 10%")]
    ExcessiveSpread,
    #[error("threshold must be between 1 and 10,000 basis points")]
    InvalidThreshold,
    #[error("pool price has not moved enough to rebalance")]
    RebalanceNotRequired,
    #[error("vault allocation did not improve")]
    AllocationDidNotImprove,
    #[error("vault allocation exceeds configured tolerance")]
    AllocationOutsideTolerance,
    #[error("another rebalance is pending")]
    RebalancePending,
    #[error("missing pending rebalance")]
    MissingPendingRebalance,
    #[error("unknown reply id")]
    UnknownReply,
    #[error("pool or price history is empty")]
    EmptyPrice,
}
