use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult,
    Uint128,
};
use cw2::set_contract_version;
use cw20::{BalanceResponse, Cw20QueryMsg};

use crate::error::ContractError;
use crate::msg::{
    ConfigResponse, EffectiveFeeResponse, ExecuteMsg, HoldingResponse, InstantiateMsg, MigrateMsg,
    QueryMsg, TierEntry, TierSource,
};
use crate::state::{Config, Holding, Tier, CONFIG, HOLDINGS, TIERS};

const CONTRACT_NAME: &str = "crates.io:cl8y-fee-registry";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

const MAX_BPS: u16 = 10_000;

/// Canonical Terra Classic mainnet addresses (docs/DEPLOY_FEE_SYSTEM.md §1,
/// docs/FEE_TIER_PROTOCOL.md §0). When the `mainnet` feature is enabled these
/// are pinned at compile time: instantiate REJECTS any other `cl8y`/`treasury`,
/// so a malicious deployer cannot wire a fake CL8Y token or a personal treasury,
/// and `update_config` refuses to re-point them.
#[cfg(feature = "mainnet")]
pub const CANONICAL_CL8Y: &str = "terra16wtml2q66g82fdkx66tap0qjkahqwp4lwq3ngtygacg5q0kzycgqvhpax3";
#[cfg(feature = "mainnet")]
pub const CANONICAL_TREASURY: &str =
    "terra16j5u6ey7a84g40sr3gd94nzg5w5fm45046k9s2347qhfpwm5fr6sem3lr2";

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    #[cfg(feature = "mainnet")]
    {
        if msg.cl8y != CANONICAL_CL8Y {
            return Err(ContractError::NonCanonicalAddress {
                field: "cl8y",
                expected: CANONICAL_CL8Y,
            });
        }
        if msg.treasury != CANONICAL_TREASURY {
            return Err(ContractError::NonCanonicalAddress {
                field: "treasury",
                expected: CANONICAL_TREASURY,
            });
        }
    }
    let governance = deps.api.addr_validate(&msg.governance)?;
    let cl8y = deps.api.addr_validate(&msg.cl8y)?;
    let treasury = deps.api.addr_validate(&msg.treasury)?;
    let fee_collector = deps.api.addr_validate(&msg.fee_collector)?;
    if msg.base_fee_bps > MAX_BPS {
        return Err(ContractError::InvalidDiscountBps {
            value: msg.base_fee_bps,
        });
    }
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    CONFIG.save(
        deps.storage,
        &Config {
            governance,
            cl8y,
            treasury,
            fee_collector,
            base_fee_bps: msg.base_fee_bps,
            ladder_version: 1,
        },
    )?;
    seed_standard_tiers(deps)?;
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("creator", info.sender))
}

/// Seeds the canonical CL8Y fee-discount ladder (CL8Y, 18 decimals) on
/// instantiate. Tiers 0 and 255 are governance-assigned (market makers /
/// blacklist) and are excluded from holder-based resolution.
fn seed_standard_tiers(deps: DepsMut) -> Result<(), ContractError> {
    let one_cl8y = Uint128::new(1_000_000_000_000_000_000u128);
    let tiers: [(u8, Uint128, u16, bool); 11] = [
        (0, Uint128::zero(), 10_000, true),
        (1, one_cl8y, 250, false),
        (2, one_cl8y * Uint128::new(5), 1_000, false),
        (3, one_cl8y * Uint128::new(20), 2_000, false),
        (4, one_cl8y * Uint128::new(75), 3_500, false),
        (5, one_cl8y * Uint128::new(200), 5_000, false),
        (6, one_cl8y * Uint128::new(500), 6_000, false),
        (7, one_cl8y * Uint128::new(1_500), 7_500, false),
        (8, one_cl8y * Uint128::new(3_500), 8_500, false),
        (9, one_cl8y * Uint128::new(7_500), 9_500, false),
        (255, Uint128::zero(), 0, true),
    ];
    for (tier_id, min_cl8y_balance, discount_bps, governance_only) in tiers {
        TIERS.save(
            deps.storage,
            tier_id,
            &Tier {
                min_cl8y_balance,
                discount_bps,
                governance_only,
            },
        )?;
    }
    Ok(())
}

