#!/usr/bin/env python3
"""不变量 E4：ropey 不得逃逸出 `yu-text`。

E4 的原文是「`yu-text` 不得让 ropey 的 char index 或 ropey 类型逃逸出 crate
边界；对外只暴露 `ByteOffset`」。这里把它拆成四条可机械判定的规则：

1. **只有 `yu-text` 可以依赖 ropey。** 别的 crate 在 Cargo.toml 里写下这条
   依赖就失败。
2. **只有适配层可以引用 ropey 的路径。** `use ropey…` 与 `ropey::…` 只允许
   出现在 `crates/yu-text/src/storage/ropey_backend.rs`。模块名特意叫
   `ropey_backend` 而不是 `ropey`，这样规则不需要「除了 `mod ropey;` 之外」
   这类例外——例外正是这种检查失效的地方。
3. **适配层不导出任何 `pub` 项。** 只允许 `pub(super)` / `pub(crate)`。
4. **不得开启 ropey 的 `metric_chars` feature。**

2 和 3 合起来就够了：ropey 的类型要出现在 `yu-text` 的公开签名里，就得先在
某处写出它的路径；能写出它的地方只有适配层，而适配层什么都不导出。

规则 4 管的是 char index 那一半。ropey 2.x 的 API 全部按字节索引，
`len_chars` / `byte_to_char_idx` 这些函数只在 `metric_chars` 下才存在。不开
这个 feature，「字节与字符索引混用」在编译期就无从发生。

这不是「检查用词」的检查——`StorageBackend` 的显示名就叫 ropey，那没有问题。
它守的是一件具体的事：字节与字符索引混用不会 panic，只会在某个 emoji 或
组合字符上悄悄切错位置。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

ADAPTER = Path("crates/yu-text/src/storage/ropey_backend.rs")
OWNER = Path("crates/yu-text/Cargo.toml")

# 路径引用：`use ropey…` / `ropey::…` / `extern crate ropey`。
# `ropey_backend` 里的 `ropey` 后面跟着 `_`，是词内字符，`\b` 不会命中。
PATH_REFERENCE = re.compile(
    r"\buse\s+ropey\b|\bropey\s*::|\bextern\s+crate\s+ropey\b"
)

# Cargo.toml 里的依赖项。
DEPENDENCY = re.compile(r"^\s*ropey\s*=")

# `pub` 后面不跟 `(` 的就是无限定的公开项。
UNQUALIFIED_PUB = re.compile(r"^\s*pub\s+(?!\()")

SOURCE_SUFFIXES = {".rs", ".swift", ".h", ".m"}


def tracked_files(suffixes: set[str]) -> list[Path]:
    """产物与工具代码。docs/ 讲的是决策，不在此列。"""
    found: list[Path] = []
    for base in ("crates", "platform", "tools"):
        for path in sorted((ROOT / base).rglob("*")):
            if path.is_file() and path.suffix in suffixes:
                found.append(path.relative_to(ROOT))
    return found


def main() -> int:
    if not (ROOT / ADAPTER).is_file():
        print(f"找不到 ropey 适配层 {ADAPTER}", file=sys.stderr)
        return 1

    problems: list[str] = []
    references = 0

    for path in tracked_files(SOURCE_SUFFIXES):
        text = (ROOT / path).read_text(encoding="utf-8", errors="replace")
        for number, line in enumerate(text.split("\n"), start=1):
            if PATH_REFERENCE.search(line) is None:
                continue
            references += 1
            if path != ADAPTER:
                problems.append(
                    f"{path}:{number}: 只有 {ADAPTER} 可以引用 ropey 的路径。"
                    f"这一行让 ropey 逃出了 yu-text 的边界（不变量 E4）。"
                )

    for path in tracked_files({".toml"}):
        text = (ROOT / path).read_text(encoding="utf-8", errors="replace")
        for number, line in enumerate(text.split("\n"), start=1):
            if DEPENDENCY.search(line) is None:
                continue
            if path != OWNER:
                problems.append(
                    f"{path}:{number}: 只有 {OWNER} 可以依赖 ropey（不变量 E4）。"
                )
            elif "metric_chars" in line or "metric_chars" in text:
                problems.append(
                    f"{path}:{number}: 开启了 ropey 的 metric_chars feature。"
                    f"Yu 的坐标是字节，char index 一旦可用就迟早会被用（不变量 E4）。"
                )

    adapter_text = (ROOT / ADAPTER).read_text()
    for number, line in enumerate(adapter_text.split("\n"), start=1):
        if UNQUALIFIED_PUB.search(line) is not None:
            problems.append(
                f"{ADAPTER}:{number}: 适配层不得有无限定的 `pub`。"
                f"用 `pub(super)`，否则 ropey 类型可能进入 yu-text 的公开签名。"
            )

    if problems:
        print("ropey 逃逸检查失败：\n", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        print(
            "\nyu-text 对外只暴露 ByteOffset。ropey 的类型与索引留在"
            f" {ADAPTER} 里面。",
            file=sys.stderr,
        )
        return 1

    print(f"ropey 未逃逸：{references} 处路径引用全部落在 {ADAPTER} 内")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
