"""Guard the acceptance ledger against overstating executed cases."""

import importlib.util
from collections import Counter
from pathlib import Path
import sys
import unittest


sys.path.insert(0, str(Path(__file__).parent))
MODULE = Path(__file__).parent / "acceptance_suites/group4_fixed.py"
SPEC = importlib.util.spec_from_file_location("group4_acceptance", MODULE)
acceptance = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(acceptance)


class LedgerTests(unittest.TestCase):
    def test_missing_selection_direction_does_not_pass(self):
        case = {"fixture": "one.case", "location": "standalone",
                "line_ending": "lf", "bom": False,
                "before_selections": [{"index": 0}, {"index": 1}]}
        observed = Counter({(False, "lf", False, False, 0): 1,
                            (False, "lf", False, False, 1): 1,
                            (False, "lf", False, True, 0): 1})
        covered, _, _ = acceptance.verify_list_case(
            case, {"one.case": "one"}, {"one": observed}, {"one"}
        )
        self.assertFalse(covered)
        observed[(False, "lf", False, True, 1)] = 1
        covered, _, _ = acceptance.verify_list_case(
            case, {"one.case": "one"}, {"one": observed}, {"one"}
        )
        self.assertTrue(covered)

    def test_failed_fixture_cannot_pass_with_printed_variants(self):
        log = ('test one ... cell=false, ending="\\n", bom=false, '
               'reverse=false, primary=0\nFAILED\n')
        variants, finished = acceptance.executed_variants(log)
        self.assertEqual(variants["one"][(False, "lf", False, False, 0)], 1)
        self.assertNotIn("one", finished)

    def test_duplicate_table_marker_is_rejected(self):
        with self.assertRaises(ValueError):
            acceptance.table_passed_ids(
                "G4CASE table/a PASS\nG4CASE table/a PASS\n"
            )


if __name__ == "__main__":
    unittest.main()
