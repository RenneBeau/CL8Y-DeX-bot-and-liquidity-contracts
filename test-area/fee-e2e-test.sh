#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
export CL8Y_DEX_DIR=${CL8Y_DEX_DIR:-/home/rennebeau/Liquidity-trading-bot/test-area/.cache/cl8y-dex-terraclassic}
# shellcheck source=common.sh
source "$SCRIPT_DIR/common.sh"
load_local_env

CONTAINER=$(localterra_container)

log() { echo "[fee-e2e] $*"; }

wait_expect_fail() {
    local contract="$1"
    local message="$2"
    local output tx_hash
    if ! output=$(terrad_tx wasm execute "$contract" "$message" 2>&1); then
        return 0
    fi
    tx_hash=$(jq -r '.txhash // empty' <<<"$output")
    if [ -z "$tx_hash" ]; then
        return 0
    fi
    if wait_tx "$tx_hash" >/dev/null 2>&1; then
        echo "ERROR: expected transaction to fail: $message" >&2
        return 1
    fi
}

store_contract() {
    local artifact="$1"
    local tx_hash result
    docker cp "$artifact" "$CONTAINER:/tmp/fee-test.wasm"
    tx_hash=$(terrad_tx wasm store "/tmp/fee-test.wasm" | jq -r '.txhash')
    result=$(wait_tx "$tx_hash")
    tx_event_value "$result" code_id
}

instantiate_contract() {
    local code_id="$1"
    local message="$2"
    local label="$3"
    local output tx_hash result address
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
    local signer="$1"
    local contract="$2"
    local message="$3"
    shift 3
    local tx_hash
    tx_hash=$(terrad_tx_from "$signer" wasm execute "$contract" "$message" "$@" | jq -r '.txhash')
    wait_tx "$tx_hash"
}

query_smart() {
    local contract="$1"
    local message="$2"
    terrad_query wasm contract-state smart "$contract" "$message"
}

cw20_balance_of() {
    local token="$1"
    local address="$2"
    query_smart "$token" "{\"balance\":{\"address\":\"$address\"}}" | jq -r '.data.balance'
}

deposit_grid_token() {
    local signer="$1"
    local token="$2"
    local vault="$3"
    local bot_id="$4"
    local amount="$5"
    local hook message
    hook=$(jq -nc --argjson bot_id "$bot_id" '{deposit:{bot_id:$bot_id}}' | base64 -w0)
    message=$(jq -nc --arg vault "$vault" --arg amount "$amount" --arg hook "$hook" \
        '{send:{contract:$vault,amount:$amount,msg:$hook}}')
    execute_from "$signer" "$token" "$message"
}

