#!/bin/zsh
#
# 构建 Rust static library 并放到 Swift Package 能链接到的位置。
# stdout 只输出 .a 的路径，构建日志走 stderr。

set -euo pipefail

shell_dir="${0:A:h}"
workspace_dir="$shell_dir/../../.."
rust_output="$shell_dir/.rust"
library="$rust_output/libyu_storage_ffi.a"

# tree-sitter 的 grammar 是 C，由 `cc` crate 编译进这个 .a（S7 第五刀）。
# `cc` 默认按**主机 SDK** 的部署目标编译，而 Package.swift 声明的是
# .macOS(.v14)，于是每一个 grammar 的 .o 都会让链接器抛一条
# 「built for newer 'macOS' version」。它只是警告，但每次构建刷十几行，
# 真正的警告会淹在里面。两边对齐到同一个版本就没有了。
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
cargo build --manifest-path "$workspace_dir/Cargo.toml" -p yu-storage-ffi >&2
mkdir -p "$rust_output"
cp "$workspace_dir/target/debug/libyu_storage_ffi.a" "$library"

# Package.swift 通过 `.unsafeFlags(["-L…", "-lyu_storage_ffi"])` 链接这个静态
# 库，而 SwiftPM **不把它当作构建依赖跟踪**：.a 更新后 `swift build` 仍然认为
# 无需重新链接，可执行文件继续用旧的 Rust 代码。
#
# 这个陷阱代价很高：Rust 侧的修复不会出现在 app 里，看起来像「修了没用」，
# 很容易让人回头去改本来正确的代码。这里记录 .a 的哈希，内容变化时删除已
# 链接的产物，强制下一次 swift build 重新链接。
stamp="$rust_output/.library-hash"
current="$(shasum -a 256 "$library" | cut -d' ' -f1)"
if [[ ! -f "$stamp" || "$(cat "$stamp")" != "$current" ]]; then
    print -r -- "Rust 静态库已变化，强制重新链接" >&2
    rm -f "$shell_dir"/.build/*/debug/Yu(N)
    print -r -- "$current" > "$stamp"
fi

print -r -- "$library"
