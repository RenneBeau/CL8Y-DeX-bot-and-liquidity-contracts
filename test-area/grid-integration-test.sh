#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=test-lib.sh
source "$SCRIPT_DIR/test-lib.sh"

execute_grid_from() {
    local signer="$1"
    local manager="$2"
    local message="$3"
    shift 3
    local tx_hash
    tx_hash=$(terrad_tx_from "$signer" wasm execute "$manager" "$message" "$@" \
        | jq -r '.txhash')
    wait_tx "$tx_hash"
}

query_grid() {
    local manager="$1"
    local message="$2"
    terrad_query wasm contract-state smart "$manager" "$message" | jq -c '.data'
}

deposit_grid_token() {
    local signer="$1"
    local token="$2"
    local manager="$3"
    local bot_id="$4"
    local amount="$5"
    local hook message
    hook=$(jq -nc --argjson bot_id "$bot_id" '{deposit:{bot_id:$bot_id}}' | base64 -w0)
    message=$(jq -nc --arg manager "$manager" --arg amount "$amount" --arg hook "$hook" \
        '{send:{contract:$manager,amount:$amount,msg:$hook}}')
    execute_wait_from "$signer" "$token" "$message"
}

fill_first_grid_ask() {
    local manager="$1"
    local pair="$2"
    local token1="$3"
    local bot_id="$4"
    local orders ask_price ask_remaining partial_offer hook swap tx_hash swap_result fill_event
    orders=$(query_grid "$manager" "{\"orders\":{\"bot_id\":$bot_id}}")
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
    LAST_FILL_HEIGHT=$(jq -r '.height' <<<"$swap_result")
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
    LAST_CHAIN_REMAINING=$(terrad_query wasm contract-state smart "$pair" \
        "{\"limit_order\":{\"order_id\":$LAST_ASK_ID}}" | jq -r '.data.remaining')
    test "$LAST_CHAIN_REMAINING" -lt "$ask_remaining"
}

reconcile_last_fill() {
    local manager="$1"
    local _bot_id="$2"
    local _pair="$3"
    local database result replay tx_hash
    database=$(mktemp)
    rm -f "$database"
    operator() {
        env PYTHONPATH="$PROJECT_ROOT/grid-operator-system/services/grid-operator" \
            GRID_DB_PATH="$database" GRID_RPC_URL=http://127.0.0.1:26657 \
            GRID_CHAIN_ID=localterra GRID_DEPLOYMENT_HEIGHT="$LAST_FILL_HEIGHT" \
            GRID_VAULTS="$manager" GRID_FINALITY_DEPTH=0 \
            GRID_TERRAD="$SCRIPT_DIR/terrad-wrapper.sh" GRID_KEY_NAME=gridkeeper \
            GRID_KEYRING_BACKEND=test GRID_FEES=20000000uluna \
            GRID_TX_POLL_SECONDS=1 GRID_TX_TIMEOUT_SECONDS=60 \
            python3 -m grid_operator.cli "$@"
    }
    operator index >/dev/null
    result=$(operator keep)
    jq -e --arg manager "$manager" '.[$manager] == "confirmed"' <<<"$result" >/dev/null
    replay=$(operator keep)
    jq -e --arg manager "$manager" '.[$manager] == "idle"' <<<"$replay" >/dev/null
    tx_hash=$(python3 -c 'import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute(
      "select tx_hash from batches where state=\"confirmed\" order by id desc limit 1").fetchone()[0])' \
        "$database")
    rm -f "$database" "$database-wal" "$database-shm"
    terrad_query tx "$tx_hash"
}

echo "[grid 1/10] Verifying manager fee tier on standard CL8Y pair"
for manager in "$GRID_ADDRESS_1" "$GRID_ADDRESS_2" "$GRID_ADDRESS_3" "$GRID_ADDRESS_4"; do
    DISCOUNT=$(terrad_query wasm contract-state smart "$FEE_REGISTRY_ADDRESS" \
        "{\"get_discount\":{\"trader\":\"$manager\",\"sender\":\"$manager\"}}")
    jq -e '.data.discount_bps == 5000 and .data.needs_deregister == false' \
        <<<"$DISCOUNT" >/dev/null
done

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

