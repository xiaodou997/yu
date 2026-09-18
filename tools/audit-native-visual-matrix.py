#!/usr/bin/env python3
"""Audit captured evidence without turning screenshot existence into parity.

Requires provenance emitted by Yu's real-window capture protocol. Real 1x
captures occupy their own cases; resampling a 2x PNG cannot fill those cases.
Optional references are candidates only until their capture conditions and
geometry review have been independently verified.
"""
import argparse
import hashlib
import itertools
import json
import pathlib
import struct

SIZES = [(900, 620), (1200, 800), (1600, 1000)]
POSITIONS = {"top": 0.0, "middle": 0.5, "bottom": 1.0}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def png_size(path):
    with path.open("rb") as stream:
        header = stream.read(24)
    if len(header) != 24 or header[:8] != b"\x89PNG\r\n\x1a\n" or header[12:16] != b"IHDR":
        raise ValueError(f"Not a PNG with IHDR: {path}")
    return struct.unpack(">II", header[16:24])


def audit(captures, app, fixture, reference_cases=None):
    app_hash, source_hash = sha(app), sha(fixture)
    fonts_directory = app.parent.parent / "Resources" / "Fonts"
    font_hashes = {path.name: sha(path) for path in fonts_directory.iterdir()
                   if path.suffix.lower() in (".ttf", ".otf")}
    actual = {}
    errors = []
    for manifest_path in sorted(captures.rglob("manifest.json")):
        try:
            manifest = json.loads(manifest_path.read_text())
            if manifest.get("schema_version") != 2:
                raise ValueError("missing v2 provenance")
            if manifest.get("app_sha256") != app_hash:
                raise ValueError("different application binary")
            if manifest.get("source_sha256") != source_hash:
                raise ValueError("different source document")
            if not font_hashes or manifest.get("font_resources_sha256") != font_hashes:
                raise ValueError("bundled font fingerprints differ from current app")
            for case in manifest["cases"]:
                width, height, scale = (case[k] for k in ("window_width", "window_height", "scale"))
                fraction = case["scroll_fraction"]
                position = next((k for k, v in POSITIONS.items() if fraction == v), None)
                if (width, height) not in SIZES or scale not in (1, 2) or position is None:
                    raise ValueError("capture outside fixed matrix")
                theme = case["theme"]
                if theme not in ("github", "night"):
                    raise ValueError("unknown theme")
                sidebar = "on" if case["sidebar_width"] > 0 else "off"
                name = f"{int(width)}x{int(height)}-{theme}-sidebar-{sidebar}-{int(scale)}x"
                if case["case"] != name:
                    raise ValueError("case name does not match measured geometry")
                path = manifest_path.parent / (name + ".png")
                if sha(path) != case["png_sha256"]:
                    raise ValueError(f"PNG fingerprint mismatch: {name}")
                pixels = png_size(path)
                if pixels != (width * scale, height * scale) or pixels != (case["pixel_width"], case["pixel_height"]):
                    raise ValueError(f"unscaled pixel dimensions mismatch: {name}")
                target = max(0, case["document_height"] - case["clip_height"]) * fraction
                if abs(case["scroll_y"] - target) > 1:
                    raise ValueError(f"scroll position mismatch: {name}")
                if abs(case["surface_top"] + case["clip_height"] - height) > 1:
                    raise ValueError(f"viewport does not reach window bottom: {name}")
                if abs(case["font_size_pt"] - 16) > 0.01:
                    raise ValueError(f"not the fixed 16pt font size: {name}")
                if not all(case.get(k) for k in ("declared_body_font", "declared_heading_font", "declared_code_font", "display_id")):
                    raise ValueError(f"missing font/display identity: {name}")
                key = (int(width), int(height), theme, sidebar, position, int(scale))
                if key in actual:
                    raise ValueError(f"duplicate matrix case: {key}")
                actual[key] = {"image": str(path.resolve()), "manifest": str(manifest_path.resolve()),
                               "png_sha256": case["png_sha256"], "scroll_y": case["scroll_y"]}
        except (ValueError, KeyError, OSError, TypeError) as error:
            errors.append({"manifest": str(manifest_path), "reason": str(error)})
    references = {}
    for case in reference_cases or []:
        key = (case["width"], case["height"], case["theme"], case["sidebar"], case["position"], case["scale"])
        path = pathlib.Path(case["image"])
        try:
            metadata = json.loads(path.with_suffix(".json").read_text())
            if metadata.get("source_sha256") != source_hash or metadata.get("version") != "1.10.8":
                raise ValueError("reference fixture/version mismatch")
            if metadata.get("scale") != case["scale"]:
                raise ValueError("reference scale metadata mismatch")
            if png_size(path) != (case["width"] * case["scale"], case["height"] * case["scale"]):
                raise ValueError("reference pixel dimensions mismatch")
            references[key] = {"image": str(path.resolve()), "png_sha256": sha(path),
                               "status": "candidate-needs-capture-condition-and-geometry-review"}
        except (ValueError, KeyError, OSError) as error:
            errors.append({"reference": str(path), "reason": str(error)})
    rows = []
    for (width, height), theme, sidebar, position, scale in itertools.product(
            SIZES, ("github", "night"), ("on", "off"), POSITIONS, (1, 2)):
        key = (width, height, theme, sidebar, position, scale)
        rows.append({"width": width, "height": height, "theme": theme, "sidebar": sidebar,
                     "position": position, "scale": scale, "actual": actual.get(key),
                     "reference": references.get(key), "visual_parity_passed": False})
    return {"app_sha256": app_hash, "source_sha256": source_hash, "cases": rows, "errors": errors,
            "actual_verified": len(actual), "required_cases": len(rows),
            "reference_candidates": len(references), "visual_parity_passed": False,
            "note": "Provenance and geometry consistency do not certify reference equivalence, IME, pointer interaction, or performance."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--captures", type=pathlib.Path, required=True)
    parser.add_argument("--app-binary", type=pathlib.Path, required=True)
    parser.add_argument("--fixture", type=pathlib.Path, required=True)
    parser.add_argument("--references", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    references = json.loads(args.references.read_text()) if args.references else None
    report = audit(args.captures, args.app_binary, args.fixture, references)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ("actual_verified", "required_cases", "reference_candidates", "errors", "visual_parity_passed")}, indent=2))
    # Incomplete coverage is reported explicitly; corrupt supplied evidence fails.
    raise SystemExit(1 if report["errors"] else 0)


if __name__ == "__main__":
    main()
