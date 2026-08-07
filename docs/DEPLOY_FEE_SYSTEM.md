# Fee-System Production Deployment

Records the **production (Terra Classic mainnet)** addresses and the exact
instantiation / configuration flow for the protocol fee system: `fee-registry`
(user-differentiated fee tiers) and `fee-collector` (fee realization to treasury).

This file is the **source of truth for mainnet**. Local test-a-net runs must use
the dummy addresses in `test-area/`, because the mainnet CL8Y token and CMM
treasury do not exist on a local chain.

## 1. Deployed addresses (Terra Classic mainnet)

| Role | Address |
|---|---|
| `cl8y` — CL8Y (`cl8y-cb`) CW20 token | `terra16wtml2q66g82fdkx66tap0qjkahqwp4lwq3ngtygacg5q0kzycgqvhpax3` |
| Treasury — CMM treasury | `terra16j5u6ey7a84g40sr3gd94nzg5w5fm45046k9s2347qhfpwm5fr6sem3lr2` |

Both are valid 32-byte (contract-style) bech32 addresses; they are the same values
referenced in `docs/FEE_TIER_PROTOCOL.md` §0.

> Contract addresses are produced by `terrad tx wasm instantiate`. They are
> deterministic per (deployer, code_id, `InstantiateMsg`); fill the three
> contract addresses below at deploy time.

## 2. Instantiate the fee-registry

```
terrad tx wasm instantiate \
  <FEE_REGISTRY_CODE_ID> \
  '{
    "governance": "<GOVERNANCE_ADMIN>",
    "cl8y": "terra16wtml2q66g82fdkx66tap0qjkahqwp4lwq3ngtygacg5q0kzycgqvhpax3",
    "treasury": "terra16j5u6ey7a84g40sr3gd94nzg5w5fm45046k9s2347qhfpwm5fr6sem3lr2",
    "fee_collector": "<FEE_COLLECTOR_ADDRESS>",
    "base_fee_bps": 1800
  }' \
  --label cl8y-fee-registry
```

`cl8y` and `treasury` are the exact mainnet addresses from §1. `base_fee_bps` is
18% (1800). Instantiation also seeds the canonical tier ladder; governance tiers
(0/255) are assigned via `add_tier`.

## 3. Instantiate the fee-collector

```
terrad tx wasm instantiate \
  <FEE_COLLECTOR_CODE_ID> \
  '{
    "governance": "<GOVERNANCE_ADMIN>",
    "registry": "<FEE_REGISTRY_ADDRESS>",
    "keeper": "<KEEPER_ADDRESS>",
    "treasury": "terra16j5u6ey7a84g40sr3gd94nzg5w5fm45046k9s2347qhfpwm5fr6sem3lr2"
  }' \
  --label cl8y-fee-collector \
  --admin "<GOVERNANCE_ADMIN>"
```

`treasury` is the exact mainnet address from §1.

## 4. Wire each vault to the fee pair

Every limit-grid `grid-vault`, market-grid `grid-vault-swap`, and rebalancer
`bot-vault` needs `fee_registry` + `fee_collector` set. Via governance:

```
POST {"update_fee_config":{
  "fee_registry": "<FEE_REGISTRY_ADDRESS>",
  "fee_collector": "<FEE_COLLECTOR_ADDRESS>"
}}
```

Fees are then charged to the **single operating user** at their CL8Y tier (mint
`V − fee` LP to the user, `fee` LP to the fee-collector; see
`docs/FEE_TIER_PROTOCOL.md` §5); the `fee-collector` realizes the accrued LP to
the CMM `treasury` (via `RedeemShares` on the grids, or an external
`bot-liquidity` `Withdraw` on the rebalancer).

## 5. Environments

| Key | Mainnet | Local (`test-area`) |
|---|---|---|
| fee-registry | prod address | `VITE_FEE_DISCOUNT_ADDRESS` (mock) |
| fee-collector | prod address | dummy contract |
| treasury | CMM (table §1) | test funding key |
| cl8y | CL8Y (table §1) | local mock token |

Supported overrides in `test-area/deploy-system.sh` are `CL8Y_FEE_REGISTRY`,
`CL8Y_FEE_TREASURY`, and `CL8Y_FEE_COLLECTOR`; leave unset for the dummy
fallback on a local chain.