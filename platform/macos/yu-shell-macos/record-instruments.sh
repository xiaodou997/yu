#!/bin/zsh
# Record the production AppKit/Metal path. Default mode waits for real interaction.
set -euo pipefail
shell_dir="${0:A:h}"
if (( $# < 2 || $# > 4 )); then
    print -u2 "usage: $0 fixture.md output-directory [duration=30s] [--self-check]"
    exit 2
fi
fixture="${1:A}"
output="${2:A}"
duration="${3:-30s}"
binary="$shell_dir/.build/Yu.app/Contents/MacOS/Yu"
[[ -x "$binary" && -f "$fixture" ]] || { print -u2 "Build Yu.app first and provide an existing fixture"; exit 2; }
[[ ! -e "$output" ]] || { print -u2 "Output directory already exists: $output"; exit 2; }
typeset -a args
args=("$fixture")
mode=manual
if [[ "${4:-}" == --self-check ]]; then
    args=(--launch-window-self-check "$fixture")
    mode=self-check
elif [[ -n "${4:-}" ]]; then
    print -u2 "Unknown mode: $4"
    exit 2
fi
mkdir -p "$output"
{
    print -r -- "mode=$mode"
    print -r -- "fixture=$fixture"
    print -r -- "duration=$duration"
    print -r -- "build=debug"
    git -C "$shell_dir" rev-parse HEAD
    git -C "$shell_dir" status --porcelain
    shasum -a 256 "$binary" "$fixture"
    sw_vers
    uname -m
    sysctl -n hw.model machdep.cpu.brand_string
    xcodebuild -version
} > "$output/environment.txt"
print -r -- "Recording $mode session. In manual mode, continuously scroll the opened document with the trackpad."
xcrun xctrace record --template 'Metal System Trace' --time-limit "$duration" \
    --output "$output/render.trace" --target-stdout "$output/render.log" \
    --env YU_RENDER_TIMING=1 --launch -- "$binary" "${args[@]}"
if [[ "$mode" == self-check ]] && ! grep -q '^Yu frame scheduling self-check:' "$output/render.log"; then
    print -u2 "The target did not complete its window self-check; inspect $output/render.log"
    exit 1
fi
python3 "$shell_dir/summarize-render-metrics.py" "$output/render.log" > "$output/summary.json"
print -r -- "$output/summary.json"
