#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
PROJECT_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RUST_TOOLCHAIN=${RUST_TOOLCHAIN:-1.81.0}

cargo "+$RUST_TOOLCHAIN" build --locked --manifest-path "$PROJECT_ROOT/fee-system/Cargo.toml" \
    --release --target wasm32-unknown-unknown
cargo "+$RUST_TOOLCHAIN" build --locked --manifest-path "$PROJECT_ROOT/limit-grid-system/Cargo.toml" \
    --release --target wasm32-unknown-unknown
cargo "+$RUST_TOOLCHAIN" build --locked --manifest-path "$PROJECT_ROOT/market-grid-system/Cargo.toml" \
    --release --target wasm32-unknown-unknown
cargo "+$RUST_TOOLCHAIN" build --locked --manifest-path "$PROJECT_ROOT/rebalancer-system/Cargo.toml" \
    --release --target wasm32-unknown-unknown

artifacts=(
    "$PROJECT_ROOT/fee-system/target/wasm32-unknown-unknown/release/cl8y_fee_registry.wasm"
    "$PROJECT_ROOT/fee-system/target/wasm32-unknown-unknown/release/cl8y_fee_collector.wasm"
    "$PROJECT_ROOT/limit-grid-system/target/wasm32-unknown-unknown/release/cl8y_grid_vault.wasm"
    "$PROJECT_ROOT/market-grid-system/target/wasm32-unknown-unknown/release/cl8y_grid_vault_swap.wasm"
    "$PROJECT_ROOT/rebalancer-system/target/wasm32-unknown-unknown/release/cl8y_bot_vault.wasm"
    "$PROJECT_ROOT/rebalancer-system/target/wasm32-unknown-unknown/release/cl8y_bot_liquidity.wasm"
    "$PROJECT_ROOT/rebalancer-system/target/wasm32-unknown-unknown/release/cl8y_swap_proxy.wasm"
)
manifest="$SCRIPT_DIR/.fee-e2e-build-manifest.json"
entries=$(mktemp)
trap 'rm -f "$entries"' EXIT
for artifact in "${artifacts[@]}"; do
    hash=$(sha256sum "$artifact" | cut -d ' ' -f 1)
    jq -nc --arg artifact "${artifact#"$PROJECT_ROOT"/}" --arg sha256 "$hash" \
        '{artifact:$artifact,sha256:$sha256}' >>"$entries"
done
jq -s --arg source_sha "$(git -C "$PROJECT_ROOT" rev-parse HEAD)" \
    --arg rust_toolchain "$RUST_TOOLCHAIN" \
    '{source_sha:$source_sha,features:"default",rust_toolchain:$rust_toolchain,artifacts:.}' \
    "$entries" >"$manifest"

"$SCRIPT_DIR/fee-e2e-test.sh"
"$SCRIPT_DIR/fee-e2e-multi.sh"
"$SCRIPT_DIR/fee-e2e-market-1000.sh"
