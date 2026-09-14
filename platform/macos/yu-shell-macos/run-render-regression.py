#!/usr/bin/env python3
"""Run scripted AppKit checks; never label generated scroll as trackpad acceptance."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import zlib

HERE = Path(__file__).resolve().parent


def fixtures(directory):
    directory.mkdir()
    paragraph = (
        "长文档滚动检查：中文 English العربية עברית，emoji 👨‍👩‍👧‍👦，组合字符 é。"
        "改变窗口宽度后，这段文字应当重新换行；前后段落的位置以同一份布局为准。 "
    )
    source = "# Long document regression\n\n"
    for index in range(350):
        source += f"## Section {index:04d}\n\n" + paragraph * 4 + "\n\n"
        if index % 25 == 0:
            source += "| Name | Value |\n| --- | --- |\n| 羽 | 123 |\n\n> 引用内容\n\n- [ ] 待办项目\n\n"
    source += "YU_END_OF_DOCUMENT\n"
    (directory / "long.md").write_text(source, encoding="utf-8")
    # A deterministic, small RGBA fixture; ImageIO still performs the decode.
    # 高度取 64：正文行高 = line_height × 1.6 ≈ 32pt，32px 高的图占位与就绪
    # 都落在同一行高里、几何不变，「图片就绪引起一次几何变化」这条断言无从
    # 判读；64px 高（两行）就绪后块高才真正变化。
    def chunk(kind, data):
        return struct.pack("!I", len(data)) + kind + data + struct.pack("!I", zlib.crc32(kind + data))
    width, height = 320, 64
    pixels = b"".join(b"\0" + bytes([30, 130, 220, 255]) * width for _ in range(height))
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack("!2I5B", width, height, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(pixels)) + chunk(b"IEND", b"")
    (directory / "latency.png").write_bytes(png)
    resources = """# Resource notification regression

![Delayed image](latency.png)

```math
x^2 + y^2 = z^2
```

```mermaid
graph TD
A --> B
```

""" + ("等待图片和 Math 时，滚动不得重新发布完整帧。\n\n" * 30)
    (directory / "resources.md").write_text(resources, encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new output directory (never overwritten)")
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--case", choices=("all", "long", "resources"), default="all")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    fixtures(output / "fixtures")
    if args.prepare_only:
        print(output / "fixtures")
        return 0
    binary = HERE / ".build/Yu.app/Contents/MacOS/Yu"
    if not binary.is_file():
        parser.error("Build Yu.app before running this check")
    environment = {"mode": "scripted-regression", "build": "debug", "binary": str(binary),
                   "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
    for key, command in {
        "macos": ["sw_vers"], "hardware": ["sysctl", "-n", "hw.model", "machdep.cpu.brand_string"],
        "xcode": ["xcodebuild", "-version"], "revision": ["git", "rev-parse", "HEAD"],
        "changes": ["git", "status", "--porcelain"],
    }.items():
        environment[key] = subprocess.run(command, cwd=HERE, text=True, capture_output=True, check=True).stdout.strip()
    environment["fixtures"] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                               for p in (output / "fixtures").iterdir()}
    (output / "environment.json").write_text(json.dumps(environment, ensure_ascii=False, indent=2))
    spec = importlib.util.spec_from_file_location("metrics", HERE / "summarize-render-metrics.py")
    metrics = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(metrics)
    results = {}
    for name, flag, markers, delays in [
        ("long", "--render-regression-self-check", ["long=true", "reopened=true", "ax_frame_resize=true", "hover_frame=true"], {}),
        ("resources", "--resource-latency-self-check", ["idle_completion=true", "resource=image delay_ms=14000", "resource=math delay_ms=16000"],
         {"YU_TEST_IMAGE_DELAY_MS": "14000", "YU_TEST_MATH_DELAY_MS": "16000"}),
    ]:
        if args.case not in ("all", name):
            continue
        env = {key: value for key, value in os.environ.items() if not key.startswith("YU_TEST_")}
        env.update({"YU_RENDER_TIMING": "1", **delays})
        print(f"Running {name} real-window regression…", flush=True)
        log_path = output / f"{name}.log"
        with log_path.open("w") as stream:
            try:
                run = subprocess.run([str(binary), flag, str(output / "fixtures" / f"{name}.md")],
                                     env=env, stdout=stream, stderr=subprocess.STDOUT, timeout=180)
                code = run.returncode
            except subprocess.TimeoutExpired:
                stream.write("\nRegression runner timed out after 180 seconds.\n")
                code = 124
        log = log_path.read_text(errors="replace")
        summary = metrics.summarize(log.splitlines())
        missing = [marker for marker in markers if marker not in log]
        retries = summary["counts"].get("resource_retry", 0)
        ax_queries = summary["counts"].get("ax_frame_query", 0)
        ax_frames = summary["counts"].get("ax_frame_geometry", 0)
        ax_fallbacks = summary["counts"].get("ax_layout_fallback", 0)
        hover_fallbacks = summary["counts"].get("hover_layout_fallback", 0)
        hover_queries = summary["counts"].get("hover_frame_query", 0)
        passed = (code == 0 and not missing and (name != "resources" or retries == 0)
                  and ax_queries > 0 and ax_frames > 0 and ax_fallbacks == 0
                  and hover_fallbacks == 0 and (name != "long" or hover_queries >= 100))
        summary.update({"exit_code": code, "passed": passed, "missing_markers": missing,
                        "mode": "scripted-regression", "delay_injection_ms": delays})
        (output / f"{name}-summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2))
        results[name] = passed
        print(f"{name}: {'PASS' if passed else 'FAIL'} — {output / (name + '.log')}", flush=True)
        if not passed:
            print("\n".join(log.splitlines()[-12:]), flush=True)
    (output / "results.json").write_text(json.dumps(results, indent=2))
    print("Scripted checks do not establish real trackpad p95 or visual/IME/VoiceOver acceptance.")
    return 0 if all(results.values()) else 1


if __name__ == "__main__":
    sys.exit(main())
