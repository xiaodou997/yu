#!/usr/bin/env python3
"""头文件声明的每个函数，在**当前这个平台**上都必须真的有符号。

存在的理由是一次真实的、门禁全绿的谎话：S7 第七刀之前，
`yu_storage_session_macos_task_checkbox_hit_test` 与
`yu_storage_session_macos_table_resize_at_point` 这两个函数整个挂在
`#[cfg(target_os = "macos")]` 下面，而 `include/yu_storage_ffi.h` 无条件声明
它们。于是在 Windows 或 Linux 上链接这个 staticlib 会 unresolved symbol，而
`tools/check-ffi-header.py`、`cargo test --workspace`、CI 的三平台矩阵**全部
是绿的**——因为从来没有人在非 macOS 上*链接*过它，只*编译*过。

`check-ffi-header.py` 看不出来，是因为它对**源码文本**做正则，正则不认识
cfg。所以这条门禁换一个判据：读**编译产物**。判断由 rustc 与归档器做出，
不由这里的字符串匹配做出。

# 它覆盖的是「当前这个平台」，不是三个平台

这台机器上交叉编译不了（tree-sitter 的 grammar 是 C，走 `cc`，交叉需要目标
平台的 C 编译器；本地 `cargo build --target x86_64-unknown-linux-gnu` 会以
`failed to find tool "x86_64-linux-gnu-gcc"` 失败）。所以本地跑它只证明
macOS 这一半。**三个平台的覆盖来自 CI 的矩阵**——`.github/workflows/ci.yml`
的 rust job 本来就在 macos / ubuntu / windows 上各跑一遍。

这就是为什么 `check-ffi-header.py` 里还有一条**便携的**规则（`pub extern "C"
fn` 不得挂在任何 cfg 下）：那一条在 macOS 开发机上立刻就红，这一条在 CI 上
把它兜住。两条判据的机制不同——一条读源码的属性，一条读产物的符号表。

用法: tools/check-ffi-symbols.py
"""

from __future__ import annotations

import json
import os
import platform
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HEADER = ROOT / "crates/yu-storage-ffi/include/yu_storage_ffi.h"
CRATE = "yu-storage-ffi"


def declared_functions() -> set[str]:
    header = HEADER.read_text(encoding="utf-8")
    return set(
        re.findall(r"\b(?:int32_t|void|size_t|uint64_t) (yu_[a-z_0-9]+)\(", header)
    )


