#!/bin/zsh
# Repeatable CPU-side baseline for the macOS long-document scroll fixture.
# Window/GPU timing is intentionally separate: run the built app with Console
# attached to capture SurfaceHost's live-scroll metrics.
set -euo pipefail

workspace="${0:A:h}/../../.."
cd "$workspace"
size_mib="${YU_SCROLL_SIZE_MIB:-2}"
iterations="${YU_SCROLL_ITERATIONS:-5}"
random_edits="${YU_SCROLL_RANDOM_EDITS:-20}"
retained="${YU_SCROLL_RETAINED_SNAPSHOTS:-3}"

print -r -- "scroll baseline: size=${size_mib}MiB iterations=${iterations}"
cargo run -p yu-bench -- \
  --size-mib "$size_mib" \
  --iterations "$iterations" \
  --random-edits "$random_edits" \
  --retained-snapshots "$retained"