#[entry_point]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::RefreshHolding { trader } => execute_refresh_holding(deps, env, info, trader),
        ExecuteMsg::AddTier {
            tier_id,
            min_cl8y_balance,
            discount_bps,
            governance_only,
        } => execute_add_tier(
            deps,
            info,
            tier_id,
            min_cl8y_balance,
            discount_bps,
            governance_only,
        ),
        ExecuteMsg::UpdateTier {
            tier_id,
            min_cl8y_balance,
            discount_bps,
            governance_only,
        } => execute_update_tier(
            deps,
            info,
            tier_id,
            min_cl8y_balance,
            discount_bps,
            governance_only,
        ),
        ExecuteMsg::RemoveTier { tier_id } => execute_remove_tier(deps, info, tier_id),
        ExecuteMsg::UpdateConfig {
            governance,
            cl8y,
            treasury,
            fee_collector,
            base_fee_bps,
        } => execute_update_config(
            deps,
            info,
            governance,
            cl8y,
            treasury,
            fee_collector,
            base_fee_bps,
        ),
    }
}

fn assert_governance(deps: &Deps, info: &MessageInfo) -> Result<(), ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.governance {
        return Err(ContractError::Unauthorized);
    }
    Ok(())
}

/// Permissionless: re-read the live CL8Y balance of `trader` and persist it as
/// the saved holding so `EffectiveFee` always has a value to read back. On a
/// read failure the previous holding is kept unchanged (never cleared).
fn execute_refresh_holding(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    trader: String,
) -> Result<Response, ContractError> {
    let trader = deps.api.addr_validate(&trader)?;
    let config = CONFIG.load(deps.storage)?;
    let live: StdResult<BalanceResponse> = deps.querier.query_wasm_smart(
        &config.cl8y,
        &Cw20QueryMsg::Balance {
            address: trader.to_string(),
        },
    );
    match live {
        Ok(balance) => {
            HOLDINGS.save(
                deps.storage,
                &trader,
                &Holding {
                    amount: balance.balance,
                    at_height: env.block.height,
                },
            )?;
            Ok(Response::new()
                .add_attribute("action", "refresh_holding")
                .add_attribute("trader", trader)
                .add_attribute("bumped", "true"))
        }
        Err(_) => Ok(Response::new()
            .add_attribute("action", "refresh_holding")
            .add_attribute("trader", trader)
            .add_attribute("bumped", "false")),
    }
}

/// Tiers 0 and 255 are reserved for governance-assigned market makers / blacklist
/// in the canonical CL8Y ladder; they must never auto-apply to a balance.
fn assert_not_reserved(tier_id: u8, governance_only: bool) -> Result<(), ContractError> {
    if (tier_id == 0 || tier_id == 255) && !governance_only {
        return Err(ContractError::ReservedTierId { tier_id });
    }
    Ok(())
}

fn execute_add_tier(
    deps: DepsMut,
    info: MessageInfo,
    tier_id: u8,
    min_cl8y_balance: Uint128,
    discount_bps: u16,
    governance_only: bool,
) -> Result<Response, ContractError> {
    assert_governance(&deps.as_ref(), &info)?;
    if discount_bps > MAX_BPS {
        return Err(ContractError::InvalidDiscountBps {
            value: discount_bps,
        });
    }
    assert_not_reserved(tier_id, governance_only)?;
    if TIERS.has(deps.storage, tier_id) {
        return Err(ContractError::TierAlreadyExists { tier_id });
    }
    TIERS.save(
        deps.storage,
        tier_id,
        &Tier {
            min_cl8y_balance,
            discount_bps,
            governance_only,
        },
    )?;
    bump_ladder_version(deps)?;
    Ok(Response::new()
        .add_attribute("action", "add_tier")
        .add_attribute("tier_id", tier_id.to_string())
        .add_attribute("discount_bps", discount_bps.to_string()))
}

