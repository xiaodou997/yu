#!/bin/zsh
# Launch the dark-mode real-window check without LaunchServices argument reuse.
set -euo pipefail

shell_dir="${0:A:h}"
fixture="${1:-$shell_dir/Fixtures/outline.md}"
app="$shell_dir/.build/Yu.app"
binary="$app/Contents/MacOS/Yu"

[[ -x "$binary" ]] || { print -u2 "missing $binary; run build-app.sh first"; exit 1; }
pkill -f "$binary" >/dev/null 2>&1 || true
sleep 0.2
exec "$binary" --dark-mode-self-check --launch-window-self-check "$fixture"
