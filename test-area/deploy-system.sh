#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=common.sh
source "$SCRIPT_DIR/common.sh"
ensure_dex_repo

DEX_FRONTEND_ENV="$DEX_DIR/frontend-dapp/.env.local"
if [ ! -f "$DEX_FRONTEND_ENV" ]; then
    echo "ERROR: CL8Y minimal deployment is missing. Run 'make local-setup'." >&2
    exit 1
fi
set -a
# shellcheck source=/dev/null
source "$DEX_FRONTEND_ENV"
set +a

EMBER_ADDRESS="$VITE_TOKEN_EMBER_ADDRESS"
CORAL_ADDRESS="$VITE_TOKEN_CORAL_ADDRESS"
CL8Y_ADDRESS="$VITE_CL8Y_TOKEN_ADDRESS"
FEE_REGISTRY_ADDRESS="$VITE_FEE_DISCOUNT_ADDRESS"
PAIR_QUERY=$(jq -nc --arg ember "$EMBER_ADDRESS" --arg coral "$CORAL_ADDRESS" \
    '{pair:{asset_infos:[{token:{contract_addr:$ember}},{token:{contract_addr:$coral}}]}}')
PAIR_ADDRESS=$(terrad_query wasm contract-state smart "$VITE_FACTORY_ADDRESS" "$PAIR_QUERY" \
    | jq -r '.data.pair.contract_addr // empty')
if [ -z "$PAIR_ADDRESS" ]; then
    echo "ERROR: CL8Y factory did not return the EMBER/CORAL pair." >&2
    exit 1
fi

echo "Building clean proxy, vault, and liquidity Wasm artifacts..."
docker run --rm \
    -v "$PROJECT_ROOT:/code" \
    --mount type=volume,source=cl8y_bot_target_cache,target=/code/target \
    --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
    cosmwasm/workspace-optimizer:0.16.1

echo "Building the isolated grid-manager Wasm artifact..."
docker run --rm \
    -v "$PROJECT_ROOT/grid-contract-system:/code" \
    --mount type=volume,source=cl8y_grid_target_cache,target=/code/target \
    --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
    cosmwasm/workspace-optimizer:0.16.1

CONTAINER=$(localterra_container)
store_contract() {
    local artifact="$1"
    local tx_hash result
    docker cp "$PROJECT_ROOT/artifacts/$artifact.wasm" "$CONTAINER:/tmp/$artifact.wasm"
    tx_hash=$(terrad_tx wasm store "/tmp/$artifact.wasm" | jq -r '.txhash')
    result=$(wait_tx "$tx_hash")
    tx_event_value "$result" code_id
}

PROXY_CODE_ID=$(store_contract cl8y_swap_proxy)
VAULT_CODE_ID=$(store_contract cl8y_bot_vault)
LIQUIDITY_CODE_ID=$(store_contract cl8y_bot_liquidity)
docker cp "$PROJECT_ROOT/grid-contract-system/artifacts/cl8y_grid_manager.wasm" \
    "$CONTAINER:/tmp/cl8y_grid_manager.wasm"
TX_HASH=$(terrad_tx wasm store /tmp/cl8y_grid_manager.wasm | jq -r '.txhash')
TX_RESULT=$(wait_tx "$TX_HASH")
GRID_CODE_ID=$(tx_event_value "$TX_RESULT" code_id)

instantiate_contract() {
    local code_id="$1"
    local message="$2"
    local label="$3"
    local tx_hash result
    tx_hash=$(terrad_tx wasm instantiate "$code_id" "$message" \
        --label "$label" --admin "$TEST_ADDRESS" | jq -r '.txhash')
    result=$(wait_tx "$tx_hash")
    tx_event_value "$result" contract_address
}

PROXY_INIT=$(jq -nc --arg admin "$TEST_ADDRESS" --arg cl8y "$CL8Y_ADDRESS" \
    --arg registry "$FEE_REGISTRY_ADDRESS" \
    '{admin:$admin,cl8y_token:$cl8y,fee_registry:$registry}')
PROXY_ADDRESS=$(instantiate_contract "$PROXY_CODE_ID" "$PROXY_INIT" cl8y-shared-swap-proxy)

VAULT_INIT=$(jq -nc --arg admin "$TEST_ADDRESS" --arg keeper "$TEST_ADDRESS" \
    --arg proxy "$PROXY_ADDRESS" --arg pair "$PAIR_ADDRESS" \
    '{admin:$admin,keeper:$keeper,proxy:$proxy,pair:$pair,twap_window_seconds:0,
      rebalance_threshold_bps:500,allocation_tolerance_bps:500}')
VAULT_ADDRESS=$(instantiate_contract "$VAULT_CODE_ID" "$VAULT_INIT" ember-coral-bot-vault)

