#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=common.sh
source "$SCRIPT_DIR/common.sh"
ensure_dex_repo

OPTIMIZER_IMAGE=${OPTIMIZER_IMAGE:-cosmwasm/workspace-optimizer:0.16.1@sha256:b9c92b2900b7ebaab3499203615c1b8589592bc557355ed3432e48851ffde69e}

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
LUNC_C_ADDRESS="$VITE_LUNC_C_TOKEN_ADDRESS"
FEE_REGISTRY_ADDRESS="$VITE_FEE_DISCOUNT_ADDRESS"
PAIR_QUERY=$(jq -nc --arg ember "$EMBER_ADDRESS" --arg coral "$CORAL_ADDRESS" \
    '{pair:{asset_infos:[{token:{contract_addr:$ember}},{token:{contract_addr:$coral}}]}}')
PAIR_ADDRESS=$(terrad_query wasm contract-state smart "$VITE_FACTORY_ADDRESS" "$PAIR_QUERY" \
    | jq -r '.data.pair.contract_addr // empty')
if [ -z "$PAIR_ADDRESS" ]; then
    echo "ERROR: CL8Y factory did not return the EMBER/CORAL pair." >&2
    exit 1
fi
SECOND_PAIR_QUERY=$(jq -nc --arg lunc "$LUNC_C_ADDRESS" --arg ember "$EMBER_ADDRESS" \
    '{pair:{asset_infos:[{token:{contract_addr:$lunc}},{token:{contract_addr:$ember}}]}}')
SECOND_PAIR_ADDRESS=$(terrad_query wasm contract-state smart "$VITE_FACTORY_ADDRESS" \
    "$SECOND_PAIR_QUERY" | jq -r '.data.pair.contract_addr // empty')
if [ -z "$SECOND_PAIR_ADDRESS" ]; then
    echo "ERROR: CL8Y factory did not return the LUNC-C/EMBER pair." >&2
    exit 1
fi

ensure_pair_oracle_history() {
    local query hook message output tx_hash
    query='{"observe":{"seconds_ago":[0,1]}}'
    if terrad_query wasm contract-state smart "$PAIR_ADDRESS" "$query" >/dev/null 2>&1; then
        return 0
    fi

    echo "Seeding the EMBER/CORAL oracle before vault instantiation..."
    hook=$(jq -nc '{swap:{belief_price:null,max_spread:"0.50",min_return:"1",to:null,
      deadline:null,trader:null,hybrid:null}}' | base64 -w0)
    message=$(jq -nc --arg pair "$PAIR_ADDRESS" --arg hook "$hook" \
        '{send:{contract:$pair,amount:"1000000",msg:$hook}}')
    if ! output=$(terrad_tx wasm execute "$EMBER_ADDRESS" "$message"); then
        echo "ERROR: failed to seed the EMBER/CORAL oracle." >&2
        return 1
    fi
    tx_hash=$(jq -r '.txhash // empty' <<<"$output")
    if [ -z "$tx_hash" ] || ! wait_tx "$tx_hash" >/dev/null; then
        echo "ERROR: oracle seed transaction was not confirmed." >&2
        return 1
    fi
    for _ in $(seq 1 30); do
        sleep 1
        if terrad_query wasm contract-state smart "$PAIR_ADDRESS" "$query" >/dev/null 2>&1; then
            return 0
        fi
    done
    echo "ERROR: EMBER/CORAL oracle did not produce one second of history." >&2
    return 1
}

ensure_pair_oracle_history

echo "Building clean proxy, vault, and liquidity Wasm artifacts..."
docker run --rm \
    -v "$PROJECT_ROOT/rebalancer-system:/code" \
    --mount type=volume,source=cl8y_bot_target_cache,target=/code/target \
    --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
    "$OPTIMIZER_IMAGE"

echo "Building the isolated grid manager and vault Wasm artifacts..."
docker run --rm \
    -v "$PROJECT_ROOT/grid-contract-system:/code" \
    --mount type=volume,source=cl8y_grid_target_cache,target=/code/target \
    --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
    "$OPTIMIZER_IMAGE"

CONTAINER=$(localterra_container)
if ! docker exec "$CONTAINER" terrad keys show gridkeeper \
    --keyring-backend test --address >/dev/null 2>&1; then
    docker exec "$CONTAINER" terrad keys add gridkeeper \
        --keyring-backend test --output json >/dev/null
fi
GRID_KEEPER_ADDRESS=$(docker exec "$CONTAINER" terrad keys show gridkeeper \
    --keyring-backend test --address)
TX_HASH=$(terrad_tx bank send test1 "$GRID_KEEPER_ADDRESS" 500000000uluna | jq -r '.txhash')
wait_tx "$TX_HASH" >/dev/null

