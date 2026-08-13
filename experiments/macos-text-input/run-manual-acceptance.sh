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
if [[ -e "$log_path" ]]; then
    print -u2 -- "refusing to overwrite existing log: $log_path"
    exit 73
fi

mkdir -p "${log_path:h}"
print -- "Scenario: $scenario"
print -- "Select the matching macOS input source manually before typing; Yu will not switch it."
print -- "When finished, settle any marked text, then press Ctrl-C to stop capture."
print -- "Raw session log: $log_path"

"$experiment_dir/build-app.sh" >/dev/null
app_binary="$experiment_dir/.build/YuMacTextInputSpike.app/Contents/MacOS/YuMacTextInputSpike"
if [[ ! -x "$app_binary" ]]; then
    print -u2 -- "built app binary is missing or not executable: $app_binary"
    exit 74
fi

set +e
YU_IME_SCENARIO="$scenario" "$app_binary" 2>&1 | tee "$log_path"
capture_status=${pipestatus[1]}
set -e

if [[ "$capture_status" -ne 0 && "$capture_status" -ne 130 && "$capture_status" -ne 143 ]]; then
    print -u2 -- "Yu capture exited with status $capture_status; raw log was preserved"
    exit "$capture_status"
fi

print -- "Capture stopped. Run audit-manual-acceptance.sh after checking the raw log."
