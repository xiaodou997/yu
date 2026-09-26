"""Pixel-oracle contracts only; these tests are not application acceptance."""
import importlib.util
from pathlib import Path
import unittest

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

    def test_unexpected_source_cannot_use_this_oracle(self):
        checks = FOLLOWUP.Checks(None, None, None, None, {})
        with self.assertRaisesRegex(AssertionError, 'Unexpected restored-preview fixture'):
            checks.restored_preview('# Arbitrary document\n')


if __name__ == '__main__':
    unittest.main()