store_contract() {
    local artifact="$1"
    local tx_hash result
    docker cp "$PROJECT_ROOT/rebalancer-system/artifacts/$artifact.wasm" \
        "$CONTAINER:/tmp/$artifact.wasm"
    tx_hash=$(terrad_tx wasm store "/tmp/$artifact.wasm" | jq -r '.txhash')
    result=$(wait_tx "$tx_hash")
    tx_event_value "$result" code_id
}

PROXY_CODE_ID=$(store_contract cl8y_swap_proxy)
VAULT_CODE_ID=$(store_contract cl8y_bot_vault)
LIQUIDITY_CODE_ID=$(store_contract cl8y_bot_liquidity)
store_grid_contract() {
    local artifact="$1"
    local tx_hash result
    docker cp "$PROJECT_ROOT/grid-contract-system/artifacts/$artifact.wasm" \
        "$CONTAINER:/tmp/$artifact.wasm"
    tx_hash=$(terrad_tx wasm store "/tmp/$artifact.wasm" | jq -r '.txhash')
    result=$(wait_tx "$tx_hash")
    tx_event_value "$result" code_id
}

GRID_MANAGER_CODE_ID=$(store_grid_contract cl8y_grid_manager)
GRID_VAULT_CODE_ID=$(store_grid_contract cl8y_grid_vault)
GRID_CODE_ID=$GRID_VAULT_CODE_ID

instantiate_contract() {
    local code_id="$1"
    local message="$2"
    local label="$3"
    local output tx_hash result address
    if ! output=$(terrad_tx wasm instantiate "$code_id" "$message" \
        --label "$label" --admin "$TEST_ADDRESS"); then
        echo "ERROR: failed to instantiate $label." >&2
        return 1
    fi
    tx_hash=$(jq -r '.txhash // empty' <<<"$output")
    if [ -z "$tx_hash" ] || ! result=$(wait_tx "$tx_hash"); then
        echo "ERROR: instantiation transaction failed for $label." >&2
        return 1
    fi
    address=$(tx_event_value "$result" contract_address)
    if [ -z "$address" ]; then
        echo "ERROR: instantiation returned no contract address for $label." >&2
        return 1
    fi
    printf '%s\n' "$address"
}

PROXY_INIT=$(jq -nc --arg admin "$TEST_ADDRESS" --arg cl8y "$CL8Y_ADDRESS" \
    --arg registry "$FEE_REGISTRY_ADDRESS" \
    '{admin:$admin,cl8y_token:$cl8y,fee_registry:$registry}')
PROXY_ADDRESS=$(instantiate_contract "$PROXY_CODE_ID" "$PROXY_INIT" cl8y-shared-swap-proxy)

VAULT_INIT=$(jq -nc --arg admin "$TEST_ADDRESS" --arg keeper "$TEST_ADDRESS" \
    --arg proxy "$PROXY_ADDRESS" --arg pair "$PAIR_ADDRESS" \
    '{admin:$admin,keeper:$keeper,proxy:$proxy,pair:$pair,twap_window_seconds:1,
      rebalance_threshold_bps:500,allocation_tolerance_bps:500,max_trade_bps:2500,
      max_execution_deviation_bps:500,quote_slippage_bps:200,
      max_spot_twap_deviation_bps:500,max_trade_pool_bps:1000,max_spread:"0.05"}')
VAULT_ADDRESS=$(instantiate_contract "$VAULT_CODE_ID" "$VAULT_INIT" ember-coral-bot-vault)

LIQUIDITY_INIT=$(jq -nc --arg admin "$TEST_ADDRESS" --arg vault "$VAULT_ADDRESS" \
    '{admin:$admin,vault:$vault,name:"EMBER CORAL Bot Liquidity",
      symbol:"ECBOTLP",decimals:6,minimum_initial_deposit:"100000",marketing:null}')
LIQUIDITY_ADDRESS=$(instantiate_contract "$LIQUIDITY_CODE_ID" "$LIQUIDITY_INIT" ember-coral-bot-liquidity)

GRID_MANAGER_INIT=$(jq -nc --arg admin "$TEST_ADDRESS" --arg keeper "$GRID_KEEPER_ADDRESS" \
    --arg factory "$VITE_FACTORY_ADDRESS" --argjson vault_code_id "$GRID_VAULT_CODE_ID" \
    '{admin:$admin,keeper:$keeper,dex_factory:$factory,vault_code_id:$vault_code_id,
      gas_denom:"uluna",keeper_reward:"30000000",minimum_gas_reserve:"30000000",
      order_timeout_seconds:604800,max_grid_count:20,max_orders_per_reconcile:20,
      max_active_orders_per_vault:100}')