echo "[grid 2/10] Creating one bot in each of two dedicated vaults"
RESULT=$(execute_grid_from test1 "$GRID_ADDRESS_1" "$CREATE" --amount 60000000uluna)
BOT_1=$(tx_event_value "$RESULT" bot_id)
ATTACKER_ADDRESS=$(provision_attacker)
TX_HASH=$(terrad_tx bank send test1 "$ATTACKER_ADDRESS" 500000000uluna | jq -r '.txhash')
wait_tx "$TX_HASH" >/dev/null
RESULT=$(execute_grid_from attacker "$GRID_ADDRESS_2" "$CREATE" --amount 60000000uluna)
BOT_2=$(tx_event_value "$RESULT" bot_id)
test -n "$BOT_1"
test -n "$BOT_2"
test "$BOT_1" = "1"
test "$BOT_2" = "1"
expect_execute_failure test1 "$GRID_ADDRESS_1" "$CREATE"

FUND=$(jq -nc --argjson bot_id "$BOT_1" '{fund_gas:{bot_id:$bot_id}}')
execute_grid_from test1 "$GRID_ADDRESS_1" "$FUND" --amount 50000000uluna >/dev/null
test "$(query_grid "$GRID_ADDRESS_1" "{\"bot\":{\"bot_id\":$BOT_1}}" | jq -r '.gas_credit')" = "110000000"

echo "[grid 3/10] Depositing and automatically allocating both assets"
deposit_grid_token test1 "$EMBER_ADDRESS" "$GRID_ADDRESS_1" "$BOT_1" 10000000
deposit_grid_token test1 "$CORAL_ADDRESS" "$GRID_ADDRESS_1" "$BOT_1" 10000000
cw20_transfer "$EMBER_ADDRESS" "$ATTACKER_ADDRESS" 2000000
cw20_transfer "$CORAL_ADDRESS" "$ATTACKER_ADDRESS" 2000000
deposit_grid_token attacker "$EMBER_ADDRESS" "$GRID_ADDRESS_2" "$BOT_2" 2000000
deposit_grid_token attacker "$CORAL_ADDRESS" "$GRID_ADDRESS_2" "$BOT_2" 2000000

ORDERS_BEFORE=$(query_grid "$GRID_ADDRESS_1" "{\"orders\":{\"bot_id\":$BOT_1}}")
ASK_COUNT_BEFORE=$(jq '[.[] | select(.side == "ask")] | length' <<<"$ORDERS_BEFORE")
BID_COUNT_BEFORE=$(jq '[.[] | select(.side == "bid")] | length' <<<"$ORDERS_BEFORE")
test "$ASK_COUNT_BEFORE" -ge 1
test "$BID_COUNT_BEFORE" -ge 1
BOT_2_BEFORE=$(query_grid "$GRID_ADDRESS_2" "{\"bot\":{\"bot_id\":$BOT_2}}")
BOT_2_ORDERS_BEFORE=$(query_grid "$GRID_ADDRESS_2" "{\"orders\":{\"bot_id\":$BOT_2}}")
BOT_1_BEFORE=$(query_grid "$GRID_ADDRESS_1" "{\"bot\":{\"bot_id\":$BOT_1}}")

echo "[grid 4/10] Partially filling one CL8Y ask"
fill_first_grid_ask "$GRID_ADDRESS_1" "$PAIR_ADDRESS" "$CORAL_ADDRESS" "$BOT_1"

echo "[grid 5/10] Reconciling from pair state and the vault balance"
RESULT=$(reconcile_last_fill "$GRID_ADDRESS_1" "$BOT_1" "$PAIR_ADDRESS")
test "$(tx_event_value "$RESULT" changed_orders)" = "1"
test "$(tx_event_value "$RESULT" keeper_reward)" = "30000000"
ORDERS_AFTER=$(query_grid "$GRID_ADDRESS_1" "{\"orders\":{\"bot_id\":$BOT_1}}")
test "$(jq --argjson id "$LAST_ASK_ID" '[.[] | select(.order_id == $id)] | length' \
    <<<"$ORDERS_AFTER")" = "1"
test "$(jq '[.[] | select(.side == "bid")] | length' <<<"$ORDERS_AFTER")" \
    -eq "$BID_COUNT_BEFORE"
BOT_1_AFTER=$(query_grid "$GRID_ADDRESS_1" "{\"bot\":{\"bot_id\":$BOT_1}}")
jq -e --argjson before "$BOT_1_BEFORE" --arg output "$LAST_FILL_OUTPUT" '
  .free_balances[0] == $before.free_balances[0] and
  (.free_balances[1] | tonumber) == (($before.free_balances[1] | tonumber) + ($output | tonumber))
' <<<"$BOT_1_AFTER" >/dev/null

echo "[grid 6/10] Verifying cross-bot state isolation"
test "$(query_grid "$GRID_ADDRESS_2" "{\"bot\":{\"bot_id\":$BOT_2}}")" = "$BOT_2_BEFORE"
test "$(query_grid "$GRID_ADDRESS_2" "{\"orders\":{\"bot_id\":$BOT_2}}")" = "$BOT_2_ORDERS_BEFORE"

