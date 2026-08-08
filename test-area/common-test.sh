#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=common.sh
source "$SCRIPT_DIR/common.sh"

temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
counter="$temporary/counter"

localterra_container() { printf 'localterra\n'; }
sleep() { :; }
docker() {
    local attempt=0
    [ -f "$counter" ] && attempt=$(<"$counter")
    attempt=$((attempt + 1))
    printf '%s' "$attempt" >"$counter"
    case "$TEST_RESPONSE" in
        success) printf '{"code":0,"txhash":"ABC123"}\n' ;;
        malformed) printf 'not-json\n' ;;
        empty-hash) printf '{"code":0,"txhash":""}\n' ;;
        rejected) printf '{"code":5,"raw_log":"rejected"}\n' ;;
        retry)
            if [ "$attempt" -eq 1 ]; then
                printf '{"code":32,"raw_log":"account sequence mismatch"}\n'
            else
                printf '{"code":0,"txhash":"RETRIED"}\n'
            fi
            ;;
        command-failure) printf 'terrad failed\n' >&2; return 9 ;;
    esac
}

run_success() {
    TEST_RESPONSE=$1
    : >"$counter"
    terrad_tx bank send a b 1uluna
}

run_failure() {
    TEST_RESPONSE=$1
    : >"$counter"
    if terrad_tx bank send a b 1uluna >/dev/null 2>&1; then
        echo "ERROR: $1 response unexpectedly succeeded" >&2
        return 1
    fi
}

jq -e '.txhash == "ABC123"' <<<"$(run_success success)" >/dev/null
jq -e '.txhash == "RETRIED"' <<<"$(run_success retry)" >/dev/null
test "$(<"$counter")" = 2
run_failure malformed
run_failure empty-hash
run_failure rejected
run_failure command-failure
echo "common.sh transaction tests passed"
