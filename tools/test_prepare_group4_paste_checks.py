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
        self.assertEqual(len(manifest["cases"]), 28)
        self.assertEqual(sum(c["expected_paste_result"] == "reject" for c in manifest["cases"]), 16)
        for case in manifest["cases"]:
            with self.subTest(case=case["id"]):
                directory = self.output / case["id"]
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

    def test_preparation_never_claims_native_or_visual_pass(self):
        manifest = PREPARE.prepare(self.output)
        self.assertEqual(manifest["native_test_status"], "not_run")
        self.assertTrue(manifest["visual_review_required"])
        self.assertTrue(all(c["status"] == "not_run" for c in manifest["cases"]))

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
