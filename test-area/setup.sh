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
if [ "$DEPLOY_STATUS" -ne 0 ]; then
    if [ ! -f "$DEX_DIR/frontend-dapp/.env.local" ]; then
        echo "ERROR: CL8Y deployment failed before producing contract addresses." >&2
        exit "$DEPLOY_STATUS"
    fi
    echo "CL8Y core deployment completed; ignoring its optional indexer bootstrap failure."
fi

"$SCRIPT_DIR/deploy-system.sh"

echo "Local environment is ready. Run: make local-test"
