"""Check phase routing and failed-case evidence without opening any GUI."""
from pathlib import Path
import os
import tempfile
import unittest
from unittest.mock import patch

from acceptance_runner import RunContext
from acceptance_suites import group5_window


class FakeWindowChecks:
    failure = None
    calls = []

    def __init__(self, ctx):
        self.ctx = ctx
        self.last_command = 'observed'
        self.second = 'second.md'
        self.second_source = 'second'

    def setup(self):
        # Seed a completed observation; no subprocess or editor is involved.
        log = self.ctx.output / 'logs/observed.log'
        log.write_text('test observation\n')
        self.ctx.commands['observed'] = {'exit_code': 0, 'log': str(log), 'log_sha256': 'test-only'}

    def act(self, name):
        self.calls.append(name)
        if name == self.failure:
            raise AssertionError('deliberate failed assertion')

    def ime(self): self.act('ime')
    def menus(self): self.act('menus')
    def untitled(self): self.act('untitled')
    def repeat(self): self.act('repeat')
    def cancellations(self): self.act('cancel')
    def open_document(self, *args): self.act('open')
    def capture(self, *args): self.act('capture')
    def cleanup(self): self.act('cleanup')


class WindowEvidenceTests(unittest.TestCase):
    def run_phase(self, phase, failure=None):
        with tempfile.TemporaryDirectory() as directory:
            ctx = RunContext(Path(directory) / 'run', 'test-window')
            FakeWindowChecks.calls = []
            FakeWindowChecks.failure = failure
            with patch.dict(os.environ, {'GROUP5_WINDOW_PHASE': phase}), \
                    patch.object(group5_window, 'WindowChecks', FakeWindowChecks):
                if failure:
                    with self.assertRaisesRegex(AssertionError, 'deliberate'):
                        group5_window.run_suite(ctx)
                else:
                    self.assertEqual(group5_window.run_suite(ctx), 0)
            self.assertEqual(FakeWindowChecks.calls[-1], 'cleanup')
            return ctx.ledger.finish(), list(FakeWindowChecks.calls)

    def test_window_phase_does_not_pretend_to_test_running_cancellation(self):
        rows, calls = self.run_phase('window')
        self.assertEqual(calls, ['ime', 'menus', 'untitled', 'repeat', 'cleanup'])
        self.assertEqual(len(rows), 4)
        self.assertTrue(all(row['status'] == 'passed' for row in rows))

    def test_cancellation_phase_uses_only_its_named_contract(self):
        rows, calls = self.run_phase('cancellation')
        self.assertEqual(calls, ['open', 'cancel', 'cleanup'])
        self.assertEqual([row['id'] for row in rows], ['cancel-retry-and-owner-close'])

    def test_failed_assertion_is_not_a_pass_and_later_cases_remain_unrun(self):
        rows, _ = self.run_phase('window', 'menus')
        self.assertEqual([row['status'] for row in rows], ['passed', 'failed', 'not_run', 'not_run'])


if __name__ == '__main__':
    unittest.main()
