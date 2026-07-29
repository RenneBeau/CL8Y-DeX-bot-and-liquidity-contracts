#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
"$SCRIPT_DIR/deploy-system.sh"
"$SCRIPT_DIR/soak-test.sh"
