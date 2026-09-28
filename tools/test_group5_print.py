import unittest
from acceptance_suites.group5_print import report_passed

class PrintingSuiteTests(unittest.TestCase):
    def report(self):
        return {'passed': True, 'physical_jobs_submitted': 0, 'cases': [{'id': name, 'passed': True} for name in ['whole', 'range', 'landscape', 'source-alias', 'image-alias', 'write-denied', 'target-change', 'publication-cancel']]}
    def test_complete_named_native_paths(self):
        self.assertTrue(report_passed(0, self.report()))
    def test_missing_failure_or_physical_submission_never_passes(self):
        report = self.report(); report['cases'].pop(); self.assertFalse(report_passed(0, report))
        report = self.report(); report['physical_jobs_submitted'] = 1; self.assertFalse(report_passed(0, report))
        self.assertFalse(report_passed(1, self.report()))
    def test_duplicate_case_is_not_full_coverage(self):
        report = self.report(); report['cases'][-1] = report['cases'][0]; self.assertFalse(report_passed(0, report))

    def test_resource_phase_does_not_shadow_footprint_samples(self):
        from acceptance_suites.group5_print_window import PrintChecks
        check = PrintChecks.__new__(PrintChecks)
        check.resources = []
        self.assertTrue(callable(check.resource_paths))

if __name__ == '__main__': unittest.main()
