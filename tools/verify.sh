#!/bin/zsh
#
# 本地全量验证。CI 跑的是同一组检查。
#
# 存在的理由：手敲验证命令容易漏。例如
#   cargo test --workspace 2>&1 | grep "^test result: ok" | awk '{s+=$4}'
# 这种统计会把 `test result: FAILED` 的行整个跳过——失败被静默吞掉，还显示
# 出一个看起来正常的用例数。本脚本以退出码为准，任何一步失败立即中止。
#
# 用法:
#   tools/verify.sh              Rust 检查 + macOS 产品壳 self-check
#   tools/verify.sh --rust-only  只跑 Rust 检查
#   tools/verify.sh --clean      产品壳用干净构建（改动 FFI 边界后必须）

set -euo pipefail

root="${0:A:h:h}"
cd "$root"

step() { printf "\n\033[1m▸ %s\033[0m\n" "$1" }

step "cargo fmt"
cargo fmt --all --check

step "cargo clippy"
cargo clippy --workspace --all-targets -- -D warnings

step "cargo test"
cargo test --workspace

step "FFI 头文件一致性"
python3 tools/check-ffi-header.py

if [[ "${1:-}" == "--rust-only" ]]; then
    printf "\n\033[1;32m✓ Rust 检查全部通过\033[0m\n"
    exit 0
fi

step "macOS 产品壳 self-check"
if [[ "${1:-}" == "--clean" ]]; then
    platform/macos/yu-shell-macos/run-self-checks.sh --clean-build
else
    platform/macos/yu-shell-macos/run-self-checks.sh --build
fi

printf "\n\033[1;32m✓ 全部验证通过\033[0m\n"
