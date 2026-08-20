#!/bin/zsh

set -euo pipefail

shell_dir="${0:A:h}"
"$shell_dir/build-rust-ffi.sh" >/dev/null
swift build --package-path "$shell_dir"

binary_dir="$(swift build --package-path "$shell_dir" --show-bin-path)"
app_dir="$shell_dir/.build/Yu.app"
contents_dir="$app_dir/Contents"

mkdir -p "$contents_dir/MacOS"
cp "$binary_dir/Yu" "$contents_dir/MacOS/Yu"
cp "$shell_dir/AppBundle/Info.plist" "$contents_dir/Info.plist"
codesign --force --sign - "$app_dir"
print -r -- "$app_dir"
