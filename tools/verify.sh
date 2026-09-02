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

# 上一条读源码，这一条读**产物的符号表**。两条判据的机制不同，是有意的：
# 源码那条在开发机上立刻红，产物这条兜住它兜不住的（cfg_attr、宏生成的
# extern、被 cfg 掉的外层 mod）。
#
# 它只证明**当前这个平台**——这台机器交叉编译不了（tree-sitter 的 grammar
# 是 C，交叉要目标平台的 C 编译器）。三个平台的覆盖来自 CI 的 rust 矩阵。
step "FFI 符号在本平台的产物里都在"
python3 tools/check-ffi-symbols.py

# 第三条 FFI 门禁，机制与前两条又不同：读 Cargo.toml 的条件依赖段 + 源码里
# 的引用位置。前两条都盖不住这一类——第一条查的是 extern 函数挂没挂 cfg，
# 不是函数体里引用了谁；第二条在 macOS 上跑，而 macOS 正是那些依赖存在的那个
# 平台，它永远绿。
#
# 它守的是一次真实事故：两个无条件函数引用了挂在 cfg(macos) 下的 yu-markdown，
# CI 的 ubuntu job 编译失败，而这里十步全绿。
step "条件依赖未被无条件代码引用"
python3 tools/check-cfg-deps.py

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
