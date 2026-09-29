#!/usr/bin/env python3
"""Generate the native renderer's locked third-party license inventory.

This is an engineering distribution audit, not a legal opinion. It records the
actual aarch64-apple-darwin Cargo closure, requires a declared license expression
for every external package, and preserves every license/COPYING/NOTICE-like
file shipped in the published package. Packages whose published crate contains
no root license file remain explicit in the manifest instead of being hidden.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess


LICENSE_TOKENS = ("license", "copying", "notice", "unlicense")


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def cargo_metadata(repo: Path) -> dict:
    cargo = shutil.which("cargo")
    if not cargo:
        raise RuntimeError("cargo is required to audit the locked helper closure")
    return json.loads(subprocess.check_output(
        [cargo, "metadata", "--format-version", "1", "--locked",
         "--filter-platform", "aarch64-apple-darwin"],
        cwd=repo, text=True))


def closure(metadata: dict) -> list[tuple[dict, dict]]:
    packages = {item["id"]: item for item in metadata["packages"]}
    nodes = {item["id"]: item for item in metadata["resolve"]["nodes"]}
    root = next(
        item for item in metadata["packages"]
        if item["name"] == "yu-document-renderer"
        and item["manifest_path"].endswith("tools/yu-document-renderer/Cargo.toml")
    )
    seen: set[str] = set()
    pending = [root["id"]]
    while pending:
        package_id = pending.pop()
        if package_id in seen:
            continue
        seen.add(package_id)
        pending.extend(dep["pkg"] for dep in nodes[package_id]["deps"])
    return sorted(
        ((packages[package_id], nodes[package_id])
         for package_id in seen if packages[package_id].get("source")),
        key=lambda pair: (pair[0]["name"], pair[0]["version"], pair[0]["id"]),
    )


def license_files(package: dict) -> list[Path]:
    root = Path(package["manifest_path"]).parent
    found: set[Path] = set()
    declared = package.get("license_file")
    if declared and (root / declared).is_file():
        found.add(root / declared)
    for path in root.iterdir():
        if path.is_file() and any(token in path.name.lower() for token in LICENSE_TOKENS):
            found.add(path)
    return sorted(found, key=lambda path: path.name)


def generate(repo: Path, output: Path) -> dict:
    metadata = cargo_metadata(repo)
    rows = []
    sections = []
    for package, node in closure(metadata):
        expression = package.get("license")
        if not expression:
            raise RuntimeError(
                f"External package has no declared license expression: "
                f"{package['name']} {package['version']}"
            )
        files = license_files(package)
        file_rows = []
        for path in files:
            digest = sha256(path)
            file_rows.append({
                "name": path.name,
                "sha256": digest,
                "bytes": path.stat().st_size,
            })
            sections.append(
                "\n".join([
                    "=" * 78,
                    f"Package: {package['name']} {package['version']}",
                    f"Cargo license expression: {expression}",
                    f"Cargo source: {package['source']}",
                    f"Published file: {path.name}",
                    f"SHA256: {digest}",
                    "=" * 78,
                    "",
                    path.read_text(encoding="utf-8", errors="replace").rstrip(),
                    "",
                ])
            )
        rows.append({
            "name": package["name"],
            "version": package["version"],
            "license": expression,
            "source": package["source"],
            "repository": package.get("repository"),
            "homepage": package.get("homepage"),
            "features": sorted(node.get("features", [])),
            "license_files": file_rows,
            "published_root_license_file_missing": not file_rows,
        })

    output.mkdir(parents=True, exist_ok=True)
    licenses_text = (
        "Yu native renderer third-party license texts\n"
        "Generated from the locked aarch64-apple-darwin Cargo closure.\n"
        "A package with no root license file remains listed in RustDependencies.json.\n\n"
        + "\n".join(sections)
    )
    licenses_path = output / "RustDependencyLicenses.txt"
    licenses_path.write_text(licenses_text, encoding="utf-8")

    lock = repo / "Cargo.lock"
    manifest = {
        "schema_version": 1,
        "root_package": "yu-document-renderer",
        "target": "aarch64-apple-darwin",
        "cargo_lock_sha256": sha256(lock),
        "external_package_count": len(rows),
        "packages_without_published_root_license_file": sum(
            row["published_root_license_file_missing"] for row in rows
        ),
        "license_text_bundle": {
            "file": licenses_path.name,
            "sha256": sha256(licenses_path),
            "bytes": licenses_path.stat().st_size,
        },
        "packages": rows,
        "scope": (
            "Locked Cargo dependency closure and published package notices; "
            "engineering distribution inventory, not a legal opinion."
        ),
    }
    (output / "RustDependencies.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parent.parent
    manifest = generate(repo, args.output.resolve())
    print(json.dumps({
        "external_package_count": manifest["external_package_count"],
        "packages_without_published_root_license_file":
            manifest["packages_without_published_root_license_file"],
        "license_text_bundle": manifest["license_text_bundle"],
    }, ensure_ascii=False))


if __name__ == "__main__":
    main()
