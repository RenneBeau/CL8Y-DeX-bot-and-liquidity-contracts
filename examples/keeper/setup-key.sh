#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
CONFIG_FILE="${1:-$SCRIPT_DIR/config.env}"
if [ -f "$CONFIG_FILE" ]; then
    set -a
    # shellcheck source=/dev/null
    source "$CONFIG_FILE"
    set +a
fi

TERRAD="${KEEPER_TERRAD:-terrad}"
KEY_NAME="${KEEPER_KEY_NAME:-cl8y-bot-keeper}"
BACKEND="${KEEPER_KEYRING_BACKEND:-os}"

command -v "$TERRAD" >/dev/null || {
    echo "ERROR: terrad was not found: $TERRAD" >&2
    exit 1
}

if "$TERRAD" keys show "$KEY_NAME" --keyring-backend "$BACKEND" --address >/dev/null 2>&1; then
    echo "Keeper key already exists."
else
    echo "Creating keeper key '$KEY_NAME' in the '$BACKEND' keyring."
    echo "Store the displayed recovery phrase offline. Never add it to config.env."
    "$TERRAD" keys add "$KEY_NAME" --keyring-backend "$BACKEND"
fi

ADDRESS=$("$TERRAD" keys show "$KEY_NAME" --keyring-backend "$BACKEND" --address)
echo "Keeper address: $ADDRESS"
echo "Fund this address with LUNC for transaction gas, then register it on the vault."
