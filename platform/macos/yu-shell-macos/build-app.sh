#!/bin/zsh
#
# 构建 Yu.app 并把它的路径打印到 stdout。
#
# 约定：stdout 只输出最终的 .app 路径，构建进度一律走 stderr。调用方因此可以
# 直接写 `open "$(build-app.sh)"`——否则 swift build 的进度会混进命令替换，
# 让 open 收到一个多行的伪路径。

set -euo pipefail

shell_dir="${0:A:h}"
"$shell_dir/build-rust-ffi.sh" >&2
swift build --package-path "$shell_dir" >&2

binary_dir="$(swift build --package-path "$shell_dir" --show-bin-path 2>/dev/null)"
app_dir="$shell_dir/.build/Yu.app"
contents_dir="$app_dir/Contents"

mkdir -p "$contents_dir/MacOS"
cp "$binary_dir/Yu" "$contents_dir/MacOS/Yu"
cp "$shell_dir/AppBundle/Info.plist" "$contents_dir/Info.plist"
codesign --force --sign - "$app_dir" >&2
print -r -- "$app_dir"
