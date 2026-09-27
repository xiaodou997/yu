"""Typed receipt parser contracts, not macOS acceptance."""
from pathlib import Path
import sys
import json
import unittest

SHELL = Path(__file__).resolve().parents[1] / 'platform/macos/yu-shell-macos'
sys.path.insert(0, str(SHELL))
from group4_history_audit import PREFIX, summarize_history


def begin(pid=10, ticket=1):
    return dict(schema=1, pid=pid, ticket=ticket, phase='begin', uptime=1,
                entry='keyEquivalent', command=8, eventTimestamp=0.5,
                state=dict(window=12, firstResponder=True, editable=True, composition=False,
                           available=True, revision=5))


def end(pid=10, ticket=1):
    return dict(schema=1, pid=pid, ticket=ticket, phase='end', uptime=2,
                revisionAfter=6, handled=True)


def lines(*records):
    return [PREFIX + json.dumps(record) + '\n' for record in records]


class HistoryAuditTests(unittest.TestCase):
    def test_complete_pair_and_process_scoped_tickets(self):
        report = summarize_history(lines(begin(), end(), begin(11), end(11)), [10, 11])
        self.assertEqual(report['pairs'], 2)
        self.assertEqual(report['entries'], {'keyEquivalent': 2})
        self.assertEqual(report['outcomes'], {'handled': 2})

    def test_empty_partial_and_missing_process_fail_closed(self):
        for records, expected in [([], [10]), ([begin()], [10]), ([end()], [10]),
                                  ([begin(), end()], [10, 11]), ([begin(), end()], [])]:
            with self.subTest(records=records), self.assertRaises(ValueError):
                summarize_history(lines(*records), expected)

    def test_duplicate_or_nonsequential_tickets_are_rejected(self):
        for records in [[begin(), begin()], [begin(), end(), end()], [begin(ticket=2), end(ticket=2)]]:
            with self.assertRaises(ValueError):
                summarize_history(lines(*records), [10])

    def test_truncated_trace_cannot_be_complete(self):
        trace = lines(begin(), end(), dict(schema=1, pid=10, ticket=1, uptime=3, phase='truncated'))
        with self.assertRaisesRegex(ValueError, 'exhausted'):
            summarize_history(trace, [10])

    def test_partial_json_and_oversize_are_not_silently_dropped(self):
        for tail in [PREFIX + '{', PREFIX + 'x' * 2049 + '\n']:
            with self.assertRaises(ValueError):
                summarize_history(lines(begin(), end()) + [tail], [10])

    def test_unknown_or_sensitive_fields_are_rejected(self):
        for extra in ['source', 'characters', 'path', 'selectedText']:
            with self.assertRaises(ValueError):
                summarize_history(lines(dict(begin(), **{extra: 'not permitted'}), end()), [10])

    def test_boolean_numeric_fields_and_nonfinite_values_are_rejected(self):
        for field, value in [('pid', True), ('ticket', 0), ('uptime', float('nan')), ('command', True),
                             ('schema', True), ('entry', 'unknown'), ('eventTimestamp', -1)]:
            with self.subTest(field=field), self.assertRaises(ValueError):
                summarize_history(lines(dict(begin(), **{field: value}), end()), [10])
        with self.assertRaises(ValueError):
            summarize_history(lines(begin(), dict(end(), handled=1)), [10])

    def test_bad_state_and_backwards_revision_are_rejected(self):
        for state in [None, dict(begin()['state'], editable=1), dict(begin()['state'], revision=-1)]:
            with self.assertRaises(ValueError):
                summarize_history(lines(dict(begin(), state=state), end()), [10])
        with self.assertRaises(ValueError):
            summarize_history(lines(begin(), dict(end(), revisionAfter=4)), [10])

    def test_declined_and_no_revision_change_remain_distinct(self):
        for handled, outcome in [(False, 'declined'), (True, 'handled_without_revision_change')]:
            report = summarize_history(lines(begin(), dict(end(), handled=handled, revisionAfter=5)), [10])
            self.assertEqual(report['outcomes'], {outcome: 1})

    def test_duplicate_json_keys_and_unexpected_pid_are_rejected(self):
        duplicated = PREFIX + json.dumps(begin()).replace('"schema": 1', '"schema": 1, "schema": 1') + '\n'
        with self.assertRaises(ValueError):
            summarize_history([duplicated] + lines(end()), [10])
        with self.assertRaises(ValueError):
            summarize_history(lines(begin(20), end(20)), [10])

    def test_non_history_logs_are_not_receipt_evidence(self):
        with self.assertRaises(ValueError):
            summarize_history(['ordinary noise\n', 'yu-resource-audit {}\n'], [10])
        self.assertEqual(summarize_history(['noise\n'] + lines(begin(), end()), [10])['pairs'], 1)


if __name__ == '__main__':
    unittest.main()
