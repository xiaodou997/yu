import unittest
from acceptance_suites.group5_png import CASES, report_passed
class PNGSuiteTests(unittest.TestCase):
    def report(self): return {'passed': True, 'cases': [{'id': i, 'passed': True} for i in sorted(CASES)]}
    def test_exact_native_coverage(self): self.assertTrue(report_passed(0, self.report()))
    def test_failures_and_missing_cases(self):
        r=self.report();r['cases'].pop();self.assertFalse(report_passed(0,r))
        self.assertFalse(report_passed(1,self.report()))
    def test_duplicate_does_not_replace_missing(self):
        r=self.report();r['cases'][-1]=r['cases'][0];self.assertFalse(report_passed(0,r))
if __name__ == '__main__': unittest.main()
