#!/bin/zsh

set -euo pipefail

experiment_dir="${0:A:h}"
workspace_dir="$experiment_dir/../.."
rust_output="$experiment_dir/.rust"

cargo build --manifest-path "$workspace_dir/Cargo.toml" -p yu-editor-ffi
mkdir -p "$rust_output"
cp "$workspace_dir/target/debug/libyu_editor_ffi.a" "$rust_output/libyu_editor_ffi.a"
print -r -- "$rust_output/libyu_editor_ffi.a"
