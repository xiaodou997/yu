#!/usr/bin/env python3
"""条件依赖不得被无条件代码引用。

`Cargo.toml` 里 `[target.'cfg(...)'.dependencies]` 段下的 crate，**只在那个
cfg 成立的平台上存在**。无条件的代码引用它们，在别的平台上就是
`error[E0433]: cannot find module or crate` —— 而开发机上一切正常。

**这是一次真实事故。** `yu-storage-ffi` 里有两个无条件函数
（`block_hidden_spans_output` / `search_match_output`）写着
`yu_markdown::Block::range`，而 `yu-markdown` 挂在
`cfg(target_os = "macos")` 下。后果：CI 的 ubuntu job 编译失败，而
`tools/verify.sh` 十步全绿——**开发机上看不见**。它躲了至少两轮，因为矩阵的
fail-fast 让 windows job 被连坐取消，一次只暴露一层。

# 为什么这是第三条 FFI 门禁，而不是前两条的一个分支

三条判据的**机制各不相同**，这是有意的（不变量 I8 的同一条思路）：

  check-ffi-header.py   读**源码的属性**：extern 函数挂没挂 cfg。
  check-ffi-symbols.py  读**产物的符号表**：只覆盖当前平台，三平台靠 CI 矩阵。
  本脚本                 读 **Cargo.toml 的依赖段 + 源码的引用位置**。

前两条都盖不住这一类：第一条查的是函数的属性，不是函数体里引用了谁；第二条
在 macOS 上跑，而 macOS 正是这些依赖存在的那个平台，它永远绿。

# 判据怎么定的

对每个有条件依赖的 crate：

  1. 从 `Cargo.toml` 收集**条件依赖的 crate 名**（`yu-markdown` → `yu_markdown`）；
  2. 扫源码，收集**挂在 `#[cfg(...)]` 下的 `use` 引进来的名字**——真实代码几乎
     不写全路径，都是先条件 `use` 再用短名，所以只查路径会漏掉绝大多数；
  3. 判断每一行**在不在某个 `#[cfg(...)]` 的管辖区里**（按缩进与花括号定界，
     嵌在函数体里的 cfg 块也算——刀 a 定的写法正是「平台差异写在函数体里，
     不写在函数上」）；
  4. **无条件的行引用了上面任何一个名字就红。**

第 3 步按 rustfmt 排好的缩进走。它是启发式的，宁可漏报不误报：漏报由 CI 的
三平台矩阵兜住，误报会让人学会忽略这条门禁。

用法: tools/check-cfg-deps.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

CONDITIONAL_SECTION = re.compile(r"^\[target\.'cfg\((?P<cfg>.+)\)'\.(?:dev-)?dependencies\]")
ANY_SECTION = re.compile(r"^\[")
DEPENDENCY = re.compile(r"^(?P<name>[A-Za-z0-9_-]+)\s*=")
CFG_ATTRIBUTE = re.compile(r"^#\[cfg\b")
USE_STATEMENT = re.compile(r"^\s*(?:pub\s+)?use\s+(?P<path>[A-Za-z0-9_:]+)(?P<rest>.*)$")
IDENTIFIER = re.compile(r"\b[A-Za-z_][A-Za-z0-9_]*\b")


def conditional_dependencies(manifest: Path) -> dict[str, str]:
    """crate 名（下划线形式）-> 它挂着的那条 cfg。"""
    found: dict[str, str] = {}
    cfg: str | None = None
    for line in manifest.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        match = CONDITIONAL_SECTION.match(stripped)
        if match is not None:
            cfg = match.group("cfg")
            continue
        if ANY_SECTION.match(stripped):
            cfg = None
            continue
        if cfg is None or not stripped or stripped.startswith("#"):
            continue
        dependency = DEPENDENCY.match(stripped)
        if dependency is not None:
            found[dependency.group("name").replace("-", "_")] = cfg
    return found


def strip_code(line: str) -> str:
    """去掉行注释与字符串字面量，免得注释里的名字被当成引用。"""
    line = re.sub(r'"(?:[^"\\]|\\.)*"', '""', line)
    comment = line.find("//")
    return line if comment < 0 else line[:comment]


def guarded_lines(lines: list[str]) -> set[int]:
    """返回落在某个 `#[cfg(...)]` 管辖区里的行号（0 基）。

    规则：一条 `#[cfg(...)]` 管辖它后面那个项/语句/块。项从紧随其后的第一行
    非属性、非文档注释开始；结束由花括号配平决定，没有花括号的（`use`、单行
    语句）到第一个以 `;` 结尾的行为止。
    """
    guarded: set[int] = set()
    for index, line in enumerate(lines):
        if not CFG_ATTRIBUTE.match(line.strip()):
            continue
        cursor = index + 1
        while cursor < len(lines):
            head = lines[cursor].strip()
            if head.startswith("#[") or head.startswith("///") or head.startswith("//"):
                cursor += 1
                continue
            break
        if cursor >= len(lines):
            break
        depth = 0
        opened = False
        while cursor < len(lines):
            guarded.add(cursor)
            code = strip_code(lines[cursor])
            depth += code.count("{") - code.count("}")
            if code.count("{"):
                opened = True
            if opened and depth <= 0:
                break
            if not opened and code.rstrip().endswith(";"):
                break
            cursor += 1
    return guarded


def imported_names(lines: list[str], rows: set[int]) -> set[str]:
    """`rows` 里那些 `use` 语句引进来的名字（只收大写开头的类型名）。"""
    names: set[str] = set()
    for index, line in enumerate(lines):
        if index not in rows:
            continue
        match = USE_STATEMENT.match(line)
        if match is None:
            continue
        # `use a::b::{C, D};` 与 `use a::b::C;` 两种形状；多行 use 由后续行
        # 各自命中 IDENTIFIER 收集。
        tail = match.group("rest")
        segment = match.group("path").split("::")[-1]
        if segment and segment[0].isupper():
            names.add(segment)
        for name in IDENTIFIER.findall(tail):
            if name and name[0].isupper():
                names.add(name)
    return names


def multiline_use_names(lines: list[str], rows: set[int]) -> set[str]:
    """跨行的 `use a::{\n  B,\n  C,\n};` —— 中间那些行也要收。"""
    names: set[str] = set()
    inside = False
    for index, line in enumerate(lines):
        if index not in rows:
            inside = False
            continue
        stripped = line.strip()
        if USE_STATEMENT.match(line) and "{" in stripped and "}" not in stripped:
            inside = True
            continue
        if not inside:
            continue
        if "}" in stripped:
            inside = False
        for name in IDENTIFIER.findall(strip_code(stripped)):
            if name and name[0].isupper():
                names.add(name)
    return names


def check(crate: Path, problems: list[str]) -> tuple[int, int]:
    manifest = crate / "Cargo.toml"
    dependencies = conditional_dependencies(manifest)
    if not dependencies:
        return 0, 0

    sources = sorted((crate / "src").rglob("*.rs"))
    guarded_names = 0
    for source in sources:
        lines = source.read_text(encoding="utf-8").splitlines()
        guarded = guarded_lines(lines)
        plain = set(range(len(lines))) - guarded
        # **同一个名字两边都 import 的不算**。从一个无条件依赖里 `#[cfg]` import
        # 一个类型是常见写法（免得在别的平台上报 unused_imports），只有**没有
        # 任何无条件 import** 的名字才是真的只在那个平台上存在。
        names = (imported_names(lines, guarded) | multiline_use_names(lines, guarded)) - (
            imported_names(lines, plain) | multiline_use_names(lines, plain)
        )
        guarded_names += len(names)
        for index, line in enumerate(lines):
            if index in guarded:
                continue
            code = strip_code(line)
            if not code.strip() or code.strip().startswith("#["):
                continue
            for dependency, cfg in dependencies.items():
                if re.search(rf"\b{dependency}::", code):
                    problems.append(
                        f"{source.relative_to(ROOT)}:{index + 1} 无条件代码引用了"
                        f"条件依赖 `{dependency}`（它挂在 cfg({cfg}) 下）:\n"
                        f"      {line.strip()}"
                    )
            # 前面带 `::` 的是写全了路径（`std::ptr::NonNull`），那不依赖 import。
            qualified = set(re.findall(r"::\s*([A-Za-z_][A-Za-z0-9_]*)", code))
            for name in IDENTIFIER.findall(code):
                if name in names and name not in qualified:
                    problems.append(
                        f"{source.relative_to(ROOT)}:{index + 1} 无条件代码用了"
                        f"只在 cfg 下 import 的 `{name}`:\n      {line.strip()}"
                    )
                    break
    return len(dependencies), guarded_names


def main() -> int:
    manifests = sorted(
        [*ROOT.glob("crates/*/Cargo.toml"), *ROOT.glob("platform/*/*/Cargo.toml")]
    )
    problems: list[str] = []
    crates = 0
    total_dependencies = 0
    for manifest in manifests:
        dependencies, _ = check(manifest.parent, problems)
        if dependencies:
            crates += 1
            total_dependencies += dependencies

    if problems:
        print(f"条件依赖被无条件代码引用（{len(problems)} 处）:", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        print(
            "\n这类错误在开发机上看不见——它只在那条 cfg 不成立的平台上编译失败。"
            "\n把引用挪进 #[cfg(...)] 的管辖区，或者把依赖改成无条件的。",
            file=sys.stderr,
        )
        return 1

    print(f"条件依赖未被无条件代码引用：{crates} 个 crate，{total_dependencies} 条条件依赖")
    return 0


if __name__ == "__main__":
    sys.exit(main())
