#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=test-lib.sh
source "$SCRIPT_DIR/test-lib.sh"

execute_grid_from() {
    local signer="$1"
    local message="$2"
    shift 2
    local tx_hash
    tx_hash=$(terrad_tx_from "$signer" wasm execute "$GRID_ADDRESS" "$message" "$@" \
        | jq -r '.txhash')
    wait_tx "$tx_hash"
}

query_grid() {
    terrad_query wasm contract-state smart "$GRID_ADDRESS" "$1" | jq -c '.data'
}

deposit_grid_token() {
    local signer="$1"
    local token="$2"
    local bot_id="$3"
    local amount="$4"
    local hook message
    hook=$(jq -nc --argjson bot_id "$bot_id" '{deposit:{bot_id:$bot_id}}' | base64 -w0)
    message=$(jq -nc --arg manager "$GRID_ADDRESS" --arg amount "$amount" --arg hook "$hook" \
        '{send:{contract:$manager,amount:$amount,msg:$hook}}')
    execute_wait_from "$signer" "$token" "$message"
}

echo "[grid 1/7] Verifying manager fee tier and settlement extension"
DISCOUNT=$(terrad_query wasm contract-state smart "$FEE_REGISTRY_ADDRESS" \
    "{\"get_discount\":{\"trader\":\"$GRID_ADDRESS\",\"sender\":\"$GRID_ADDRESS\"}}")
jq -e '.data.discount_bps == 5000 and .data.needs_deregister == false' \
    <<<"$DISCOUNT" >/dev/null

POOL=$(query_pool)
RESERVE_0=$(jq -r '.assets[0].amount' <<<"$POOL")
RESERVE_1=$(jq -r '.assets[1].amount' <<<"$POOL")
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

echo "[grid 2/7] Creating two independently funded bots"
RESULT=$(execute_grid_from test1 "$CREATE" --amount 60000000uluna)
BOT_1=$(tx_event_value "$RESULT" bot_id)
ATTACKER_ADDRESS=$(provision_attacker)
TX_HASH=$(terrad_tx bank send test1 "$ATTACKER_ADDRESS" 500000000uluna | jq -r '.txhash')
wait_tx "$TX_HASH" >/dev/null
RESULT=$(execute_grid_from attacker "$CREATE" --amount 60000000uluna)
BOT_2=$(tx_event_value "$RESULT" bot_id)
test -n "$BOT_1"
test -n "$BOT_2"
test "$BOT_1" != "$BOT_2"

FUND=$(jq -nc --argjson bot_id "$BOT_1" '{fund_gas:{bot_id:$bot_id}}')
execute_grid_from test1 "$FUND" --amount 50000000uluna >/dev/null
test "$(query_grid "{\"bot\":{\"bot_id\":$BOT_1}}" | jq -r '.gas_credit')" = "110000000"

echo "[grid 3/7] Depositing and automatically allocating both assets"
deposit_grid_token test1 "$EMBER_ADDRESS" "$BOT_1" 10000000
deposit_grid_token test1 "$CORAL_ADDRESS" "$BOT_1" 10000000
cw20_transfer "$EMBER_ADDRESS" "$ATTACKER_ADDRESS" 2000000
cw20_transfer "$CORAL_ADDRESS" "$ATTACKER_ADDRESS" 2000000
deposit_grid_token attacker "$EMBER_ADDRESS" "$BOT_2" 2000000
deposit_grid_token attacker "$CORAL_ADDRESS" "$BOT_2" 2000000

ORDERS_BEFORE=$(query_grid "{\"orders\":{\"bot_id\":$BOT_1}}")
ASK_COUNT_BEFORE=$(jq '[.[] | select(.side == "ask")] | length' <<<"$ORDERS_BEFORE")
BID_COUNT_BEFORE=$(jq '[.[] | select(.side == "bid")] | length' <<<"$ORDERS_BEFORE")
test "$ASK_COUNT_BEFORE" -ge 1
test "$BID_COUNT_BEFORE" -ge 1
BOT_2_BEFORE=$(query_grid "{\"bot\":{\"bot_id\":$BOT_2}}")
BOT_2_ORDERS_BEFORE=$(query_grid "{\"orders\":{\"bot_id\":$BOT_2}}")

