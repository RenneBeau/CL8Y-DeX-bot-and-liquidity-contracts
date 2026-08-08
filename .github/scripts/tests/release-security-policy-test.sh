#!/usr/bin/env bash
set -euo pipefail

root=$(git rev-parse --show-toplevel)
script="$root/.github/scripts/release-security-policy.sh"
policy="$root/.github/release-policy.json"
fixtures=$(mktemp -d)
trap 'rm -rf "$fixtures"' EXIT

failures=0
expect_pass() {
    local name=$1
    shift
    if "$@" >/dev/null 2>&1; then printf 'ok - %s\n' "$name"; else printf 'not ok - %s\n' "$name"; failures=$((failures + 1)); fi
}
expect_fail() {
    local name=$1
    shift
    if "$@" >/dev/null 2>&1; then printf 'not ok - %s\n' "$name"; failures=$((failures + 1)); else printf 'ok - %s\n' "$name"; fi
}

write_reports() {
    local advisory=$1 package=$2 version=$3 extra=${4:-false} workspace
    for workspace in $(jq -r '.workspaces[].path' "$policy"); do
        jq -n --arg advisory "$advisory" --arg package "$package" --arg version "$version" \
            --argjson extra "$extra" '{vulnerabilities:{list:
              ([{advisory:{id:$advisory},package:{name:$package,version:$version}}]
               + (if $extra then [{advisory:{id:"RUSTSEC-2099-0001"},package:{name:"extra",version:"1.0.0"}}] else [] end))}}' \
            >"$fixtures/$workspace-audit.json"
    done
}

expect_pass "valid stable tag and production versions" "$script" release v0.2.0
expect_fail "mismatched production version" "$script" release v0.2.1
expect_fail "prerelease tag forbidden" "$script" release v0.2.0-rc.1
expect_fail "tag prefix required" "$script" release 0.2.0

write_reports RUSTSEC-2024-0344 curve25519-dalek 3.2.0
expect_pass "expected advisory exception" env AUDIT_JSON_DIR="$fixtures" POLICY_DATE=2026-08-08 "$script" audit
expect_fail "expired advisory exception" env AUDIT_JSON_DIR="$fixtures" POLICY_DATE=2027-02-01 "$script" audit
write_reports RUSTSEC-2024-0344 curve25519-dalek 4.1.3
expect_fail "wrong locked package version" env AUDIT_JSON_DIR="$fixtures" POLICY_DATE=2026-08-08 "$script" audit
write_reports RUSTSEC-2024-0344 wrong-package 3.2.0
expect_fail "wrong locked package" env AUDIT_JSON_DIR="$fixtures" POLICY_DATE=2026-08-08 "$script" audit
write_reports RUSTSEC-2024-0344 curve25519-dalek 3.2.0 true
expect_fail "extra advisory" env AUDIT_JSON_DIR="$fixtures" POLICY_DATE=2026-08-08 "$script" audit

[ "$failures" -eq 0 ] || exit 1
