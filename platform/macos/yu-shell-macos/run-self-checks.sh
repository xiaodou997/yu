#!/bin/zsh
#
# 运行 macOS document host 的 self-check。
#
# 这些 self-check 验证 Rust↔Swift 边界上的真实行为（剪贴板、selection、undo、
# 投影、命中测试、IME、Accessibility）。v1 时期它们从未进入 CI，没有反馈回路，
# 因而无节制地长到了 3800 行；本脚本把配对关系固定下来并交给 CI 执行。
#
# 用法:
#   run-self-checks.sh            运行全部 headless self-check
#   run-self-checks.sh --build    先构建 Rust static library 与 Swift 可执行文件
#
# 需要窗口服务的 self-check（launch-window）不在此列，见文件末尾说明。

set -euo pipefail

host_dir="${0:A:h}"
cd "$host_dir"

# --build 是增量构建。删除一个 C 类型或 FFI 函数后，SwiftPM 可能不会重编引用
# 它的文件，本地因此看到「构建通过」而 CI 的干净检出会失败。改动 FFI 边界后
# 用 --clean-build 验证。
if [[ "${1:-}" == "--clean-build" ]]; then
    rm -rf .build
    ./build-rust-ffi.sh >/dev/null
    swift build >/dev/null
elif [[ "${1:-}" == "--build" ]]; then
    ./build-rust-ffi.sh >/dev/null
    swift build >/dev/null
fi

binary="$(swift build --show-bin-path 2>/dev/null)/Yu"
if [[ ! -x "$binary" ]]; then
    print -r -- "未找到可执行文件 $binary，请先运行 $0 --build" >&2
    exit 1
fi

# self-check 名 -> fixture。fixture 必须真正含有该检查断言的语法结构，
# 否则 precondition 会以「缺少某某 block」失败，看起来像回归。
typeset -A checks=(
    accessibility                       Fixtures/block-projection.md
    block-layout                        Fixtures/block-projection.md
    block-projection                    Fixtures/block-projection.md
    clipboard                           Fixtures/block-projection.md
    composition-hit-test                Fixtures/composition-cross-block.md
    composition-projection              Fixtures/block-projection.md
    document-interaction                Fixtures/composition-cross-block.md
    document-workflow                   Fixtures/render-surface.md
    macos-table-resize-coordinator      Fixtures/block-projection.md
    macos-task-checkbox                 Fixtures/render-tasks.md
    projection                          Fixtures/block-projection.md
    projection-hit-test                 Fixtures/block-projection.md
    selection                           Fixtures/sample.md
    shaped-projection-hit-test          Fixtures/block-projection.md
    shaped-vertical                     Fixtures/block-projection.md
    shaped-viewport                     Fixtures/block-projection.md
    undo                                Fixtures/block-projection.md
)

typeset -a failed
for check in ${(ok)checks}; do
    fixture="${checks[$check]}"
    printf "%-34s " "$check"
    if output="$("$binary" "--${check}-self-check" "$fixture" 2>&1)"; then
        print -r -- "OK"
    else
        print -r -- "FAILED"
        print -r -- "$output" | sed 's/^/    /'
        failed+=("$check")
    fi
done

print -r -- ""
if (( ${#failed} > 0 )); then
    print -r -- "${#failed} 个 self-check 失败: ${failed[*]}" >&2
    exit 1
fi
print -r -- "全部 ${#checks} 个 self-check 通过"

# launch-window-self-check 会经由 applicationDidFinishLaunching 真正创建
# NSWindow，需要可用的窗口服务，因此不在此列。它顺带跑帧调度自检：只有真实
# 的 NSWindow 与 Metal surface 才会产生「已提交的帧」，headless 覆盖不到
# 「屏幕上那一帧还算不算数」这个判断。改动帧调度后必须在本地跑一次：
#   "$binary" --launch-window-self-check Fixtures/block-projection.md
