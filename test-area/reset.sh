#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=common.sh
source "$SCRIPT_DIR/common.sh"
ensure_dex_repo
docker compose -f "$DEX_DIR/docker-compose.yml" -f "$COMPOSE_OVERRIDE" \
    --project-directory "$DEX_DIR" down -v
rm -f "$LOCAL_ENV"
