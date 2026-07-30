#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
"$SCRIPT_DIR/setup.sh"
"$SCRIPT_DIR/integration-test.sh"
"$SCRIPT_DIR/grid-integration-test.sh"
"$SCRIPT_DIR/soak-test.sh"
