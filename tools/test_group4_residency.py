"""Contracts for the bounded residency runner; these are not real-window tests."""
from contextlib import redirect_stderr
import importlib.util
import io
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

HERE = Path(__file__).resolve().parents[1] / 'platform/macos/yu-shell-macos'
sys.path.insert(0, str(HERE))
spec = importlib.util.spec_from_file_location('residency_tests_runner', HERE / 'run-resource-residency.py')
runner = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = runner
spec.loader.exec_module(runner)


class ResidencyTests(unittest.TestCase):
    def test_defaults(self):
        args = runner.parse_args(['unused'])
        self.assertEqual((args.documents, args.lifetimes, args.passes, args.idle_seconds), (4, 4, 3, 70))
        self.assertFalse(args.history_audit)
        self.assertFalse(args.allocation_stacks)

    def test_invalid_cli_has_no_filesystem_effect(self):
        for flag, bad in [('--lifetimes','1'),('--lifetimes','9'),('--passes','0'),('--passes','11'),
                          ('--documents','1'),('--idle-seconds','69'),('--sample-seconds','0')]:
            with tempfile.TemporaryDirectory() as tmp, redirect_stderr(io.StringIO()):
                path = Path(tmp)/'output'
                with self.assertRaises(SystemExit):
                    runner.main([str(path), flag, bad])
                self.assertFalse(path.exists())

    def test_non_macos_does_not_build_or_create_output(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(runner.sys, 'platform', 'linux'):
            path = Path(tmp)/'output'
            with self.assertRaisesRegex(SystemExit, 'requires macOS'):
                runner.main([str(path)])
            self.assertFalse(path.exists())

    def test_child_environment_is_clean_and_parent_is_unchanged(self):
        for diagnostic in (False, True):
            with tempfile.TemporaryDirectory() as tmp:
                args = runner.parse_args([tmp] + (['--allocation-stacks'] if diagnostic else []))
                soak = runner.Residency(args, Path(tmp), {'passed':False})
                parent = {'PATH':'/usr/bin', 'MallocStackLogging':'old', 'MallocNanoZone':'0',
                          'DYLD_INSERT_LIBRARIES':'diagnostic', 'NSZombieEnabled':'YES',
                          'YU_HISTORY_AUDIT':'1', 'YU_OTHER':'private'}
                try:
                    with mock.patch.dict(os.environ, parent, clear=True):
                        child = soak.application_environment()
                        self.assertEqual(dict(os.environ), parent)
                    self.assertNotIn('DYLD_INSERT_LIBRARIES', child)
                    self.assertNotIn('NSZombieEnabled', child)
                    self.assertNotIn('MallocNanoZone', child)
                    self.assertNotIn('YU_OTHER', child)
                    self.assertNotIn('YU_HISTORY_AUDIT', child)
                    self.assertEqual(child['YU_RESOURCE_AUDIT'], '1')
                    self.assertEqual(child.get('MallocStackLogging'), '1' if diagnostic else None)
                finally:
                    soak.cleanup()

    def test_history_boundaries_never_record_raw_source_or_path(self):
        state = {'AXValue':'private body', 'focused_window_identifier':'/private/path',
                 'AXSelectedText':'private selection', 'AXFrontmost':False, 'frontmost_pid':71}
        receipt = runner.boundary_identity(state)
        self.assertFalse(receipt['ax_frontmost'])
        self.assertEqual(receipt['frontmost_pid'], 71)
        self.assertNotIn('private', str(receipt))
        self.assertEqual(receipt['source_identity'], runner.native.text_identity('private body'))

    def test_history_receipts_are_opt_in_and_child_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = runner.parse_args([tmp, '--history-audit'])
            soak = runner.Residency(args, Path(tmp), {'passed':False})
            try:
                with mock.patch.dict(os.environ, {}, clear=True):
                    self.assertEqual(soak.application_environment()['YU_HISTORY_AUDIT'], '1')
                    self.assertNotIn('YU_HISTORY_AUDIT', os.environ)
            finally:
                soak.cleanup()

    def test_input_boundary_observation_does_not_repeat_shortcut(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = runner.parse_args([tmp, '--history-audit'])
            soak = runner.Residency(args, Path(tmp), {'passed':False})
            try:
                with mock.patch.object(runner.native.NativeSoak, 'run', side_effect=[{},None,{}]) as run:
                    soak.run('key', 6, 'cmd')
                self.assertEqual(run.call_args_list, [mock.call('snapshot'), mock.call('key',6,'cmd'), mock.call('snapshot')])
            finally:
                soak.cleanup()

    def test_normal_input_has_no_boundary_snapshot_overhead(self):
        with tempfile.TemporaryDirectory() as tmp:
            soak = runner.Residency(runner.parse_args([tmp]), Path(tmp), {'passed':False})
            try:
                with mock.patch.object(runner.native.NativeSoak, 'run') as run:
                    soak.run('key',6,'cmd')
                run.assert_called_once_with('key',6,'cmd')
            finally:
                soak.cleanup()

    def test_install_requires_consumption_receipt_without_repeating_paste(self):
        doc = runner.native.Document(0, Path('unused'), 'before')
        soak = mock.Mock()
        soak.run.side_effect = [None, {'verified_document_paste': True}, None]
        runner.Residency.install(soak, doc, 'after', 'plain')
        self.assertEqual(soak.run.call_args_list, [mock.call('select',0,6),
            mock.call('paste-document','after'), mock.call('select',0,0)])
        self.assertEqual(doc.source, 'after')
        soak.await_frame.assert_called_once_with(doc, 'plain')

    def test_missing_paste_receipt_cannot_advance_source_or_frame(self):
        doc = runner.native.Document(0, Path('unused'), 'before')
        soak = mock.Mock()
        soak.run.return_value = None
        with self.assertRaisesRegex(AssertionError, 'consumption receipt'):
            runner.Residency.install(soak, doc, 'after', 'plain')
        self.assertEqual(doc.source, 'before')
        soak.await_frame.assert_not_called()

    def fake_lifetime(self, path, *, bad_history=False):
        args = runner.parse_args([str(path), '--documents', '2', '--passes', '1', '--lifetimes','2'])
        sentinel = runner.native.Document(-1, path/'sentinel.md', '# Untouched sentinel\r\n', window='sentinel')
        documents = []
        for index in range(2):
            doc = runner.native.Document(index, path/f'{index}.md', runner.fixture_source(index, 0, 'plain'))
            doc.path.write_bytes(runner.exact_bytes(doc.source))
            documents.append(doc)
        soak = mock.Mock(args=args, result={})
        active = {-1: sentinel}
        def open_document(doc):
            doc.window = str(doc.index)
            active[doc.index] = doc
        def install(doc, source, stage):
            doc.source = source
            doc.audit = {'undo_entries':0 if bad_history else 3, 'redo_entries':0}
        def close(doc):
            del active[doc.index]
        soak.open_document.side_effect = open_document
        soak.install.side_effect = install
        soak.close_document.side_effect = close
        soak.window_ids.side_effect = lambda: {d.window for d in active.values()}
        return soak, sentinel, documents

    def test_lifetime_preserves_history_before_close_and_never_relaunches(self):
        with tempfile.TemporaryDirectory() as tmp:
            soak, sentinel, docs = self.fake_lifetime(Path(tmp))
            returned = runner.lifetime(soak, sentinel, docs, 1)
            self.assertEqual(soak.result['completed_lifetimes'], 1)
            self.assertEqual([c.args[0] for c in soak.idle.call_args_list],
                ['lifetime_01_retained_idle', 'lifetime_01_closed_idle'])
            self.assertEqual(soak.close_document.call_count, 2)
            self.assertEqual(soak.run.call_count, 8)
            soak.launch.assert_not_called()
            soak.quit.assert_not_called()
            self.assertTrue(all(old is not new for old,new in zip(docs,returned)))
            self.assertEqual(sentinel.source, '# Untouched sentinel\r\n')
            calls = soak.mock_calls
            first_close = next(i for i,c in enumerate(calls) if c[0] == 'close_document')
            last_history = max(i for i,c in enumerate(calls) if c[0] == 'event' and c.args[0] == 'retained_history_verified')
            self.assertLess(last_history, first_close)

    def test_bad_history_cannot_finish_or_close_documents(self):
        with tempfile.TemporaryDirectory() as tmp:
            soak, sentinel, docs = self.fake_lifetime(Path(tmp), bad_history=True)
            with self.assertRaisesRegex(AssertionError, 'history count differs'):
                runner.lifetime(soak, sentinel, docs, 1)
            soak.close_document.assert_not_called()
            self.assertNotIn('completed_lifetimes', soak.result)

    def test_changed_saved_bytes_fail_before_open(self):
        with tempfile.TemporaryDirectory() as tmp:
            soak, sentinel, docs = self.fake_lifetime(Path(tmp))
            docs[0].path.write_bytes(b'changed')
            with self.assertRaisesRegex(AssertionError, 'bytes changed'):
                runner.lifetime(soak, sentinel, docs, 1)
            soak.open_document.assert_not_called()

    def test_extra_window_cannot_be_a_comparable_closed_state(self):
        with tempfile.TemporaryDirectory() as tmp:
            soak, sentinel, docs = self.fake_lifetime(Path(tmp))
            soak.window_ids.side_effect = lambda: {'sentinel','extra'}
            with self.assertRaisesRegex(AssertionError, 'sentinel-only'):
                runner.lifetime(soak, sentinel, docs, 1)
            self.assertNotIn('completed_lifetimes', soak.result)

    def test_baseline_forbids_allocation_tools(self):
        soak = mock.Mock(args=SimpleNamespace(allocation_stacks=False))
        with self.assertRaisesRegex(AssertionError, 'forbidden'):
            runner.Residency.capture_allocations(soak, 'test')
        soak.live.assert_not_called()

    def test_nonzero_leaks_is_retained_without_calling_it_zero(self):
        with tempfile.TemporaryDirectory() as tmp:
            soak = mock.Mock(args=SimpleNamespace(allocation_stacks=True), out=Path(tmp), result={})
            soak.process.pid = 100
            identity = {'pid':100, 'pgid':100, 'kind':'app', 'start_seconds':1,
                        'start_microseconds':2, 'executable':'/isolated/Yu'}
            soak.process_identities = {100:identity}
            with mock.patch.object(runner, 'same_process_identity', return_value=True), \
                 mock.patch.object(runner.subprocess, 'run', side_effect=[
                    SimpleNamespace(returncode=c) for c in [0,0,1,0]]):
                runner.Residency.capture_allocations(soak, 'closed')
            self.assertFalse(soak.result['leaks_scans_all_zero'])
            self.assertEqual([r['exit_code'] for r in soak.result['allocation_captures']], [0,0,1,0])
            self.assertNotIn('passed', soak.result)
            self.assertTrue(all('elapsed_seconds' in r and 'seconds' not in r for r in soak.result['allocation_captures']))


if __name__ == '__main__':
    unittest.main()
