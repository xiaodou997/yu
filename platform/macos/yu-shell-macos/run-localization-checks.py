#!/usr/bin/env python3
"""Validate shipped macOS localization resources and source-key coverage."""

from __future__ import annotations

import json
import plistlib
import re
import subprocess
import sys
from pathlib import Path


LANGUAGES = ("en", "zh-Hans", "zh-Hant", "ja", "ko")


def fail(message: str) -> None:
    raise SystemExit(f"localization-check: {message}")


def load_strings(path: Path) -> dict[str, str]:
    if not path.is_file():
        fail(f"missing {path}")
    result = subprocess.run(
        ["plutil", "-convert", "json", "-o", "-", str(path)],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        fail(f"invalid strings file {path}: {result.stderr.strip()}")
    value = json.loads(result.stdout)
    if not isinstance(value, dict) or not all(
        isinstance(key, str) and isinstance(text, str) for key, text in value.items()
    ):
        fail(f"{path} is not a string dictionary")
    return value


def main() -> None:
    shell_dir = Path(__file__).resolve().parent
    app = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else shell_dir / ".build/Yu.app"
    source_resources = shell_dir / "AppBundle/Resources"
    bundled_resources = app / "Contents/Resources"

    source_tables = {
        language: load_strings(source_resources / f"{language}.lproj/Localizable.strings")
        for language in LANGUAGES
    }
    base_keys = set(source_tables["en"])
    if not base_keys:
        fail("English Localizable.strings is empty")
    for language, table in source_tables.items():
        missing = sorted(base_keys - set(table))
        extra = sorted(set(table) - base_keys)
        if missing or extra:
            fail(f"{language} key mismatch: missing={missing[:8]} extra={extra[:8]}")

    han = re.compile(r"[\u4e00-\u9fff]")
    english_han = sorted(key for key, value in source_tables["en"].items() if han.search(value))
    if english_han:
        fail(f"English translations contain Han text: {english_han[:8]}")

    placeholders = re.compile(r"%(?:\d+\$)?(?:@|d|ld|lld|u|lu|llu|f|\.\d+f)")
    for key in sorted(base_keys):
        expected = placeholders.findall(source_tables["en"][key])
        for language in LANGUAGES[1:]:
            actual = placeholders.findall(source_tables[language][key])
            if actual != expected:
                fail(
                    f"{language} format placeholders differ for {key!r}: "
                    f"expected={expected} actual={actual}"
                )

    info_tables = {
        language: load_strings(source_resources / f"{language}.lproj/InfoPlist.strings")
        for language in LANGUAGES
    }
    info_keys = set(info_tables["en"])
    for language, table in info_tables.items():
        if set(table) != info_keys:
            fail(f"{language} InfoPlist.strings key mismatch")

    with (app / "Contents/Info.plist").open("rb") as handle:
        info = plistlib.load(handle)
    if info.get("CFBundleDevelopmentRegion") != "en":
        fail("CFBundleDevelopmentRegion must be en")
    declared = set(info.get("CFBundleLocalizations", []))
    if not set(LANGUAGES).issubset(declared):
        fail(f"Info.plist localizations missing: {sorted(set(LANGUAGES) - declared)}")

    for language in LANGUAGES:
        bundled = load_strings(bundled_resources / f"{language}.lproj/Localizable.strings")
        if bundled != source_tables[language]:
            fail(f"{language} bundled Localizable.strings differs from source")
        bundled_info = load_strings(bundled_resources / f"{language}.lproj/InfoPlist.strings")
        if bundled_info != info_tables[language]:
            fail(f"{language} bundled InfoPlist.strings differs from source")

    # Every literal localization key used by Swift must exist in the English
    # source table. Dynamic keys are deliberately not supported.
    key_pattern = re.compile(r'L10n\.(?:tr|format)\(\s*"((?:\\.|[^"])*)"')
    used: set[str] = set()
    for source in (shell_dir / "Sources/Yu").glob("*.swift"):
        text = source.read_text(encoding="utf-8")
        used.update(json.loads(f'"{match}"') for match in key_pattern.findall(text))
    missing_used = sorted(used - base_keys)
    if missing_used:
        fail(f"Swift uses localization keys missing from English table: {missing_used[:12]}")

    print(
        f"localization-check: OK ({len(base_keys)} keys, "
        + ", ".join(LANGUAGES)
        + ")"
    )


if __name__ == "__main__":
    main()