GRID_MANAGER_ADDRESS=$(instantiate_contract "$GRID_MANAGER_CODE_ID" "$GRID_MANAGER_INIT" cl8y-grid-manager)

if ! docker exec "$CONTAINER" terrad keys show attacker \
    --keyring-backend test --address >/dev/null 2>&1; then
    docker exec "$CONTAINER" terrad keys add attacker \
        --keyring-backend test --output json >/dev/null
fi
ATTACKER_ADDRESS=$(docker exec "$CONTAINER" terrad keys show attacker \
    --keyring-backend test --address)
TX_HASH=$(terrad_tx bank send test1 "$ATTACKER_ADDRESS" 100000000uluna | jq -r '.txhash')
wait_tx "$TX_HASH" >/dev/null

create_grid_vault() {
    local signer="$1"
    local label="$2"
    local tx_hash result message
    message=$(jq -nc --arg label "$label" '{create_vault:{label:$label}}')
    tx_hash=$(terrad_tx_from "$signer" wasm execute "$GRID_MANAGER_ADDRESS" "$message" \
        | jq -r '.txhash')
    result=$(wait_tx "$tx_hash")
    tx_event_value "$result" vault
}

GRID_ADDRESS_1=$(create_grid_vault test1 cl8y-grid-vault-1)
GRID_ADDRESS_2=$(create_grid_vault attacker cl8y-grid-vault-2)
GRID_ADDRESS_3=$(create_grid_vault test1 cl8y-grid-vault-3)
GRID_ADDRESS_4=$(create_grid_vault attacker cl8y-grid-vault-4)
GRID_ADDRESS=$GRID_ADDRESS_1

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

for grid_address in "$GRID_ADDRESS_1" "$GRID_ADDRESS_2" "$GRID_ADDRESS_3" "$GRID_ADDRESS_4"; do
    FUND_GRID=$(jq -nc --arg recipient "$grid_address" --arg amount "$CL8Y_FUND" \
        '{transfer:{recipient:$recipient,amount:$amount}}')
    TX_HASH=$(terrad_tx wasm execute "$CL8Y_ADDRESS" "$FUND_GRID" | jq -r '.txhash')
    wait_tx "$TX_HASH" >/dev/null
    REGISTER_GRID=$(jq -nc --arg wallet "$grid_address" \
        '{register_wallet:{wallet:$wallet,tier_id:5}}')
    TX_HASH=$(terrad_tx wasm execute "$FEE_REGISTRY_ADDRESS" "$REGISTER_GRID" | jq -r '.txhash')
    wait_tx "$TX_HASH" >/dev/null
done

cat > "$LOCAL_ENV" <<ENVEOF
CL8Y_DEX_DIR=$DEX_DIR
PAIR_ADDRESS=$PAIR_ADDRESS
SECOND_PAIR_ADDRESS=$SECOND_PAIR_ADDRESS
EMBER_ADDRESS=$EMBER_ADDRESS
CORAL_ADDRESS=$CORAL_ADDRESS
LUNC_C_ADDRESS=$LUNC_C_ADDRESS
CL8Y_ADDRESS=$CL8Y_ADDRESS
FEE_REGISTRY_ADDRESS=$FEE_REGISTRY_ADDRESS
PROXY_ADDRESS=$PROXY_ADDRESS
VAULT_ADDRESS=$VAULT_ADDRESS
LIQUIDITY_ADDRESS=$LIQUIDITY_ADDRESS
GRID_CODE_ID=$GRID_CODE_ID
GRID_MANAGER_CODE_ID=$GRID_MANAGER_CODE_ID
GRID_VAULT_CODE_ID=$GRID_VAULT_CODE_ID
GRID_MANAGER_ADDRESS=$GRID_MANAGER_ADDRESS
GRID_ADDRESS=$GRID_ADDRESS
GRID_ADDRESS_1=$GRID_ADDRESS_1
GRID_ADDRESS_2=$GRID_ADDRESS_2
GRID_ADDRESS_3=$GRID_ADDRESS_3
GRID_ADDRESS_4=$GRID_ADDRESS_4
GRID_KEEPER_ADDRESS=$GRID_KEEPER_ADDRESS
FACTORY_ADDRESS=$VITE_FACTORY_ADDRESS
TEST_ADDRESS=$TEST_ADDRESS
ENVEOF

echo "Swap proxy: $PROXY_ADDRESS"
echo "Bot vault: $VAULT_ADDRESS"
echo "Bot liquidity token: $LIQUIDITY_ADDRESS"
echo "Grid vaults: $GRID_ADDRESS_1 $GRID_ADDRESS_2 $GRID_ADDRESS_3 $GRID_ADDRESS_4"
