#!/usr/bin/env python3
"""校验 C 头文件与 Rust FFI 实现一致。

`crates/yu-storage-ffi/include/yu_storage_ffi.h` 是手写的，与 Rust 实现之间
没有编译期约束：删掉一个 `pub extern "C" fn` 不会让头文件报错，头文件里留下
的孤儿声明也不会让任何构建失败。S1 期间就因此漏下了 3 个函数声明和 7 个类型
定义。本脚本把这层约束补上，由 CI 执行。

检查四件事：
  1. 每个 `pub extern "C" fn` 都在头文件里有声明；
  2. 头文件里没有已无实现的函数声明；
  3. 头文件里没有已无实现的 `YuStorage*` 类型定义；
  4. 没有一个 `pub extern "C" fn` 挂在 cfg 下面。

第 4 条是 S7 第七刀补的，起因是前三条**全绿而头文件在撒谎**：
`yu_storage_session_macos_task_checkbox_hit_test` 与
`yu_storage_session_macos_table_resize_at_point` 整个挂在
`#[cfg(target_os = "macos")]` 下，而头文件无条件声明它们——在 Windows 或
Linux 上链接这个 staticlib 会 unresolved symbol。前三条看不见它，因为它们
匹配的是源码文本，而正则不认识 cfg。

头文件是无条件的，所以实现也必须是无条件的。**平台差异写在函数体里**
（`#[cfg(not(target_os = "macos"))]` 早退一个状态码），不写在函数上——
其余 10 个 `macos_*` 函数一直是这么写的，那两个是漏网的。

这一条是**便携的**：不需要编译，不需要交叉工具链，在开发机上立刻就红。
它兜不住的（`cfg_attr`、宏生成的 extern、被 cfg 掉的外层 mod）由
`tools/check-ffi-symbols.py` 在 CI 的三平台矩阵上读符号表兜住。两条判据
的机制不同：一条读源码的属性，一条读产物的符号表。

用法: tools/check-ffi-header.py
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HEADER = ROOT / "crates/yu-storage-ffi/include/yu_storage_ffi.h"
SOURCE = ROOT / "crates/yu-storage-ffi/src/lib.rs"



FUNCTION = re.compile(r'^pub (?:unsafe )?extern "C" fn ([a-z_0-9]+)', re.M)


def conditionally_compiled(source: str) -> list[tuple[str, str]]:
    """返回 [(函数名, 那条 cfg)]，按源码顺序。

    判据是**紧挨着函数的那一串属性行**：从函数签名往上走，只要还在属性
    (`#[...]`) 或文档注释 (`///`) 上就继续，遇到别的就停。cfg 只可能出现在
    这一段里。
    """
    lines = source.split("\n")
    found: list[tuple[str, str]] = []
    for index, line in enumerate(lines):
        match = FUNCTION.match(line)
        if match is None:
            continue
        cursor = index - 1
        while cursor >= 0:
            above = lines[cursor].strip()
            if not (above.startswith("#[") or above.startswith("///")):
                break
            if above.startswith("#[cfg"):
                found.append((match.group(1), above))
            cursor -= 1
    return found


def main() -> int:
    header = HEADER.read_text(encoding="utf-8")
    source = SOURCE.read_text(encoding="utf-8")

    declared = set(re.findall(r"\b(?:int32_t|void|size_t|uint64_t) (yu_[a-z_0-9]+)\(", header))
    implemented = set(re.findall(r'pub (?:unsafe )?extern "C" fn ([a-z_0-9]+)', source))

    header_types = set(re.findall(r"\}\s*(YuStorage[A-Za-z0-9]+);", header))
    # Rust 侧的 repr(C) 类型可以是 struct 或 enum，也可能只作为别名出现。
    source_types = set(re.findall(r"pub (?:struct|enum|union) (YuStorage[A-Za-z0-9]+)", source))
    source_types |= set(re.findall(r"\b(YuStorage[A-Za-z0-9]+)\b", source))

    problems: list[str] = []

    for name, attributes in conditionally_compiled(source):
        problems.append(f"这个 extern 函数挂在 cfg 下，头文件却无条件声明它: {name} ({attributes})")
    for name in sorted(implemented - declared):
        problems.append(f"实现了但头文件缺少声明: {name}")
    for name in sorted(declared - implemented):
        problems.append(f"头文件有声明但已无实现: {name}")
    for name in sorted(header_types - source_types):
        problems.append(f"头文件有类型定义但已无实现: {name}")

    if problems:
        print(f"FFI 头文件与实现不一致（{len(problems)} 处）:", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        print(
            "\n修改 FFI 后请同步 crates/yu-storage-ffi/include/yu_storage_ffi.h。",
            file=sys.stderr,
        )
        return 1

    print(f"FFI 头文件一致: {len(implemented)} 个函数，{len(header_types)} 个类型")
    return 0


if __name__ == "__main__":
    sys.exit(main())