fn execute_update_tier(
    deps: DepsMut,
    info: MessageInfo,
    tier_id: u8,
    min_cl8y_balance: Option<Uint128>,
    discount_bps: Option<u16>,
    governance_only: Option<bool>,
) -> Result<Response, ContractError> {
    assert_governance(&deps.as_ref(), &info)?;
    let mut tier = TIERS
        .may_load(deps.storage, tier_id)?
        .ok_or(ContractError::TierNotFound { tier_id })?;
    if let Some(amount) = min_cl8y_balance {
        tier.min_cl8y_balance = amount;
    }
    if let Some(bps) = discount_bps {
        if bps > MAX_BPS {
            return Err(ContractError::InvalidDiscountBps { value: bps });
        }
        tier.discount_bps = bps;
    }
    if let Some(gov_only) = governance_only {
        assert_not_reserved(tier_id, gov_only)?;
        tier.governance_only = gov_only;
    }
    TIERS.save(deps.storage, tier_id, &tier)?;
    bump_ladder_version(deps)?;
    Ok(Response::new()
        .add_attribute("action", "update_tier")
        .add_attribute("tier_id", tier_id.to_string()))
}

fn execute_remove_tier(
    deps: DepsMut,
    info: MessageInfo,
    tier_id: u8,
) -> Result<Response, ContractError> {
    assert_governance(&deps.as_ref(), &info)?;
    if !TIERS.has(deps.storage, tier_id) {
        return Err(ContractError::TierNotFound { tier_id });
    }
    TIERS.remove(deps.storage, tier_id);
    bump_ladder_version(deps)?;
    Ok(Response::new()
        .add_attribute("action", "remove_tier")
        .add_attribute("tier_id", tier_id.to_string()))
}

fn execute_update_config(
    deps: DepsMut,
    info: MessageInfo,
    governance: Option<String>,
    cl8y: Option<String>,
    treasury: Option<String>,
    fee_collector: Option<String>,
    base_fee_bps: Option<u16>,
) -> Result<Response, ContractError> {
    assert_governance(&deps.as_ref(), &info)?;
    let mut config = CONFIG.load(deps.storage)?;
    // `cl8y` and `treasury` are pinned to the canonical mainnet addresses when
    // the `mainnet` feature is enabled; any attempt to re-point them is a
    // hard error (the other config fields remain governance-updatable).
    #[cfg(feature = "mainnet")]
    {
        if let Some(address) = &cl8y {
            if address != CANONICAL_CL8Y {
                return Err(ContractError::NonCanonicalAddress {
                    field: "cl8y",
                    expected: CANONICAL_CL8Y,
                });
            }
        }
        if let Some(address) = &treasury {
            if address != CANONICAL_TREASURY {
                return Err(ContractError::NonCanonicalAddress {
                    field: "treasury",
                    expected: CANONICAL_TREASURY,
                });
            }
        }
    }
    if let Some(address) = governance {
        config.governance = deps.api.addr_validate(&address)?;
    }
    if let Some(address) = cl8y {
        config.cl8y = deps.api.addr_validate(&address)?;
    }
    if let Some(address) = treasury {
        config.treasury = deps.api.addr_validate(&address)?;
    }
    if let Some(address) = fee_collector {
        config.fee_collector = deps.api.addr_validate(&address)?;
    }
    if let Some(bps) = base_fee_bps {
        if bps > MAX_BPS {
            return Err(ContractError::InvalidDiscountBps { value: bps });
        }
        config.base_fee_bps = bps;
    }
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new().add_attribute("action", "update_config"))
}

fn bump_ladder_version(deps: DepsMut) -> StdResult<()> {
    CONFIG.update(deps.storage, |mut config| -> StdResult<_> {
        config.ladder_version = config
            .ladder_version
            .checked_add(1)
            .ok_or_else(|| cosmwasm_std::StdError::generic_err("ladder version overflow"))?;
        Ok(config)
    })?;
    Ok(())
}

#[entry_point]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&query_config(deps)?),
        QueryMsg::EffectiveFee { trader } => {
            to_json_binary(&query_effective_fee(deps, env, trader)?)
        }
        QueryMsg::Holding { trader } => to_json_binary(&query_holding(deps, trader)?),
        QueryMsg::Tiers {} => to_json_binary(&query_tiers(deps)?),
        QueryMsg::Tier { tier_id } => to_json_binary(&query_tier(deps, tier_id)?),
    }
}

fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config = CONFIG.load(deps.storage)?;
    Ok(ConfigResponse {
        governance: config.governance.to_string(),
        cl8y: config.cl8y.to_string(),
        treasury: config.treasury.to_string(),
        fee_collector: config.fee_collector.to_string(),
        base_fee_bps: config.base_fee_bps,
        ladder_version: config.ladder_version,
    })
}

fn query_holding(deps: Deps, trader: String) -> StdResult<HoldingResponse> {
    let trader = deps.api.addr_validate(&trader)?;
    let holding = HOLDINGS.may_load(deps.storage, &trader)?;
    Ok(HoldingResponse {
        holding: holding.as_ref().map(|h| h.amount),
        at_height: holding.map(|h| h.at_height),
    })
}

fn query_tiers(deps: Deps) -> StdResult<Vec<TierEntry>> {
    TIERS
        .range(deps.storage, None, None, cosmwasm_std::Order::Ascending)
        .map(|item| {
            let (tier_id, tier) = item?;
            Ok(TierEntry {
                tier_id,
                min_cl8y_balance: tier.min_cl8y_balance,
                discount_bps: tier.discount_bps,
                governance_only: tier.governance_only,
            })
        })
        .collect()
}

fn query_tier(deps: Deps, tier_id: u8) -> StdResult<TierEntry> {
    let tier = TIERS
        .may_load(deps.storage, tier_id)?
        .ok_or_else(|| cosmwasm_std::StdError::generic_err(format!("tier {tier_id} not found")))?;
    Ok(TierEntry {
        tier_id,
        min_cl8y_balance: tier.min_cl8y_balance,
        discount_bps: tier.discount_bps,
        governance_only: tier.governance_only,
    })
}

/// Highest discount among non-governance tiers whose minimum is met. A holder
/// with no eligible tier gets `discount_bps = 0` (full base fee).
fn resolve_discount(deps: Deps, amount: Uint128) -> StdResult<(u16, Option<u8>)> {
    let mut best: (u16, Option<u8>) = (0, None);
    for item in TIERS.range(deps.storage, None, None, cosmwasm_std::Order::Ascending) {
        let (tier_id, tier) = item?;
        if tier.governance_only {
            continue;
        }
        if amount >= tier.min_cl8y_balance && tier.discount_bps > best.0 {
            best = (tier.discount_bps, Some(tier_id));
        }
    }
    Ok(best)
}

fn query_effective_fee(deps: Deps, _env: Env, trader: String) -> StdResult<EffectiveFeeResponse> {
    let trader = deps.api.addr_validate(&trader)?;
    let config = CONFIG.load(deps.storage)?;

    // Live-first: a fee must reflect the holder's current balance, never a stale
    // snapshot. The persisted holding (written via `RefreshHolding`) is only a
    // fallback for a transient live-read failure, and the lowest tier (full base
    // fee) is used when neither is available, so a holder is never under-fee.
    let (discount_bps, tier_id, holding, source) = match deps.querier.query_wasm_smart(
        &config.cl8y,
        &Cw20QueryMsg::Balance {
            address: trader.to_string(),
        },
    ) {
        Ok(BalanceResponse { balance }) => {
            let (discount, tier) = resolve_discount(deps, balance)?;
            (discount, tier, Some(balance), TierSource::Live)
        }
        Err(_) => match HOLDINGS.may_load(deps.storage, &trader)? {
            Some(saved) => {
                let (discount, tier) = resolve_discount(deps, saved.amount)?;
                (discount, tier, Some(saved.amount), TierSource::Cached)
            }
            None => (0, None, None, TierSource::Lowest),
        },
    };

    let fee_bps = effective_fee(config.base_fee_bps, discount_bps);
    Ok(EffectiveFeeResponse {
        fee_bps,
        discount_bps,
        tier_id,
        holding,
        source,
    })
}

fn effective_fee(base_bps: u16, discount_bps: u16) -> u16 {
    let discounted = u32::from(MAX_BPS) - u32::from(discount_bps);
    (u32::from(base_bps) * discounted / u32::from(MAX_BPS)) as u16
}

#[entry_point]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    cw2::ensure_from_older_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    Ok(Response::new().add_attribute("action", "migrate"))
}
