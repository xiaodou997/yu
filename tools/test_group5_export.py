#!/usr/bin/env python3
"""Validate suite registration and conservative scope; no product-pass claims."""
from pathlib import Path
import importlib.util
import unittest
from acceptance_suites import group5_export

class Group5SuiteTests(unittest.TestCase):
    def test_all_24_groups_are_unique(self):
        self.assertEqual(len(group5_export.GROUPS), 24)
        self.assertEqual(len(set(group5_export.GROUPS)), 24)
    def test_core_names_exist_in_rust_tests(self):
        source = (group5_export.ROOT / 'crates/yu-export/tests/document_export.rs').read_text()
        for name in group5_export.CORE_CASES:
            self.assertIn('fn ' + name + '(', source)
    def test_artifact_parser_distinguishes_escaped_script(self):
        spec = importlib.util.spec_from_file_location('group5_html_check', Path(__file__).with_name('check-group5-html.py'))
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        safe = module.HTMLInspection(); safe.feed('<pre>&lt;script&gt;alert(1)&lt;/script&gt;</pre>')
        self.assertEqual(safe.errors, [])
        unsafe = module.HTMLInspection(); unsafe.feed('<script>alert(1)</script>')
        self.assertTrue(unsafe.errors)
if __name__ == '__main__': unittest.main()
