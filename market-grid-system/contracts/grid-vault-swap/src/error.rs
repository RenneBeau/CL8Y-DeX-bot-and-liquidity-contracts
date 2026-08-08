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
    #[error("unsupported token")]
    UnsupportedToken,
    #[error("amount must be greater than zero")]
    ZeroAmount,
    #[error("deadline has expired")]
    Expired,
    #[error("maximum spread cannot exceed 10%")]
    ExcessiveSpread,
    #[error("grid parameters must have an upper price above the lower price")]
    InvalidGrid,
    #[error("grid count must be greater than zero")]
    InvalidGridCount,
    #[error("TWAP window must be greater than zero")]
    InvalidTwapWindow,
    #[error("risk control exceeds its hard safety bound")]
    InvalidRiskControl,
    #[error("grid allocation did not improve")]
    AllocationDidNotImprove,
    #[error("grid rebalance is not safely bounded")]
    InvalidRebalanceSwap,
    #[error("pool spot price deviates too far from TWAP")]
    UnsafePoolPrice,
    #[error("grid trade is too large relative to pool depth")]
    InsufficientPoolDepth,
    #[error("grid allocation exceeds configured tolerance")]
    AllocationOutsideTolerance,
    #[error("another rebalance is pending")]
    RebalancePending,
    #[error("missing pending rebalance")]
    MissingPendingRebalance,
    #[error("unknown reply id")]
    UnknownReply,
    #[error("pool or price history is empty")]
    EmptyPrice,
    #[error("grid rebalance is not required")]
    RebalanceNotRequired,
    #[error("insufficient shares")]
    InsufficientShares,
    #[error("contract is paused")]
    Paused,
    #[error("contract is not paused")]
    NotPaused,
    #[error("non-canonical address for {field}; expected {expected}")]
    NonCanonicalAddress {
        field: &'static str,
        expected: &'static str,
    },
}
