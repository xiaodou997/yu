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
#   tools/verify.sh --fuzz       只跑随机 fuzz（不是确定性门禁，见下）

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

step "本地门禁与 CI 一致"
python3 tools/check-ci-parity.py

step "crate 依赖方向"
python3 tools/check-deps.py

step "FFI 头文件一致性"
python3 tools/check-ffi-header.py

step "ropey 未逃逸出 yu-text"
python3 tools/check-rope-leak.py

step "视觉坐标只有一套实现"
python3 tools/check-geometry.py

# 随机 fuzz 不是确定性门禁，默认不跑。它在这里出现有两个理由：
#
#   1. CI 有一个跑它的 job，而 tools/check-ci-parity.py 要求本地门禁**知道**
#      CI 的每一条命令——「本地全绿而 CI 会红」是这个项目踩过的坑；
#   2. 让人知道有这么个东西可以手动跑。
#
# 分工见 crates/yu-syntax/tests/corpus/README.md：fuzz 负责发现，
# corpus 负责不复发，只有后者进确定性门禁。
if [[ "${1:-}" == "--fuzz" ]]; then
    step "语法解析 fuzz"
    tools/fuzz.sh 120
    printf "\n\033[1;32m✓ fuzz 跑完\033[0m\n"
    exit 0
fi

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