LIQUIDITY_INIT=$(jq -nc --arg vault "$VAULT_ADDRESS" \
    '{vault:$vault,name:"EMBER CORAL Bot Liquidity",
      symbol:"ECBOTLP",decimals:6,minimum_initial_deposit:"100000",marketing:null}')
LIQUIDITY_ADDRESS=$(instantiate_contract "$LIQUIDITY_CODE_ID" "$LIQUIDITY_INIT" ember-coral-bot-liquidity)

GRID_INIT=$(jq -nc --arg admin "$TEST_ADDRESS" --arg keeper "$TEST_ADDRESS" \
    --arg factory "$VITE_FACTORY_ADDRESS" \
    '{admin:$admin,keeper:$keeper,factory:$factory,gas_denom:"uluna",
      keeper_reward:"30000000",minimum_gas_reserve:"30000000",max_grid_count:20,
      max_orders_per_reconcile:20,max_active_orders_per_bot:100}')
GRID_ADDRESS=$(instantiate_contract "$GRID_CODE_ID" "$GRID_INIT" cl8y-grid-manager)

SET_LIQUIDITY=$(jq -nc --arg address "$LIQUIDITY_ADDRESS" \
    '{set_liquidity_contract:{liquidity_contract:$address}}')
TX_HASH=$(terrad_tx wasm execute "$VAULT_ADDRESS" "$SET_LIQUIDITY" | jq -r '.txhash')
wait_tx "$TX_HASH" >/dev/null

REGISTER_VAULT=$(jq -nc --arg vault "$VAULT_ADDRESS" --arg pair "$PAIR_ADDRESS" \
    '{register_vault:{vault:$vault,pair:$pair}}')
TX_HASH=$(terrad_tx wasm execute "$PROXY_ADDRESS" "$REGISTER_VAULT" | jq -r '.txhash')
wait_tx "$TX_HASH" >/dev/null

CL8Y_FUND="200000000000000000000"
FUND_PROXY=$(jq -nc --arg recipient "$PROXY_ADDRESS" --arg amount "$CL8Y_FUND" \
    '{transfer:{recipient:$recipient,amount:$amount}}')
TX_HASH=$(terrad_tx wasm execute "$CL8Y_ADDRESS" "$FUND_PROXY" | jq -r '.txhash')
wait_tx "$TX_HASH" >/dev/null
REGISTER_PROXY=$(jq -nc --arg wallet "$PROXY_ADDRESS" \
    '{register_wallet:{wallet:$wallet,tier_id:5}}')
TX_HASH=$(terrad_tx wasm execute "$FEE_REGISTRY_ADDRESS" "$REGISTER_PROXY" | jq -r '.txhash')
wait_tx "$TX_HASH" >/dev/null

FUND_GRID=$(jq -nc --arg recipient "$GRID_ADDRESS" --arg amount "$CL8Y_FUND" \
    '{transfer:{recipient:$recipient,amount:$amount}}')
TX_HASH=$(terrad_tx wasm execute "$CL8Y_ADDRESS" "$FUND_GRID" | jq -r '.txhash')
wait_tx "$TX_HASH" >/dev/null
REGISTER_GRID=$(jq -nc --arg wallet "$GRID_ADDRESS" \
    '{register_wallet:{wallet:$wallet,tier_id:5}}')
TX_HASH=$(terrad_tx wasm execute "$FEE_REGISTRY_ADDRESS" "$REGISTER_GRID" | jq -r '.txhash')
wait_tx "$TX_HASH" >/dev/null

cat > "$LOCAL_ENV" <<ENVEOF
CL8Y_DEX_DIR=$DEX_DIR
PAIR_ADDRESS=$PAIR_ADDRESS
EMBER_ADDRESS=$EMBER_ADDRESS
CORAL_ADDRESS=$CORAL_ADDRESS
CL8Y_ADDRESS=$CL8Y_ADDRESS
FEE_REGISTRY_ADDRESS=$FEE_REGISTRY_ADDRESS
PROXY_ADDRESS=$PROXY_ADDRESS
VAULT_ADDRESS=$VAULT_ADDRESS
LIQUIDITY_ADDRESS=$LIQUIDITY_ADDRESS
GRID_CODE_ID=$GRID_CODE_ID
GRID_ADDRESS=$GRID_ADDRESS
FACTORY_ADDRESS=$VITE_FACTORY_ADDRESS
TEST_ADDRESS=$TEST_ADDRESS
ENVEOF

echo "Swap proxy: $PROXY_ADDRESS"
echo "Bot vault: $VAULT_ADDRESS"
echo "Bot liquidity token: $LIQUIDITY_ADDRESS"
echo "Grid manager: $GRID_ADDRESS"
