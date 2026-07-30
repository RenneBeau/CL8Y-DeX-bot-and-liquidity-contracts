#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=test-lib.sh
source "$SCRIPT_DIR/test-lib.sh"

echo "[1/10] Verifying clean contract boundaries and fee tier"
VAULT_CONFIG=$(terrad_query wasm contract-state smart "$VAULT_ADDRESS" '{"config":{}}')
jq -e --arg proxy "$PROXY_ADDRESS" --arg pair "$PAIR_ADDRESS" \
    --arg liquidity "$LIQUIDITY_ADDRESS" \
    '.data.proxy == $proxy and .data.pair == $pair and
     .data.liquidity_contract == $liquidity and .data.rebalance_threshold_bps == 500' \
    <<<"$VAULT_CONFIG" >/dev/null
DISCOUNT=$(terrad_query wasm contract-state smart "$FEE_REGISTRY_ADDRESS" \
    "{\"get_discount\":{\"trader\":\"$PROXY_ADDRESS\",\"sender\":\"$PROXY_ADDRESS\"}}")
jq -e '.data.discount_bps == 5000 and .data.needs_deregister == false' \
    <<<"$DISCOUNT" >/dev/null
PAIR_INFO=$(terrad_query wasm contract-state smart "$PAIR_ADDRESS" '{"pair":{}}')
DEX_LP_TOKEN=$(jq -r '.data.liquidity_token' <<<"$PAIR_INFO")
test "$(cw20_balance "$DEX_LP_TOKEN" "$VAULT_ADDRESS")" = "0"

echo "[2/10] Verifying proxy and vault authorization isolation"
ATTACKER_ADDRESS=$(provision_attacker)
cw20_transfer "$EMBER_ADDRESS" "$ATTACKER_ADDRESS" "10000000"
DEADLINE=$(($(date +%s) + 600))
PROXY_HOOK=$(jq -nc --arg pair "$PAIR_ADDRESS" --argjson deadline "$DEADLINE" \
    '{swap:{pair:$pair,min_return:"1",max_spread:"0.10",deadline:$deadline}}' | base64 -w0)
DIRECT_PROXY=$(jq -nc --arg proxy "$PROXY_ADDRESS" --arg hook "$PROXY_HOOK" \
    '{send:{contract:$proxy,amount:"1000000",msg:$hook}}')
expect_execute_failure attacker "$EMBER_ADDRESS" "$DIRECT_PROXY"
UNAUTHORIZED_TRANSFER=$(jq -nc --arg token "$EMBER_ADDRESS" --arg recipient "$ATTACKER_ADDRESS" \
    '{transfer_to:{token:$token,amount:"1",recipient:$recipient}}')
expect_execute_failure attacker "$VAULT_ADDRESS" "$UNAUTHORIZED_TRANSFER"

echo "[3/10] Verifying first proportional deposit and bot LP mint"
POOL=$(query_pool)
RESERVE_0=$(jq -r '.assets[0].amount' <<<"$POOL")
RESERVE_1=$(jq -r '.assets[1].amount' <<<"$POOL")
DEPOSIT_0=$((RESERVE_0 / 100))
DEPOSIT_1=$((RESERVE_1 / 100))
proportional_deposit test1 "$DEPOSIT_0" "$DEPOSIT_1"
USER_SHARES=$(cw20_balance "$LIQUIDITY_ADDRESS" "$TEST_ADDRESS")
SUPPLY=$(token_supply "$LIQUIDITY_ADDRESS")
if [ "$USER_SHARES" -le 0 ] || [ $((SUPPLY - USER_SHARES)) -ne 1000 ]; then
    echo "ERROR: initial locked shares or user mint is incorrect" >&2
    exit 1
fi

echo "[4/10] Verifying donation-safe second-user share pricing"
cw20_transfer "$EMBER_ADDRESS" "$VAULT_ADDRESS" "100000000"
cw20_transfer "$CORAL_ADDRESS" "$VAULT_ADDRESS" "100000000"
ATTACKER_0=$((RESERVE_0 / 200))
ATTACKER_1=$((RESERVE_1 / 200))
cw20_transfer "$EMBER_ADDRESS" "$ATTACKER_ADDRESS" "$ATTACKER_0"
cw20_transfer "$CORAL_ADDRESS" "$ATTACKER_ADDRESS" "$ATTACKER_1"
proportional_deposit attacker "$ATTACKER_0" "$ATTACKER_1"
ATTACKER_SHARES=$(cw20_balance "$LIQUIDITY_ADDRESS" "$ATTACKER_ADDRESS")
if [ "$ATTACKER_SHARES" -le 0 ]; then
    echo "ERROR: second user received no bot LP shares" >&2
    exit 1
fi

