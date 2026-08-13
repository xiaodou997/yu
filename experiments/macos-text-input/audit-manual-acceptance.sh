#!/bin/zsh

set -euo pipefail

experiment_dir="${0:A:h}"

if [[ $# -ne 2 ]]; then
    print -u2 -- "usage: $0 SCENARIO LOG_PATH"
    print -u2 -- "example: $0 japanese-romaji /tmp/yu-ime-japanese-romaji.log"
    exit 64
fi

scenario="$1"
log_path="${2:A}"

if [[ -z "$scenario" || "$scenario" == *[!a-z0-9-]* ]]; then
    print -u2 -- "scenario must contain only lowercase letters, digits and hyphens"
    exit 64
fi
if [[ ! -f "$log_path" ]]; then
    print -u2 -- "log does not exist: $log_path"
    exit 66
fi

"$experiment_dir/build-rust-ffi.sh" >/dev/null
swift run \
    --package-path "$experiment_dir" \
    YuMacTextInputSpike \
    --audit-ime-log "$log_path" \
    --strict \
    --expect-scenario "$scenario"
