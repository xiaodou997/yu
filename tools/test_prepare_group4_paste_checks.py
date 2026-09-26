"""Tests for fixture preparation only; not substitutes for Rust or native tests."""
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("prepare-group4-paste-checks.py")
SPEC = importlib.util.spec_from_file_location("group4_paste_fixtures", SCRIPT)
PREPARE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREPARE)


class PreparePasteChecksTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.output = self.root / "checks"

    def test_all_cases_have_exact_history_bytes_hashes_and_utf16_ranges(self):
        manifest = PREPARE.prepare(self.output)
        self.assertEqual(manifest["schema_version"], 4)
        self.assertEqual(len(manifest["cases"]), 76)
        self.assertEqual(sum(c["expected_paste_result"] == "reject" for c in manifest["cases"]), 32)
        for case in manifest["cases"]:
            with self.subTest(case=case["id"]):
                directory = self.output / case["id"]
                self.assertTrue((self.output / case["payload"]).is_file())
                initial = (directory / "expected-0.md").read_bytes()
                first = (directory / "expected-a.md").read_bytes()
                second = (directory / "expected-b.md").read_bytes()
                self.assertEqual((directory / "input.md").read_bytes(), initial)
                self.assertEqual(first, initial + PREPARE.HISTORY_A.encode("utf-8"))
                self.assertEqual(second, first + PREPARE.HISTORY_B.encode("utf-8"))
                self.assertEqual(initial.startswith(b"\xef\xbb\xbf"), case["bom"])
                canonical = initial.decode("utf-8-sig")
                if case["newline"] == "crlf":
                    self.assertIn("\r\n", canonical)
                    self.assertNotIn("\n", canonical.replace("\r\n", ""))
                    self.assertNotIn("\r", canonical.replace("\r\n", ""))
                else:
                    self.assertNotIn("\r", canonical)
                utf16 = canonical.encode("utf-16-le")
                for field, label in (("target_utf16", "目标中文🙂"),
                                     ("rectangle_end_utf16", "矩形终点🙂")):
                    start = case[field]["location"] * 2
                    end = start + case[field]["length"] * 2
                    self.assertEqual(utf16[start:end].decode("utf-16-le"), label)
                for filename, expected in case["sha256"].items():
                    self.assertEqual(hashlib.sha256((directory / filename).read_bytes()).hexdigest(), expected)
        self.assertEqual(json.loads((self.output / "manifest.json").read_text(encoding="utf-8")), manifest)

    def test_math_expectations_evolve_without_weakening_unsupported_rejections(self):
        manifest = PREPARE.prepare(self.output)
        for case in manifest["cases"]:
            with self.subTest(case=case["id"]):
                if case["id"].startswith("math-target-"):
                    self.assertEqual(case["expected_paste_result"], "accept")
                    self.assertEqual(case["expected_inline_math_tex"], ["x^2"])
                elif case["id"].startswith("math-mixed-target-"):
                    self.assertEqual(case["expected_paste_result"], "accept")
                    self.assertEqual(case["expected_inline_math_tex"], ["h", "x^2", "a+b", "z"])
                elif case["id"].startswith(("math-invalid-target-", "cross-")):
                    self.assertEqual(case["expected_paste_result"], "reject")
        self.assertIn("arbitrary multiple selections still reject", manifest["selection_scope"])

    def test_footnote_expectations_keep_labels_and_rejection_controls(self):
        manifest = PREPARE.prepare(self.output)
        for case in manifest["cases"]:
            with self.subTest(case=case["id"]):
                if case["id"].startswith("footnote-target-"):
                    self.assertEqual(case["expected_paste_result"], "accept")
                    self.assertEqual(case["expected_footnote_numbers"], [1])
                elif case["id"].startswith("footnote-mixed-target-"):
                    self.assertEqual(case["expected_paste_result"], "accept")
                    self.assertEqual(case["expected_footnote_numbers"], [1, 2, 2, 3, 2])
                    self.assertEqual(case["expected_inline_math_tex"], ["x^2"])
                elif case["id"].startswith(("footnote-missing-", "footnote-duplicate-", "footnote-invalid-")):
                    self.assertEqual(case["expected_paste_result"], "reject")

    def test_preparation_never_claims_native_or_visual_pass(self):
        manifest = PREPARE.prepare(self.output)
        self.assertEqual(manifest["native_test_status"], "not_run")
        self.assertTrue(manifest["visual_review_required"])
        self.assertTrue(all(c["status"] == "not_run" for c in manifest["cases"]))

    def test_row_group_samples_pair_legal_and_rejected_native_payloads(self):
        manifest = PREPARE.prepare(self.output)
        cases = [c for c in manifest["cases"] if c["id"].startswith("row-groups-")]
        self.assertEqual(len(cases), 24)
        for case in cases:
            with self.subTest(case=case["id"]):
                rejected = case["id"].startswith(("row-groups-span-conflict-", "row-groups-whole-reject-"))
                self.assertEqual(case["expected_paste_result"], "reject" if rejected else "accept")
                self.assertTrue(case["preserve_target_row_groups"])
                self.assertEqual(case["required_selection"], "whole_table" if "-whole-" in case["id"] else "target_or_rectangle")
                payload = (self.output / case["payload"]).read_bytes()
                stem = Path(case["payload"]).stem
                self.assertEqual(hashlib.sha256(payload).hexdigest(), manifest["fixture_sha256"][stem])
                canonical = (self.output / case["input"]).read_bytes().decode("utf-8-sig")
                self.assertIn("id='body'", canonical)
                if case["id"].startswith("row-groups-body-body-"):
                    self.assertNotIn("<thead", canonical)
                    self.assertEqual(canonical.count("<tbody"), 2)
                elif case["id"].startswith("row-groups-body-foot-"):
                    self.assertIn("<tfoot", canonical)
                if "-whole-" in case["id"]:
                    self.assertIn("<tbody id='empty'></tbody>", canonical)

    def test_existing_output_is_not_modified(self):
        self.output.mkdir()
        sentinel = self.output / "keep.txt"
        sentinel.write_bytes(b"existing evidence")
        with self.assertRaises(FileExistsError):
            PREPARE.prepare(self.output)
        self.assertEqual(sentinel.read_bytes(), b"existing evidence")
        self.assertEqual(list(self.output.iterdir()), [sentinel])

    def test_missing_fixture_leaves_no_output(self):
        with self.assertRaises(FileNotFoundError):
            PREPARE.prepare(self.output, self.root / "missing")
        self.assertFalse(self.output.exists())

    def test_invalid_utf8_leaves_no_output(self):
        fixtures = self.root / "fixtures"
        shutil.copytree(PREPARE.FIXTURES, fixtures)
        (fixtures / "math-target.md").write_bytes(b"\xff")
        with self.assertRaises(UnicodeDecodeError):
            PREPARE.prepare(self.output, fixtures)
        self.assertFalse(self.output.exists())

    def test_duplicate_label_leaves_no_output(self):
        fixtures = self.root / "fixtures"
        shutil.copytree(PREPARE.FIXTURES, fixtures)
        with (fixtures / "math-target.md").open("a", encoding="utf-8") as stream:
            stream.write("目标中文🙂")
        with self.assertRaises(ValueError):
            PREPARE.prepare(self.output, fixtures)
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
