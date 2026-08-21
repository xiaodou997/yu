#!/usr/bin/env python3
"""视觉坐标只有一套实现，坐标空间必须进类型。

`yu-core::geometry` 把 `Point` / `Size` / `Rect` 收敛成一份带空间参数的实现。
收敛容易，保持收敛难：下一次有人需要一个矩形时，最省事的写法永远是在手边的
结构体里再加四个 `f32`。那样加出来的东西没有校验，也说不出自己是 block 局部
坐标、文档坐标还是物理像素——而这正是 `768b5e3`（绝对坐标当成相对坐标）与
`5fac1fe`（逻辑坐标上又乘了一次 backing scale）两次事故的形状：不报错，只是
画错，要靠真实窗口才能发现。

因此这里查两件事：

1. **不得再出现散装的 `x/y/width/height: f32` 四元组。** 一个结构体同时有
   `width: f32` 与 `height: f32` 字段就算命中。用 `yu_core::Rect<S>`。
2. **不得再定义第二个 `Point` / `Size` / `Rect`。** 名字里带这些词的结构体
   只能出现在 `yu-core`。

例外只有两类，逐个登记在下面并且必须写清单位：跨 C ABI 的平铺结构体（那一侧
没有泛型），以及根本不是 f32 视觉坐标的整数量（atlas 纹理坐标、图片自身的像素
尺寸）。「说清单位」不是形式要求——写不出「这是逻辑坐标还是物理像素」的结构
体，正是这两次事故的起点。

登记表还会反向检查：表里有而代码里没有的条目同样算失败，免得例外表变成一张
没人清理的旧账。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# 允许定义几何原语的地方。
CORE = Path("crates/yu-core/src/geometry.rs")

# 登记在册的例外。key 是 `文件:结构体名`，value 必须说清它是什么单位——
# 说不出来就说明这个结构体自己也不知道自己是什么，那正是要拦的东西。
#
# 只有两类可以登记：跨 C ABI 的平铺结构体（那一侧没有泛型），以及不是 f32
# 视觉坐标的整数量（纹理坐标、图片自身的像素尺寸）。
REGISTERED: dict[str, str] = {
    "crates/yu-storage-ffi/src/lib.rs:YuStorageTaskCheckboxHit": "Document 逻辑坐标，随 C ABI 平铺",
    "crates/yu-storage-ffi/src/lib.rs:YuStorageTableResizeAccessibilityDivider": (
        "Document 逻辑坐标，AX 分隔线几何随 C ABI 平铺"
    ),
    "platform/macos/yu-render-macos/src/lib.rs:NativeDrawCommand": (
        "Document 逻辑坐标，绘制指令平铺给 Metal 桥"
    ),
    "platform/macos/yu-render-macos/src/lib.rs:NativeDamageRect": (
        "Document 逻辑坐标，scissor 由 ObjC 侧乘 backing scale 转成 Device"
    ),
    "crates/yu-font/src/raster.rs:AtlasRect": (
        "atlas 页内的整数纹理坐标，不是视觉坐标，也不是 f32"
    ),
    "crates/yu-layout/src/lib.rs:ImageIntrinsicSize": (
        "解码后图片自身的整数像素尺寸，不落在任何视觉坐标空间里"
    ),
}

# 名字里带这些词的结构体只能定义在 yu-core。
RESERVED = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?struct\s+(\w*(?:Point|Size|Rect)\w*)\b")

STRUCT = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?struct\s+(\w+)")
FIELD = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(\w+)\s*:\s*f32\s*,")


def rust_files() -> list[Path]:
    found: list[Path] = []
    for base in ("crates", "platform", "tools"):
        for path in sorted((ROOT / base).rglob("*.rs")):
            found.append(path.relative_to(ROOT))
    return found


def scan(path: Path) -> tuple[list[tuple[int, str]], list[tuple[int, str]]]:
    """返回 (散装四元组, 重名几何类型)。"""
    quadruples: list[tuple[int, str]] = []
    reserved: list[tuple[int, str]] = []
    name: str | None = None
    start = 0
    fields: set[str] = set()
    in_test = False

    lines = (ROOT / path).read_text(encoding="utf-8", errors="replace").split("\n")
    for number, line in enumerate(lines, start=1):
        if re.match(r"^\s*(#\[cfg\(test\)\]|mod tests\b)", line):
            in_test = True
        match = STRUCT.match(line)
        if match is not None:
            if name is not None and {"width", "height"} <= fields:
                quadruples.append((start, name))
            name, start, fields = match.group(1), number, set()
            if not in_test:
                reserved_match = RESERVED.match(line)
                if reserved_match is not None:
                    reserved.append((number, reserved_match.group(1)))
            continue
        if name is None:
            continue
        if line.startswith("}"):
            if {"width", "height"} <= fields:
                quadruples.append((start, name))
            name, fields = None, set()
            continue
        field = FIELD.match(line)
        if field is not None:
            fields.add(field.group(1))

    if name is not None and {"width", "height"} <= fields:
        quadruples.append((start, name))
    return quadruples, reserved


def main() -> int:
    if not (ROOT / CORE).is_file():
        print(f"找不到几何原语 {CORE}", file=sys.stderr)
        return 1

    problems: list[str] = []
    used_exceptions: set[str] = set()
    checked = 0

    for path in rust_files():
        if path == CORE:
            continue
        quadruples, reserved = scan(path)
        checked += 1
        for number, name in quadruples:
            key = f"{path}:{name}"
            if key in REGISTERED:
                used_exceptions.add(key)
                continue
            problems.append(
                f"{path}:{number}: `{name}` 自己摊开了 width/height: f32。"
                f"用 yu_core::Rect<S>——散装四元组说不出自己是哪个坐标空间。"
            )
        for number, name in reserved:
            key = f"{path}:{name}"
            if key in REGISTERED:
                used_exceptions.add(key)
                continue
            problems.append(
                f"{path}:{number}: `{name}` 在 {CORE} 之外定义了几何类型。"
                f"视觉坐标只有一套实现，空间用类型参数区分。"
            )

    for stale in sorted(set(REGISTERED) - used_exceptions):
        problems.append(f"{stale}: 登记在例外表里，但已经不存在了")

    if problems:
        print("视觉坐标检查失败：\n", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        print(
            f"\n几何原语在 {CORE}。跨 C ABI 平铺、或者根本不是 f32 视觉坐标"
            "（纹理坐标、图片像素尺寸）时，把结构体连同它的单位登记进 "
            "tools/check-geometry.py 的 REGISTERED。",
            file=sys.stderr,
        )
        return 1

    print(
        f"视觉坐标已收敛：{checked} 个文件，{len(REGISTERED)} 个已登记例外"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
