#!/usr/bin/env bash
set -euo pipefail

root=$(git rev-parse --show-toplevel)
policy=${POLICY_FILE:-"$root/.github/release-policy.json"}

die() {
    printf 'ERROR: %s\n' "$*" >&2
    exit 1
}

workspaces() {
    jq -r '.workspaces[].path' "$policy"
}

inventory() {
    local workspace metadata expected actual
    for workspace in $(workspaces); do
        metadata=$(cargo metadata --no-deps --locked --format-version 1 \
            --manifest-path "$root/$workspace/Cargo.toml")
        expected=$(jq -cS --arg workspace "$workspace" \
            '.workspaces[] | select(.path == $workspace) | .packages
             | map({name, manifest, artifact}) | sort_by(.name)' "$policy")
        actual=$(jq -cS --arg root "$root/$workspace/" \
            '[.packages[] | {
                name: .name,
                manifest: (.manifest_path | sub(("^" + $root); "")),
                artifact: (any(.targets[]; any(.crate_types[]; . == "cdylib")))
             }] | sort_by(.name)' <<<"$metadata")
        [ "$actual" = "$expected" ] || {
            printf 'Expected %s inventory: %s\nActual %s inventory:   %s\n' \
                "$workspace" "$expected" "$workspace" "$actual" >&2
            die "release package/artifact inventory mismatch"
        }
    done
    printf 'Release package/artifact inventory passed\n'
}

release_versions() {
    local tag=${1:-} version workspace metadata mismatch
    [[ "$tag" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] \
        || die "release tag must be stable semver vMAJOR.MINOR.PATCH (prereleases are forbidden): $tag"
    version=${tag#v}
    inventory
    for workspace in $(workspaces); do
        metadata=$(cargo metadata --no-deps --locked --format-version 1 \
            --manifest-path "$root/$workspace/Cargo.toml")
        mismatch=$(jq -r --arg workspace "$workspace" --arg version "$version" \
            --slurpfile policy "$policy" '
              ($policy[0].workspaces[] | select(.path == $workspace)
               | [.packages[] | select(.tier == "production") | .name]) as $production
              | [.packages[] | select((.name as $name | $production | index($name)) and .version != $version)
                 | "\(.name)=\(.version)"] | join(", ")' <<<"$metadata")
        [ -z "$mismatch" ] || die "$workspace production versions do not match $version: $mismatch"
    done
    printf 'Release tag %s matches every production package; PoC packages remain artifact-only\n' "$tag"
}

audit_policy() {
    local output_dir=${AUDIT_JSON_DIR:-} temporary='' workspace report
    local today expires advisory package version count actual
    today=${POLICY_DATE:-$(date -u +%F)}
    expires=$(jq -r '.rustsec_exception.expires_utc' "$policy")
    [[ "$today" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die "invalid UTC policy date: $today"
    [[ "$expires" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die "invalid exception expiry: $expires"
    [[ "$today" < "$expires" ]] || die "RustSec exception expired on $expires (UTC)"
    advisory=$(jq -r '.rustsec_exception.advisory' "$policy")
    package=$(jq -r '.rustsec_exception.package' "$policy")
    version=$(jq -r '.rustsec_exception.version' "$policy")

    if [ -z "$output_dir" ]; then
        temporary=$(mktemp -d)
        trap 'rm -rf "$temporary"' RETURN
        output_dir=$temporary
        mkdir -p "$output_dir"
        for workspace in $(workspaces); do
            cargo +stable audit --file "$root/$workspace/Cargo.lock" --json \
                >"$output_dir/$workspace-audit.json" || true
        done
    fi

    for workspace in $(workspaces); do
        report="$output_dir/$workspace-audit.json"
        jq -e . "$report" >/dev/null 2>&1 || die "missing or invalid cargo-audit JSON for $workspace"
        count=$(jq '.vulnerabilities.list | length' "$report")
        [ "$count" -eq 1 ] || die "$workspace has $count vulnerability advisories; expected only $advisory"
        actual=$(jq -r '.vulnerabilities.list[0] | [.advisory.id, .package.name, .package.version] | @tsv' "$report")
        [ "$actual" = "$advisory"$'\t'"$package"$'\t'"$version" ] \
            || die "$workspace advisory/package/version is '$actual'; expected '$advisory $package $version'"
        if [ -n "${AUDIT_EVIDENCE_DIR:-}" ]; then
            mkdir -p "$AUDIT_EVIDENCE_DIR"
            cp "$report" "$AUDIT_EVIDENCE_DIR/$workspace-audit.json"
        fi
    done
    printf 'RustSec policy passed: only %s for %s %s; expires %s UTC\n' \
        "$advisory" "$package" "$version" "$expires"
}

case "${1:-}" in
    inventory) inventory ;;
    release) release_versions "${2:-}" ;;
    audit) audit_policy ;;
    workspaces) workspaces ;;
    artifact-count) jq '[.workspaces[].packages[] | select(.artifact)] | length' "$policy" ;;
    *) die "usage: $0 {inventory|release TAG|audit|workspaces|artifact-count}" ;;
esac
