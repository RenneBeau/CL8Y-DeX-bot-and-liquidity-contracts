#!/usr/bin/env bash
# On-chain E2E: per-executed-swap protocol fees for market-grid (grid-vault-swap)
# and rebalancer (bot-vault), sharing the already-tested fee-registry / fee-collector
# from fee-e2e-test.sh. Proves:
#   * market-grid charges fee_bps against the executed swap value (reply), mints LP to
#     the collector, collector redeem -> dummy treasury.
#   * rebalancer accrues FEE_SHARES against the executed rebalance swap (reply),
#     collector RedeemShares pays out a pro-rata slice of both vault balances.
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
export CL8Y_DEX_DIR=${CL8Y_DEX_DIR:-/home/rennebeau/Liquidity-trading-bot/test-area/.cache/cl8y-dex-terraclassic}
# shellcheck source=common.sh
source "$SCRIPT_DIR/common.sh"
load_local_env
# shellcheck disable=SC1091
source "$SCRIPT_DIR/.fee-e2e-artifacts"

CONTAINER=$(localterra_container)

log() { echo "[fee-e2e-multi] $*"; }

wait_expect_fail() {
    local contract="$1" message="$2" output tx_hash
    if ! output=$(terrad_tx wasm execute "$contract" "$message" 2>&1); then
        return 0
    fi
    tx_hash=$(jq -r '.txhash // empty' <<<"$output")
    [ -z "$tx_hash" ] && return 0
    if wait_tx "$tx_hash" >/dev/null 2>&1; then
        echo "ERROR: expected transaction to fail: $message" >&2
        return 1
    fi
}

