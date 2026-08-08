//! Canonical fee-collector embedded in `mainnet` production-lock builds.

use cosmwasm_std::Addr;

use crate::error::ContractError;

const fn require_populated(value: &'static str) -> &'static str {
    if value.as_bytes().is_empty() {
        panic!("CL8Y_CANONICAL_FEE_COLLECTOR must not be empty");
    }
    value
}

pub const CANONICAL_FEE_COLLECTOR: &str = require_populated(env!("CL8Y_CANONICAL_FEE_COLLECTOR"));
pub const CANONICAL_FEE_REGISTRY: &str = require_populated(env!("CL8Y_CANONICAL_FEE_REGISTRY"));

pub fn assert_fee_registry_canonical_mainnet(registry: Option<&Addr>) -> Result<(), ContractError> {
    match registry {
        Some(registry) if registry.as_str() == CANONICAL_FEE_REGISTRY => Ok(()),
        _ => Err(ContractError::NonCanonicalAddress {
            field: "fee_registry",
            expected: CANONICAL_FEE_REGISTRY,
        }),
    }
}

pub fn assert_fee_collector_canonical_mainnet(
    collector: Option<&Addr>,
) -> Result<(), ContractError> {
    match collector {
        Some(collector) if collector.as_str() == CANONICAL_FEE_COLLECTOR => Ok(()),
        _ => Err(ContractError::NonCanonicalAddress {
            field: "fee_collector",
            expected: CANONICAL_FEE_COLLECTOR,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(value: &str) -> Addr {
        Addr::unchecked(value)
    }

    #[test]
    fn collector_pin_rejects_non_canonical_and_none() {
        assert!(
            assert_fee_collector_canonical_mainnet(Some(&addr(CANONICAL_FEE_COLLECTOR))).is_ok()
        );
        assert!(assert_fee_collector_canonical_mainnet(Some(&addr("terra1fake"))).is_err());
        assert!(assert_fee_collector_canonical_mainnet(None).is_err());
    }

    #[test]
    fn registry_pin_rejects_non_canonical_and_none() {
        assert!(assert_fee_registry_canonical_mainnet(Some(&addr(CANONICAL_FEE_REGISTRY))).is_ok());
        assert!(assert_fee_registry_canonical_mainnet(Some(&addr("terra1fake"))).is_err());
        assert!(assert_fee_registry_canonical_mainnet(None).is_err());
    }
}
