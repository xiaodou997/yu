"""Reusable acceptance execution and evidence ledger for any Yu feature suite.

Suite modules own their fixtures and assertions. This module owns isolated
outputs, serial command execution, evidence provenance, and case status.
"""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time


ROOT = Path(__file__).resolve().parents[1]
STEP_NAME = re.compile(r"[a-z][a-z0-9-]*\Z")
LAYERS = {"core", "native", "real_window", "cold_reopen", "system_event", "visual"}


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


class RunContext:
    def __init__(self, output: Path, suite: str):
        self.output = output.resolve()
        self.output.mkdir(parents=True, exist_ok=False)
        (self.output / "logs").mkdir()
        (self.output / "reports").mkdir()
        self.suite = suite
        self.commands: dict[str, dict] = {}
        self.ledger = CaseLedger(self)

    def run(self, name: str, command: list[str], *, env_overrides: dict[str, str] | None = None,
            cwd: Path = ROOT) -> int:
        if not STEP_NAME.fullmatch(name) or name in self.commands:
            raise ValueError(f"Invalid or duplicate acceptance step: {name}")
        if not command or not all(isinstance(part, str) for part in command):
            raise ValueError(f"Step {name} needs an argv list")
        log = self.output / "logs" / f"{name}.log"
        started = time.monotonic()
        with log.open("x", encoding="utf-8") as output:
            try:
                result = subprocess.run(command, cwd=cwd,
                                        env={**os.environ, **(env_overrides or {})},
                                        stdout=output, stderr=subprocess.STDOUT,
                                        check=False)
                code = result.returncode
            except OSError as error:
                output.write(f"Could not start step: {error}\n")
                code = 127
        self.commands[name] = {
            "command": command, "cwd": str(cwd), "exit_code": code,
            "log": str(log), "log_sha256": sha(log.read_bytes()),
            "duration_seconds": round(time.monotonic() - started, 3),
            "environment_override_keys": sorted((env_overrides or {}).keys()),
        }
        self.save_commands()
        return code

    def log(self, name: str) -> Path:
        return Path(self.commands[name]["log"])

    def artifact(self, path: Path) -> dict:
        """Bind a screenshot, result JSON, or other suite output to its bytes."""
        resolved = path.resolve(strict=True)
        if not resolved.is_relative_to(self.output) or not resolved.is_file():
            raise ValueError(f"Evidence artifact must be a file inside {self.output}: {path}")
        return {"path": str(resolved), "sha256": sha(resolved.read_bytes())}

    def save_commands(self) -> None:
        write_json(self.output / "reports" / "commands.json", self.commands)


class CaseLedger:
    """Compute status from required evidence layers instead of suite claims."""

    def __init__(self, context: RunContext):
        self.context = context
        self.rows: dict[str, dict] = {}

    def add(self, case_id: str, required_layers: list[str], *, metadata: dict | None = None,
            unobserved_fields: list[str] | None = None) -> None:
        if not case_id or case_id in self.rows:
            raise ValueError(f"Missing or duplicate case ID: {case_id}")
        if not required_layers or len(required_layers) != len(set(required_layers)):
            raise ValueError(f"Invalid required layers for {case_id}")
        if set(required_layers) - LAYERS:
            raise ValueError(f"Unknown evidence layers for {case_id}")
        self.rows[case_id] = {
            "id": case_id, "required_layers": required_layers,
            "fixture_manifest_case": metadata, "status": "not_run",
            "evidence_layer_results": {}, "runs": [],
            "unobserved_fields": unobserved_fields or [],
        }

    def record(self, case_id: str, layer: str, result: str, command_name: str,
               *, evidence: dict | None = None) -> None:
        row = self.rows[case_id]
        command = self.context.commands[command_name]
        if layer not in row["required_layers"] or layer in row["evidence_layer_results"]:
            raise ValueError(f"Invalid or duplicate layer {layer} for {case_id}")
        if result not in ("passed", "failed", "blocked"):
            raise ValueError(f"Invalid evidence result: {result}")
        if result == "passed" and command["exit_code"] != 0:
            raise ValueError(f"Cannot pass {case_id}: command {command_name} failed")
        reserved = {"evidence_origin", "evidence_layer", "result", "command_name",
                    "log", "log_sha256"}
        if reserved.intersection(evidence or {}):
            raise ValueError(f"Evidence for {case_id} overrides provenance fields")
        row["evidence_layer_results"][layer] = result
        row["runs"].append({
            "evidence_origin": "new_run", "evidence_layer": layer,
            "result": result, "command_name": command_name,
            "log": command["log"], "log_sha256": command["log_sha256"],
            **(evidence or {}),
        })

    def finish(self) -> list[dict]:
        for row in self.rows.values():
            observed = row["evidence_layer_results"]
            if "failed" in observed.values():
                row["status"] = "failed"
            elif all(observed.get(layer) == "passed" for layer in row["required_layers"]):
                row["status"] = "passed"
            elif "passed" in observed.values():
                row["status"] = "partial"
            elif "blocked" in observed.values():
                row["status"] = "blocked"
        return list(self.rows.values())
