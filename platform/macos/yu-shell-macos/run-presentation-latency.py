#!/usr/bin/env python3
"""Measure scripted editor/zoom actions and a retained-frame presentation control.

Redraw validates the control only; it has no editing-performance threshold.
This is not physical keyboard, IME, or trackpad end-to-end acceptance.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--case", choices=("all", "input", "zoom", "redraw"), default="all")
    parser.add_argument("--trace", action="store_true", help="Diagnostic timing logs; not a clean benchmark")
    args = parser.parse_args()
    binary = HERE / ".build/Yu.app/Contents/MacOS/Yu"
    build = json.loads((HERE / ".build/build-manifest.json").read_text())
    if build["configuration"] != "release" or digest(binary) != build["app_sha256"]:
        parser.error("A matching audited release build is required")
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    cases = [("input", 100, "--presentation-latency-self-check", 32),
             ("zoom", 1024, "--zoom-latency-self-check", 100),
             ("redraw", 100, "--redraw-latency-self-check", None)]
    env = {k: v for k, v in os.environ.items() if not k.startswith("YU_")}
    if args.trace:
        env["YU_RENDER_TIMING"] = "1"
    environment = {"build": build, "trace": args.trace,
                   "hardware": subprocess.check_output(["sysctl", "-n", "hw.model", "machdep.cpu.brand_string"], text=True).strip(),
                   "clock": "CACurrentMediaTime to MTLDrawable.presentedTime, current source/geometry validated by Rust",
                   "percentile": "nearest rank", "fixtures": {}}
    results = {}
    for name, kib, flag, target in cases:
        if args.case not in ("all", name):
            continue
        prefix = "Latency anchor: 原生输入\n\n".encode()
        unit = "中文长文写作 English text and reliable recovery. No images or formulas.\n\n".encode()
        data = prefix + unit * ((kib * 1024 - len(prefix)) // len(unit))
        fixture = out / f"{name}-{kib}KiB.md"
        fixture.write_bytes(data + b" " * (kib * 1024 - len(data)))
        environment["fixtures"][fixture.name] = digest(fixture)
        (out / "environment.json").write_text(json.dumps(environment, ensure_ascii=False, indent=2))
        print(f"Running {name} {kib}KiB…", flush=True)
        log = out / f"{name}.log"
        result_path = out / f"{name}-measurement.json"
        env["YU_LATENCY_RESULT_PATH"] = str(result_path)
        with log.open("w") as stream:
            try:
                code = subprocess.run([str(binary), flag, str(fixture)], env=env,
                                      stdout=stream, stderr=subprocess.STDOUT, timeout=240).returncode
            except subprocess.TimeoutExpired:
                code = 124
        result = {"exit_code": code, "passed": False, "log_sha256": digest(log)}
        if result_path.exists():
            measurement = json.loads(result_path.read_text())
            result["measurement_sha256"] = digest(result_path)
            samples = measurement["samples"]
            summaries = {}
            for kind in sorted({s["kind"] for s in samples}):
                values = sorted(s["presented_ms"] for s in samples if s["kind"] == kind)
                summaries[kind] = {"samples": len(values),
                                   "p50_ms": values[math.ceil(len(values) * .5) - 1],
                                   "p95_ms": values[math.ceil(len(values) * .95) - 1],
                                   "max_ms": values[-1]}
            valid = (all(s["active_before"] and s["active_after"] and s["presented_ms"] >= 0 for s in samples)
                     and all(v["samples"] == 24 for v in summaries.values())
                     and set(summaries) == ({"insert", "undo"} if name == "input" else {name})
                     and measurement["source_and_save_correct"]
                     and digest(fixture) == environment["fixtures"][fixture.name])
            result.update({"measurement": measurement, "summaries": summaries,
                           "valid": valid, "target_p95_ms": target,
                           "control_only": name == "redraw"})
            result["passed"] = code == 0 and valid and not args.trace and (target is None or all(v["p95_ms"] <= target for v in summaries.values()))
        results[name] = result
        (out / "results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2))
        print(name, ("VALID CONTROL" if name == "redraw" else "PASS") if result["passed"] else "FAIL", result.get("summaries", {}), flush=True)
    if digest(binary) != build["app_sha256"]:
        raise RuntimeError("Executable changed during measurement")
    return 0 if all(r["passed"] for r in results.values()) else 1


if __name__ == "__main__":
    raise SystemExit(main())
