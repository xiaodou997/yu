#!/usr/bin/env python3
"""Run a registered Yu acceptance suite in a new evidence directory."""

from __future__ import annotations

import argparse
import importlib
from pathlib import Path
import re
import sys
import traceback

from acceptance_runner import RunContext, write_json


SUITE_NAME = re.compile(r"[a-z][a-z0-9-]*\Z")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("suite", help="registered suite name, for example group4-fixed")
    parser.add_argument("output", type=Path, help="new output directory; never overwritten")
    args = parser.parse_args(argv)
    if not SUITE_NAME.fullmatch(args.suite):
        parser.error("suite must use lowercase letters, digits and hyphens")
    try:
        module = importlib.import_module(
            f"acceptance_suites.{args.suite.replace('-', '_')}"
        )
    except ModuleNotFoundError as error:
        if error.name == f"acceptance_suites.{args.suite.replace('-', '_')}":
            parser.error(f"unknown acceptance suite: {args.suite}")
        raise
    context = RunContext(args.output, args.suite)
    try:
        code = module.run_suite(context)
        if not isinstance(code, int):
            raise TypeError("Suite run_suite(ctx) must return an integer exit code")
        return code
    except Exception as error:
        write_json(context.output / "reports" / "runner-error.json", {
            "suite": args.suite, "error": str(error),
            "traceback": traceback.format_exc(),
        })
        print(f"Acceptance suite {args.suite} failed: {error}", file=sys.stderr)
        return 1
    finally:
        context.save_commands()


if __name__ == "__main__":
    raise SystemExit(main())
