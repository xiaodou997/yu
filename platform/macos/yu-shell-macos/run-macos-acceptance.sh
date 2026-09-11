#!/bin/zsh
# Repeatable macOS acceptance pass. Clipboard checks may require an interactive
# pasteboard-capable session; all other checks remain useful in headless CI.
set -euo pipefail

shell_dir="${0:A:h}"
workspace="$shell_dir/../../.."
cd "$workspace"

print -r -- "== build app =="
"$shell_dir/build-app.sh" >/dev/null

print -r -- "== headless shell checks =="
if ! "$shell_dir/run-self-checks.sh"; then
    print -u2 -- "headless checks reported failures (often NSPasteboard permissions)"
fi

print -r -- "== retained scroll fixture baseline =="
YU_SCROLL_SIZE_MIB="${YU_SCROLL_SIZE_MIB:-1}" \
YU_SCROLL_ITERATIONS="${YU_SCROLL_ITERATIONS:-3}" \
"$shell_dir/run-scroll-bench.sh"

print -r -- "== real-window Dark Aqua check =="
"$shell_dir/run-dark-self-check.sh"
