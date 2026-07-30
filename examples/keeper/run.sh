#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
CONFIG_FILE="${1:-$SCRIPT_DIR/config.env}"
if [ ! -f "$CONFIG_FILE" ]; then
    echo "ERROR: missing $CONFIG_FILE; copy config.example.env to config.env" >&2
    exit 1
fi
set -a
# shellcheck source=/dev/null
source "$CONFIG_FILE"
set +a

ARGS=()
if [ "${KEEPER_BROADCAST:-0}" = "1" ]; then
    ARGS+=(--broadcast)
fi
if [ "${KEEPER_ONCE:-0}" = "1" ]; then
    ARGS+=(--once)
fi

exec python3 "$SCRIPT_DIR/keeper.py" "${ARGS[@]}"