echo "[grid 4/7] Partially filling one CL8Y ask"
ASK_ID=$(jq -r '[.[] | select(.side == "ask")][0].order_id' <<<"$ORDERS_BEFORE")
ASK_PRICE=$(jq -r '[.[] | select(.side == "ask")][0].price' <<<"$ORDERS_BEFORE")
ASK_REMAINING=$(jq -r '[.[] | select(.side == "ask")][0].remaining' <<<"$ORDERS_BEFORE")
PARTIAL_OFFER=$(python3 -c '
from decimal import Decimal, ROUND_DOWN
import sys
value = (Decimal(sys.argv[1]) * Decimal(sys.argv[2]) / 2).to_integral_value(rounding=ROUND_DOWN)
print(max(1, int(value)))
' "$ASK_REMAINING" "$ASK_PRICE")
HOOK=$(jq -nc --arg amount "$PARTIAL_OFFER" --argjson hint "$ASK_ID" '
  {swap:{belief_price:null,max_spread:"1",min_return:"1",to:null,deadline:null,trader:null,
    hybrid:{pool_input:"0",book_input:$amount,max_maker_fills:1,book_start_hint:$hint}}}' \
    | base64 -w0)
SWAP=$(jq -nc --arg pair "$PAIR_ADDRESS" --arg amount "$PARTIAL_OFFER" --arg hook "$HOOK" \
    '{send:{contract:$pair,amount:$amount,msg:$hook}}')
execute_wait "$CORAL_ADDRESS" "$SWAP"

SETTLEMENT=$(terrad_query wasm contract-state smart "$PAIR_ADDRESS" \
    "{\"limit_order_settlement\":{\"order_id\":$ASK_ID}}" | jq -c '.data')
test "$(jq -r '.status' <<<"$SETTLEMENT")" = "open"
test "$(jq -r '.remaining' <<<"$SETTLEMENT")" -lt "$ASK_REMAINING"
test "$(jq -r '.cumulative_output' <<<"$SETTLEMENT")" -gt 0

echo "[grid 5/7] Reconciling exact output into only the opposite portion"
RECONCILE=$(jq -nc --argjson bot_id "$BOT_1" \
    '{reconcile:{bot_id:$bot_id,start_after:null}}')
RESULT=$(execute_grid_from test1 "$RECONCILE")
test "$(tx_event_value "$RESULT" changed_orders)" = "1"
test "$(tx_event_value "$RESULT" keeper_reward)" = "30000000"
ORDERS_AFTER=$(query_grid "{\"orders\":{\"bot_id\":$BOT_1}}")
test "$(jq --argjson id "$ASK_ID" '[.[] | select(.order_id == $id)] | length' \
    <<<"$ORDERS_AFTER")" = "1"
test "$(jq '[.[] | select(.side == "bid")] | length' <<<"$ORDERS_AFTER")" \
    -eq $((BID_COUNT_BEFORE + 1))

echo "[grid 6/7] Verifying cross-bot state isolation"
test "$(query_grid "{\"bot\":{\"bot_id\":$BOT_2}}")" = "$BOT_2_BEFORE"
test "$(query_grid "{\"orders\":{\"bot_id\":$BOT_2}}")" = "$BOT_2_ORDERS_BEFORE"

echo "[grid 7/7] Cancelling, settling, and withdrawing bot LP shares"
SHARES=$(query_grid "{\"shares\":{\"bot_id\":$BOT_1,\"address\":\"$TEST_ADDRESS\"}}" \
    | jq -r '.shares')
WITHDRAW=$(jq -nc --argjson bot_id "$BOT_1" --arg shares "$SHARES" \
    '{withdraw:{bot_id:$bot_id,shares:$shares,recipient:null}}')
expect_execute_failure test1 "$GRID_ADDRESS" "$WITHDRAW"
EMBER_BEFORE=$(cw20_balance "$EMBER_ADDRESS" "$TEST_ADDRESS")
CORAL_BEFORE=$(cw20_balance "$CORAL_ADDRESS" "$TEST_ADDRESS")
CANCEL=$(jq -nc --argjson bot_id "$BOT_1" '{cancel_all:{bot_id:$bot_id}}')
execute_grid_from test1 "$CANCEL" >/dev/null
execute_grid_from test1 "$WITHDRAW" >/dev/null
test "$(query_grid "{\"shares\":{\"bot_id\":$BOT_1,\"address\":\"$TEST_ADDRESS\"}}" \
    | jq -r '.shares')" = "0"
test "$(cw20_balance "$EMBER_ADDRESS" "$TEST_ADDRESS")" -gt "$EMBER_BEFORE"
test "$(cw20_balance "$CORAL_ADDRESS" "$TEST_ADDRESS")" -gt "$CORAL_BEFORE"

echo "Grid-manager signed integration suite passed."
