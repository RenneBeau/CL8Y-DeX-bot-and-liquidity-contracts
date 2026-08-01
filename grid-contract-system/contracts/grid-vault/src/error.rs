use cosmwasm_std::{OverflowError, StdError};
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),
    #[error("{0}")]
    Overflow(#[from] OverflowError),
    #[error("unauthorized")]
    Unauthorized,
    #[error("invalid grid parameters")]
    InvalidGrid,
    #[error("invalid CL8Y pair")]
    InvalidPair,
    #[error("CL8Y pair implementation no longer matches the pinned code id")]
    PairCodeMismatch,
    #[error("only CW20 pair assets are supported")]
    UnsupportedAsset,
    #[error("unexpected native funds")]
    UnexpectedFunds,
    #[error("the configured gas denom must be funded")]
    MissingGasFunds,
    #[error("unsupported deposit token")]
    UnsupportedToken,
    #[error("token is not on the admin allowlist")]
    TokenNotAllowed,
    #[error("token is quarantined by the admin")]
    TokenQuarantined,
    #[error("amount must be greater than zero")]
    ZeroAmount,
    #[error("bot has no grids on this side of the reference price")]
    EmptySide,
    #[error("no free assets can be allocated")]
    NothingToAllocate,
    #[error("no order changes require reconciliation")]
    NothingToReconcile,
    #[error("bot gas credit is below the reimbursement reserve")]
    InsufficientGasCredit,
    #[error("CL8Y order does not belong to this contract")]
    InvalidOrderOwner,
    #[error("CL8Y order differs from recorded grid order")]
    InvalidOrder,
    #[error("limit placement reply did not contain all order ids")]
    InvalidPlacementReply,
    #[error("unknown reply id")]
    UnknownReply,
    #[error("insufficient free balance")]
    InsufficientBalance,
    #[error("cancel all active orders before withdrawing")]
    ActiveOrders,
    #[error("insufficient bot LP shares")]
    InsufficientShares,
    #[error("bot active-order limit reached")]
    ActiveOrderLimit,
    #[error("fill report does not match CL8Y order state")]
    InvalidFillReport,
    #[error("CW20 balance delta does not match the requested transfer")]
    UnsupportedTokenBehavior,
    #[error("order changed since its last indexed reconciliation")]
    UnsettledOrder,
    #[error("this dedicated vault already has its bot")]
    BotAlreadyExists,
    #[error("operation is disabled in the current vault mode")]
    InvalidMode,
    #[error("emergency exit still has tracked orders")]
    ExitOrdersRemain,
}
