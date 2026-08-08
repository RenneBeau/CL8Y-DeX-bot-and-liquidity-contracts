pub mod contract;
pub mod error;
#[cfg(feature = "mainnet")]
pub mod mainnet;
pub mod msg;
pub mod state;

#[cfg(not(feature = "library"))]
pub use crate::contract::{execute, instantiate, migrate, query, reply};
