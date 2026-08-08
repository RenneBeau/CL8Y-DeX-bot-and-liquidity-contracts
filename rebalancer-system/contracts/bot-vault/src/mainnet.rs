//! Canonical mainnet addresses for the `mainnet` production-lock feature.
//!
//! The fee-collector and the shared swap-proxy are not yet deployed on mainnet,
//! so these are `None`. Once they are live, put their addresses here: with the
//! `mainnet` feature enabled, `instantiate` rejects any other `fee_collector` /
//! `proxy` and the update messages refuse to re-point them, so a deploying or
//! admin key cannot wire the vault to a personal collector or callback proxy.

use cosmwasm_std::Addr;

use crate::error::ContractError;

/// Canonical fee-collector address on Terra Classic mainnet.
#[cfg(feature = "mainnet")]
pub const CANONICAL_FEE_COLLECTOR: Option<&str> = None;
/// Canonical shared swap-proxy address on Terra Classic mainnet.
#[cfg(feature = "mainnet")]
pub const CANONICAL_SWAP_PROXY: Option<&str> = None;

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

/// Verify `proxy` against a pinned canonical address.
fn assert_proxy_canonical(
    canonical: Option<&'static str>,
    proxy: &Addr,
) -> Result<(), ContractError> {
    match (canonical, proxy.as_str()) {
        (Some(expected), actual) if actual == expected => Ok(()),
        (Some(expected), _) => Err(ContractError::NonCanonicalAddress {
            field: "proxy",
            expected,
        }),
        (None, _) => Ok(()),
    }
}

/// The public entrypoint for the swap-proxy pin.
#[cfg(feature = "mainnet")]
pub fn assert_proxy_canonical_mainnet(proxy: &Addr) -> Result<(), ContractError> {
    assert_proxy_canonical(CANONICAL_SWAP_PROXY, proxy)
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
    fn proxy_pin_rejects_non_canonical_and_is_inert_while_unset() {
        let canonical = Some("terra1proxy");
        assert!(assert_proxy_canonical(canonical, &addr("terra1proxy")).is_ok());
        assert!(assert_proxy_canonical(canonical, &addr("terra1fake")).is_err());
        assert!(assert_proxy_canonical(None, &addr("terra1whatever")).is_ok());
    }

    #[test]
    fn public_endpoints_use_the_config_constants() {
        assert_eq!(
            assert_fee_collector_canonical_mainnet(Some(&addr("anything"))).is_ok(),
            CANONICAL_FEE_COLLECTOR.is_none()
        );
        assert_eq!(
            assert_proxy_canonical_mainnet(&addr("anything")).is_ok(),
            CANONICAL_SWAP_PROXY.is_none()
        );
    }
}