fill_first_ask() {
    local vault="$1"
    local pair="$2"
    local token1="$3"
    local bot_id="$4"
    local orders ask_price ask_remaining partial_offer hook swap tx_hash swap_result fill_event
    orders=$(query_smart "$vault" "{\"orders\":{\"bot_id\":$bot_id}}" | jq -c '.data')
    LAST_ASK_ID=$(jq -r '[.[] | select(.side == "ask")][0].order_id' <<<"$orders")
    ask_price=$(jq -r '[.[] | select(.side == "ask")][0].price' <<<"$orders")
    ask_remaining=$(jq -r '[.[] | select(.side == "ask")][0].remaining' <<<"$orders")
    partial_offer=$(python3 -c '
from decimal import Decimal, ROUND_DOWN
import sys
value = (Decimal(sys.argv[1]) * Decimal(sys.argv[2]) / 2).to_integral_value(rounding=ROUND_DOWN)
print(max(1, int(value)))
' "$ask_remaining" "$ask_price")
    hook=$(jq -nc --arg amount "$partial_offer" --argjson hint "$LAST_ASK_ID" '
      {swap:{belief_price:null,max_spread:"1",min_return:"1",to:null,deadline:null,trader:null,
        hybrid:{pool_input:"0",book_input:$amount,max_maker_fills:1,book_start_hint:$hint}}}' \
        | base64 -w0)
    swap=$(jq -nc --arg pair "$pair" --arg amount "$partial_offer" --arg hook "$hook" \
        '{send:{contract:$pair,amount:$amount,msg:$hook}}')
    tx_hash=$(terrad_tx wasm execute "$token1" "$swap" | jq -r '.txhash')
    swap_result=$(wait_tx "$tx_hash")
    fill_event=$(jq -c --arg id "$LAST_ASK_ID" '
      [[.logs[]?.events[]?, .events[]?][]
       | select(any(.attributes[]?; .key == "action" and .value == "limit_order_fill"))
       | select(any(.attributes[]?; .key == "order_id" and .value == $id))][0]
    ' <<<"$swap_result")
    LAST_FILL_INPUT=$(jq -r '[.attributes[] | select(.key == "token0_amount")][0].value' \
        <<<"$fill_event")
    LAST_FILL_OUTPUT=$(jq -r '[.attributes[] | select(.key == "token1_amount")][0].value' \
        <<<"$fill_event")
    test "$LAST_FILL_INPUT" -gt 0
    test "$LAST_FILL_OUTPUT" -gt 0
}

reconcile_via_keeper() {
    local vault="$1"
    local bot_id="$2"
    local order_id="$3"
    local message
    message=$(jq -nc --argjson bot_id "$bot_id" --argjson order_id "$order_id" \
        '{reconcile:{bot_id:$bot_id,order_ids:[$order_id]}}')
    local tx_hash
    tx_hash=$(terrad_tx_from gridkeeper wasm execute "$vault" "$message" | jq -r '.txhash')
    wait_tx "$tx_hash"
}

set -a
# shellcheck disable=SC1091
source "$PROJECT_ROOT/test-area/.env"
set +a

log "== step 0: deploy fresh dummy CL8Y fee token =="
# Reuse the on-chain cw20-base code (id 1, 18 decimals). 200 CL8Y -> tier 5.
CL8Y_CODE_ID=$(terrad_query wasm contract "$CL8Y_ADDRESS" | jq -r '.contract_info.code_id')
DUMMY_INIT=$(jq -nc --arg name "Dummy CL8Y" --arg symbol "DCL8Y" \
    --argjson decimals 18 \
    --arg a0 "$TEST_ADDRESS" --arg amt0 "200000000000000000000" \
    '{name:$name,symbol:$symbol,decimals:$decimals,
      initial_balances:[{address:$a0,amount:$amt0}],mint:null,marketing:null}')
DUMMY_CL8Y=$(instantiate_contract "$CL8Y_CODE_ID" "$DUMMY_INIT" cl8y-fee-e2e-dummy-token)
test "$(cw20_balance_of "$DUMMY_CL8Y" "$TEST_ADDRESS")" = "200000000000000000000"
log "dummy CL8Y token: $DUMMY_CL8Y (200 CL8Y for $TEST_ADDRESS)"

log "== step 1: dummy treasury =="
if ! docker exec "$CONTAINER" terrad keys show fee-treasury --keyring-backend test --address >/dev/null 2>&1; then
    docker exec "$CONTAINER" terrad keys add fee-treasury --keyring-backend test --output json >/dev/null
fi
FEE_TREASURY=$(docker exec "$CONTAINER" terrad keys show fee-treasury --keyring-backend test --address)
TX_HASH=$(terrad_tx bank send test1 "$FEE_TREASURY" 100000000uluna | jq -r '.txhash')
wait_tx "$TX_HASH" >/dev/null
log "dummy treasury: $FEE_TREASURY"

ATTACKER_ADDRESS=$(docker exec "$CONTAINER" terrad keys show attacker --keyring-backend test --address)
TX_HASH=$(terrad_tx bank send test1 "$ATTACKER_ADDRESS" 600000000uluna | jq -r '.txhash')
wait_tx "$TX_HASH" >/dev/null
log "attacker (zero-CL8Y owner): $ATTACKER_ADDRESS"

log "== step 2: store fee-registry / fee-collector / grid-vault wasm =="
FEE_REGISTRY_CODE_ID=$(store_contract "$PROJECT_ROOT/fee-system/target/wasm32-unknown-unknown/release/cl8y_fee_registry.wasm")
FEE_COLLECTOR_CODE_ID=$(store_contract "$PROJECT_ROOT/fee-system/target/wasm32-unknown-unknown/release/cl8y_fee_collector.wasm")
FEE_VAULT_CODE_ID=$(store_contract "$PROJECT_ROOT/limit-grid-system/target/wasm32-unknown-unknown/release/cl8y_grid_vault.wasm")
log "code ids: registry=$FEE_REGISTRY_CODE_ID collector=$FEE_COLLECTOR_CODE_ID vault=$FEE_VAULT_CODE_ID"

log "== step 3: instantiate fee-registry (placeholder collector, then wire) =="
REGISTRY_INIT=$(jq -nc --arg gov "$TEST_ADDRESS" --arg cl8y "$DUMMY_CL8Y" \
    --arg treasury "$FEE_TREASURY" --arg collector "$TEST_ADDRESS" \
    '{governance:$gov,cl8y:$cl8y,treasury:$treasury,fee_collector:$collector,base_fee_bps:180}')
FEE_REGISTRY=$(instantiate_contract "$FEE_REGISTRY_CODE_ID" "$REGISTRY_INIT" cl8y-fee-e2e-registry)

COLLECTOR_INIT=$(jq -nc --arg gov "$TEST_ADDRESS" --arg registry "$FEE_REGISTRY" \
    --arg keeper "$GRID_KEEPER_ADDRESS" --arg treasury "$FEE_TREASURY" \
    '{governance:$gov,registry:$registry,keeper:$keeper,treasury:$treasury}')
FEE_COLLECTOR=$(instantiate_contract "$FEE_COLLECTOR_CODE_ID" "$COLLECTOR_INIT" cl8y-fee-e2e-collector)
log "registry=$FEE_REGISTRY collector=$FEE_COLLECTOR"

CONFIG_FIX=$(jq -nc --arg collector "$FEE_COLLECTOR" '{update_config:{fee_collector:$collector}}')
execute_from "$TEST_ADDRESS" "$FEE_REGISTRY" "$CONFIG_FIX" >/dev/null
REG_CONFIG=$(query_smart "$FEE_REGISTRY" '{"config":{}}')
jq -e --arg collector "$FEE_COLLECTOR" '.data.fee_collector == $collector and .data.base_fee_bps == 180 and .data.ladder_version == 1' \
    <<<"$REG_CONFIG" >/dev/null
log "registry config ok (base_fee_bps=180, ladder_version=1)"

log "== step 4: on-chain EffectiveFee tiers =="
EFF_T5=$(query_smart "$FEE_REGISTRY" "{\"effective_fee\":{\"trader\":\"$TEST_ADDRESS\"}}")
jq -e '.data.fee_bps == 90 and .data.tier_id == 5 and .data.source == "live" and .data.discount_bps == 5000' \
    <<<"$EFF_T5" >/dev/null
log "tier-5 holder -> fee_bps=90 (live)"
EFF_FULL=$(query_smart "$FEE_REGISTRY" "{\"effective_fee\":{\"trader\":\"$FEE_TREASURY\"}}")
jq -e '.data.fee_bps == 180 and .data.discount_bps == 0' <<<"$EFF_FULL" >/dev/null
log "zero-CL8Y address -> fee_bps=180 (full base fee, never under-fee)"

log "== step 5: instantiate fee-enabled grid-vault =="
VAULT_INIT=$(jq -nc --arg admin "$TEST_ADDRESS" --arg owner "$TEST_ADDRESS" \
    --arg keeper "$GRID_KEEPER_ADDRESS" --arg factory "$FACTORY_ADDRESS" \
    --arg registry "$FEE_REGISTRY" --arg collector "$FEE_COLLECTOR" \
    '{admin:$admin,owner:$owner,keeper:$keeper,factory:$factory,gas_denom:"uluna",
      keeper_reward:"30000000",minimum_gas_reserve:"30000000",order_timeout_seconds:604800,
      max_grid_count:20,max_orders_per_reconcile:20,max_active_orders_per_bot:100,
      fee_registry:$registry,fee_collector:$collector}')
VAULT_INIT_2=$(jq -nc --arg admin "$TEST_ADDRESS" --arg owner "$ATTACKER_ADDRESS" \
    --arg keeper "$GRID_KEEPER_ADDRESS" --arg factory "$FACTORY_ADDRESS" \
    --arg registry "$FEE_REGISTRY" --arg collector "$FEE_COLLECTOR" \
    '{admin:$admin,owner:$owner,keeper:$keeper,factory:$factory,gas_denom:"uluna",
      keeper_reward:"30000000",minimum_gas_reserve:"30000000",order_timeout_seconds:604800,
      max_grid_count:20,max_orders_per_reconcile:20,max_active_orders_per_bot:100,
      fee_registry:$registry,fee_collector:$collector}')
FEE_VAULT=$(instantiate_contract "$FEE_VAULT_CODE_ID" "$VAULT_INIT" cl8y-fee-e2e-vault)
FEE_VAULT_2=$(instantiate_contract "$FEE_VAULT_CODE_ID" "$VAULT_INIT_2" cl8y-fee-e2e-vault-2)
log "fee vault: $FEE_VAULT (tier-5) ; fee vault 2: $FEE_VAULT_2 (full-fee)"

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
print(render(price * 8 // 10), render(price * 12 // 10))
' "$RESERVE_0" "$RESERVE_1")

CREATE=$(jq -nc --arg pair "$PAIR_ADDRESS" --arg lower "$LOWER_PRICE" \
    --arg upper "$UPPER_PRICE" \
    '{create_bot:{pair:$pair,lower_price:$lower,upper_price:$upper,grid_count:5}}')
RESULT=$(wait_tx "$(terrad_tx_from test1 wasm execute "$FEE_VAULT" "$CREATE" --amount 200000000uluna | jq -r '.txhash')")
BOT_1=$(tx_event_value "$RESULT" bot_id)
test "$BOT_1" = "1"
FUND=$(jq -nc --argjson bot_id "$BOT_1" '{fund_gas:{bot_id:$bot_id}}')
terrad_tx_from test1 wasm execute "$FEE_VAULT" "$FUND" --amount 500000000uluna >/dev/null
log "bot 1 created by tier-5 owner ($BOT_1)"

log "== step 6: deposit, auto-allocate, first fill, reconcile =="
deposit_grid_token test1 "$EMBER_ADDRESS" "$FEE_VAULT" "$BOT_1" 10000000
deposit_grid_token test1 "$CORAL_ADDRESS" "$FEE_VAULT" "$BOT_1" 10000000
ASK_COUNT=$(query_smart "$FEE_VAULT" "{\"orders\":{\"bot_id\":$BOT_1}}" | jq '[.data[] | select(.side == "ask")] | length')
test "$ASK_COUNT" -ge 1

fill_first_ask "$FEE_VAULT" "$PAIR_ADDRESS" "$CORAL_ADDRESS" "$BOT_1"
RECONCILE_RESULT=$(reconcile_via_keeper "$FEE_VAULT" "$BOT_1" "$LAST_ASK_ID")
test "$(tx_event_value "$RECONCILE_RESULT" changed_orders)" = "1"
FEE_BPS=$(tx_event_value "$RECONCILE_RESULT" fee_bps)
FEE_TIER=$(tx_event_value "$RECONCILE_RESULT" fee_tier)
FEE_SOURCE=$(tx_event_value "$RECONCILE_RESULT" fee_source)
FEE_SHARES=$(tx_event_value "$RECONCILE_RESULT" fee_shares)
log "fill 1: fee_bps=$FEE_BPS fee_tier=$FEE_TIER fee_source=$FEE_SOURCE fee_shares=$FEE_SHARES"
test "$FEE_BPS" = "90"
test "$FEE_TIER" = "5"
test "$FEE_SOURCE" = "Live"
test "$FEE_SHARES" -gt 0

COLLECTOR_SHARES=$(query_smart "$FEE_VAULT" \
    "{\"shares\":{\"bot_id\":$BOT_1,\"address\":\"$FEE_COLLECTOR\"}}" | jq -r '.data.shares')
test "$COLLECTOR_SHARES" -gt 0
log "collector LP in vault after fill 1: $COLLECTOR_SHARES"

log "== step 7: soak -- fill and reconcile remaining asks, fee must track EffectiveFee each time =="
FILLED=1
while :; do
    test "$FILLED" -lt 6 || break
    ASKS=$(query_smart "$FEE_VAULT" "{\"orders\":{\"bot_id\":$BOT_1}}" \
        | jq '[.data[] | select(.side == "ask" and (.remaining | tonumber) > 0)] | length')
    test "${ASKS:-0}" -ge 1 || break
    fill_first_ask "$FEE_VAULT" "$PAIR_ADDRESS" "$CORAL_ADDRESS" "$BOT_1"
    RES=$(reconcile_via_keeper "$FEE_VAULT" "$BOT_1" "$LAST_ASK_ID")
    BPS=$(tx_event_value "$RES" fee_bps)
    TIER=$(tx_event_value "$RES" fee_tier)
    SRC=$(tx_event_value "$RES" fee_source)
    SHARES=$(tx_event_value "$RES" fee_shares)
    test "$BPS" = "90"
    test "$TIER" = "5"
    test "$SRC" = "Live"
    test "$SHARES" -gt 0
    FILLED=$((FILLED + 1))
    log "fill $FILLED: fee_bps=$BPS fee_tier=$TIER fee_source=$SRC fee_shares=$SHARES"
done
test "$FILLED" -ge 3
COLLECTOR_SHARES_AFTER=$(query_smart "$FEE_VAULT" \
    "{\"shares\":{\"bot_id\":$BOT_1,\"address\":\"$FEE_COLLECTOR\"}}" | jq -r '.data.shares')
test "$COLLECTOR_SHARES_AFTER" -gt "$COLLECTOR_SHARES"
log "collector LP grew over soak: $COLLECTOR_SHARES -> $COLLECTOR_SHARES_AFTER"

log "== step 8: wind down, keeper collects to dummy treasury =="
CANCEL=$(jq -nc --argjson bot_id "$BOT_1" '{cancel_all:{bot_id:$bot_id}}')
execute_from test1 "$FEE_VAULT" "$CANCEL" >/dev/null
TREASURY_0_BEFORE=$(cw20_balance_of "$EMBER_ADDRESS" "$FEE_TREASURY")
TREASURY_1_BEFORE=$(cw20_balance_of "$CORAL_ADDRESS" "$FEE_TREASURY")
COLLECT=$(jq -nc --arg vault "$FEE_VAULT" --argjson bot_id "$BOT_1" \
    '{collect:{vault:$vault,bot_id:$bot_id}}')
wait_tx "$(terrad_tx_from gridkeeper wasm execute "$FEE_COLLECTOR" "$COLLECT" | jq -r '.txhash')" >/dev/null
TREASURY_0_AFTER=$(cw20_balance_of "$EMBER_ADDRESS" "$FEE_TREASURY")
TREASURY_1_AFTER=$(cw20_balance_of "$CORAL_ADDRESS" "$FEE_TREASURY")
test "$TREASURY_0_AFTER" -gt "$TREASURY_0_BEFORE"
test "$TREASURY_1_AFTER" -gt "$TREASURY_1_BEFORE"
log "treasury received EMBER +$((TREASURY_0_AFTER - TREASURY_0_BEFORE)) and CORAL +$((TREASURY_1_AFTER - TREASURY_1_BEFORE))"
COLLECTOR_SHARES_ZERO=$(query_smart "$FEE_VAULT" \
    "{\"shares\":{\"bot_id\":$BOT_1,\"address\":\"$FEE_COLLECTOR\"}}" | jq -r '.data.shares')
test "$COLLECTOR_SHARES_ZERO" = "0"
log "collector redeemed all its LP to the dummy treasury (vault shares now 0)"

log "== step 9: second lifecycle -- a zero-CL8Y owner pays the full base fee =="
cw20_transfer_to() {
    local token="$1" recipient="$2" amount="$3"
    local m; m=$(jq -nc --arg r "$recipient" --arg a "$amount" '{transfer:{recipient:$r,amount:$a}}')
    execute_from test1 "$token" "$m" >/dev/null
}
cw20_transfer_to "$EMBER_ADDRESS" "$ATTACKER_ADDRESS" 2000000
cw20_transfer_to "$CORAL_ADDRESS" "$ATTACKER_ADDRESS" 2000000
RESULT=$(wait_tx "$(terrad_tx_from attacker wasm execute "$FEE_VAULT_2" "$CREATE" --amount 200000000uluna | jq -r '.txhash')")
BOT_2=$(tx_event_value "$RESULT" bot_id)
test "$BOT_2" = "1"
FUND_2=$(jq -nc --argjson bot_id "$BOT_2" '{fund_gas:{bot_id:$bot_id}}')
terrad_tx_from attacker wasm execute "$FEE_VAULT_2" "$FUND_2" --amount 300000000uluna >/dev/null
deposit_grid_token attacker "$EMBER_ADDRESS" "$FEE_VAULT_2" "$BOT_2" 2000000
deposit_grid_token attacker "$CORAL_ADDRESS" "$FEE_VAULT_2" "$BOT_2" 2000000
fill_first_ask "$FEE_VAULT_2" "$PAIR_ADDRESS" "$CORAL_ADDRESS" "$BOT_2"
RES=$(reconcile_via_keeper "$FEE_VAULT_2" "$BOT_2" "$LAST_ASK_ID")
log "  bot2 reconcile: changed=$(tx_event_value "$RES" changed_orders) fee_bps=$(tx_event_value "$RES" fee_bps) fee_source=$(tx_event_value "$RES" fee_source) fee_shares=$(tx_event_value "$RES" fee_shares)"
test "$(tx_event_value "$RES" changed_orders)" = "1"
test "$(tx_event_value "$RES" fee_bps)" = "180"
test "$(tx_event_value "$RES" fee_source)" = "Live"
test "$(tx_event_value "$RES" fee_shares)" -gt 0
log "bot 2 (zero-CL8Y owner): fee_bps=180 full base fee, live source"

echo "FEE-E2E PASSED"
printf 'DUMMY_CL8Y=%s\nFEE_TREASURY=%s\nFEE_REGISTRY=%s\nFEE_COLLECTOR=%s\nFEE_VAULT_1=%s\nFEE_VAULT_2=%s\nFILLED_ASKS=%s\n' \
    "$DUMMY_CL8Y" "$FEE_TREASURY" "$FEE_REGISTRY" "$FEE_COLLECTOR" "$FEE_VAULT" "$FEE_VAULT_2" "$FILLED" \
    | tee "$SCRIPT_DIR/.fee-e2e-artifacts"
