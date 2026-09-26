"""Headless accounting/runner-contract tests, NOT macOS visual acceptance."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

SHELL = Path(__file__).resolve().parents[1] / 'platform/macos/yu-shell-macos'
sys.path.insert(0, str(SHELL))
import group4_resource_soak as model

spec = importlib.util.spec_from_file_location('resource_soak_runner', SHELL / 'run-resource-soak.py')
runner = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = runner
spec.loader.exec_module(runner)


def record(**changes):
    return dict({'revision': 1, 'source_bytes': 4096, 'embedded_gpu_textures': 0,
                 'embedded_gpu_rgba_bytes': 0, 'embedded_failures': 0,
                 'embedded_cache_entries': 0, 'undo_entries': 7, 'redo_entries': 0, 'gpu_evictions': 0}, **changes)


def line(value):
    return model.PREFIX + json.dumps(value, ensure_ascii=False).encode() + b'\n'


class OptionsTests(unittest.TestCase):
    def test_defaults_and_boundary_options(self):
        self.assertEqual(model.Options().validate().documents, 4)
        model.Options(2, 30, 70, 1).validate()
        model.Options(12, 86400, 3600, 60).validate()

    def test_invalid_bounds_and_types(self):
        for name, values in {'documents': [1, 13, True, 2.5], 'seconds': [0, 29, 86401],
                             'idle_seconds': [0, 69, 3601], 'sample_seconds': [0, 61]}.items():
            for value in values:
                with self.subTest(name=name, value=value), self.assertRaises(ValueError):
                    model.Options(**{name: value}).validate()

    def test_invalid_cli_has_no_output_side_effect(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory) / 'never-created'
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as caught:
                runner.main([str(out), '--documents', '1'])
            self.assertEqual(caught.exception.code, 2)
            self.assertFalse(out.exists())

    def test_non_macos_fails_before_build_or_filesystem_mutation(self):
        with tempfile.TemporaryDirectory() as directory, mock.patch.object(runner.sys, 'platform', 'linux'):
            out = Path(directory) / 'never-created'
            with self.assertRaises(SystemExit):
                runner.main([str(out)])
            self.assertFalse(out.exists())


class FixtureTests(unittest.TestCase):
    def test_every_document_and_stage_has_a_disjoint_slot(self):
        sizes = set()
        for index in range(12):
            for stage in model.STAGES:
                text = model.fixture_source(index, 12, stage)
                size = len(text.encode())
                self.assertNotIn(size, sizes)
                sizes.add(size)
                self.assertEqual(size, len(model.fixture_source(index, 999999, stage).encode()))
                self.assertIn(f'document {index:02}', text)
                self.assertIn('中文🙂', text)
                self.assertNotIn('\n', text.replace('\r\n', ''))
                self.assertTrue(model.exact_bytes(text).startswith(b'\xef\xbb\xbf'))

    def test_plain_has_no_resource_and_visible_resources_precede_padding(self):
        self.assertNotIn('$', model.fixture_source(0, 1, 'plain'))
        valid = model.fixture_source(11, 1, 'valid')
        self.assertLess(valid.index('flowchart LR'), 256)
        self.assertIn('yuUnsupportedGraph', model.fixture_source(11, 1, 'error'))

    def test_generation_changes_content_not_identity_slot(self):
        first, second = [model.fixture_source(2, r, 'valid') for r in (1, 2)]
        self.assertEqual(len(first.encode()), len(second.encode()))
        self.assertNotEqual(model.text_identity(first)['sha256'], model.text_identity(second)['sha256'])

    def test_bad_fixture_inputs_fail(self):
        for args in [(-1, 1, 'plain'), (12, 1, 'plain'), (0, -1, 'plain'),
                     (0, 1000000, 'plain'), (0, 1, 'unknown'), (True, 1, 'plain')]:
            with self.subTest(args=args), self.assertRaises(ValueError):
                model.fixture_source(*args)


class AuditTailTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / 'app.log'
        self.path.write_bytes(b'')

    def append(self, data):
        with self.path.open('ab') as stream:
            stream.write(data)

    def test_partial_json_and_utf8_are_not_published_early(self):
        payload = line(record(label='中文🙂'))
        split = payload.index('中'.encode()) + 1
        tail = model.AuditTail(self.path)
        self.append(payload[:split])
        self.assertEqual(tail.poll(), [])
        self.append(payload[split:-1])
        self.assertEqual(tail.poll(), [])
        self.append(b'\n')
        self.assertEqual(tail.poll(), [record(label='中文🙂')])
        self.assertEqual(tail.poll(), [])

    def test_noise_and_blank_lines_are_not_audit_records(self):
        self.append(b'noise\n\n' + line(record()))
        tail = model.AuditTail(self.path)
        self.assertEqual(tail.poll(), [record()])
        self.assertEqual(tail.records, 1)

    def test_completed_malformed_json_fails_closed(self):
        self.append(model.PREFIX + b'{not-json}\n')
        with self.assertRaises(ValueError):
            model.AuditTail(self.path).poll()

    def test_duplicate_fields_and_nonfinite_json_fail(self):
        for payload in [b'{"revision":1,"revision":2}', b'{"extra":NaN}', b'{"extra":Infinity}']:
            self.path.write_bytes(model.PREFIX + payload + b'\n')
            with self.subTest(payload=payload), self.assertRaises(ValueError):
                model.AuditTail(self.path).poll()

    def test_missing_negative_boolean_or_non_object_counters_fail(self):
        values = [[], record(revision=-1), record(embedded_gpu_textures=True), record(source_bytes='4096')]
        absent = record()
        del absent['embedded_failures']
        values.append(absent)
        for value in values:
            self.path.write_bytes(line(value))
            with self.subTest(value=value), self.assertRaises(ValueError):
                model.AuditTail(self.path).poll()

    def test_small_chunks_are_linear_and_bounded(self):
        self.append(b''.join(line(record(revision=i)) for i in range(400)))
        tail = model.AuditTail(self.path, read_bytes=113, max_line_bytes=1024)
        seen = []
        while not tail.caught_up:
            before = tail.offset
            seen.extend(r['revision'] for r in tail.poll())
            self.assertLessEqual(tail.offset - before, 113)
            self.assertLessEqual(len(tail.pending), 1024)
        self.assertEqual(seen, list(range(400)))
        self.assertEqual(tail.offset, self.path.stat().st_size)
        self.assertEqual(tail.poll(), [])

    def test_truncation_and_rotation_are_rejected(self):
        self.append(line(record()))
        tail = model.AuditTail(self.path)
        tail.poll()
        self.path.write_bytes(b'')
        with self.assertRaises(ValueError):
            tail.poll()
        tail = model.AuditTail(self.path)
        self.path.rename(self.path.with_suffix('.old'))
        self.path.write_bytes(line(record()))
        with self.assertRaises(ValueError):
            tail.poll()

    def test_oversized_complete_and_partial_lines_are_rejected(self):
        for suffix in (b'', b'\n'):
            self.path.write_bytes(b'x' * 513 + suffix)
            with self.subTest(suffix=suffix), self.assertRaises(ValueError):
                model.AuditTail(self.path, max_line_bytes=512).poll()

    def test_new_process_cursor_does_not_replay_old_log(self):
        self.append(line(record(revision=90)))
        tail = model.AuditTail(self.path, start=self.path.stat().st_size)
        self.assertEqual(tail.poll(), [])
        self.append(line(record(revision=0)))
        self.assertEqual([r['revision'] for r in tail.poll()], [0])

    def test_ownership_boundary_validates_and_discards_existing_records(self):
        self.append(line(record(revision=90)))
        tail = model.AuditTail(self.path, read_bytes=31)
        self.assertEqual(tail.drain(), self.path.stat().st_size)
        self.append(line(record(revision=0, undo_entries=0)))
        values = []
        while not tail.caught_up:
            values.extend(tail.poll())
        self.assertEqual([r['revision'] for r in values], [0])

    def test_ownership_boundary_never_hides_bad_or_partial_writes(self):
        for payload in (b'yu-resource-audit {bad}\n', b'yu-resource-audit {'):
            self.path.write_bytes(payload)
            with self.subTest(payload=payload), self.assertRaises((ValueError, AssertionError)):
                model.AuditTail(self.path).drain()

    def test_ownership_boundary_has_a_backlog_budget(self):
        self.append(b'x\n' * 1000)
        with self.assertRaisesRegex(AssertionError, 'backlog'):
            model.AuditTail(self.path, read_bytes=10).drain(max_bytes=20)


class CounterTests(unittest.TestCase):
    def test_same_length_stale_revision_is_not_current(self):
        text = model.fixture_source(0, 2, 'plain')
        self.assertFalse(model.matches_frame(record(revision=5), text, 5))
        self.assertFalse(model.matches_frame(record(revision=4), text, 5))
        self.assertTrue(model.matches_frame(record(revision=6), text, 5))
        self.assertFalse(model.matches_frame(record(revision=6), model.fixture_source(1, 2, 'plain'), 5))

    def test_plain_requires_all_three_release_counters(self):
        self.assertTrue(model.settled(record(), 'plain'))
        for name in ('embedded_gpu_textures', 'embedded_gpu_rgba_bytes', 'embedded_failures'):
            self.assertFalse(model.settled(record(**{name: 1}), 'plain'))

    def test_valid_error_and_cold_are_distinct(self):
        valid = record(embedded_gpu_textures=2, embedded_gpu_rgba_bytes=4096)
        self.assertTrue(model.settled(valid, 'valid'))
        self.assertFalse(model.settled(record(embedded_gpu_textures=1), 'valid'))
        self.assertTrue(model.settled(record(embedded_gpu_textures=1, embedded_failures=1), 'error'))
        self.assertFalse(model.settled(valid, 'error'))
        self.assertTrue(model.settled(record(embedded_gpu_textures=1), 'cold'))

    def test_existing_cache_limit_not_relaxed(self):
        self.assertTrue(model.settled(record(embedded_cache_entries=32), 'plain'))
        with self.assertRaises(AssertionError):
            model.settled(record(embedded_cache_entries=33), 'plain')


class SummaryTests(unittest.TestCase):
    def test_signed_deltas_and_process_sessions_stay_separate(self):
        summary = model.FootprintSummary()
        for time_s, value in [(0, 100), (1, 120), (2, 80)]:
            summary.add('retained', 'idle', time_s, {'pid': 10, 'physical_footprint_bytes': value})
        summary.add('cold', 'idle', 3, {'pid': 11, 'physical_footprint_bytes': 20})
        first, second = summary.report()['phases']
        self.assertEqual((first['samples'], first['min_bytes'], first['max_bytes'], first['delta_bytes']), (3, 80, 120, -20))
        self.assertEqual(second['pid'], 11)
        self.assertNotIn('leak_free', summary.report())

    def test_statistics_do_not_retain_samples_or_add_gpu_estimates(self):
        summary = model.FootprintSummary()
        for i in range(10000):
            summary.add('retained', 'churn', i, {'pid': 10, 'physical_footprint_bytes': 200,
                                                'logical_gpu_bytes': 999999})
        self.assertEqual(len(summary.phases), 1)
        self.assertEqual(summary.report()['phases'][0]['samples'], 10000)
        self.assertEqual(summary.report()['phases'][0]['max_bytes'], 200)

    def test_pid_change_and_backwards_or_nonfinite_time_fail(self):
        summary = model.FootprintSummary()
        summary.add('s', 'p', 1, {'pid': 1, 'physical_footprint_bytes': 10})
        for pid, stamp in [(2, 2), (1, 0)]:
            with self.assertRaises(AssertionError):
                summary.add('s', 'p', stamp, {'pid': pid, 'physical_footprint_bytes': 10})
        with self.assertRaises(ValueError):
            summary.add('s', 'p', float('nan'), {'pid': 1, 'physical_footprint_bytes': 10})

    def test_invalid_footprints_are_not_coerced_to_zero(self):
        for value in (None, True, -1, '20'):
            with self.assertRaises(ValueError):
                model.FootprintSummary().add('s', 'p', 0, {'pid': 1, 'physical_footprint_bytes': value})

    def test_process_inventory_separates_roles_and_keeps_birth_identity(self):
        helper = dict(kind='helpers', pid=11, ppid=10, pgid=10, start_seconds=100, start_microseconds=7)
        app = dict(helper, kind='app', pid=10, ppid=1)
        rows, identities, unreadable = model.process_inventory({
            'schema_version': 1, 'unreadable_unrelated': 2, 'processes': [helper, app]})
        self.assertEqual(rows, {'app': [10], 'helpers': [11]})
        self.assertEqual(identities[11], helper)
        self.assertEqual(unreadable, 2)
        self.assertTrue(model.same_process_identity(helper, dict(helper, ppid=1)))
        self.assertFalse(model.same_process_identity(helper, dict(helper, start_microseconds=8)))
        self.assertFalse(model.same_process_identity(helper, None))

    def test_malformed_process_inventory_is_not_an_empty_success(self):
        helper = dict(kind='helpers', pid=11, ppid=10, pgid=10, start_seconds=100, start_microseconds=7)
        valid = dict(schema_version=1, unreadable_unrelated=0, processes=[helper])
        bad = [None, {}, dict(valid, schema_version=True), dict(valid, processes=None),
               dict(valid, unreadable_unrelated=-1), dict(valid, processes=[helper, helper])]
        for field, value in [('pid', True), ('pgid', 0), ('ppid', -1), ('start_seconds', 0),
                             ('start_microseconds', 1000000), ('kind', 'other')]:
            bad.append(dict(valid, processes=[dict(helper, **{field: value})]))
        for value in bad:
            with self.subTest(value=value), self.assertRaises(ValueError):
                model.process_inventory(value)


class RunnerContractTests(unittest.TestCase):
    def test_pixel_failure_cannot_be_recorded_as_pass(self):
        # Exercise the real orchestration against a fake desktop, not a GPU.
        class Fake:
            def __init__(self, out):
                self.out = out
                self.args = runner.parse_args([str(out), '--documents', '2', '--seconds', '30'])
                self.result = {'passed': False, 'checks': []}
                self.process = type('Process', (), {'pid': 100})()
                self.session = 'retained'
                self.events = []
            def launch(self, path):
                if self.session == 'cold_reopen':
                    self.process.pid = 101
            def install(self, doc, text, stage):
                doc.source = text
            def save(self, doc):
                doc.path.write_bytes(model.exact_bytes(doc.source))
            def live(self):
                return {'helpers': []}
            def event(self, name, **fields):
                self.events.append(name)
            def __getattr__(self, name):
                return lambda *a, **kw: None
        class PixelFailure:
            def __init__(self, *args):
                self.result = args[-1]
            def restored_preview(self, expected):
                self.result['restored_preview_status'] = 'blank'
                raise AssertionError('blank formula')
        module = type('Followup', (), {'Checks': PixelFailure})()
        with tempfile.TemporaryDirectory() as directory:
            fake = Fake(Path(directory))
            # One complete pass, then expiration: no waiting or desktop input.
            with mock.patch.dict(sys.modules, {'group4_followup': module}), \
                 mock.patch.object(runner.time, 'monotonic', side_effect=[0, 0, 1, 31]), \
                 self.assertRaisesRegex(AssertionError, 'blank formula'):
                runner.exercise(fake)
            self.assertFalse(fake.result['passed'])
            self.assertEqual(fake.result['restored_preview_status'], 'blank')
            self.assertTrue(fake.result['same_process_phases_completed'])
            self.assertEqual(fake.events.count('retained_history_verified'), 2)

    def test_new_document_ignores_old_lifetime_history(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            args = runner.parse_args([str(out)])
            soak = runner.NativeSoak(args, out, {'passed': False})
            log = out / 'app.log'
            source = model.fixture_source(0, 2, 'plain')
            log.write_bytes(line(record(revision=90, undo_entries=9))
                            + line(record(revision=0, undo_entries=0)))
            soak.tail = model.AuditTail(log)
            soak.live = lambda: {'helpers': []}
            soak.expect = lambda expected: self.assertEqual(expected, source)
            doc = runner.Document(0, out / 'doc.md', source)
            try:
                soak.await_frame(doc, 'plain')
                self.assertEqual(doc.revision, 0)
                self.assertEqual(doc.audit['undo_entries'], 0)
            finally:
                soak.cleanup()

    def test_restored_fresh_history_does_not_accept_old_plain_frame(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            soak = runner.NativeSoak(runner.parse_args([str(out)]), out, {'passed': False})
            log = out / 'app.log'
            source = model.fixture_source(0, 2, 'plain')
            log.write_bytes(line(record(revision=90, undo_entries=9))
                            + line(record(revision=2, undo_entries=0, redo_entries=1)))
            soak.tail = model.AuditTail(log)
            soak.live = lambda: {'helpers': []}
            soak.expect = lambda expected: self.assertEqual(expected, source)
            doc = runner.Document(0, out / 'doc.md', source, revision=1)
            try:
                soak.await_frame(doc, 'plain', history=(0, 1))
                self.assertEqual(doc.revision, 2)
            finally:
                soak.cleanup()

    def test_resource_frame_cannot_pass_without_a_positive_helper_observation(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            soak = runner.NativeSoak(runner.parse_args([str(out)]), out, {})
            source = model.fixture_source(0, 1, 'valid')
            log = out / 'app.log'
            log.write_bytes(line(record(source_bytes=len(source.encode()), undo_entries=0,
                                        embedded_gpu_textures=2, embedded_gpu_rgba_bytes=1024)))
            soak.tail = model.AuditTail(log)
            soak.live = lambda: {'helpers': []}
            soak.expect = lambda text: self.assertEqual(text, source)
            try:
                with self.assertRaisesRegex(AssertionError, 'kernel-observed helper'):
                    soak.await_frame(runner.Document(0, out / 'doc.md', source), 'valid')
            finally:
                soak.cleanup()

    def sampled_helper(self, after, *, returncode=0, succeeds=True):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            soak = runner.NativeSoak(runner.parse_args([str(out)]), out, {})
            app = dict(kind='app', pid=10, ppid=1, pgid=10, start_seconds=100, start_microseconds=1)
            helper = dict(app, kind='helpers', pid=11, ppid=10, start_microseconds=2)
            soak.process = type('Process', (), {'pid': 10})()
            observations = iter([{10: app, 11: helper}, {10: app, **({11: dict(helper, **after)} if after is not None else {})}])
            def live():
                soak.process_identities = next(observations)
                return {'app': [10], 'helpers': [pid for pid in soak.process_identities if pid != 10]}
            soak.live = live
            response = type('Response', (), {'returncode': returncode,
                'stdout': json.dumps({'pid': 11, 'physical_footprint_bytes': 77})})()
            try:
                with mock.patch.object(runner.subprocess, 'check_output', return_value=json.dumps({
                        'pid': 10, 'physical_footprint_bytes': 100})), \
                     mock.patch.object(runner.subprocess, 'run', return_value=response):
                    if succeeds:
                        soak.sample('test')
                        events = [json.loads(line) for line in (out / 'events.jsonl').read_text().splitlines()]
                        return events[-1]['helpers'][0]
                    with self.assertRaisesRegex(AssertionError, 'live identified helper'):
                        soak.sample('test')
            finally:
                soak.process = None
                soak.cleanup()

    def test_helper_memory_is_bound_to_kernel_birth_identity(self):
        sample = self.sampled_helper({})
        self.assertEqual(sample['physical_footprint_bytes'], 77)
        self.assertEqual(sample['identity']['start_microseconds'], 2)

    def test_exit_or_pid_reuse_does_not_publish_unrelated_memory_or_zero(self):
        for after in (None, {'start_microseconds': 3}):
            sample = self.sampled_helper(after)
            self.assertEqual(sample['status'], 'exited_or_replaced_during_sample')
            self.assertNotIn('physical_footprint_bytes', sample)

    def test_failed_sample_of_the_same_live_helper_is_not_ignored(self):
        self.sampled_helper({}, returncode=1, succeeds=False)

    def test_atomic_json_leaves_no_partial_result(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'result.json'
            runner.atomic_json(path, {'passed': False})
            runner.atomic_json(path, {'passed': True})
            self.assertTrue(json.loads(path.read_text())['passed'])
            self.assertFalse(path.with_name('result.json.tmp').exists())


if __name__ == '__main__':
    unittest.main()
