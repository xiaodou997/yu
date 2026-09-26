#!/usr/bin/env python3
"""Prepare isolated native-test inputs; this does not run or certify Yu tests."""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "crates/yu-editor/tests/fixtures/group4-paste"
HISTORY_A = " HISTORY-A-中文🙂"
HISTORY_B = " HISTORY-B-中文🙂"
NAMES = ("cross-groups", "math-target", "footnote-target", "merged-payload", "math-mixed-target", "footnote-mixed-target")


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def utf16_range(text: str, label: str) -> dict:
    start = text.index(label)
    return {
        "location": len(text[:start].encode("utf-16-le")) // 2,
        "length": len(label.encode("utf-16-le")) // 2,
    }


def prepare(output: Path, fixtures: Path = FIXTURES) -> dict:
    # Validate all inputs before creating anything. Byte reads make hashes and
    # BOM handling independent of the operating system's text newline mode.
    raw = {name: (fixtures / f"{name}.md").read_bytes() for name in NAMES}
    text = {name: data.decode("utf-8").replace("\r\n", "\n") for name, data in raw.items()}
    for name, value in text.items():
        if "\r" in value or value.startswith("\ufeff"):
            raise ValueError(f"Canonical fixture must have no BOM or bare CR: {name}")
    cases = [
        ("cross-groups", text["cross-groups"], "reject"),
        ("math-target", text["math-target"], "accept"),
        ("footnote-target", text["footnote-target"], "accept"),
        ("footnote-mixed-target", text["footnote-mixed-target"], "accept"),
        ("footnote-missing-target", text["footnote-target"].replace("[^note]", "[^missing]", 1), "reject"),
        ("footnote-duplicate-target", text["footnote-target"] + "\n[^NOTE]: duplicate\n", "reject"),
        ("footnote-invalid-target", text["footnote-target"].replace("[^note]",
         "<span data-yu-footnote='reference'><b>[^note]</b></span>", 1), "reject"),
        ("cross-head-body", text["cross-groups"].replace("<tbody>", "<thead>", 1)
         .replace("</tbody>", "</thead>", 1), "reject"),
        ("same-group-control", text["cross-groups"].replace("</tbody><tbody>", "", 1), "accept"),
        ("math-control", text["math-target"].replace("$x^2$", "**保留中文🙂**", 1), "accept"),
        ("footnote-control", text["footnote-target"].replace("[^note]", "**保留中文🙂**", 1), "accept"),
        ("math-mixed-target", text["math-mixed-target"], "accept"),
        ("math-invalid-target", text["math-target"].replace(
            "$x^2$", "<span data-math-style='inline'><em>x</em></span>", 1), "reject"),
    ]
    for _, source, _ in cases:
        for label in ("目标中文🙂", "矩形终点🙂"):
            if source.count(label) != 1:
                raise ValueError(f"Expected exactly one selection label: {label}")

    output = output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    (output / "merged-payload.md").write_bytes(text["merged-payload"].encode("utf-8"))
    (output / "legal-one-row-payload.md").write_bytes(
        "<table><tr><td colspan='2'>传入中文🙂</td></tr></table>\n".encode("utf-8")
    )
    manifest = {
        "schema_version": 3,
        "native_test_status": "not_run",
        "visual_review_required": True,
        "fixture_sha256": {name: digest(data) for name, data in raw.items()},
        "history_a": HISTORY_A,
        "history_b": HISTORY_B,
        "selection_offset_unit": "UTF-16 code units in canonical source, excluding file BOM",
        "selection_scope": "caret, forward/backward text selection, or table rectangle; arbitrary multiple selections still reject",
        "cases": [],
    }
    for name, source, expected in cases:
        for newline_name, newline in (("lf", "\n"), ("crlf", "\r\n")):
            for bom in (False, True):
                case_id = f"{name}-{newline_name}" + ("-bom" if bom else "")
                directory = output / case_id
                directory.mkdir()
                canonical = source.replace("\n", newline)
                prefix = b"\xef\xbb\xbf" if bom else b""
                states = {
                    "expected-0.md": prefix + canonical.encode("utf-8"),
                    "expected-a.md": prefix + (canonical + HISTORY_A).encode("utf-8"),
                    "expected-b.md": prefix + (canonical + HISTORY_A + HISTORY_B).encode("utf-8"),
                }
                for filename, data in states.items():
                    (directory / filename).write_bytes(data)
                (directory / "input.md").write_bytes(states["expected-0.md"])
                manifest["cases"].append({
                    "id": case_id,
                    "expected_paste_result": expected,
                    "expected_inline_math_tex": {
                        "math-target": ["x^2"],
                        "math-mixed-target": ["h", "x^2", "a+b", "z"],
                        "footnote-mixed-target": ["x^2"],
                    }.get(name),
                    "expected_footnote_numbers": {
                        "footnote-target": [1],
                        "footnote-mixed-target": [1, 2, 2, 3, 2],
                    }.get(name),
                    "newline": newline_name,
                    "bom": bom,
                    "input": f"{case_id}/input.md",
                    "sha256": {filename: digest(data) for filename, data in states.items()},
                    "target_utf16": utf16_range(canonical, "目标中文🙂"),
                    "rectangle_end_utf16": utf16_range(canonical, "矩形终点🙂"),
                    "status": "not_run",
                })
    (output / "manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="New isolated output directory; never overwritten")
    args = parser.parse_args()
    try:
        manifest = prepare(args.output)
    except (OSError, UnicodeError, ValueError) as error:
        parser.exit(1, f"Fixture preparation failed: {error}\n")
    print(f"Prepared {len(manifest['cases'])} inputs in {args.output}; native tests NOT RUN.")


if __name__ == "__main__":
    main()
