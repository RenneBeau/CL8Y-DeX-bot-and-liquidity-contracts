#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=test-lib.sh
source "$SCRIPT_DIR/test-lib.sh"

ROUNDS="${SOAK_ROUNDS:-25}"
if ! [[ "$ROUNDS" =~ ^[1-9][0-9]*$ ]]; then
    echo "ERROR: SOAK_ROUNDS must be a positive integer." >&2
    exit 1
fi

if [ "$(token_supply "$LIQUIDITY_ADDRESS")" = "0" ]; then
    POOL=$(query_pool)
    proportional_deposit test1 \
        "$(( $(jq -r '.assets[0].amount' <<<"$POOL") / 100 ))" \
        "$(( $(jq -r '.assets[1].amount' <<<"$POOL") / 100 ))"
fi

STARTED_AT=$(date +%s)
echo "Starting $ROUNDS alternating inventory-rebalance rounds..."
for ROUND in $(seq 1 "$ROUNDS"); do
    POOL=$(query_pool)
    RESERVE_0=$(jq -r '.assets[0].amount' <<<"$POOL")
    RESERVE_1=$(jq -r '.assets[1].amount' <<<"$POOL")
    VAULT_BALANCES=$(query_vault_balances)
    BALANCE_0=$(jq -r '.[0]' <<<"$VAULT_BALANCES")
    BALANCE_1=$(jq -r '.[1]' <<<"$VAULT_BALANCES")
    if [ $((ROUND % 2)) -eq 1 ]; then
        SHOCK_TOKEN="$EMBER_ADDRESS"
        SHOCK_AMOUNT=$((RESERVE_0 * 4 / 100))
        REBALANCE_TOKEN="$CORAL_ADDRESS"
        REBALANCE_AMOUNT=0
    else
        SHOCK_TOKEN="$CORAL_ADDRESS"
        SHOCK_AMOUNT=$((RESERVE_1 * 4 / 100))
        REBALANCE_TOKEN="$EMBER_ADDRESS"
        REBALANCE_AMOUNT=0
    fi
    pool_swap "$SHOCK_TOKEN" "$SHOCK_AMOUNT"
    jq -e '.should_rebalance == true' <<<"$(query_vault_status)" >/dev/null
    POOL=$(query_pool)
    VAULT_BALANCES=$(query_vault_balances)
    REBALANCE_AMOUNT=$(calculate_rebalance_amount \
        "$REBALANCE_TOKEN" "$POOL" "$VAULT_BALANCES")
    BEFORE=$(cw20_balance "$REBALANCE_TOKEN" "$VAULT_ADDRESS")
    SHARES_BEFORE=$(token_supply "$LIQUIDITY_ADDRESS")
    execute_wait "$VAULT_ADDRESS" \
        "$(vault_rebalance_message "$REBALANCE_TOKEN" "$REBALANCE_AMOUNT")"
    AFTER=$(cw20_balance "$REBALANCE_TOKEN" "$VAULT_ADDRESS")
    test $((BEFORE - AFTER)) -eq "$REBALANCE_AMOUNT"
    test "$(token_supply "$LIQUIDITY_ADDRESS")" = "$SHARES_BEFORE"
    jq -e '.price_deviation_bps == 0' <<<"$(query_vault_status)" >/dev/null
    printf 'Round %d/%d passed\n' "$ROUND" "$ROUNDS"
done
echo "Soak test passed: $ROUNDS rounds in $(($(date +%s) - STARTED_AT))s."
