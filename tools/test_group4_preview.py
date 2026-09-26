"""Pixel-oracle contracts only; these tests are not application acceptance."""
import importlib.util
from pathlib import Path
import unittest
import tempfile

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    'group4_followup', ROOT/'platform/macos/yu-shell-macos/group4_followup.py')
FOLLOWUP = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FOLLOWUP)


class PreviewPixelTests(unittest.TestCase):
    def setUp(self):
        self.frame = dict(X=100, Y=200, Width=100, Height=100)
        self.rect = dict(x=110, y=220, width=30, height=20)

    def test_cold_case_requires_reopen_before_side_effects(self):
        self.assertIn('requires --reopen', FOLLOWUP.option_error({'cold_resource_reopen': True}))

    def test_cold_case_can_use_audit_and_theme(self):
        self.assertIsNone(FOLLOWUP.option_error(dict(cold_resource_reopen=True,
            reopen=True, resource_audit=True, dark=True)))

    def test_cold_case_cannot_silently_replace_stress(self):
        self.assertIn('Choose one', FOLLOWUP.option_error(dict(cold_resource_reopen=True,
            reopen=True, stress_seconds=30)))

    def test_blank_light_and_dark_do_not_pass(self):
        for value in (0, 32, 255):
            image = Image.new('RGB', (200, 200), (value,)*3)
            ink, box, patch = FOLLOWUP.preview_ink(image, self.rect, self.frame)
            self.assertEqual(ink, 0)
            self.assertEqual(box, (20, 40, 80, 80))
            self.assertEqual(patch.size, (60, 40))

    def test_contrast_is_counted_in_both_themes(self):
        for background, foreground in ((255, 0), (32, 240)):
            image = Image.new('RGB', (200, 200), (background,)*3)
            for x in range(30, 35):
                for y in range(50, 54):
                    image.putpixel((x, y), (foreground,)*3)
            self.assertEqual(FOLLOWUP.preview_ink(image, self.rect, self.frame)[0], 20)

    def test_title_pixels_outside_gap_are_not_counted(self):
        image = Image.new('RGB', (200, 200), 'white')
        image.paste('black', (0, 0, 200, 35))
        image.paste('black', (0, 90, 200, 200))
        self.assertEqual(FOLLOWUP.preview_ink(image, self.rect, self.frame)[0], 0)

    def test_offscreen_geometry_is_rejected_not_clipped(self):
        image = Image.new('RGB', (200, 200), 'white')
        with self.assertRaises(ValueError):
            FOLLOWUP.preview_ink(image, dict(self.rect, x=90), self.frame)

    def test_nonfinite_and_empty_geometry_is_rejected(self):
        image = Image.new('RGB', (200, 200), 'white')
        for rect in (dict(self.rect, x=float('nan')), dict(self.rect, height=0),
                     dict(self.rect, width=-1)):
            with self.assertRaises(ValueError):
                FOLLOWUP.preview_ink(image, rect, self.frame)
        with self.assertRaises(ValueError):
            FOLLOWUP.preview_ink(image, self.rect, dict(self.frame, Width=0))

    def test_launch_arguments_pin_both_themes_without_persisting_defaults(self):
        for dark, flag in ((False, '--light-mode'), (True, '--dark-mode')):
            self.assertEqual(FOLLOWUP.appearance_arguments(dark), [flag, '-Yu.readingTheme', '0'])
        for invalid in (None, 0, 'dark'):
            with self.assertRaises(ValueError):
                FOLLOWUP.appearance_arguments(invalid)

    def test_theme_interaction_suite_can_change_reading_theme(self):
        for dark, flag in ((False, '--light-mode'), (True, '--dark-mode')):
            self.assertEqual(FOLLOWUP.appearance_arguments(dark, pin_theme=False), [flag])
        with self.assertRaises(ValueError):
            FOLLOWUP.appearance_arguments(False, pin_theme='no')

    def test_appearance_is_measured_and_ambiguous_backgrounds_fail_closed(self):
        for level, expected in ((32, 'dark'), (128, 'indeterminate'), (250, 'light')):
            patch = Image.new('RGB', (20, 20), (level,) * 3)
            for dark in (False, True):
                result = FOLLOWUP.preview_appearance(patch, dark)
                self.assertEqual(result['observed'], expected)
                self.assertEqual(result['background_level'], level)
                self.assertEqual(result['matches'], expected == ('dark' if dark else 'light'))
        with self.assertRaises(ValueError):
            FOLLOWUP.preview_appearance(Image.new('RGB', (0, 0)), False)

    def exercise_oracle(self, background, dark):
        source = '# Stress restored\r\n\r\n$x^2$\r\n\r\nTAIL\r\n'
        result = {'checks': [], 'options': {'dark': dark}}
        calls = []
        image = Image.new('RGB', (400, 300), (background,) * 3)
        image.paste('white' if background < 128 else 'black', (28, 30, 33, 34))
        def run(*args):
            calls.append(args[0])
            if args[0] == 'capture':
                image.save(args[1] + '-1.png')
                return None
            self.assertEqual(args, ('snapshot',))
            return {'AXValue': source, 'windows': [{'kCGWindowLayer': 0, 'kCGWindowNumber': 1,
                    'kCGWindowBounds': dict(X=0, Y=0, Width=400, Height=300)}]}
        def bounds(location, length):
            return dict(x=24, y=10 if location == 2 else 60, width=30, height=10)
        with tempfile.TemporaryDirectory() as directory:
            checks = FOLLOWUP.Checks(run, bounds, Path(directory), None, result)
            if dark == (background < 128):
                checks.restored_preview(source)
                self.assertEqual(result['restored_preview_status'], 'visible')
            else:
                with self.assertRaisesRegex(AssertionError, 'requested light/dark'):
                    checks.restored_preview(source)
                self.assertEqual(result['restored_preview_status'], 'wrong_appearance')
                self.assertFalse(result['checks'])
            self.assertGreaterEqual(result['restored_preview'][0]['contrasting_pixels'], 12)
            self.assertTrue(set(calls) <= {'snapshot', 'capture'})
        return result

    def test_visible_formula_in_wrong_appearance_cannot_pass(self):
        self.exercise_oracle(32, False)
        self.exercise_oracle(250, True)

    def test_matching_appearance_and_formula_pass_without_extra_input(self):
        self.exercise_oracle(32, True)
        self.exercise_oracle(250, False)

    def test_unexpected_source_cannot_use_this_oracle(self):
        checks = FOLLOWUP.Checks(None, None, None, None, {})
        with self.assertRaisesRegex(AssertionError, 'Unexpected restored-preview fixture'):
            checks.restored_preview('# Arbitrary document\n')


if __name__ == '__main__':
    unittest.main()
