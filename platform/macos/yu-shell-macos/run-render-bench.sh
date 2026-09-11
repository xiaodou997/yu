#!/bin/zsh
# Capture the real-window render timing stream for a reproducible smoke pass.
# This intentionally does not claim to replace Instruments: it exercises the
# production surface, retained scroll, resource refresh and resize checks while
# preserving the per-frame Rust timing lines for later percentile analysis.
set -euo pipefail

shell_dir="${0:A:h}"
fixture="${1:-$shell_dir/Fixtures/outline.md}"
app="$shell_dir/.build/Yu.app"
binary="$app/Contents/MacOS/Yu"

[[ -x "$binary" ]] || {
    print -u2 "missing $binary; run build-app.sh first"
    exit 1
}
[[ -f "$fixture" ]] || {
    print -u2 "missing fixture: $fixture"
    exit 1
}

pkill -f "$binary" >/dev/null 2>&1 || true
sleep 0.2

print -r -- "render benchmark fixture=$fixture"
YU_RENDER_TIMING=1 "$binary" \
    --launch-window-self-check \
    "$fixture"
