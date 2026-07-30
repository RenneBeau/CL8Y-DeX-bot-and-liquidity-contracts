#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=common.sh
source "$SCRIPT_DIR/common.sh"
load_local_env

query_pool() {
    terrad_query wasm contract-state smart "$PAIR_ADDRESS" '{"pool":{}}' | jq -c '.data'
}

query_vault_status() {
    terrad_query wasm contract-state smart "$VAULT_ADDRESS" \
        '{"rebalance_status":{}}' | jq -c '.data'
}

query_vault_plan() {
    terrad_query wasm contract-state smart "$VAULT_ADDRESS" \
        '{"rebalance_plan":{}}' | jq -c '.data'
}

query_vault_balances() {
    terrad_query wasm contract-state smart "$VAULT_ADDRESS" \
        '{"balances":{}}' | jq -c '.data.balances'
}

cw20_balance() {
    local token="$1"
    local address="$2"
    terrad_query wasm contract-state smart "$token" \
        "{\"balance\":{\"address\":\"$address\"}}" | jq -r '.data.balance'
}

token_supply() {
    local token="$1"
    terrad_query wasm contract-state smart "$token" '{"token_info":{}}' \
        | jq -r '.data.total_supply'
}

execute_wait() {
    local contract="$1"
    local message="$2"
    local tx_hash
    tx_hash=$(terrad_tx wasm execute "$contract" "$message" | jq -r '.txhash')
    wait_tx "$tx_hash" >/dev/null
}

execute_wait_from() {
    local signer="$1"
    local contract="$2"
    local message="$3"
    local tx_hash
    tx_hash=$(terrad_tx_from "$signer" wasm execute "$contract" "$message" | jq -r '.txhash')
    wait_tx "$tx_hash" >/dev/null
}

expect_execute_failure() {
    local signer="$1"
    local contract="$2"
    local message="$3"
    local output tx_hash
    if ! output=$(terrad_tx_from "$signer" wasm execute "$contract" "$message" 2>&1); then
        return 0
    fi
    tx_hash=$(jq -r '.txhash // empty' <<<"$output")
    if [ -z "$tx_hash" ]; then
        return 0
    fi
    if wait_tx "$tx_hash" >/dev/null 2>&1; then
        echo "ERROR: transaction unexpectedly succeeded: $message" >&2
        return 1
    fi
}

cw20_transfer() {
    local token="$1"
    local recipient="$2"
    local amount="$3"
    local message
    message=$(jq -nc --arg recipient "$recipient" --arg amount "$amount" \
        '{transfer:{recipient:$recipient,amount:$amount}}')
    execute_wait "$token" "$message"
}

approve_liquidity() {
    local signer="$1"
    local token="$2"
    local amount="$3"
    local message
    message=$(jq -nc --arg spender "$LIQUIDITY_ADDRESS" --arg amount "$amount" \
        '{increase_allowance:{spender:$spender,amount:$amount,expires:null}}')
    execute_wait_from "$signer" "$token" "$message"
}

pool_swap() {
    local offer_token="$1"
    local amount="$2"
    local deadline hook message
    deadline=$(($(date +%s) + 600))
    hook=$(jq -nc --argjson deadline "$deadline" '
        {swap:{belief_price:null,max_spread:"0.50",min_return:"1",to:null,
          deadline:$deadline,trader:null,hybrid:null}}' | base64 -w0)
    message=$(jq -nc --arg pair "$PAIR_ADDRESS" --arg amount "$amount" --arg hook "$hook" \
        '{send:{contract:$pair,amount:$amount,msg:$hook}}')
    execute_wait "$offer_token" "$message"
}

vault_rebalance_message() {
    local deadline="${1:-$(($(date +%s) + 600))}"
    jq -nc --argjson deadline "$deadline" '{rebalance:{deadline:$deadline}}'
}

provision_attacker() {
    local container attacker_address tx_hash
    container=$(localterra_container)
    if ! docker exec "$container" terrad keys show attacker \
        --keyring-backend test --address >/dev/null 2>&1; then
        docker exec "$container" terrad keys add attacker \
            --keyring-backend test --output json >/dev/null
    fi
    attacker_address=$(docker exec "$container" terrad keys show attacker \
        --keyring-backend test --address)
    tx_hash=$(terrad_tx bank send test1 "$attacker_address" 100000000uluna | jq -r '.txhash')
    wait_tx "$tx_hash" >/dev/null
    printf '%s\n' "$attacker_address"
}

proportional_deposit() {
    local signer="$1"
    local amount_0="$2"
    local amount_1="$3"
    local deadline message
    approve_liquidity "$signer" "$EMBER_ADDRESS" "$amount_0"
    approve_liquidity "$signer" "$CORAL_ADDRESS" "$amount_1"
    deadline=$(($(date +%s) + 600))
    message=$(jq -nc --arg a0 "$amount_0" --arg a1 "$amount_1" --argjson deadline "$deadline" '
      {deposit:{amounts:[$a0,$a1],min_shares:"1",deadline:$deadline,swap:null}}')
    execute_wait_from "$signer" "$LIQUIDITY_ADDRESS" "$message"
}
