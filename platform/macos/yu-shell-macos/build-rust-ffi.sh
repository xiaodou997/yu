#!/bin/zsh
#
# 构建 Rust static library 并放到 Swift Package 能链接到的位置。
# stdout 只输出 .a 的路径，构建日志走 stderr。

set -euo pipefail

shell_dir="${0:A:h}"
workspace_dir="$shell_dir/../../.."
python3 "$workspace_dir/tools/check-ffi-header.py" >&2
rust_output="$shell_dir/.rust"
library="$rust_output/libyu_storage_ffi.a"

source "$shell_dir/toolchain.sh"
profile="debug"
for option in "$@"; do
    case "$option" in
        --release) profile="release" ;;
        *) print -r -- "Unknown build option: $option" >&2; exit 2 ;;
    esac
done
typeset -a profile_args
[[ "$profile" == "release" ]] && profile_args+=(--release)
mkdir -p "$rust_output"
cargo build --locked --manifest-path "$workspace_dir/Cargo.toml" -p yu-storage-ffi \
    --target aarch64-apple-darwin "${profile_args[@]}" >&2
cp "$workspace_dir/target/aarch64-apple-darwin/$profile/libyu_storage_ffi.a" "$library"

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
    rm -f "$shell_dir"/.build/**/debug/Yu(N) "$shell_dir"/.build/**/release/Yu(N) \
        "$shell_dir"/.build/out/Products/Debug/Yu(N) \
        "$shell_dir"/.build/out/Products/Release/Yu(N)
    print -r -- "$current" > "$stamp"
fi

print -r -- "$library"