def build_staticlib() -> Path:
    """构建并返回 staticlib 的路径。

    路径问 cargo 要，不自己拼 `target/debug/...`——`CARGO_TARGET_DIR` 与
    `--target` 都会让拼出来的那个不存在，而「文件不存在」会被误读成
    「符号不存在」。
    """
    process = subprocess.run(
        ["cargo", "build", "-p", CRATE, "--message-format=json-render-diagnostics"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if process.returncode != 0:
        print(process.stderr, file=sys.stderr)
        raise SystemExit(f"cargo build -p {CRATE} 失败")
    artifacts: list[str] = []
    for line in process.stdout.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if message.get("reason") != "compiler-artifact":
            continue
        # 按 kind + package_id 认。两条都必要：
        # - cargo 报的 target 名是 lib 名（`yu_storage_ffi`，下划线），不是
        #   包名，按包名比会一个都匹配不到，而那看起来像「符号全没了」；
        # - 依赖里还有别的 staticlib（tree-sitter-highlight），只按 kind 会
        #   挑到别人的产物。
        if "staticlib" not in (message.get("target", {}).get("kind") or []):
            continue
        if CRATE not in message.get("package_id", ""):
            continue
        artifacts.extend(message.get("filenames") or [])
    libraries = [Path(name) for name in artifacts if Path(name).suffix in (".a", ".lib")]
    if not libraries:
        raise SystemExit(
            f"cargo 没有报告 {CRATE} 的 staticlib 产物；"
            "crate-type 还是 [\"staticlib\"] 吗？"
        )
    return libraries[0]


def symbol_reader() -> list[str]:
    """`nm` 的命令前缀。找不到就失败，不降级。

    一条会自己跳过的门禁等于没有门禁——那正是这个脚本要修的失败模式。
    """
    sysroot = subprocess.run(
        ["rustc", "--print", "sysroot"], capture_output=True, text=True, check=True
    ).stdout.strip()
    host = subprocess.run(
        ["rustc", "-vV"], capture_output=True, text=True, check=True
    ).stdout
    # `.strip()` 不能省：Windows 上 `rustc -vV` 的行尾是 CRLF，`.+` 会把 `\r`
    # 一起吃进 triple，拼出来的路径找不到 llvm-nm，于是在**唯一没有 `nm` 的
    # 平台上**退到「找不到工具」硬失败。
    triple = re.search(r"^host: (.+)$", host, re.M).group(1).strip()
    llvm_nm = Path(sysroot) / "lib/rustlib" / triple / "bin" / (
        "llvm-nm.exe" if platform.system() == "Windows" else "llvm-nm"
    )
    if llvm_nm.is_file():
        return [str(llvm_nm)]
    system_nm = shutil.which("nm")
    if system_nm is not None:
        return [system_nm]
    raise SystemExit(
        "找不到 nm，也找不到 rustup 的 llvm-nm。\n"
        "CI 上装 llvm-tools component；本机装 Xcode CLT 或 binutils。\n"
        "这条门禁不会因为读不到符号表就放行。"
    )


def defined_symbols(library: Path) -> set[str]:
    """产物里**已定义的全局**符号。

    `nm` 的类型字母大写表示全局；代码符号是 `T`。Mach-O 给 C 符号加下划线
    前缀，ELF 与 COFF/x86-64 不加，所以两种都收进来。
    """
    # `-g` 是 --extern-only 的短形式；Xcode 的 nm 认短的，GNU 与 llvm 两种都认。
    output = subprocess.run(
        [*symbol_reader(), "-g", "--defined-only", str(library)],
        capture_output=True,
        text=True,
    )
    # **退出码不当致命。** rustc 把 std / compiler_builtins 的目标文件一并归档
    # 进 staticlib，而 Xcode 的 nm 读不动它们（"Unknown attribute kind"，
    # rustc 的 LLVM 比 Xcode 的新）。那些目标文件里没有 `yu_*` 符号，读不到
    # 不影响这条判断；而万一连**我们**那个目标文件也读不到，比对会少符号从而
    # **变红**——方向是安全的：读不出来只会假红，不会假绿。
    if output.returncode != 0 and not output.stdout.strip():
        print(output.stderr, file=sys.stderr)
        raise SystemExit(f"读取 {library} 的符号表失败：一个符号都没读到")
    found: set[str] = set()
    for line in output.stdout.splitlines():
        parts = line.split()
        if len(parts) < 2 or len(parts[-2]) != 1 or not parts[-2].isupper():
            continue
        name = parts[-1]
        found.add(name.removeprefix("_"))
    return found


def main() -> int:
    declared = declared_functions()
    if not declared:
        print("头文件里一个函数声明都没找到，正则该改了", file=sys.stderr)
        return 1
    library = build_staticlib()
    exported = defined_symbols(library)
    missing = sorted(declared - exported)
    if missing:
        print(
            f"头文件声明了这些函数，但 {platform.system()} 上的产物里没有它们的符号：\n",
            file=sys.stderr,
        )
        for name in missing:
            print(f"  {name}", file=sys.stderr)
        print(
            f"\n产物：{library}\n"
            "在这个平台上链接这个 staticlib 会 unresolved symbol。头文件是无条件的，"
            "实现也必须是无条件的——平台差异写在函数体里（早退一个状态码），"
            "不写在函数上。",
            file=sys.stderr,
        )
        return 1
    print(
        f"头文件的 {len(declared)} 个函数在 {platform.system()} 产物里都有符号"
        f"（{os.path.basename(library)}）"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
