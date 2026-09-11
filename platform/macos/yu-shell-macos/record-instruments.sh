#!/bin/zsh
# Record the production AppKit/Metal path. Default mode waits for real interaction.
set -euo pipefail
shell_dir="${0:A:h}"
if (( $# < 2 || $# > 4 )); then
    print -u2 "usage: $0 fixture.md output-directory [duration=30s] [--self-check|--render-regression|--resource-regression]"
    exit 2
fi
fixture="${1:A}"
output="${2:A}"
duration="${3:-30s}"
binary="$shell_dir/.build/Yu.app/Contents/MacOS/Yu"
[[ -x "$binary" && -f "$fixture" ]] || { print -u2 "Build Yu.app first and provide an existing fixture"; exit 2; }
[[ ! -e "$output" ]] || { print -u2 "Output directory already exists: $output"; exit 2; }
typeset -a args extra_env
args=("$fixture")
extra_env=()
mode=manual
marker=""
if [[ "${4:-}" == --self-check ]]; then
    args=(--launch-window-self-check "$fixture")
    mode=self-check
    marker='Yu frame scheduling self-check:'
elif [[ "${4:-}" == --render-regression ]]; then
    args=(--render-regression-self-check "$fixture")
    mode=scripted-regression
    marker='Yu render regression self-check: reopened=true'
elif [[ "${4:-}" == --resource-regression ]]; then
    args=(--resource-latency-self-check "$fixture")
    extra_env=(--env YU_TEST_IMAGE_DELAY_MS=14000 --env YU_TEST_MATH_DELAY_MS=16000)
    mode=scripted-resource-regression
    marker='Yu render regression self-check: resources=true'
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
    print -r -- "extra_env=${extra_env[*]}"
    git -C "$shell_dir" rev-parse HEAD
    git -C "$shell_dir" status --porcelain
    shasum -a 256 "$binary" "$fixture"
    sw_vers
    uname -m
    sysctl -n hw.model machdep.cpu.brand_string
    xcodebuild -version
} > "$output/environment.txt"
print -r -- "Recording $mode session. In manual mode, continuously scroll the opened document with the trackpad."
record_status=0
xcrun xctrace record --template 'Metal System Trace' --time-limit "$duration" \
    --output "$output/render.trace" --target-stdout "$output/render.log" \
    --env YU_RENDER_TIMING=1 "${extra_env[@]}" --launch -- "$binary" "${args[@]}" || record_status=$?
print -r -- "xctrace_exit=$record_status" >> "$output/environment.txt"
[[ -f "$output/render.log" ]] || { print -u2 "Missing target log; recording is incomplete"; exit 1; }
python3 "$shell_dir/summarize-render-metrics.py" "$output/render.log" > "$output/summary.json"
if [[ -n "$marker" ]] && ! grep -q "^$marker" "$output/render.log"; then
    print -u2 "The target did not complete its window self-check; inspect $output/render.log"
    exit 1
fi
(( record_status == 0 )) || exit "$record_status"
print -r -- "$output/summary.json"
