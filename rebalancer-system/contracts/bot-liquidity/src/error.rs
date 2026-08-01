use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),
    #[error("CW20 operation failed: {0}")]
    Cw20(String),
    #[error("another liquidity operation is pending")]
    OperationPending,
    #[error("amount must be greater than zero")]
    ZeroAmount,
    #[error("deadline has expired")]
    Expired,
    #[error("deposit swap can spend only the deposited offer token")]
    InvalidDepositSwap,
    #[error("withdrawal swap must spend exactly the proportional unwanted token claim")]
    InvalidWithdrawalSwap,
    #[error("vault balance decreased below its pre-deposit balance")]
    InvalidDepositSettlement,
    #[error("deposit or withdrawal produced less than the user minimum")]
    MinimumNotMet,
    #[error("initial deposit is too small")]
    InitialDepositTooSmall,
    #[error("minimum initial deposit must exceed permanently locked initial shares")]
    InvalidMinimumInitialDeposit,
    #[error("minimum initial deposit cannot change after bootstrap")]
    BootstrapComplete,
    #[error("deposit minted zero shares")]
    ZeroShares,
    #[error("vault allocation exceeds its configured tolerance")]
    AllocationOutsideTolerance,
    #[error("insufficient shares")]
    InsufficientShares,
    #[error("unauthorized")]
    Unauthorized,
    #[error("invalid vault configuration")]
    InvalidVault,
    #[error("unknown reply id")]
    UnknownReply,
}