store_contract() {
    local artifact="$1" tx_hash result
    docker cp "$artifact" "$CONTAINER:/tmp/fee-multi.wasm"
    tx_hash=$(terrad_tx wasm store "/tmp/fee-multi.wasm" | jq -r '.txhash')
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

now_sec() { date +%s; }

# A plain pool swap churns the pair so Observe(TWAP) has fresh cumulative history.
refresh_twap() {
    local amount="$1" hook swap tx_hash
    hook="$(jq -nc --arg amount "$amount" '{swap:{belief_price:null,max_spread:"1",min_return:"1",to:null,deadline:null,trader:null,hybrid:{pool_input:$amount,book_input:"0",max_maker_fills:1,book_start_hint:null}}}' | base64 -w0)"
    swap=$(jq -nc --arg pair "$PAIR_ADDRESS" --arg amount "$amount" --arg hook "$hook" \
        '{send:{contract:$pair,amount:$amount,msg:$hook}}')
    tx_hash=$(terrad_tx wasm execute "$EMBER_ADDRESS" "$swap" | jq -r '.txhash')
    wait_tx "$tx_hash" >/dev/null
}

cw20_balance_of() {
    local token="$1" address="$2"
    query_smart "$token" "{\"balance\":{\"address\":\"$address\"}}" | jq -r '.data.balance'
}

deposit_market_token() {
    local signer="$1" token="$2" vault="$3" amount="$4" hook message
    hook=$(printf '{"deposit":{}}' | base64 -w0)
    message=$(jq -nc --arg vault "$vault" --arg amount "$amount" --arg hook "$hook" \
        '{send:{contract:$vault,amount:$amount,msg:$hook}}')
    execute_from "$signer" "$token" "$message" >/dev/null
}

transfer_token_to() {
    local signer="$1" token="$2" recipient="$3" amount="$4" message
    message=$(jq -nc --arg recipient "$recipient" --arg amount "$amount" \
        '{transfer:{recipient:$recipient,amount:$amount}}')
    execute_from "$signer" "$token" "$message" >/dev/null
}

log "== reuse fee-system wiring from fee-e2e (still live) =="
REG_CONFIG=$(query_smart "$FEE_REGISTRY" '{"config":{}}')
jq -e --arg collector "$FEE_COLLECTOR" '.data.fee_collector == $collector and .data.base_fee_bps == 180' \
    <<<"$REG_CONFIG" >/dev/null
log "registry=$FEE_REGISTRY collector=$FEE_COLLECTOR treasury=$FEE_TREASURY base_fee_bps=180"

log "== store market-grid + rebalancer wasms =="
MG_CODE_ID=$(store_contract "$PROJECT_ROOT/market-grid-system/target/wasm32-unknown-unknown/release/cl8y_grid_vault_swap.wasm")
BV_CODE_ID=$(store_contract "$PROJECT_ROOT/rebalancer-system/target/wasm32-unknown-unknown/release/cl8y_bot_vault.wasm")
SP_CODE_ID=$(store_contract "$PROJECT_ROOT/rebalancer-system/target/wasm32-unknown-unknown/release/cl8y_swap_proxy.wasm")
LIQ_CODE_ID=$(store_contract "$PROJECT_ROOT/rebalancer-system/target/wasm32-unknown-unknown/release/cl8y_bot_liquidity.wasm")
log "code ids: market_grid=$MG_CODE_ID bot_vault=$BV_CODE_ID swap_proxy=$SP_CODE_ID bot_liquidity=$LIQ_CODE_ID"

log "-- deploy ONE shared swap-proxy used by both the market-grid and the rebalancer --"
SP_INIT=$(jq -nc --arg admin "$TEST_ADDRESS" \
    '{admin:$admin}')
SP_PROXY=$(instantiate_contract "$SP_CODE_ID" "$SP_INIT" cl8y-fee-e2e-shared-swap-proxy)
log "shared swap-proxy: $SP_PROXY"

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

############################################################################
echo
log "############ MARKET-GRID (grid-vault-swap) ############"
echo
refresh_twap 100000000
PAIR_CODE_ID=$(terrad_query wasm contract "$PAIR_ADDRESS" | jq -er '.contract_info.code_id')
MG_INIT=$(jq -nc --arg admin "$TEST_ADDRESS" --arg pair "$PAIR_ADDRESS" --argjson twap 60 \
    --argjson grid_count 5 --arg lower "$LOWER_PRICE" --arg upper "$UPPER_PRICE" \
    --arg factory "$FACTORY_ADDRESS" --argjson pair_code_id "$PAIR_CODE_ID" \
    --arg registry "$FEE_REGISTRY" --arg collector "$FEE_COLLECTOR" \
    --arg proxy "$SP_PROXY" \
    '{admin:$admin,pair:$pair,factory:$factory,pair_code_id:$pair_code_id,
      twap_window_seconds:$twap,grid_count:$grid_count,
      lower_price:$lower,upper_price:$upper,allocation_tolerance_bps:600,
      max_spread:"0.1",max_execution_deviation_bps:1000,quote_slippage_bps:500,
      max_spot_twap_deviation_bps:1000,
      fee_registry:$registry,fee_collector:$collector,proxy:$proxy}')
MG_VAULT=$(instantiate_contract "$MG_CODE_ID" "$MG_INIT" cl8y-fee-e2e-market-grid)
log "market-grid vault: $MG_VAULT"

log "-- register the market-grid route on the SHARED proxy (single provider) --"
MG_REG=$(jq -nc --arg vault "$MG_VAULT" --arg pair "$PAIR_ADDRESS" \
    '{register_vault:{vault:$vault,pair:$pair}}')
execute_from "$TEST_ADDRESS" "$SP_PROXY" "$MG_REG" >/dev/null
log "market-grid route registered on shared proxy"

log "-- deposit EMBER only (token0), forcing allocation deviation = 10000 bps --"
deposit_market_token test1 "$EMBER_ADDRESS" "$MG_VAULT" 10000000000
STATUS=$(query_smart "$MG_VAULT" '{"grid_status":{}}')
jq -e '.data.should_rebalance == true and .data.allocation_deviation_bps == 10000' \
    <<<"$STATUS" >/dev/null
log "grid_status: should_rebalance=true deviation=10000"

log "-- rebalance: swap EMBER->CORAL via pair; fee charged in reply --"
refresh_twap 100000000
MG_DEADLINE=$(($(now_sec) + 300))
MG_RES=$(execute_from test1 "$MG_VAULT" "{\"rebalance\":{\"deadline\":$MG_DEADLINE}}")
MG_BPS=$(tx_event_value "$MG_RES" fee_bps)
MG_TIER=$(tx_event_value "$MG_RES" fee_tier)
MG_SRC=$(tx_event_value "$MG_RES" fee_source)
MG_SHARES=$(tx_event_value "$MG_RES" fee_shares)
log "market-grid fee: fee_bps=$MG_BPS fee_tier=$MG_TIER fee_source=$MG_SRC fee_shares=$MG_SHARES"
# The market-grid bills its operator (config.admin = TEST_ADDRESS), who holds
# 200 CL8Y -> tier 5 -> 50% discount -> 90 bps (see FEE_TIER_PROTOCOL §5).
test "$MG_BPS" = "90"
test "$MG_TIER" = "5"
test "$MG_SRC" = "live"
test "$MG_SHARES" -gt 0

MG_COLLECTOR_SHARES=$(query_smart "$MG_VAULT" \
    "{\"shares\":{\"bot_id\":0,\"address\":\"$FEE_COLLECTOR\"}}" | jq -r '.data.shares')
test "$MG_COLLECTOR_SHARES" -gt 0
test "$MG_COLLECTOR_SHARES" = "$MG_SHARES"
log "collector LP in market vault: $MG_COLLECTOR_SHARES (matches fee_shares)"

log "-- collector collect -> dummy treasury --"
MSG_TREASURY_0_BEFORE=$(cw20_balance_of "$EMBER_ADDRESS" "$FEE_TREASURY")
MSG_TREASURY_1_BEFORE=$(cw20_balance_of "$CORAL_ADDRESS" "$FEE_TREASURY")
COLLECT=$(jq -nc --arg vault "$MG_VAULT" --argjson bot_id 0 '{collect:{vault:$vault,bot_id:$bot_id}}')
wait_tx "$(terrad_tx_from gridkeeper wasm execute "$FEE_COLLECTOR" "$COLLECT" | jq -r '.txhash')" >/dev/null
MSG_TREASURY_0_AFTER=$(cw20_balance_of "$EMBER_ADDRESS" "$FEE_TREASURY")
MSG_TREASURY_1_AFTER=$(cw20_balance_of "$CORAL_ADDRESS" "$FEE_TREASURY")
test "$MSG_TREASURY_0_AFTER" -gt "$MSG_TREASURY_0_BEFORE"
test "$MSG_TREASURY_1_AFTER" -gt "$MSG_TREASURY_1_BEFORE"
MG_COLLECTOR_SHARES_ZERO=$(query_smart "$MG_VAULT" \
    "{\"shares\":{\"bot_id\":0,\"address\":\"$FEE_COLLECTOR\"}}" | jq -r '.data.shares')
test "$MG_COLLECTOR_SHARES_ZERO" = "0"
log "treasury received EMBER +$((MSG_TREASURY_0_AFTER - MSG_TREASURY_0_BEFORE)) CORAL +$((MSG_TREASURY_1_AFTER - MSG_TREASURY_1_BEFORE)); collector shares now 0"

############################################################################
echo
log "############ REBALANCER (bot-vault via the SAME shared swap-proxy) ############"
echo
log "reusing shared proxy $SP_PROXY (already registered market-grid route)"

refresh_twap 100000000
SECOND_PAIR_CODE_ID=$(terrad_query wasm contract "$SECOND_PAIR_ADDRESS" | jq -er '.contract_info.code_id')
BV_INIT=$(jq -nc --arg admin "$TEST_ADDRESS" --arg keeper "$GRID_KEEPER_ADDRESS" \
    --arg proxy "$SP_PROXY" --arg pair "$SECOND_PAIR_ADDRESS" --argjson twap 60 \
    --arg factory "$FACTORY_ADDRESS" --argjson pair_code_id "$SECOND_PAIR_CODE_ID" \
    --argjson liquidity_code_id "$LIQ_CODE_ID" \
    --arg registry "$FEE_REGISTRY" --arg collector "$FEE_COLLECTOR" \
    '{admin:$admin,keeper:$keeper,proxy:$proxy,pair:$pair,factory:$factory,pair_code_id:$pair_code_id,
      twap_window_seconds:$twap,
      liquidity_code_id:$liquidity_code_id,allocation_tolerance_bps:600,
      max_trade_bps:5000,rebalance_threshold_bps:2000,max_spread:"0.1",
      max_execution_deviation_bps:1000,quote_slippage_bps:500,
      max_spot_twap_deviation_bps:1000,
      fee_registry:$registry,fee_collector:$collector}')
BV_VAULT=$(instantiate_contract "$BV_CODE_ID" "$BV_INIT" cl8y-fee-e2e-bot-vault)
log "bot-vault: $BV_VAULT"

REG=$(jq -nc --arg vault "$BV_VAULT" --arg pair "$SECOND_PAIR_ADDRESS" \
    '{register_vault:{vault:$vault,pair:$pair}}')
execute_from "$TEST_ADDRESS" "$SP_PROXY" "$REG" >/dev/null
log "swap-proxy route registered for bot-vault"

# The rebalancer fee-only mint (model B, FEE_TIER_PROTOCOL §7) requires the
# bot-liquidity LP pool to exist and be wired onto the vault. Without it the
# vault holds the tokens directly and charge_fee has no LP to mint into, so it
# skips the fee. Provision the pool so the fee path is exercised end to end.
LIQ_INIT=$(jq -nc --arg admin "$TEST_ADDRESS" --arg vault "$BV_VAULT" \
    '{admin:$admin,vault:$vault,name:"Fee E2E Bot Liquidity",
      symbol:"FEELIQ",decimals:6,minimum_initial_deposit:"100000",marketing:null}')
BV_LIQUIDITY=$(instantiate_contract "$LIQ_CODE_ID" "$LIQ_INIT" cl8y-fee-e2e-bot-liquidity)
SET_LIQ=$(jq -nc --arg liquidity "$BV_LIQUIDITY" \
    '{set_liquidity_contract:{liquidity_contract:$liquidity}}')
execute_from "$TEST_ADDRESS" "$BV_VAULT" "$SET_LIQ" >/dev/null
log "bot-liquidity pool: $BV_LIQUIDITY wired onto bot-vault"

log "-- bootstrap the bot-liquidity pool with a proportional deposit --"
BV_POOL=$(query_smart "$SECOND_PAIR_ADDRESS" '{"pool":{}}')
BV_RES_0=$(jq -r '.data.assets[0].amount' <<<"$BV_POOL")
BV_RES_1=$(jq -r '.data.assets[1].amount' <<<"$BV_POOL")
BV_A0=$((BV_RES_0 / 50))
BV_A1=$((BV_RES_1 / 50))
APPROVE_L0=$(jq -nc --arg spender "$BV_LIQUIDITY" --arg amount "$BV_A0" \
    '{increase_allowance:{spender:$spender,amount:$amount,expires:null}}')
APPROVE_L1=$(jq -nc --arg spender "$BV_LIQUIDITY" --arg amount "$BV_A1" \
    '{increase_allowance:{spender:$spender,amount:$amount,expires:null}}')
execute_from "$TEST_ADDRESS" "$LUNC_C_ADDRESS" "$APPROVE_L0" >/dev/null
execute_from "$TEST_ADDRESS" "$EMBER_ADDRESS" "$APPROVE_L1" >/dev/null
BV_DEADLINE_D=$(($(now_sec) + 300))
BV_DEPOSIT=$(jq -nc --arg a0 "$BV_A0" --arg a1 "$BV_A1" --argjson deadline "$BV_DEADLINE_D" \
    '{deposit:{amounts:[$a0,$a1],min_shares:"1",deadline:$deadline,swap:null}}')
execute_from "$TEST_ADDRESS" "$BV_LIQUIDITY" "$BV_DEPOSIT" >/dev/null
log "bot-liquidity bootstrapped (deposit $BV_A0 + $BV_A1)"

log "-- inject EMBER only on top, forcing allocation deviation --"
BV_INJECT=$(( BV_A1 * 4 + 10000000000 ))
transfer_token_to test1 "$EMBER_ADDRESS" "$BV_VAULT" "$BV_INJECT"
BV_STATUS=$(query_smart "$BV_VAULT" '{"rebalance_status":{}}')
jq -e '.data.should_rebalance == true' <<<"$BV_STATUS" >/dev/null
log "rebalance_status: should_rebalance=true deviation=$(jq -r '.data.allocation_deviation_bps' <<<"$BV_STATUS")"

log "-- keeper rebalance via swap-proxy; fee accrued in reply --"
BV_DEADLINE=$(($(now_sec) + 300))
BV_RES=$(execute_from gridkeeper "$BV_VAULT" "{\"rebalance\":{\"deadline\":$BV_DEADLINE}}")
BV_BPS=$(tx_event_value "$BV_RES" fee_bps)
BV_TIER=$(tx_event_value "$BV_RES" fee_tier)
BV_SRC=$(tx_event_value "$BV_RES" fee_source)
BV_SHARES=$(tx_event_value "$BV_RES" fee_shares)
log "rebalancer fee: fee_bps=$BV_BPS fee_tier=$BV_TIER fee_source=$BV_SRC fee_shares=$BV_SHARES"
# The rebalancer bills its operator (config.admin = TEST_ADDRESS, tier-5 -> 90 bps).
test "$BV_BPS" = "90"
test "$BV_TIER" = "5"
test "$BV_SRC" = "live"
test "$BV_SHARES" -gt 0

BV_COLLECTOR_SHARES=$(query_smart "$BV_VAULT" \
    "{\"shares\":{\"bot_id\":0,\"address\":\"$FEE_COLLECTOR\"}}" | jq -r '.data.shares')
test "$BV_COLLECTOR_SHARES" -gt 0
test "$BV_COLLECTOR_SHARES" = "$BV_SHARES"
log "collector fee-shares in bot-vault: $BV_COLLECTOR_SHARES (matches fee_shares)"

log "-- collector collect -> dummy treasury (pro-rata of both balances) --"
RB_TREASURY_0_BEFORE=$(cw20_balance_of "$LUNC_C_ADDRESS" "$FEE_TREASURY")
RB_TREASURY_1_BEFORE=$(cw20_balance_of "$EMBER_ADDRESS" "$FEE_TREASURY")
RCOLLECT=$(jq -nc --arg vault "$BV_VAULT" --argjson bot_id 0 '{collect:{vault:$vault,bot_id:$bot_id}}')
wait_tx "$(terrad_tx_from gridkeeper wasm execute "$FEE_COLLECTOR" "$RCOLLECT" | jq -r '.txhash')" >/dev/null
RB_TREASURY_0_AFTER=$(cw20_balance_of "$LUNC_C_ADDRESS" "$FEE_TREASURY")
RB_TREASURY_1_AFTER=$(cw20_balance_of "$EMBER_ADDRESS" "$FEE_TREASURY")
test "$RB_TREASURY_0_AFTER" -gt "$RB_TREASURY_0_BEFORE"
test "$RB_TREASURY_1_AFTER" -gt "$RB_TREASURY_1_BEFORE"
BV_COLLECTOR_SHARES_ZERO=$(query_smart "$BV_VAULT" \
    "{\"shares\":{\"bot_id\":0,\"address\":\"$FEE_COLLECTOR\"}}" | jq -r '.data.shares')
test "$BV_COLLECTOR_SHARES_ZERO" = "0"
log "treasury received LUNC-C +$((RB_TREASURY_0_AFTER - RB_TREASURY_0_BEFORE)) EMBER +$((RB_TREASURY_1_AFTER - RB_TREASURY_1_BEFORE)); collector fee-shares now 0"

echo
log "ALL PASS: market-grid + rebalancer per-execution fees verified on-chain"

cat > "$SCRIPT_DIR/.fee-e2e-multi-artifacts" <<EOF
MG_VAULT=$MG_VAULT
SP_PROXY=$SP_PROXY
BV_VAULT=$BV_VAULT
MG_FEE_SHARES=$MG_SHARES
BV_FEE_SHARES=$BV_SHARES
EOF
log "artifacts written: $SCRIPT_DIR/.fee-e2e-multi-artifacts"
