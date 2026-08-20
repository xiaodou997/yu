#!/usr/bin/env python3
"""校验 C 头文件与 Rust FFI 实现一致。

`crates/yu-storage-ffi/include/yu_storage_ffi.h` 是手写的，与 Rust 实现之间
没有编译期约束：删掉一个 `pub extern "C" fn` 不会让头文件报错，头文件里留下
的孤儿声明也不会让任何构建失败。S1 期间就因此漏下了 3 个函数声明和 7 个类型
定义。本脚本把这层约束补上，由 CI 执行。

检查三件事：
  1. 每个 `pub extern "C" fn` 都在头文件里有声明；
  2. 头文件里没有已无实现的函数声明；
  3. 头文件里没有已无实现的 `YuStorage*` 类型定义。

用法: tools/check-ffi-header.py
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HEADER = ROOT / "crates/yu-storage-ffi/include/yu_storage_ffi.h"
SOURCE = ROOT / "crates/yu-storage-ffi/src/lib.rs"


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
