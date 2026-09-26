"""Harness-only tests; these do not run the application or mark UI checks passed."""
import importlib.util
from pathlib import Path
import unittest

MODULE = Path(__file__).resolve().parents[1] / 'platform/macos/yu-shell-macos/group4_followup.py'
spec = importlib.util.spec_from_file_location('group4_followup_checks', MODULE)
followup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(followup)


class FollowupHarnessTests(unittest.TestCase):
    def test_utf16_uses_code_units_not_python_characters(self):
        self.assertEqual(followup.utf16('父项🙂\r\n'), 6)
        self.assertEqual(followup.utf16(''), 0)

    def test_center_preserves_screen_coordinates(self):
        self.assertEqual(followup.center({'x': -30, 'y': 40, 'width': 10, 'height': 20}), (-25, 50))

    def test_legacy_flags_remain_under_existing_validation(self):
        self.assertIsNone(followup.option_error({'output': Path('unused'), 'math_suite': True}))

    def test_table_modes_may_combine_with_dark_and_reopen(self):
        self.assertIsNone(followup.option_error({'output': Path('unused'), 'table_interactions': True, 'table_resize': True, 'dark': True, 'reopen': True}))

    def test_followup_modes_cannot_silently_override_each_other(self):
        for first in ['table_resize', 'smoke_document', 'list_gestures']:
            with self.subTest(first=first):
                self.assertIsNotNone(followup.option_error({first: True, 'stress_seconds': 30}))

    def test_followup_rejects_legacy_combinations(self):
        for legacy in ['html_blocks', 'math_suite', 'diagram_suite', 'promotion_suite', 'lifecycle', 'cancel_helper', 'ime', 'list_inputs']:
            with self.subTest(legacy=legacy):
                self.assertIsNotNone(followup.option_error({'smoke_document': True, legacy: True}))

    def test_stress_duration_is_bounded_before_side_effects(self):
        for seconds in [-1, 1, 29, 1801]:
            self.assertIsNotNone(followup.option_error({'stress_seconds': seconds}))
        for seconds in [0, 30, 300, 1800]:
            self.assertIsNone(followup.option_error({'stress_seconds': seconds}))

    def test_pure_validation_does_not_touch_output(self):
        target = Path('this-path-is-never-created-by-option-validation')
        existed = target.exists()
        followup.option_error({'output': target, 'smoke_document': True, 'table_resize': True})
        self.assertEqual(target.exists(), existed)


if __name__ == '__main__':
    unittest.main()
