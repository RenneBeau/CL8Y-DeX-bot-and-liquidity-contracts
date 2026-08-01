#!/usr/bin/env bash
set -euo pipefail

docker exec -i "${LOCALTERRA_CONTAINER:-cl8y-dex-terraclassic-localterra-1}" terrad "$@"
