#!/usr/bin/env bash
# On-chain E2E: market-grid (grid-vault-swap) funded with exactly 1000 EMBER,
# rebalanced through the whitelisted shared swap-proxy, fee realized to the
# single shared fee-collector -> treasury. Reports the exact fee and redemption.
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
export CL8Y_DEX_DIR=${CL8Y_DEX_DIR:-/home/rennebeau/Liquidity-trading-bot/test-area/.cache/cl8y-dex-terraclassic}
# shellcheck source=common.sh
source "$SCRIPT_DIR/common.sh"
load_local_env
# shellcheck disable=SC1091
source "$SCRIPT_DIR/.fee-e2e-artifacts"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/.env"

CONTAINER=$(localterra_container)

log() { echo "[fee-1000] $*"; }

store_contract() {
    local artifact="$1" tx_hash result
    docker cp "$artifact" "$CONTAINER:/tmp/fee-1000.wasm"
    tx_hash=$(terrad_tx wasm store "/tmp/fee-1000.wasm" | jq -r '.txhash')
    result=$(wait_tx "$tx_hash")
    tx_event_value "$result" code_id
}

instantiate_contract() {
    local code_id="$1" message="$2" label="$3" output tx_hash result address
    output=$(terrad_tx wasm instantiate "$code_id" "$message" \
        --label "$label" --admin "$TEST_ADDRESS")
    tx_hash=$(jq -r '.txhash // empty' <<<"$output")
    result=$(wait_tx "$tx_hash")
    address=$(tx_event_value "$result" contract_address)
    if [ -z "$address" ]; then
        echo "ERROR: instantiation returned no address for $label" >&2
        exit 1
    fi
    printf '%s\n' "$address"
}

execute_from() {
    local signer="$1" contract="$2" message="$3"
    shift 3
    local tx_hash
    tx_hash=$(terrad_tx_from "$signer" wasm execute "$contract" "$message" "$@" | jq -r '.txhash')
    wait_tx "$tx_hash"
}

query_smart() {
    local contract="$1" message="$2"
    terrad_query wasm contract-state smart "$contract" "$message"
}

cw20_balance_of() {
    local token="$1" address="$2"
    query_smart "$token" "{\"balance\":{\"address\":\"$address\"}}" | jq -r '.data.balance'
}

now_sec() { date +%s; }

refresh_twap() {
    local amount="$1" hook swap tx_hash
    hook="$(jq -nc --arg amount "$amount" '{swap:{belief_price:null,max_spread:"1",min_return:"1",to:null,deadline:null,trader:null,hybrid:{pool_input:$amount,book_input:"0",max_maker_fills:1,book_start_hint:null}}}' | base64 -w0)"
    swap=$(jq -nc --arg pair "$PAIR_ADDRESS" --arg amount "$amount" --arg hook "$hook" \
        '{send:{contract:$pair,amount:$amount,msg:$hook}}')
    tx_hash=$(terrad_tx wasm execute "$EMBER_ADDRESS" "$swap" | jq -r '.txhash')
    wait_tx "$tx_hash" >/dev/null
}

log "== reuse the live single fee-collector (serves every bot) =="
REG_CONFIG=$(query_smart "$FEE_REGISTRY" '{"config":{}}')
jq -e --arg collector "$FEE_COLLECTOR" '.data.fee_collector == $collector and .data.base_fee_bps == 180' \
    <<<"$REG_CONFIG" >/dev/null
log "registry=$FEE_REGISTRY collector=$FEE_COLLECTOR treasury=$FEE_TREASURY base_fee_bps=180"

MG_CODE_ID=$(store_contract "$PROJECT_ROOT/market-grid-system/target/wasm32-unknown-unknown/release/cl8y_grid_vault_swap.wasm")
log "market-grid code: $MG_CODE_ID"

