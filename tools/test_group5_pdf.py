"""Evidence predicate tests only, not PDF/GUI pass claims."""
import unittest
from acceptance_suites.group5_pdf import CASES, report_passed


class PDFEvidenceTests(unittest.TestCase):
    def report(self, text=''):
        return {'passed': True, 'source_identity_preserved': True, 'continued_edit_undo': True,
                'page_count': 2, 'pages': [{'text': text}]}

    def test_case_ids_are_fixed_and_unique(self):
        self.assertEqual(len(CASES), 6)
        self.assertEqual(len({name for name, _ in CASES}), 6)

    def test_failed_command_and_missing_identity_never_pass(self):
        value = self.report()
        self.assertFalse(report_passed('a4', 1, value))
        value.pop('source_identity_preserved')
        self.assertFalse(report_passed('a4', 0, value))

    def test_table_checks_all_bands_and_headers(self):
        text = 'HEADER-A TABLE-END ' + ' '.join(f'GROUP-{n:03} CELL-{n:03}-A CELL-{n:03}-B' for n in range(40))
        self.assertTrue(report_passed('table-pagination', 0, self.report(text)))
        self.assertFalse(report_passed('table-pagination', 0, self.report(text.replace('CELL-039-B', ''))))
        self.assertFalse(report_passed('table-pagination', 0, self.report(text.replace('HEADER-A', ''))))

    def test_code_missing_or_duplicate_line_is_not_complete(self):
        text = 'LONG-CODE-END ' + ' '.join(f'CODE-{n:03}' for n in range(140))
        self.assertTrue(report_passed('long-text-code', 0, self.report(text)))
        self.assertFalse(report_passed('long-text-code', 0, self.report(text + ' CODE-020')))
        self.assertFalse(report_passed('long-text-code', 0, self.report(text.replace('CODE-139', ''))))


if __name__ == '__main__': unittest.main()
