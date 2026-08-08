//! Canonical mainnet address for the `mainnet` production-lock feature.
//!
//! This vault has no proxy field (it fills orders on the CL8Y DEX directly and
//! pays its protocol fee straight to the fee-collector), so only the
//! `fee_collector` is pinned. The fee-collector is not yet deployed on
//! mainnet, so `CANONICAL_FEE_COLLECTOR` is `None`; once it is live, put its
//! address here and `instantiate` will reject any other collector, so a
//! deploying key cannot route the vault's fees to a personal address.

use cosmwasm_std::Addr;

use crate::error::ContractError;

/// Canonical fee-collector address on Terra Classic mainnet.
#[cfg(feature = "mainnet")]
pub const CANONICAL_FEE_COLLECTOR: Option<&str> = None;

/// Verify `collector` against a pinned canonical address.
///
/// The pin is active only when `canonical` is `Some`: a differing address or a
/// missing collector (`None`, which would stop the vault charging any protocol
/// fee) is rejected. While `canonical` is `None` (contract not yet deployed)
/// any value is accepted, so local/E2E dummy-address deployments keep working
/// without the feature.
fn assert_fee_collector_canonical(
    canonical: Option<&'static str>,
    collector: Option<&Addr>,
) -> Result<(), ContractError> {
    match (canonical, collector) {
        (Some(expected), Some(collector)) if collector.as_str() == expected => Ok(()),
        (Some(expected), _) => Err(ContractError::NonCanonicalAddress {
            field: "fee_collector",
            expected,
        }),
        (None, _) => Ok(()),
    }
}

/// The public entrypoint for the fee-collector pin.
#[cfg(feature = "mainnet")]
pub fn assert_fee_collector_canonical_mainnet(
    collector: Option<&Addr>,
) -> Result<(), ContractError> {
    assert_fee_collector_canonical(CANONICAL_FEE_COLLECTOR, collector)
}

#[cfg(feature = "mainnet")]
#[cfg(test)]
mod tests {
    use super::*;

    fn addr(value: &str) -> Addr {
        Addr::unchecked(value)
    }

    #[test]
    fn collector_pin_rejects_non_canonical_and_none() {
        let canonical = Some("terra1collector");
        assert!(assert_fee_collector_canonical(canonical, Some(&addr("terra1collector"))).is_ok());
        assert!(assert_fee_collector_canonical(canonical, Some(&addr("terra1fake"))).is_err());
        assert!(assert_fee_collector_canonical(canonical, None).is_err());
    }

    #[test]
    fn collector_pin_is_inert_while_unset() {
        assert!(assert_fee_collector_canonical(None, Some(&addr("terra1whatever"))).is_ok());
        assert!(assert_fee_collector_canonical(None, None).is_ok());
    }

    #[test]
    fn public_endpoint_uses_the_config_constant() {
        assert_eq!(
            assert_fee_collector_canonical_mainnet(Some(&addr("anything"))).is_ok(),
            CANONICAL_FEE_COLLECTOR.is_none()
        );
    }
}