echo "[grid 7/10] Cancelling, settling, and withdrawing bot LP shares"
SHARES=$(query_grid "$GRID_ADDRESS_1" \
    "{\"shares\":{\"bot_id\":$BOT_1,\"address\":\"$TEST_ADDRESS\"}}" \
    | jq -r '.shares')
WITHDRAW=$(jq -nc --argjson bot_id "$BOT_1" --arg shares "$SHARES" \
    '{withdraw:{bot_id:$bot_id,shares:$shares,recipient:null}}')
expect_execute_failure test1 "$GRID_ADDRESS_1" "$WITHDRAW"
EMBER_BEFORE=$(cw20_balance "$EMBER_ADDRESS" "$TEST_ADDRESS")
CORAL_BEFORE=$(cw20_balance "$CORAL_ADDRESS" "$TEST_ADDRESS")
CANCEL=$(jq -nc --argjson bot_id "$BOT_1" '{cancel_all:{bot_id:$bot_id}}')
execute_grid_from test1 "$GRID_ADDRESS_1" "$CANCEL" >/dev/null
execute_grid_from test1 "$GRID_ADDRESS_1" "$WITHDRAW" >/dev/null
test "$(query_grid "$GRID_ADDRESS_1" \
    "{\"shares\":{\"bot_id\":$BOT_1,\"address\":\"$TEST_ADDRESS\"}}" \
    | jq -r '.shares')" = "0"
test "$(cw20_balance "$EMBER_ADDRESS" "$TEST_ADDRESS")" -gt "$EMBER_BEFORE"
test "$(cw20_balance "$CORAL_ADDRESS" "$TEST_ADDRESS")" -gt "$CORAL_BEFORE"

BOT_2_SHARES=$(query_grid "$GRID_ADDRESS_2" \
    "{\"shares\":{\"bot_id\":$BOT_2,\"address\":\"$ATTACKER_ADDRESS\"}}" | jq -r '.shares')
ATTACKER_EMBER_BEFORE=$(cw20_balance "$EMBER_ADDRESS" "$ATTACKER_ADDRESS")
ATTACKER_CORAL_BEFORE=$(cw20_balance "$CORAL_ADDRESS" "$ATTACKER_ADDRESS")
CANCEL_2=$(jq -nc --argjson bot_id "$BOT_2" '{cancel_all:{bot_id:$bot_id}}')
WITHDRAW_2=$(jq -nc --argjson bot_id "$BOT_2" --arg shares "$BOT_2_SHARES" \
    '{withdraw:{bot_id:$bot_id,shares:$shares,recipient:null}}')
execute_grid_from attacker "$GRID_ADDRESS_2" "$CANCEL_2" >/dev/null
execute_grid_from attacker "$GRID_ADDRESS_2" "$WITHDRAW_2" >/dev/null
test "$(cw20_balance "$EMBER_ADDRESS" "$ATTACKER_ADDRESS")" -gt "$ATTACKER_EMBER_BEFORE"
test "$(cw20_balance "$CORAL_ADDRESS" "$ATTACKER_ADDRESS")" -gt "$ATTACKER_CORAL_BEFORE"

echo "[grid 8/10] Creating two dedicated vaults on the second CL8Y pair"
PAIR_2_INFO=$(terrad_query wasm contract-state smart "$SECOND_PAIR_ADDRESS" '{"pair":{}}')
jq -e --arg token0 "$LUNC_C_ADDRESS" --arg token1 "$EMBER_ADDRESS" \
    '.data.asset_infos[0].token.contract_addr == $token0 and
     .data.asset_infos[1].token.contract_addr == $token1' <<<"$PAIR_2_INFO" >/dev/null
