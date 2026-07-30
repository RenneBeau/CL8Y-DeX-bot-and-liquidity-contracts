#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
CONFIG_FILE="${1:-$SCRIPT_DIR/config.env}"
if [ ! -f "$CONFIG_FILE" ]; then
    echo "ERROR: missing keeper config: $CONFIG_FILE" >&2
    exit 1
fi
set -a
# shellcheck source=/dev/null
source "$CONFIG_FILE"
set +a

: "${KEEPER_VAULT_ADDRESS:?Set KEEPER_VAULT_ADDRESS}"
: "${VAULT_ADMIN_KEY_NAME:?Set VAULT_ADMIN_KEY_NAME for this command}"

TERRAD="${KEEPER_TERRAD:-terrad}"
KEEPER_BACKEND="${KEEPER_KEYRING_BACKEND:-os}"
ADMIN_BACKEND="${VAULT_ADMIN_KEYRING_BACKEND:-os}"
KEEPER_ADDRESS=$("$TERRAD" keys show "${KEEPER_KEY_NAME:-cl8y-bot-keeper}" \
    --keyring-backend "$KEEPER_BACKEND" --address)
MESSAGE=$(printf '{"update_keeper":{"keeper":"%s"}}' "$KEEPER_ADDRESS")

"$TERRAD" tx wasm execute "$KEEPER_VAULT_ADDRESS" "$MESSAGE" \
    --from "$VAULT_ADMIN_KEY_NAME" \
    --keyring-backend "$ADMIN_BACKEND" \
    --chain-id "${KEEPER_CHAIN_ID:-columbus-5}" \
    --node "${KEEPER_RPC_URL:?Set KEEPER_RPC_URL}" \
    --gas auto \
    --gas-adjustment "${KEEPER_GAS_ADJUSTMENT:-1.4}" \
    --gas-prices "${KEEPER_GAS_PRICES:-28.325uluna}" \
    --broadcast-mode sync \
    --yes \
    --output json

echo "Keeper registration submitted: $KEEPER_ADDRESS"
