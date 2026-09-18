#!/usr/bin/env python3
"""Native restoration/AX checks and measured inactive CPU/physical footprint.

These checks do not certify actual VoiceOver speech, IME interaction or an OS
logout/relaunch. CPU percent is relative to one core; memory is phys_footprint.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--case", choices=("all", "restoration", "idle"), default="all")
    parser.add_argument("--idle-kib", type=int, nargs="+", choices=(100, 1024), default=[100, 1024],
                        help="Sizes to sample when idle checks are requested")
    args = parser.parse_args()
    app = HERE / ".build/Yu.app/Contents/MacOS/Yu"
    build = json.loads((HERE / ".build/build-manifest.json").read_text())
    if build["app_sha256"] != digest(app) or build["configuration"] != "release":
        parser.error("A matching audited release build is required")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    fixtures = output / "fixtures"
    fixtures.mkdir()
    restoration = "# 窗口恢复检查\n\n恢复选区 English 🙂 é.\n\n" + "".join(
        f"## Section {i}\n\n恢复阅读位置和中英文混排。This paragraph stays unchanged after reopening.\n\n"
        for i in range(150)
    )
    (fixtures / "restoration.md").write_text(restoration)
    unit = "中文长文写作 English text and reliable recovery. No images or formulas.\n\n".encode()
    for kib in (100, 1024):
        data = unit * ((kib * 1024) // len(unit))
        (fixtures / f"plain-{kib}KiB.md").write_bytes(data + b" " * (kib * 1024 - len(data)))
    environment = {"build": build, "fixtures": {p.name: digest(p) for p in fixtures.iterdir()},
                   "hardware": subprocess.check_output(["sysctl", "-n", "hw.model", "machdep.cpu.brand_string"], text=True).strip(),
                   "memory_metric": "task_vm_info.phys_footprint", "cpu_metric": "getrusage process CPU / monotonic wall time, one core"}
    (output / "environment.json").write_text(json.dumps(environment, ensure_ascii=False, indent=2))
    cases = []
    if args.case in ("all", "restoration"):
        cases += [("restoration-light", ["--window-state-self-check"], "restoration.md", 90),
                  ("restoration-dark", ["--window-state-self-check", "--dark-mode-self-check"], "restoration.md", 90)]
    if args.case in ("all", "idle"):
        cases += [(f"idle-{kib}KiB", ["--idle-resource-self-check"], f"plain-{kib}KiB.md", 190) for kib in dict.fromkeys(args.idle_kib)]
    results = {}
    env = {k: v for k, v in os.environ.items() if not k.startswith("YU_")}
    for name, flags, fixture, timeout in cases:
        print(f"Running {name}…", flush=True)
        log = output / f"{name}.log"
        with log.open("w") as stream:
            try:
                code = subprocess.run([str(app), *flags, str(fixtures / fixture)],
                                      stdout=stream, stderr=subprocess.STDOUT, env=env, timeout=timeout).returncode
            except subprocess.TimeoutExpired:
                code = 124
        text = log.read_text(errors="replace")
        result = {"exit_code": code, "passed": False, "log_sha256": digest(log)}
        if name.startswith("restoration"):
            result["passed"] = code == 0 and "Yu window state self-check passed:" in text
        else:
            lines = [line.split(": ", 1)[1] for line in text.splitlines() if line.startswith("Yu idle resource measurement: ")]
            if lines:
                measurement = json.loads(lines[-1])
                limit = 150 if "100KiB" in name else 250
                stable = max(measurement["physical_footprint_bytes"][-3:]) / 1024**2
                result.update({"measurement": measurement, "stable_footprint_mib": stable, "memory_target_mib": limit})
                result["passed"] = (code == 0 and measurement["cpu_target_passed"] and stable <= limit
                                    and measurement.get("remained_hidden") is True
                                    and measurement["wall_seconds"] >= 59.5
                                    and measurement["start_frame_serial"] == measurement["end_frame_serial"])
        results[name] = result
        (output / "results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2))
        print(f"{name}: {'PASS' if result['passed'] else 'FAIL'}", flush=True)
        if not result["passed"]:
            print("\n".join(text.splitlines()[-5:]), flush=True)
    if digest(app) != build["app_sha256"]:
        raise RuntimeError("Executable changed during verification")
    return 0 if all(case["passed"] for case in results.values()) else 1


if __name__ == "__main__":
    raise SystemExit(main())
