pub mod contract;
pub mod error;
#[cfg(feature = "mainnet")]
pub mod mainnet;
pub mod msg;
pub mod state;

pub use crate::error::ContractError;

#[cfg(not(feature = "library"))]
pub use crate::contract::{execute, instantiate, migrate, query, reply};