POOL_2=$(terrad_query wasm contract-state smart "$SECOND_PAIR_ADDRESS" '{"pool":{}}' | jq -c '.data')
RESERVE_2_0=$(jq -r '.assets[0].amount' <<<"$POOL_2")
RESERVE_2_1=$(jq -r '.assets[1].amount' <<<"$POOL_2")
read -r LOWER_2 UPPER_2 < <(python3 -c '
import sys
a, b = map(int, sys.argv[1:])
scale = 10**18
price = b * scale // a
def render(value):
    text = f"{value // scale}.{value % scale:018d}".rstrip("0").rstrip(".")
    return text or "0"
print(render(price * 8 // 10), render(price * 12 // 10))
' "$RESERVE_2_0" "$RESERVE_2_1")
CREATE_2=$(jq -nc --arg pair "$SECOND_PAIR_ADDRESS" --arg lower "$LOWER_2" \
    --arg upper "$UPPER_2" \
    '{create_bot:{pair:$pair,lower_price:$lower,upper_price:$upper,grid_count:5}}')
RESULT=$(execute_grid_from test1 "$GRID_ADDRESS_3" "$CREATE_2" --amount 60000000uluna)
BOT_3=$(tx_event_value "$RESULT" bot_id)
RESULT=$(execute_grid_from attacker "$GRID_ADDRESS_4" "$CREATE_2" --amount 60000000uluna)
BOT_4=$(tx_event_value "$RESULT" bot_id)
deposit_grid_token test1 "$LUNC_C_ADDRESS" "$GRID_ADDRESS_3" "$BOT_3" 8000000
deposit_grid_token test1 "$EMBER_ADDRESS" "$GRID_ADDRESS_3" "$BOT_3" 8000000
cw20_transfer "$LUNC_C_ADDRESS" "$ATTACKER_ADDRESS" 2000000
cw20_transfer "$EMBER_ADDRESS" "$ATTACKER_ADDRESS" 2000000
deposit_grid_token attacker "$LUNC_C_ADDRESS" "$GRID_ADDRESS_4" "$BOT_4" 2000000
deposit_grid_token attacker "$EMBER_ADDRESS" "$GRID_ADDRESS_4" "$BOT_4" 2000000
BOT_4_BEFORE=$(query_grid "$GRID_ADDRESS_4" "{\"bot\":{\"bot_id\":$BOT_4}}")
BOT_4_ORDERS_BEFORE=$(query_grid "$GRID_ADDRESS_4" "{\"orders\":{\"bot_id\":$BOT_4}}")
BOT_3_BIDS_BEFORE=$(query_grid "$GRID_ADDRESS_3" "{\"orders\":{\"bot_id\":$BOT_3}}" \
    | jq '[.[] | select(.side == "bid")] | length')

echo "[grid 9/10] Filling and reconciling the second pair with the same keeper"
fill_first_grid_ask "$GRID_ADDRESS_3" "$SECOND_PAIR_ADDRESS" "$EMBER_ADDRESS" "$BOT_3"
RECONCILE_2=$(jq -nc --argjson bot_id "$BOT_3" --argjson order_id "$LAST_ASK_ID" \
    '{reconcile:{bot_id:$bot_id,order_ids:[$order_id]}}')
RESULT=$(execute_grid_from attacker "$GRID_ADDRESS_3" "$RECONCILE_2")
test "$(tx_event_value "$RESULT" changed_orders)" = "1"
test "$(query_grid "$GRID_ADDRESS_3" "{\"orders\":{\"bot_id\":$BOT_3}}" \
    | jq '[.[] | select(.side == "bid")] | length')" -eq "$BOT_3_BIDS_BEFORE"
test "$(query_grid "$GRID_ADDRESS_4" "{\"bot\":{\"bot_id\":$BOT_4}}")" = "$BOT_4_BEFORE"
test "$(query_grid "$GRID_ADDRESS_4" "{\"orders\":{\"bot_id\":$BOT_4}}")" = "$BOT_4_ORDERS_BEFORE"

echo "[grid 10/10] Closing both second-pair bots and verifying solvency"
for owner_bot in \
    "test1:$GRID_ADDRESS_3:$BOT_3:$TEST_ADDRESS" \
    "attacker:$GRID_ADDRESS_4:$BOT_4:$ATTACKER_ADDRESS"; do
    IFS=: read -r signer manager bot_id owner_address <<<"$owner_bot"
    shares=$(query_grid "$manager" \
        "{\"shares\":{\"bot_id\":$bot_id,\"address\":\"$owner_address\"}}" | jq -r '.shares')
    cancel=$(jq -nc --argjson bot_id "$bot_id" '{cancel_all:{bot_id:$bot_id}}')
    withdraw=$(jq -nc --argjson bot_id "$bot_id" --arg shares "$shares" \
        '{withdraw:{bot_id:$bot_id,shares:$shares,recipient:null}}')
    execute_grid_from "$signer" "$manager" "$cancel" >/dev/null
    execute_grid_from "$signer" "$manager" "$withdraw" >/dev/null
    test "$(query_grid "$manager" \
        "{\"shares\":{\"bot_id\":$bot_id,\"address\":\"$owner_address\"}}" | jq -r '.shares')" = "0"
    solvency=$(query_grid "$manager" "{\"solvency\":{\"bot_id\":$bot_id}}")
    jq -e '
      .token_0_expected == .token_0_actual and
      .token_1_expected == .token_1_actual and
      .unverifiable_orders == 0 and
      (.warnings | length) == 0
    ' <<<"$solvency" >/dev/null
done

echo "Grid-manager signed integration suite passed."
