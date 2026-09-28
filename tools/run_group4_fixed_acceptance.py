#!/usr/bin/env python3
"""Compatibility shortcut for the global Group 4 acceptance suite."""

import argparse
from pathlib import Path

from run_acceptance import main


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new evidence directory")
    args = parser.parse_args()
    raise SystemExit(main(["group4-fixed", str(args.output)]))