echo "[5/10] Verifying single-token deposit settlement"
SINGLE_INPUT=$((RESERVE_0 / 1000))
SINGLE_SWAP=$((SINGLE_INPUT / 2))
approve_liquidity test1 "$EMBER_ADDRESS" "$SINGLE_INPUT"
DEADLINE=$(($(date +%s) + 600))
SINGLE_DEPOSIT=$(jq -nc --arg token "$EMBER_ADDRESS" --arg amount "$SINGLE_INPUT" \
    --arg swap "$SINGLE_SWAP" --argjson deadline "$DEADLINE" '
    {deposit:{amounts:[$amount,"0"],min_shares:"1",deadline:$deadline,
      swap:{offer_token:$token,amount:$swap,min_return:"1",max_spread:"0.10",deadline:$deadline}}}')
execute_wait "$LIQUIDITY_ADDRESS" "$SINGLE_DEPOSIT"

echo "[6/10] Verifying withdrawal at the vault's current A/B ratio"
ATTACKER_WITHDRAW=$((ATTACKER_SHARES / 2))
ATTACKER_EMBER_BEFORE=$(cw20_balance "$EMBER_ADDRESS" "$ATTACKER_ADDRESS")
ATTACKER_CORAL_BEFORE=$(cw20_balance "$CORAL_ADDRESS" "$ATTACKER_ADDRESS")
DEADLINE=$(($(date +%s) + 600))
PRO_RATA_WITHDRAW=$(jq -nc --arg shares "$ATTACKER_WITHDRAW" --argjson deadline "$DEADLINE" '
    {withdraw:{shares:$shares,recipient:null,deadline:$deadline,
      output:{pro_rata:{min_assets:["1","1"]}}}}')
execute_wait_from attacker "$LIQUIDITY_ADDRESS" "$PRO_RATA_WITHDRAW"
test "$(cw20_balance "$EMBER_ADDRESS" "$ATTACKER_ADDRESS")" -gt "$ATTACKER_EMBER_BEFORE"
test "$(cw20_balance "$CORAL_ADDRESS" "$ATTACKER_ADDRESS")" -gt "$ATTACKER_CORAL_BEFORE"

echo "[7/10] Verifying single-token proportional withdrawal"
USER_SHARES=$(cw20_balance "$LIQUIDITY_ADDRESS" "$TEST_ADDRESS")
WITHDRAW_SHARES=$((USER_SHARES / 10))
SUPPLY=$(token_supply "$LIQUIDITY_ADDRESS")
VAULT_BALANCES=$(query_vault_balances)
VAULT_0=$(jq -r '.[0]' <<<"$VAULT_BALANCES")
VAULT_1=$(jq -r '.[1]' <<<"$VAULT_BALANCES")
CLAIM_1=$((VAULT_1 * WITHDRAW_SHARES / SUPPLY))
DEADLINE=$(($(date +%s) + 600))
TOKEN0_WITHDRAW=$(jq -nc --arg shares "$WITHDRAW_SHARES" --arg token "$CORAL_ADDRESS" \
    --arg claim "$CLAIM_1" --argjson deadline "$DEADLINE" '
    {withdraw:{shares:$shares,recipient:null,deadline:$deadline,
      output:{token0:{min_amount:"1",swap:{offer_token:$token,amount:$claim,
        min_return:"1",max_spread:"0.10",deadline:$deadline}}}}}')
USER_EMBER_BEFORE=$(cw20_balance "$EMBER_ADDRESS" "$TEST_ADDRESS")
execute_wait "$LIQUIDITY_ADDRESS" "$TOKEN0_WITHDRAW"
test "$(cw20_balance "$EMBER_ADDRESS" "$TEST_ADDRESS")" -gt "$USER_EMBER_BEFORE"

echo "[8/10] Verifying 5% price trigger and constrained inventory rebalance"
POOL=$(query_pool)
RESERVE_0=$(jq -r '.assets[0].amount' <<<"$POOL")
pool_swap "$EMBER_ADDRESS" "$((RESERVE_0 * 4 / 100))"
sleep 2
STATUS=$(query_vault_status)
jq -e '.should_rebalance == true and .price_deviation_bps >= 500' <<<"$STATUS" >/dev/null
PLAN=$(query_vault_plan)
REBALANCE_TOKEN=$(jq -r '.offer_token' <<<"$PLAN")
REBALANCE_AMOUNT=$(jq -r '.amount' <<<"$PLAN")
PRE_DEVIATION=$(jq -r '.allocation_deviation_bps' <<<"$PLAN")
expect_execute_failure attacker "$VAULT_ADDRESS" "$(vault_rebalance_message)"
SHARES_BEFORE=$(token_supply "$LIQUIDITY_ADDRESS")
CORAL_BEFORE=$(cw20_balance "$REBALANCE_TOKEN" "$VAULT_ADDRESS")
CORRECT=$(vault_rebalance_message)
execute_wait "$VAULT_ADDRESS" "$CORRECT"
CORAL_AFTER=$(cw20_balance "$REBALANCE_TOKEN" "$VAULT_ADDRESS")
test $((CORAL_BEFORE - CORAL_AFTER)) -eq "$REBALANCE_AMOUNT"
test "$(token_supply "$LIQUIDITY_ADDRESS")" = "$SHARES_BEFORE"
POST_STATUS=$(query_vault_status)
jq -e --argjson pre "$PRE_DEVIATION" \
    '.allocation_deviation_bps < $pre and
     (if .allocation_deviation_bps <= 500 then .price_deviation_bps == 0
      else .price_deviation_bps >= 500 end)' <<<"$POST_STATUS" >/dev/null

echo "[9/10] Verifying no DEX LP custody and no protocol fee"
test "$(cw20_balance "$DEX_LP_TOKEN" "$VAULT_ADDRESS")" = "0"
test "$(cw20_balance "$DEX_LP_TOKEN" "$LIQUIDITY_ADDRESS")" = "0"
test "$(cw20_balance "$CL8Y_ADDRESS" "$PROXY_ADDRESS")" = "200000000000000000000"

echo "[10/10] Verifying failure limits"
expect_execute_failure test1 "$LIQUIDITY_ADDRESS" \
    '{"deposit":{"amounts":["0","0"],"min_shares":"1","deadline":9999999999,"swap":null}}'
expect_execute_failure attacker "$PROXY_ADDRESS" \
    "{\"withdraw_cl8y\":{\"amount\":\"1\",\"recipient\":\"$ATTACKER_ADDRESS\"}}"

echo "Clean bot-vault integration suite passed."
