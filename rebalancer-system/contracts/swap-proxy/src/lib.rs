pub mod contract;
pub mod error;
pub mod msg;
pub mod state;

#[cfg(not(feature = "library"))]
pub use crate::contract::{execute, instantiate, migrate, query};
