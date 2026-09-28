"""Group 4 fixed-input suite for the global acceptance runner.

Only Group 4 fixtures, assertions and per-case attribution belong here.
"""

from __future__ import annotations

from collections import Counter, defaultdict
import json
from pathlib import Path
import re
import sys

from acceptance_runner import RunContext, ROOT, sha, write_json

LIST_TEST = ROOT / "crates/yu-editor/tests/html_list_cross_level.rs"
LIST_COMMAND = [
    "cargo", "test", "--locked", "-p", "yu-editor",
    "--test", "html_list_cross_level", "--", "--nocapture", "--test-threads=1",
]
TABLE_COMMAND = [
    "cargo", "test", "--locked", "-p", "yu-editor",
    "--test", "html_paste_atomicity", "--test", "html_table_math",
    "--test", "html_table_footnotes", "--test", "html_table_row_groups",
    "--test", "html_table_edit",
]
TABLE_FIXED_COMMAND = [
    "cargo", "test", "--locked", "-p", "yu-editor",
    "--test", "group4_fixed_table_acceptance", "--", "--ignored", "--nocapture", "--test-threads=1",
]
CASE_DECLARATION = re.compile(
    r'case!\(\s*([a-z_][a-z_0-9]*),\s*"([^"]+\.case)"', re.MULTILINE
)
TEST_START = re.compile(r"^test ([a-z_][a-z_0-9]*) \.\.\. ?(.*)$")
VARIANT = re.compile(
    r'cell=(true|false), ending="(\\r\\n|\\n)", bom=(true|false), '
    r'reverse=(true|false), primary=(\d+)'
)
TABLE_PASS = re.compile(r"^G4CASE table/([^ ]+) PASS$")


def case_declarations(source: str) -> dict[str, str]:
    declarations = CASE_DECLARATION.findall(source)
    mapping = {fixture: test for test, fixture in declarations}
    if len(mapping) != 13 or len({test for test, _ in declarations}) != 13:
        raise ValueError("Expected 13 unique list fixture/test declarations")
    return mapping


def executed_variants(log: str) -> tuple[dict[str, Counter], set[str]]:
    variants: dict[str, Counter] = defaultdict(Counter)
    finished: set[str] = set()
    active: str | None = None
    for line in log.splitlines():
        start = TEST_START.match(line)
        if start:
            active = start.group(1)
            fragment = start.group(2)
        else:
            fragment = line
        if active is None:
            continue
        found = VARIANT.search(fragment)
        if found:
            cell, ending, bom, reverse, primary = found.groups()
            variants[active][(
                cell == "true", "crlf" if ending == r"\r\n" else "lf",
                bom == "true", reverse == "true", int(primary),
            )] += 1
        if fragment.strip() == "ok" or fragment.strip() == "FAILED":
            if fragment.strip() == "ok":
                finished.add(active)
            active = None
    return variants, finished


def verify_list_case(case: dict, mapping: dict[str, str], variants: dict[str, Counter],
                     finished: set[str]) -> tuple[bool, str, list[dict]]:
    test = mapping.get(case["fixture"])
    if test is None or test not in finished:
        return False, "fixture test did not finish successfully", []
    location = case["location"]
    cell = location == "cell"
    if location not in ("cell", "standalone", "document"):
        return False, f"unsupported location {location}", []
    n = len(case["before_selections"])
    primary_values = (0, n - 1)
    required = Counter(
        (cell, case["line_ending"], case["bom"], reverse, primary)
        for reverse in (False, True) for primary in primary_values
    )
    observed = variants[test]
    missing = required - observed
    if missing:
        return False, f"missing exact variant loops: {dict(missing)}", []
    dimensions = [
        {"reverse": reverse, "primary_index": primary,
         "observed_count": observed[(cell, case["line_ending"], case["bom"], reverse, primary)]}
        for reverse in (False, True) for primary in sorted(set(primary_values))
    ]
    return True, test, dimensions


def read_manifest(path: Path, expected: int) -> tuple[dict, str]:
    raw = path.read_bytes()
    manifest = json.loads(raw)
    cases = manifest["cases"]
    if len(cases) != expected or len({case["id"] for case in cases}) != expected:
        raise ValueError(f"{path}: expected {expected} unique cases")
    return manifest, sha(raw)


def verify_files(manifest: dict, root: Path) -> int:
    count = 0
    for case in manifest["cases"]:
        directory = root / case["id"]
        for name, digest in case["sha256"].items():
            if sha((directory / name).read_bytes()) != digest:
                raise ValueError(f"Input hash mismatch: {directory / name}")
            count += 1
    return count


def verify_fixture_sources(table: dict, lists: dict, table_root: Path) -> int:
    """Bind generated manifests to the exact source files used by core tests."""
    count = 0
    table_fixtures = ROOT / "crates/yu-editor/tests/fixtures/group4-paste"
    for name, digest in table["fixture_sha256"].items():
        path = table_fixtures / f"{name}.md"
        if sha(path.read_bytes()) != digest:
            raise ValueError(f"Table source fixture hash mismatch: {path}")
        count += 1
    payloads = {case["payload"] for case in table["cases"]}
    for name in payloads:
        path = table_root / name
        source = table_fixtures / name
        expected = source.read_bytes().decode("utf-8").replace("\r\n", "\n").encode("utf-8")
        if path.read_bytes() != expected:
            raise ValueError(f"Generated payload differs from source fixture: {path}")
        count += 1
    list_fixtures = ROOT / "crates/yu-editor/tests/fixtures/group4-list"
    for case in lists["cases"]:
        path = list_fixtures / case["fixture"]
        if sha(path.read_bytes()) != case["fixture_sha256"]:
            raise ValueError(f"List source fixture hash mismatch: {path}")
        count += 1
    return count


