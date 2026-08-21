#!/usr/bin/env python3
"""本地门禁必须覆盖 CI 跑的每一条命令。

存在的理由是一次真实事故：删掉 `yu-editor-ffi` 之后，CI 里链接
`-lyu_editor_ffi` 的 `macos-input-spike` job 必然失败，而 `tools/verify.sh`
不覆盖那个 job——本地全绿，CI 会红。本地门禁给了一个它无权给的绿灯，属于这个
项目最危险的失败模式：静默地做错事。

检查方式很直接：`.github/workflows/*.yml` 里每一条 `run:` 命令，都必须在
`tools/verify.sh` 里出现。verify.sh 可以按参数跳过其中一部分（例如
`--rust-only`），但不能不知道它们的存在。

反过来不检查：verify.sh 比 CI 多做检查是好事。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VERIFY = ROOT / "tools/verify.sh"
WORKFLOWS = ROOT / ".github/workflows"


def ci_commands() -> list[tuple[str, str]]:
    """返回 [(工作流文件名, 命令)]。"""
    found: list[tuple[str, str]] = []
    for workflow in sorted(WORKFLOWS.glob("*.yml")):
        for line in workflow.read_text().split("\n"):
            match = re.match(r"\s*-\s+run:\s+(.+?)\s*$", line)
            if match is not None:
                found.append((workflow.name, match.group(1)))
    return found


def main() -> int:
    if not VERIFY.is_file():
        print(f"找不到 {VERIFY}", file=sys.stderr)
        return 1
    verify = VERIFY.read_text()

    missing = [
        (workflow, command)
        for workflow, command in ci_commands()
        if command not in verify
    ]
    if missing:
        print("本地门禁没有覆盖 CI 的这些命令：\n", file=sys.stderr)
        for workflow, command in missing:
            print(f"  {workflow}: {command}", file=sys.stderr)
        print(
            "\ntools/verify.sh 必须能跑到 CI 跑的每一条命令，否则本地绿灯不代表"
            "CI 会绿。",
            file=sys.stderr,
        )
        return 1

    total = len(ci_commands())
    print(f"本地门禁覆盖 CI 的全部 {total} 条命令")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
