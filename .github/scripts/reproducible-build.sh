#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    echo "usage: $0 WORKSPACE OUTPUT_DIRECTORY [default|mainnet]" >&2
    exit 2
fi

workspace=$(realpath "$1")
output=$(realpath -m "$2")
features=${3:-default}
workspace_name=$(basename "$workspace")
repository_root=$(git rev-parse --show-toplevel)
optimizer_image=${OPTIMIZER_IMAGE:?OPTIMIZER_IMAGE must include an immutable digest}
source_sha=${SOURCE_SHA:-$(git rev-parse HEAD)}
case "$optimizer_image" in
    *@sha256:*) ;;
    *) echo "OPTIMIZER_IMAGE must be pinned by sha256 digest" >&2; exit 2 ;;
esac
case "$features" in
    default) ;;
    mainnet)
        : "${CL8Y_CANONICAL_FEE_COLLECTOR:?CL8Y_CANONICAL_FEE_COLLECTOR is required for mainnet builds}"
        : "${CL8Y_CANONICAL_FEE_REGISTRY:?CL8Y_CANONICAL_FEE_REGISTRY is required for mainnet builds}"
        : "${CL8Y_CANONICAL_SWAP_PROXY:?CL8Y_CANONICAL_SWAP_PROXY is required for mainnet builds}"
        ;;
    *) echo "unsupported feature set: $features" >&2; exit 2 ;;
esac

temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
mkdir -p "$output" "$temporary/first" "$temporary/second"

copy_source() {
    local destination=$1 manifest
    mkdir -p "$destination/source"
    tar -C "$workspace" --exclude=target --exclude=artifacts -cf - . \
        | tar -C "$destination/source" -xf -
    if [ "$workspace_name" != fee-system ]; then
        mkdir -p "$destination/fee-system"
        tar -C "$repository_root/fee-system" --exclude=target --exclude=artifacts -cf - . \
            | tar -C "$destination/fee-system" -xf -
    fi
    if [ "$features" = mainnet ]; then
        # workspace-optimizer has no feature flag. Enabling mainnet as a default
        # feature in each isolated source copy gives it the exact Cargo graph.
        for manifest in "$destination/source/Cargo.toml" \
            "$destination/source"/contracts/*/Cargo.toml; do
            [ -f "$manifest" ] || continue
            if grep -q '^mainnet = \[\]$' "$manifest"; then
                perl -0pi -e 's/\[features\]\n(?!default\s*=)/[features]\ndefault = ["mainnet"]\n/' "$manifest"
            fi
        done
    fi
}

build_once() {
    local directory=$1
    local mounts=()
    if [ -d "$directory/fee-system" ]; then
        mounts+=(--volume "$directory/fee-system:/fee-system:ro")
    fi
    docker run --rm \
        --env SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-0}" \
        --env CL8Y_CANONICAL_FEE_COLLECTOR="${CL8Y_CANONICAL_FEE_COLLECTOR:-}" \
        --env CL8Y_CANONICAL_FEE_REGISTRY="${CL8Y_CANONICAL_FEE_REGISTRY:-}" \
        --env CL8Y_CANONICAL_SWAP_PROXY="${CL8Y_CANONICAL_SWAP_PROXY:-}" \
        --volume "$directory/source:/code" \
        "${mounts[@]}" \
        "$optimizer_image"
    (cd "$directory/source/artifacts" && sha256sum -- *.wasm | LC_ALL=C sort) \
        >"$directory/checksums.txt"
}

copy_source "$temporary/first"
copy_source "$temporary/second"
build_once "$temporary/first"
build_once "$temporary/second"
diff -u "$temporary/first/checksums.txt" "$temporary/second/checksums.txt"

manifest_entries="$temporary/manifest-entries.jsonl"
for artifact in "$temporary/first/source/artifacts/"*.wasm; do
    stem=$(basename "$artifact" .wasm)
    destination="$output/${stem}-${features}.wasm"
    cp "$artifact" "$destination"
    hash=$(sha256sum "$destination" | cut -d ' ' -f 1)
    jq -nc \
        --arg contract "$stem" --arg artifact "$(basename "$destination")" --arg sha256 "$hash" \
        '{contract:$contract,artifact:$artifact,sha256:$sha256}' >>"$manifest_entries"
done

jq -s \
    --arg source_sha "$source_sha" \
    --arg workspace "$workspace_name" \
    --arg features "$features" \
    --arg optimizer_image "$optimizer_image" \
    --arg fee_collector "${CL8Y_CANONICAL_FEE_COLLECTOR:-}" \
    --arg fee_registry "${CL8Y_CANONICAL_FEE_REGISTRY:-}" \
    --arg swap_proxy "${CL8Y_CANONICAL_SWAP_PROXY:-}" \
    '{source_sha:$source_sha,workspace:$workspace,features:$features,
      canonical:{fee_collector:$fee_collector,fee_registry:$fee_registry,swap_proxy:$swap_proxy},
      optimizer_image:$optimizer_image,artifacts:.}' \
    "$manifest_entries" >"$output/${workspace_name}-${features}-manifest.json"

printf 'Reproducible double build passed for %s (%s) with %s\n' \
    "$workspace_name" "$features" "$optimizer_image"