def table_case_tsv(manifest: dict, root: Path, path: Path) -> None:
    rows = []
    for case in manifest["cases"]:
        directory = root / case["id"]
        fields = [
            case["id"], str(directory / "input.md"),
            str(directory / "expected-a.md"), str(directory / "expected-b.md"),
            str(root / case["payload"]), case["expected_paste_result"],
            "rectangle" if case["id"].startswith("row-groups-") else "text",
            "|".join(case["expected_inline_math_tex"] or []),
            ",".join(map(str, case["expected_footnote_numbers"] or [])),
        ]
        if any("\t" in field or "\n" in field for field in fields):
            raise ValueError(f"Cannot encode case {case['id']} in TSV")
        rows.append("\t".join(fields))
    path.write_text("\n".join(rows) + "\n", encoding="utf-8")


def table_passed_ids(log: str) -> set[str]:
    ids = [match.group(1) for line in log.splitlines()
           if (match := TABLE_PASS.fullmatch(line))]
    if len(ids) != len(set(ids)):
        raise ValueError("Duplicate table PASS marker")
    return set(ids)


def run_suite(ctx: RunContext) -> int:
    out = ctx.output
    for name, command in (
        ("prepare-table", [sys.executable, str(ROOT / "tools/prepare-group4-paste-checks.py"),
                           str(out / "paste-inputs")]),
        ("prepare-list", [sys.executable, str(ROOT / "tools/prepare-group4-list-checks.py"),
                          str(out / "list-inputs")]),
    ):
        if (code := ctx.run(name, command)) != 0:
            return code
    table, table_hash = read_manifest(out / "paste-inputs/manifest.json", 76)
    lists, list_hash = read_manifest(out / "list-inputs/manifest.json", 96)
    if table.get("schema_version") != 4 or sum(
        c["expected_paste_result"] == "accept" for c in table["cases"]
    ) != 44:
        raise ValueError("Table v4 accept/reject scope changed")
    file_counts = {"table": verify_files(table, out / "paste-inputs"),
                   "list": verify_files(lists, out / "list-inputs")}
    file_counts["source_fixtures_and_payloads"] = verify_fixture_sources(
        table, lists, out / "paste-inputs"
    )
    table_tsv = out / "table-cases.tsv"
    table_case_tsv(table, out / "paste-inputs", table_tsv)
    for name, command in (("list-core", LIST_COMMAND), ("table-core", TABLE_COMMAND)):
        ctx.run(name, command)
    ctx.run("table-fixed-core", TABLE_FIXED_COMMAND,
            env_overrides={"GROUP4_TABLE_CASES": str(table_tsv)})
    ctx.commands["table-fixed-core"]["case_tsv_sha256"] = sha(table_tsv.read_bytes())
    ctx.save_commands()

    mapping = case_declarations(LIST_TEST.read_text())
    variants, finished = executed_variants(ctx.log("list-core").read_text())
    table_ids = table_passed_ids(ctx.log("table-fixed-core").read_text())
    expected_table_ids = {case["id"] for case in table["cases"]}
    if not table_ids <= expected_table_ids:
        raise ValueError(f"Unknown table PASS IDs: {table_ids - expected_table_ids}")
    list_covered = 0
    table_covered = 0
    for kind, manifest in (("table", table), ("list", lists)):
        for case in manifest["cases"]:
            case_id = f"{kind}/{case['id']}"
            ctx.ledger.add(case_id, ["core", "real_window", "cold_reopen"],
                           metadata=case, unobserved_fields=[
                               "per_case_real_window_result", "per_case_cold_reopen"])
            if kind == "list" and ctx.commands["list-core"]["exit_code"] == 0:
                covered, detail, dimensions = verify_list_case(
                    case, mapping, variants, finished
                )
                if covered:
                    list_covered += 1
                    ctx.ledger.record(case_id, "core", "passed", "list-core",
                                      evidence={"test_name": detail,
                                                "dimensions": dimensions})
                else:
                    ctx.ledger.rows[case_id]["notes"] = detail
            elif kind == "table":
                if (ctx.commands["table-fixed-core"]["exit_code"] == 0
                        and case["id"] in table_ids):
                    table_covered += 1
                    ctx.ledger.record(case_id, "core", "passed", "table-fixed-core",
                                      evidence={"test_name":
                                                "generated_table_inputs_have_per_id_core_results",
                                                "case_tsv_sha256": sha(table_tsv.read_bytes()),
                                                "selection_kind": "rectangle"
                                                if case["id"].startswith("row-groups-")
                                                else "backward_text"})
                else:
                    ctx.ledger.rows[case_id]["notes"] = (
                        "Per-ID table core contract did not finish successfully")
    rows = ctx.ledger.finish()
    report = {"schema": 2, "suite": ctx.suite,
              "scope": "fixed inputs; core and real window are separate",
              "manifests": {"table": table_hash, "list": list_hash},
              "verified_file_hash_counts": file_counts, "commands": ctx.commands,
              "cases": rows, "summary": {"table_total": 76, "list_total": 96,
                                     "list_core_passed": list_covered,
                                     "table_core_passed": table_covered,
                                     "full_case_passed": sum(
                                         row["status"] == "passed" for row in rows)}}
    write_json(out / "reports/fixed-cases.json", report)
    print(f"List core variants: {list_covered}/96; table core variants: {table_covered}/76")
    return 0 if (all(item["exit_code"] == 0 for item in ctx.commands.values())
                 and list_covered == 96 and table_covered == 76) else 1
