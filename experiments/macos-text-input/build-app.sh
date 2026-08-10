#!/bin/zsh

set -euo pipefail

experiment_dir="${0:A:h}"
"$experiment_dir/build-rust-ffi.sh"
swift build --package-path "$experiment_dir"

binary_dir="$(swift build --package-path "$experiment_dir" --show-bin-path)"
app_dir="$experiment_dir/.build/YuMacTextInputSpike.app"
contents_dir="$app_dir/Contents"

mkdir -p "$contents_dir/MacOS"
cp "$binary_dir/YuMacTextInputSpike" "$contents_dir/MacOS/YuMacTextInputSpike"
cp "$experiment_dir/AppBundle/Info.plist" "$contents_dir/Info.plist"
codesign --force --sign - "$app_dir"

print -r -- "$app_dir"
