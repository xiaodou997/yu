#!/usr/bin/env python3
"""crate 依赖方向检查。

不变量 E2 要求依赖图是严格 DAG 且方向正确，「反向依赖是 CI 失败，不是待办
事项」。cargo 本身只拦得住环，拦不住方向——`yu-font` 依赖 `yu-layout` 在
cargo 看来完全合法，而它正是 overview-v2 第 2.4 节点名的那条反向依赖。

这里用**显式白名单**而不是层号比较。层号只能表达「更低层」，表达不了
「布局不该知道资源加载」这类同层禁止；而白名单让每加一条边都必须先在这张
表里写下来，`cargo add` 一个兄弟 crate 会直接失败。

白名单只覆盖 workspace 内部的 path 依赖。外部 crate（ropey、comrak…）不在
此列，那是选型问题，不是分层问题。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# crate -> 允许的 workspace 内部**产物**依赖。
#
# 顺序即分层。每一条都要能说出理由；加一条边之前先问它是否让某一层知道了
# 本不该知道的东西（overview-v2 第 4.3 节的职责表）。
ALLOWED: dict[str, set[str]] = {
    # 0 层：不依赖任何东西。
    "yu-core": set(),
    # 1 层：只依赖 yu-core。
    #
    # yu-font 在这一层，不在 overview-v2 第 4.2 节那条链的末尾——那条链把它
    # 画在 yu-render 之后，但同一节紧接着写明「yu-font 只能依赖 yu-core」。
    # 实际方向是 scene/render 依赖 font，所以它是靠近 core 的叶子。
    "yu-text": {"yu-core"},
    "yu-font": {"yu-core"},
    "yu-assets": {"yu-core"},
    # 2 层：解析与资源渲染。
    #
    # yu-syntax 在 yu-markdown 下方：它只认识语法树，不产出 decoration。
    # 现阶段 yu-markdown 还没有依赖它——S3 只建解析器，接线在 S4/S5 随
    # yu-decoration 与布局重写一起做（理由见 docs/architecture/overview-v2.md
    # 第 8 节）。这条边先登记，是为了让方向从一开始就被约束住。
    "yu-syntax": {"yu-core", "yu-text"},
    # yu-markdown 产出 decoration（第 4.3 节的职责），所以它依赖
    # yu-decoration——那是个不认识 Markdown 的原语，在它下方。
    "yu-markdown": {"yu-core", "yu-decoration", "yu-syntax", "yu-text"},
    "yu-embedded-math": {"yu-assets"},
    # 3 层：装饰中枢。
    #
    # yu-decoration 不认识 Markdown（第 4.3 节的禁止项），所以它只依赖
    # yu-core（坐标）与 yu-text（ChangeSet，`map` 的输入）。它**不**依赖
    # yu-syntax：装饰由 yu-markdown 的 extension 产出，中枢只承载结果。
    "yu-decoration": {"yu-core", "yu-text"},
    # yu-projection 是 v1 的实现，S4 结束时删除。在那之前两者并存，
    # 由 yu-decoration 的差分测试逐点比对。
    "yu-projection": {"yu-core", "yu-markdown", "yu-text"},
    # 4-6 层：布局 → 场景 → 绘制指令。
    "yu-layout": {"yu-core", "yu-markdown", "yu-projection", "yu-text"},
    "yu-scene": {"yu-core", "yu-font", "yu-layout"},
    "yu-render": {"yu-assets", "yu-core", "yu-font", "yu-scene"},
    # 7 层及以上：编辑状态、持久化、工作区。
    "yu-editor": {"yu-core", "yu-layout", "yu-markdown", "yu-projection", "yu-text"},
    "yu-export": {"yu-core", "yu-markdown", "yu-text"},
    "yu-storage": {"yu-core", "yu-editor", "yu-text"},
    "yu-workspace": {
        "yu-assets",
        "yu-core",
        "yu-editor",
        "yu-font",
        "yu-layout",
        "yu-render",
        "yu-scene",
        "yu-storage",
        "yu-text",
    },
    # 平台层：可以向下依赖，但不得被任何 core crate 依赖。
    "yu-font-macos": {"yu-core", "yu-font"},
    "yu-render-macos": {
        "yu-assets",
        "yu-core",
        "yu-editor",
        "yu-font",
        "yu-font-macos",
        "yu-render",
        "yu-scene",
        "yu-workspace",
    },
    # C ABI 边界：产品壳唯一的入口，聚合所有下层。
    "yu-storage-ffi": {
        "yu-assets",
        "yu-core",
        "yu-editor",
        "yu-embedded-math",
        "yu-export",
        "yu-font",
        "yu-font-macos",
        "yu-markdown",
        "yu-render",
        "yu-render-macos",
        "yu-scene",
        "yu-storage",
        "yu-text",
        "yu-workspace",
    },
    # 工具：不在产物链路上。
    "yu-bench": {
        "yu-assets",
        "yu-core",
        "yu-editor",
        "yu-layout",
        "yu-markdown",
        "yu-storage",
        "yu-syntax",
        "yu-text",
    },
    "yu-inspect": {"yu-markdown", "yu-text"},
}

# 测试专用的边，单独登记。
#
# dev-dependency 不进产物依赖图，但它同样表达耦合：如果一个 crate 的测试必须
# 依赖上层，通常说明这个用例放错了地方（它断言的是消费侧契约）。因此这里也要
# 显式列出，每一条都附理由。
ALLOWED_DEV: dict[str, set[str]] = {
    # 「布局能消费真实字体后端」是消费侧契约，用例住在 yu-layout。
    "yu-layout": {"yu-font"},
    # 平台层本就允许向下依赖；这些是 CoreText 与布局的集成断言。
    "yu-font-macos": {"yu-layout", "yu-projection", "yu-text"},
    "yu-render": {"yu-layout", "yu-markdown", "yu-projection", "yu-text"},
    "yu-scene": {"yu-markdown", "yu-projection", "yu-text"},
    "yu-embedded-math": {"yu-core"},
    # 临时：yu-projection 是 yu-decoration 的 source↔visual 映射的 oracle。
    # 一个已经在产品里跑着的实现比自证性质更强。这条边随 yu-projection
    # 在 S4 末尾被删除而消失。
    "yu-decoration": {"yu-projection"},
}


def crate_manifests() -> list[Path]:
    return sorted(
        [
            *ROOT.glob("crates/*/Cargo.toml"),
            *ROOT.glob("platform/*/*/Cargo.toml"),
            *ROOT.glob("tools/*/Cargo.toml"),
        ]
    )


def parse(manifest: Path) -> tuple[str, set[str], set[str]]:
    """返回 (crate 名, 产物 path 依赖, 测试 path 依赖)。"""
    name_match = re.search(r"^name = \"([^\"]+)\"", manifest.read_text(), re.M)
    if name_match is None:
        raise SystemExit(f"{manifest} 缺少 [package] name")
    section = None
    normal: set[str] = set()
    dev: set[str] = set()
    for line in manifest.read_text().split("\n"):
        header = re.match(r"^\[([^\]]+)\]", line)
        if header is not None:
            section = header.group(1)
            continue
        if section is None:
            continue
        entry = re.match(r"^([A-Za-z0-9_-]+) = .*path = \"", line)
        if entry is None or not entry.group(1).startswith("yu-"):
            continue
        # `[target.'cfg(...)'.dependencies]` 也是产物依赖。
        if section.endswith("dev-dependencies"):
            dev.add(entry.group(1))
        elif section.endswith("dependencies"):
            normal.add(entry.group(1))
    return name_match.group(1), normal, dev


def main() -> int:
    problems: list[str] = []
    seen: set[str] = set()
    for manifest in crate_manifests():
        name, normal, dev = parse(manifest)
        seen.add(name)
        if name not in ALLOWED:
            problems.append(
                f"{name}: 不在 tools/check-deps.py 的分层表里。"
                f"新 crate 必须先决定它属于哪一层。"
            )
            continue
        for extra in sorted(normal - ALLOWED[name]):
            problems.append(f"{name} -> {extra}: 未登记的产物依赖")
        for extra in sorted(dev - ALLOWED_DEV.get(name, set())):
            problems.append(f"{name} -> {extra}: 未登记的测试依赖（dev-dependency）")

    for stale in sorted(set(ALLOWED) - seen):
        problems.append(f"{stale}: 分层表里有，但 workspace 里已不存在")

    if problems:
        print("依赖方向检查失败：\n", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        print(
            "\n每一条依赖都要先在 tools/check-deps.py 里登记，并说明它为什么"
            "不违反不变量 E2。",
            file=sys.stderr,
        )
        return 1

    edges = sum(len(v) for v in ALLOWED.values())
    print(f"依赖方向正确：{len(seen)} 个 crate，{edges} 条产物依赖")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
