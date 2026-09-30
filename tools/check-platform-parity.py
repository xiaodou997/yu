#!/usr/bin/env python3
"""Keep both product shells on the build/verification surface.

This is the admission gate for the second platform. It intentionally checks the
shell/build boundary rather than pretending Group 2 already has TSF/UIA/D3D
feature parity. Deeper self-check parity is added as those Windows groups land.
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CI = ROOT / ".github/workflows/ci.yml"
VERIFY = ROOT / "tools/verify.sh"
WORKSPACE = ROOT / "Cargo.toml"

SHELLS = {
    "macos": {
        "dir": ROOT / "platform/macos/yu-shell-macos",
        "job": "  macos-shell:",
        "ci_command": "platform/macos/yu-shell-macos/run-self-checks.sh --build",
        "workspace_member": None,
    },
    "windows": {
        "dir": ROOT / "platform/windows/yu-shell-windows",
        "job": "  windows-shell:",
        "ci_command": "./platform/windows/yu-shell-windows/run-self-checks.ps1",
        "workspace_member": '"platform/windows/yu-shell-windows"',
    },
}


def main() -> int:
    ci = CI.read_text(encoding="utf-8")
    verify = VERIFY.read_text(encoding="utf-8")
    workspace = WORKSPACE.read_text(encoding="utf-8")
    failures: list[str] = []

    for platform, spec in SHELLS.items():
        directory = spec["dir"]
        if not directory.is_dir():
            failures.append(f"{platform}: missing shell directory {directory.relative_to(ROOT)}")
        if spec["job"] not in ci:
            failures.append(f"{platform}: missing dedicated CI shell job")
        command = spec["ci_command"]
        if command not in ci:
            failures.append(f"{platform}: CI job does not run {command}")
        if command not in verify:
            failures.append(f"{platform}: verify.sh does not acknowledge {command}")
        member = spec["workspace_member"]
        if member is not None and member not in workspace:
            failures.append(f"{platform}: shell is not a Cargo workspace member")

    windows_script = ROOT / "platform/windows/yu-shell-windows/run-self-checks.ps1"
    if windows_script.is_file():
        script = windows_script.read_text(encoding="utf-8")
        for required in [
            "cargo test -p yu-font-windows -p yu-render-windows -p yu-shell-windows",
            "cargo check -p yu-shell-windows",
            "cargo run -p yu-shell-windows -- --window-self-check",
        ]:
            if required not in script:
                failures.append(f"windows: self-check missing {required}")

    if failures:
        print("平台壳门禁失败：", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1

    print("平台壳门禁通过：macOS / Windows 都有独立产品壳与 CI 验证入口")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
