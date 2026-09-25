import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('prepare_lists', Path(__file__).with_name('prepare-group4-list-checks.py'))
prepare_lists = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prepare_lists)


class PreparationTests(unittest.TestCase):
    def test_exact_bytes_and_endpoint_positions(self):
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp) / 'prepared'
            result = prepare_lists.prepare(output)
            self.assertEqual(len(result['cases']), 96)
            self.assertEqual(result['product_verdict'], 'not_run')
            for case in result['cases']:
                self.assertEqual(case['status'], 'not_run')
                folder = output / case['id']
                for filename, checksum in case['sha256'].items():
                    raw = (folder / filename).read_bytes()
                    self.assertEqual(prepare_lists.sha(raw), checksum)
                    self.assertEqual(raw.startswith(b'\xef\xbb\xbf'), case['bom'])
                    if case['line_ending'] == 'crlf':
                        self.assertNotIn(b'\n', raw.replace(b'\r\n', b''))
                    else:
                        self.assertNotIn(b'\r', raw)
                text = (folder / 'input.md').read_bytes().decode('utf-8-sig')
                self.assertNotIn('«', text)
                for point in case['before_selections']:
                    for key in ('anchor', 'focus'):
                        prefix = text.encode()[:point[key + '_utf8']].decode()
                        self.assertEqual(len(prefix.encode('utf-16-le')) // 2, point[key + '_utf16'])
                a = (folder / 'expected-a.md').read_bytes()
                b = (folder / 'expected-b.md').read_bytes()
                self.assertEqual(b, a + prepare_lists.B.encode())
                if not case['expected_changed']:
                    self.assertEqual(a, (folder / 'expected-list-a.md').read_bytes())
            self.assertEqual(result, json.loads((output / 'manifest.json').read_text(encoding='utf-8')))

    def test_crlf_checkout_of_fixtures_preserves_prepared_inputs(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            fixtures = root / 'fixtures'
            shutil.copytree(prepare_lists.FIXTURES, fixtures)
            for path in fixtures.glob('*.case'):
                path.write_bytes(path.read_bytes().replace(b'\n', b'\r\n'))
            first = prepare_lists.prepare(root / 'lf')
            second = prepare_lists.prepare(root / 'crlf', fixtures)
            self.assertEqual(len(first['cases']), len(second['cases']))
            for a, b in zip(first['cases'], second['cases']):
                self.assertEqual(a['sha256'], b['sha256'])
                self.assertEqual(a['before_selections'], b['before_selections'])
                self.assertEqual(a['after_selections'], b['after_selections'])

    def test_repeated_text_uses_distinct_markers(self):
        source, pairs = prepare_lists.marked('«A0»同文«F0»--«F1»同文«A1»')
        self.assertEqual(source, '同文--同文')
        self.assertEqual(pairs[0]['anchor_utf8'], 0)
        self.assertEqual(pairs[1]['anchor_utf8'], len(source.encode()))
        self.assertGreater(pairs[1]['focus_utf8'], pairs[0]['focus_utf8'])

    def test_invalid_markers_are_rejected(self):
        for text in ('none', '«A0»missing', '«A0»a«F0»«A0»', '«A1»gap«F1»', '«X0»wrong'):
            with self.subTest(text=text), self.assertRaises(ValueError):
                prepare_lists.marked(text)

    def test_existing_output_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'output'
            path.mkdir()
            (path / 'keep').write_bytes(b'original')
            with self.assertRaises(FileExistsError):
                prepare_lists.prepare(path)
            self.assertEqual((path / 'keep').read_bytes(), b'original')
            self.assertEqual(len(list(path.iterdir())), 1)

    def test_damaged_fixture_does_not_create_output(self):
        with tempfile.TemporaryDirectory() as temp:
            fixtures = Path(temp) / 'fixtures'
            shutil.copytree(prepare_lists.FIXTURES, fixtures)
            (fixtures / (prepare_lists.NAMES[0] + '.case')).write_text('damaged', encoding='utf-8')
            output = Path(temp) / 'output'
            with self.assertRaises(ValueError):
                prepare_lists.prepare(output, fixtures)
            self.assertFalse(output.exists())

    def test_rejection_cannot_claim_changed_expected_source(self):
        with tempfile.TemporaryDirectory() as temp:
            fixtures = Path(temp) / 'fixtures'
            shutil.copytree(prepare_lists.FIXTURES, fixtures)
            path = fixtures / 'cross-cells.reject-outdent.case'
            path.write_text(path.read_text(encoding='utf-8') + 'changed', encoding='utf-8')
            output = Path(temp) / 'output'
            with self.assertRaises(ValueError):
                prepare_lists.prepare(output, fixtures)
            self.assertFalse(output.exists())


if __name__ == '__main__':
    unittest.main()