POOL=$(query_smart "$PAIR_ADDRESS" '{"pool":{}}')
RESERVE_0=$(jq -r '.data.assets[0].amount' <<<"$POOL")
RESERVE_1=$(jq -r '.data.assets[1].amount' <<<"$POOL")
read -r LOWER_PRICE UPPER_PRICE < <(python3 -c '
import sys
a, b = map(int, sys.argv[1:])
scale = 10**18
price = b * scale // a
def render(value):
    text = f"{value // scale}.{value % scale:018d}".rstrip("0").rstrip(".")
    return text or "0"
print(render(price * 85 // 100), render(price * 115 // 100))
' "$RESERVE_0" "$RESERVE_1")
log "grid bounds: lower=$LOWER_PRICE upper=$UPPER_PRICE"

log "-- instantiate market-grid; fund exactly 1000 EMBER (1000e6 raw) --"
MG_INIT=$(jq -nc --arg admin "$TEST_ADDRESS" --arg pair "$PAIR_ADDRESS" --argjson twap 60 \
    --argjson grid_count 5 --arg lower "$LOWER_PRICE" --arg upper "$UPPER_PRICE" \
    --arg registry "$FEE_REGISTRY" --arg collector "$FEE_COLLECTOR" \
    '{admin:$admin,pair:$pair,twap_window_seconds:$twap,grid_count:$grid_count,
      lower_price:$lower,upper_price:$upper,allocation_tolerance_bps:600,
      max_spread:"0.1",max_execution_deviation_bps:1000,quote_slippage_bps:500,
      max_spot_twap_deviation_bps:1000,
      fee_registry:$registry,fee_collector:$collector}')
MG_VAULT=$(instantiate_contract "$MG_CODE_ID" "$MG_INIT" cl8y-1000-ember-market-grid)
log "market-grid vault: $MG_VAULT"

hook=$(printf '{"deposit":{}}' | base64 -w0)
msg=$(jq -nc --arg vault "$MG_VAULT" --arg amount 1000000000 --arg hook "$hook" \
    '{send:{contract:$vault,amount:$amount,msg:$hook}}')
execute_from test1 "$EMBER_ADDRESS" "$msg" >/dev/null
STATUS=$(query_smart "$MG_VAULT" '{"grid_status":{}}')
jq -e '.data.should_rebalance == true and .data.allocation_deviation_bps == 10000' \
    <<<"$STATUS" >/dev/null
log "grid_status: should_rebalance=true deviation=10000 (1000 EMBER only)"

log "-- rebalance (swap EMBER->CORAL); fee charged in reply --"
refresh_twap 100000000
MG_DEADLINE=$(($(now_sec) + 300))
MG_RES=$(execute_from test1 "$MG_VAULT" "{\"rebalance\":{\"deadline\":$MG_DEADLINE}}")
MG_BPS=$(tx_event_value "$MG_RES" fee_bps)
MG_TIER=$(tx_event_value "$MG_RES" fee_tier)
MG_SRC=$(tx_event_value "$MG_RES" fee_source)
MG_SHARES=$(tx_event_value "$MG_RES" fee_shares)
MG_DEV=$(tx_event_value "$MG_RES" allocation_deviation_bps)
log "market-grid fee: fee_bps=$MG_BPS fee_tier=$MG_TIER fee_source=$MG_SRC fee_shares=$MG_SHARES"

if [ -z "${MG_BPS:-}" ]; then
    log "!! no fee charged (registry unset or rate zero)"
else
    VALUE=$(( MG_SHARES * 10000 / MG_BPS ))
    log "=> executed swap value ~ $VALUE raw token0 (0.${MG_BPS}% of it = $MG_SHARES)"
fi

MG_COLLECTOR_SHARES=$(query_smart "$MG_VAULT" \
    "{\"shares\":{\"bot_id\":0,\"address\":\"$FEE_COLLECTOR\"}}" | jq -r '.data.shares')
log "collector LP in market vault: $MG_COLLECTOR_SHARES"

log "-- collector collect -> single shared treasury --"
T_0_BEFORE=$(cw20_balance_of "$EMBER_ADDRESS" "$FEE_TREASURY")
T_1_BEFORE=$(cw20_balance_of "$CORAL_ADDRESS" "$FEE_TREASURY")
COLLECT=$(jq -nc --arg vault "$MG_VAULT" --argjson bot_id 0 '{collect:{vault:$vault,bot_id:$bot_id}}')
wait_tx "$(terrad_tx_from gridkeeper wasm execute "$FEE_COLLECTOR" "$COLLECT" | jq -r '.txhash')" >/dev/null
T_0_AFTER=$(cw20_balance_of "$EMBER_ADDRESS" "$FEE_TREASURY")
T_1_AFTER=$(cw20_balance_of "$CORAL_ADDRESS" "$FEE_TREASURY")
MG_COLLECTOR_SHARES_ZERO=$(query_smart "$MG_VAULT" \
    "{\"shares\":{\"bot_id\":0,\"address\":\"$FEE_COLLECTOR\"}}" | jq -r '.data.shares')
log "treasury received EMBER +$((T_0_AFTER - T_0_BEFORE)) CORAL +$((T_1_AFTER - T_1_BEFORE)); collector shares now $MG_COLLECTOR_SHARES_ZERO"

log "DONE: 1000-EMBER market-grid fee realized via the single shared collector"
