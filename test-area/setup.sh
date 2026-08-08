#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=common.sh
source "$SCRIPT_DIR/common.sh"

ensure_dex_repo

echo "Starting the pinned CL8Y LocalTerra stack..."
export COMPOSE_FILE="$DEX_DIR/docker-compose.yml:$COMPOSE_OVERRIDE"
export COMPOSE_PROJECT_NAME="cl8y-dex-terraclassic"
docker compose --project-directory "$DEX_DIR" up -d
make -C "$DEX_DIR" wait-healthy

echo "Building and deploying the minimal CL8Y test DEX..."
rm -f "$DEX_DIR/frontend-dapp/.env.local"
set +e
QA_DEPLOY_SEED=wallet make -C "$DEX_DIR" deploy-local
DEPLOY_STATUS=$?
set -e

validate_core_deployment() {
    local env_file="$DEX_DIR/frontend-dapp/.env.local" name address
    local required=(VITE_TOKEN_EMBER_ADDRESS VITE_TOKEN_CORAL_ADDRESS VITE_CL8Y_TOKEN_ADDRESS
        VITE_LUNC_C_TOKEN_ADDRESS VITE_FEE_DISCOUNT_ADDRESS VITE_FACTORY_ADDRESS)
    [ -f "$env_file" ] || return 1
    set -a
    # shellcheck source=/dev/null
    source "$env_file"
    set +a
    for name in "${required[@]}"; do
        address=${!name:-}
        if [ -z "$address" ] || ! terrad_query wasm contract "$address" >/dev/null 2>&1; then
            echo "ERROR: CL8Y deployment has no live contract for $name." >&2
            return 1
        fi
    done
}

if ! validate_core_deployment; then
    echo "ERROR: CL8Y deploy-local did not produce a complete, live deployment." >&2
    exit 1
fi
if [ "$DEPLOY_STATUS" -ne 0 ]; then
    echo "CL8Y core contracts validated; deploy-local failed only after core deployment."
fi

"$SCRIPT_DIR/deploy-system.sh"

echo "Local environment is ready. Run: make local-test"
