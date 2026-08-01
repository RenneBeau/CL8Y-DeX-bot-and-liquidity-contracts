#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
DEX_REPO_URL="https://github.com/RenneBeau/cl8y-dex-terraclassic.git"
DEX_REVISION="c1f669b06c98936005b665cf56d5540a33a49edd"
DEX_DIR="${CL8Y_DEX_DIR:-$PROJECT_ROOT/test-area/.cache/cl8y-dex-terraclassic}"
LOCAL_ENV="$PROJECT_ROOT/test-area/.env"
COMPOSE_OVERRIDE="$PROJECT_ROOT/test-area/docker-compose.override.yml"
CHAIN_ID="localterra"
TEST_ADDRESS="terra1x46rqay4d3cssq8gxxvqz8xt6nwlz4td20k38v"

ensure_dex_repo() {
    if [ ! -d "$DEX_DIR/.git" ]; then
        mkdir -p "$(dirname "$DEX_DIR")"
        git clone --filter=blob:none "$DEX_REPO_URL" "$DEX_DIR"
    fi

    if ! git -C "$DEX_DIR" diff --quiet -- . ':(exclude)smartcontracts/artifacts/checksums.txt' \
        || ! git -C "$DEX_DIR" diff --cached --quiet; then
        echo "ERROR: managed CL8Y checkout has local changes: $DEX_DIR" >&2
        exit 1
    fi
    git -C "$DEX_DIR" fetch --depth 1 origin "$DEX_REVISION"
    if [ "$(git -C "$DEX_DIR" rev-parse HEAD)" != "$DEX_REVISION" ]; then
        git -C "$DEX_DIR" checkout --detach "$DEX_REVISION"
    fi
}

localterra_container() {
    local container
    container=$(docker compose -f "$DEX_DIR/docker-compose.yml" -f "$COMPOSE_OVERRIDE" \
        --project-directory "$DEX_DIR" ps -q localterra)
    if [ -z "$container" ]; then
        echo "ERROR: LocalTerra is not running. Run 'make local-setup'." >&2
        exit 1
    fi
    printf '%s\n' "$container"
}

load_local_env() {
    if [ ! -f "$LOCAL_ENV" ]; then
        echo "ERROR: deployment file not found: $LOCAL_ENV" >&2
        echo "Run 'make local-setup' first." >&2
        exit 1
    fi
    set -a
    # shellcheck source=/dev/null
    source "$LOCAL_ENV"
    set +a
}

terrad_tx() {
    terrad_tx_from test1 "$@"
}

terrad_tx_from() {
    local signer="$1"
    shift
    local container attempt output err rc
    container=$(localterra_container)
    for attempt in $(seq 1 10); do
        err=$(mktemp)
        output=$(docker exec "$container" terrad tx "$@" \
            --from "$signer" \
            --keyring-backend test \
            --chain-id "$CHAIN_ID" \
            --gas auto \
            --gas-adjustment 1.4 \
            --gas-prices 28.325uluna \
            --node http://127.0.0.1:26657 \
            --broadcast-mode sync \
            --yes \
            --output json 2>"$err")
        rc=$?
        if [ "$rc" -ne 0 ]; then
            if ! grep -q "account sequence mismatch" "$err"; then
                cat "$err" >&2
                rm -f "$err"
                return "$rc"
            fi
            rm -f "$err"
            sleep 1
            continue
        fi
        rm -f "$err"
        printf '%s\n' "$output"
        return 0
    done
    cat "$err" >&2
    return 1
}

terrad_query() {
    local container
    container=$(localterra_container)
    docker exec "$container" terrad query "$@" \
        --node http://127.0.0.1:26657 \
        --output json
}

wait_tx() {
    local tx_hash="$1"
    local result
    for _ in $(seq 1 60); do
        if result=$(terrad_query tx "$tx_hash" 2>/dev/null); then
            if [ "$(jq -r '.code // 0' <<<"$result")" != "0" ]; then
                jq -r '.raw_log // "transaction failed"' <<<"$result" >&2
                return 1
            fi
            printf '%s\n' "$result"
            return 0
        fi
        sleep 1
    done
    echo "ERROR: transaction was not included: $tx_hash" >&2
    return 1
}

tx_event_value() {
    local result="$1"
    local key="$2"
    jq -r --arg key "$key" '
        [
          .logs[]?.events[]?.attributes[]?,
          .events[]?.attributes[]?
        ]
        | map(select(.key == $key or .key == ("_" + $key)))
        | .[0].value // empty
    ' <<<"$result"
}
