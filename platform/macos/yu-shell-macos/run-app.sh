#!/bin/zsh
#
# 构建并（重新）启动 Yu.app。
#
# 存在的理由：macOS 的 `open` 对已经在运行的 app 只会把它带到前台，不会加载
# 新的二进制。改完代码后直接 `open` 会一直看到旧版本——包括已经修好的 bug
# 仍然「复现」。这里先终止已有实例再启动。
#
# 用法:
#   run-app.sh                  打开文件选择面板
#   run-app.sh path/to/file.md  直接打开该文件

set -euo pipefail

shell_dir="${0:A:h}"
app="$("$shell_dir/build-app.sh")"
binary="$app/Contents/MacOS/Yu"

if pgrep -f "$binary" >/dev/null 2>&1; then
    print -r -- "终止已在运行的实例" >&2
    pkill -f "$binary" || true
    # 等待进程真正退出，否则 open 可能又激活到正在退出的实例上
    for _ in {1..20}; do
        pgrep -f "$binary" >/dev/null 2>&1 || break
        sleep 0.1
    done
fi

if (( $# > 0 )); then
    open "$app" --args "${@:A}"
else
    open "$app"
fi
print -r -- "$app" >&2
