#!/bin/sh
set -eu

CDPATH=''
repo=$(cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo"

cargo build --release --locked --package greenlit-cli
export PATH="$repo/target/release:$PATH"

run_action() {
    "$repo/action/entrypoint.sh" "$@"
}

run_action samples/clean.xml "" US shopping-ads 1 true false >/dev/null
run_action samples/broken.xml "" US shopping-ads 1 false false >/dev/null

set +e
run_action samples/broken.xml "" US shopping-ads 1 true false >/dev/null
status=$?
set -e
if [ "$status" -ne 1 ]; then
    echo "strict audit returned $status, expected 1" >&2
    exit 1
fi

run_action samples/clean.xml xml US free-listings 2.5 true true \
    | grep -q '"schema_version": 1'

set +e
run_action samples/clean.xml "" US shopping-ads 1 sometimes false >/dev/null 2>&1
status=$?
set -e
if [ "$status" -ne 2 ]; then
    echo "invalid strict input returned $status, expected 2" >&2
    exit 1
fi

echo "action entrypoint contract: OK"
