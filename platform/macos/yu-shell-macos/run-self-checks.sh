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

# Scheduler gate is intentionally headless and independent of the window
# service. Run it on every self-check pass so burst starvation/regression is
# caught even when a real AppKit window is unavailable.
swiftc -module-cache-path /tmp/yu-scroll-swift-cache \
    Sources/Yu/FrameWakeGate.swift Tests/FrameWakeGateChecks.swift \
    -o /tmp/yu-frame-wake-checks
/tmp/yu-frame-wake-checks >/dev/null

# Calendar publication and the midnight timer can be checked without an app window.
swiftc -swift-version 5 -warnings-as-errors Sources/Yu/RenderCalendarContext.swift Tests/RenderCalendarChecks.swift -o /tmp/yu-render-calendar-checks
/tmp/yu-render-calendar-checks

swiftc Sources/Yu/TypewriterGeometry.swift Tests/TypewriterGeometryChecks.swift -o /tmp/yu-typewriter-geometry-checks
/tmp/yu-typewriter-geometry-checks

# Uses the installed system dictionary, with explicit English fixture language.
swiftc Sources/Yu/NativeSpelling.swift Tests/NativeSpellingChecks.swift -o /tmp/yu-native-spelling-check
/tmp/yu-native-spelling-check

# Filesystem import checks use private temporary resources and remove them.
swiftc Sources/Yu/ImageResources.swift Sources/Yu/WritingPreferences.swift Tests/ImageResourceChecks.swift -o /tmp/yu-image-resource-checks
/tmp/yu-image-resource-checks

# --build 是增量构建。删除一个 C 类型或 FFI 函数后，SwiftPM 可能不会重编引用
# 它的文件，本地因此看到「构建通过」而 CI 的干净检出会失败。改动 FFI 边界后
# 用 --clean-build 验证。
if [[ "${1:-}" == "--clean-build" ]]; then
    rm -rf .build
    ./build-app.sh >/dev/null
elif [[ "${1:-}" == "--build" ]]; then
    ./build-app.sh >/dev/null
fi

binary="$host_dir/.build/Yu.app/Contents/MacOS/Yu"
if [[ ! -x "$binary" ]]; then
    print -r -- "未找到可执行文件 $binary，请先运行 $0 --build" >&2
    exit 1
fi

# self-check 名 -> fixture。fixture 必须真正含有该检查断言的语法结构，
# 否则 precondition 会以「缺少某某 block」失败，看起来像回归。
typeset -A checks=(
    calendar                            Fixtures/sample.md
    spelling-coordinator              Fixtures/block-projection.md
    reading-preferences                 Fixtures/block-projection.md
    image-batch                         Fixtures/assets/yu-mark.png
    image-properties                    Fixtures/assets/yu-mark.png
    accessibility                       Fixtures/block-projection.md
    clipboard                           Fixtures/block-projection.md
    code-highlight                      Fixtures/render-code.md
    document-interaction                Fixtures/composition-cross-block.md
    document-workflow                   Fixtures/render-surface.md
    macos-table-resize-coordinator      Fixtures/block-projection.md
    macos-task-checkbox                 Fixtures/render-tasks.md
    multi-cursor                        Fixtures/multi-cursor.md
    outline-panel                       Fixtures/outline.md
    search-panel                        Fixtures/search.md
    selection                           Fixtures/sample.md
    shaped-projection-hit-test          Fixtures/block-projection.md
    shaped-vertical                     Fixtures/block-projection.md
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
# 「屏幕上那一帧还算不算数」这个判断。它还压着大纲面板导航之后视口真的滚了
# ——headless 那边没有 scroll view，`revealCaretIfNeeded` 一进门就返回。
# 它还压着搜索高亮真的进了屏幕上那一帧——headless 那边没有 surface，
# 场景根本不提交，`search_decoration_count` 无从谈起。代码高亮同理：
# `--code-highlight-self-check` 数的是 retained frame 里的字形颜色，而
# 「那一帧真的上了屏」只有真实 surface 说得清。
# 改动帧调度、大纲面板或搜索后必须在本地跑一次：
#   "$binary" --launch-window-self-check Fixtures/outline.md
# fixture 必须有一条落在首屏之外的标题（outline.md 有），否则「选中之后滚了」
# 这一条会以「视口没有滚动」假红。
