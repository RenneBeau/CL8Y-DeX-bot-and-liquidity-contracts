#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 2 ]; then
    echo "usage: $0 WORKSPACE OUTPUT_DIRECTORY" >&2
    exit 2
fi

workspace=$(realpath "$1")
output=$(realpath -m "$2")
optimizer_image=${OPTIMIZER_IMAGE:?OPTIMIZER_IMAGE must include an immutable digest}
case "$optimizer_image" in
    *@sha256:*) ;;
    *) echo "OPTIMIZER_IMAGE must be pinned by sha256 digest" >&2; exit 2 ;;
esac

temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
mkdir -p "$output" "$temporary/first" "$temporary/second"

copy_source() {
    local destination=$1
    mkdir -p "$destination/source"
    tar -C "$workspace" --exclude=target --exclude=artifacts -cf - . \
        | tar -C "$destination/source" -xf -
}

build_once() {
    local directory=$1
    docker run --rm \
        --env SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-0}" \
        --volume "$directory/source:/code" \
        "$optimizer_image"
    (cd "$directory/source/artifacts" && sha256sum -- *.wasm | LC_ALL=C sort) \
        >"$directory/checksums.txt"
}

copy_source "$temporary/first"
copy_source "$temporary/second"
build_once "$temporary/first"
build_once "$temporary/second"
diff -u "$temporary/first/checksums.txt" "$temporary/second/checksums.txt"

cp "$temporary/first/source/artifacts/"*.wasm "$output/"
workspace_name=$(basename "$workspace")
cp "$temporary/first/checksums.txt" "$output/${workspace_name}-checksums.txt"
printf 'Reproducible double build passed for %s with %s\n' \
    "$workspace_name" "$optimizer_image" >"$output/${workspace_name}-build.txt"
