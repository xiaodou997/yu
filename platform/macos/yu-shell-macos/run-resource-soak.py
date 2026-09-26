#!/usr/bin/env python3
"""Same-process multi-document Group 4 stress, with isolated native windows.

Run on an unlocked macOS desktop after building the audited release bundle.
This runner does not change system time, clear history, or repair rendering.
The existing source-preserving cold-reopen pixel oracle remains mandatory.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import signal
import subprocess
import sys
import time
import traceback
import uuid

from group4_resource_soak import (AuditTail, COLD_SOURCE, FootprintSummary, Options,
                                 digest, exact_bytes, fixture_source, matches_frame,
                                 process_rows, require, settled, text_identity)

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--documents', type=int, default=4, help='2..12 work documents, plus one plain sentinel')
    parser.add_argument('--seconds', type=int, default=600, help='30..86400; finish the current full pass, record actual duration')
    parser.add_argument('--idle-seconds', type=int, default=70, help='70..3600, observed both before and after closing work documents')
    parser.add_argument('--sample-seconds', type=int, default=5, help='1..60, read-only idle sampling interval')
    parser.add_argument('--dark', action='store_true')
    args = parser.parse_args(argv)
    try:
        Options(args.documents, args.seconds, args.idle_seconds, args.sample_seconds).validate()
    except ValueError as error:
        parser.error(str(error))
    return args


def code_digest(binary):
    commands = subprocess.check_output(['xcrun', 'otool', '-l', str(binary)], text=True, timeout=30)
    match = re.search(r'sectname __text\s+segname __TEXT\s+addr \S+\s+size (0x[0-9a-fA-F]+)\s+offset (\d+)', commands)
    require(match is not None, 'Cannot verify executable instructions after isolation')
    size, offset = int(match[1], 16), int(match[2])
    with binary.open('rb') as stream:
        stream.seek(offset)
        section = stream.read(size)
    require(len(section) == size, 'Truncated executable text section')
    return hashlib.sha256(section).hexdigest()


def atomic_json(path, value):
    temporary = path.with_name(path.name + '.tmp')
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2, allow_nan=False) + '\n')
    temporary.replace(path)


@dataclass
class Document:
    index: int
    path: Path
    source: str
    window: str = ''
    revision: int = -1
    audit: dict | None = None
    audit_seconds: float | None = None


class NativeSoak:
    def __init__(self, args, out, result):
        self.args, self.out, self.result = args, out, result
        self.app = out / 'YuResourceSoak.app'
        self.binary = self.app / 'Contents/MacOS/Yu'
        self.helper = self.app / 'Contents/Helpers/yu-document-renderer'
        self.driver = out / 'native-event-driver'
        self.probe = out / 'process-footprint'
        self.process = None
        self.log = None
        self.tail = None
        self.session = 'retained'
        self.documents = {}
        self.started = time.monotonic()
        self.sequence = 0
        self.summary = FootprintSummary()
        self.journal = (out / 'events.jsonl').open('x')

    def event(self, kind, **fields):
        self.journal.write(json.dumps({'event': kind, 'seconds': time.monotonic() - self.started,
                                       'session': self.session, **fields}, ensure_ascii=False, allow_nan=False) + '\n')
        self.journal.flush()

    def progress(self):
        self.result['footprint'] = self.summary.report()
        self.result['elapsed_seconds'] = time.monotonic() - self.started
        self.result['native_events'] = self.sequence
        atomic_json(self.out / 'result.json', self.result)

    def processes(self):
        text = subprocess.check_output(['ps', '-ww', '-axo', 'pid=,command='], text=True, timeout=10)
        return process_rows(text, self.binary, self.helper)

    def live(self):
        require(self.process is not None and self.process.poll() is None, 'The original application process exited')
        rows = self.processes()
        require(rows['app'] == [self.process.pid], 'Expected exactly the same isolated application PID')
        require(len(rows['helpers']) <= self.args.documents, 'Helper count exceeded the number of work documents')
        return rows

    def run(self, *arguments):
        require(self.process is not None and self.process.poll() is None, 'No live target for native input')
        response = subprocess.run([str(self.driver), str(self.process.pid), *map(str, arguments)],
                                  capture_output=True, text=True, timeout=20)
        self.sequence += 1
        # Do not accumulate whole long-page sources in RAM or a file per snapshot.
        recorded = list(arguments)
        if recorded and recorded[0] == 'paste-text':
            recorded[1] = text_identity(recorded[1])
        self.event('native', number=self.sequence, arguments=recorded, exit_code=response.returncode,
                   stdout_sha256=hashlib.sha256(response.stdout.encode()).hexdigest())
        if response.returncode:
            (self.out / f'failed-event-{self.sequence}.txt').write_text(response.stdout + '\n' + response.stderr)
            raise RuntimeError('Native event failed; exact output retained')
        return json.loads(response.stdout) if response.stdout.strip().startswith('{') else None

    def expect(self, expected, timeout=5):
        deadline = time.monotonic() + timeout
        actual = None
        while time.monotonic() < deadline:
            actual = self.run('snapshot').get('AXValue')
            if actual == expected:
                return
            time.sleep(.1)
        (self.out / 'mismatch-expected.txt').write_bytes(expected.encode())
        (self.out / 'mismatch-actual.txt').write_bytes(str(actual).encode())
        raise AssertionError('Exact focused-document source mismatch; evidence retained')

    def window_ids(self):
        controls = self.run('controls')['controls']
        ids = [c['AXIdentifier'] for c in controls if c.get('AXRole') == 'AXWindow'
               and isinstance(c.get('AXIdentifier'), str) and c['AXIdentifier']]
        require(len(ids) == len(set(ids)), 'Duplicate AX window identities')
        return set(ids)

    def activate(self):
        deadline = time.monotonic() + 20
        while True:
            try:
                self.run('activate')
                return
            except RuntimeError:
                if self.process.poll() is not None or time.monotonic() >= deadline:
                    raise
                time.sleep(.2)

    def launch(self, path):
        require(not self.processes()['app'] and not self.processes()['helpers'], 'Old isolated processes remain')
        log_path = self.out / 'app.log'
        self.log = log_path.open('ab')
        self.tail = AuditTail(log_path, start=log_path.stat().st_size)
        env = {k: v for k, v in os.environ.items() if not k.startswith('YU_')}
        env.update(YU_DOCUMENT_STATE_DIR=str(self.out / 'state'),
                   YU_PRESENTATION_STATE_DIR=str(self.out / 'columns'), YU_RESOURCE_AUDIT='1')
        from group4_followup import appearance_arguments
        command = [str(self.binary), str(path)] + appearance_arguments(self.args.dark)
        self.process = subprocess.Popen(command, env=env, stdout=self.log,
                                        stderr=subprocess.STDOUT, start_new_session=True)
        self.result.setdefault('application_pids', []).append(self.process.pid)
        self.event('launch', pid=self.process.pid, path=path.name)
        self.activate()
        self.live()

    def open_document(self, doc, *, initial=False):
        before = set() if initial else self.window_ids()
        if not initial:
            self.event('audit_boundary', offset=self.tail.drain())
            subprocess.run(['open', '-a', str(self.app), str(doc.path)], check=True, timeout=15)
        self.expect(doc.source, timeout=20)
        deadline = time.monotonic() + 10
        while True:
            added = self.window_ids() - before
            if len(added) == 1:
                doc.window = added.pop()
                break
            require(time.monotonic() < deadline, 'Opening a document did not create exactly one identified window')
            time.sleep(.1)
        require(doc.window not in {d.window for d in self.documents.values()}, 'Reused an open document window')
        self.documents[doc.index] = doc
        self.focus(doc)
        self.event('document_open', document=doc.index, window=doc.window, source=text_identity(doc.source))

    def focus(self, doc):
        self.live()
        require(self.window_ids() == {d.window for d in self.documents.values()}, 'Unexpected document window set')
        self.run('raise-window', doc.window)
        self.expect(doc.source)

    def await_frame(self, doc, stage, history=None):
        deadline = time.monotonic() + 20
        candidate = None
        while time.monotonic() < deadline:
            self.live()
            for record in self.tail.poll():
                if matches_frame(record, doc.source, doc.revision):
                    required_history = (0, 0) if doc.revision < 0 else history
                    if required_history is not None and (record['undo_entries'], record['redo_entries']) != required_history:
                        continue
                    candidate = record
                    if settled(record, stage):
                        self.expect(doc.source)
                        doc.revision, doc.audit = record['revision'], record
                        doc.audit_seconds = time.monotonic() - self.started
                        self.event('settled_frame', document=doc.index, stage=stage,
                                   source=text_identity(doc.source), counters=record)
                        return
            time.sleep(.1)
        self.event('frame_timeout', document=doc.index, stage=stage, last_candidate=candidate)
        raise AssertionError('No newer settled frame for the exact controlled document')

    def install(self, doc, text, stage):
        self.focus(doc)
        self.run('select', 0, len(doc.source.encode('utf-16-le')) // 2)
        self.run('paste-text', text)
        self.expect(text)
        doc.source = text
        self.run('select', 0, 0)
        self.await_frame(doc, stage)

    def save(self, doc):
        self.expect(doc.source)
        self.run('key', 1, 'cmd')
        deadline = time.monotonic() + 5
        while doc.path.read_bytes() != exact_bytes(doc.source):
            require(time.monotonic() < deadline, 'Saved BOM/CRLF source differs')
            time.sleep(.1)

    def sample(self, phase):
        rows = self.live()
        app = json.loads(subprocess.check_output([str(self.probe), str(self.process.pid)], text=True, timeout=10))
        helpers = []
        for pid in rows['helpers']:
            response = subprocess.run([str(self.probe), str(pid)], capture_output=True, text=True, timeout=10)
            if response.returncode:
                require(pid not in self.processes()['helpers'], 'Failed to sample a live helper')
                helpers.append({'pid': pid, 'status': 'exited_during_sample'})
            else:
                helpers.append(json.loads(response.stdout))
        now = time.monotonic() - self.started
        self.summary.add(self.session, phase, now, app)
        audits = [{'document': d.index, 'window': d.window, 'source': text_identity(d.source),
                   'counters_observed_seconds': d.audit_seconds, 'last_settled_counters': d.audit}
                  for d in self.documents.values()]
        self.event('memory', phase=phase, open_documents=len(self.documents), app=app,
                   helpers=helpers, documents=audits)
        self.progress()

    def idle(self, phase):
        started = time.monotonic()
        while True:
            self.sample(phase)
            # Consume new diagnostic output incrementally, without forcing a frame.
            self.tail.poll()
            remaining = self.args.idle_seconds - (time.monotonic() - started)
            if remaining <= 0:
                break
            time.sleep(min(self.args.sample_seconds, remaining))
        require(not self.live()['helpers'], 'Helper remained after the existing idle allowance')
        self.event('idle_finished', phase=phase, elapsed_seconds=time.monotonic() - started,
                   history_cleared=False)

    def close_document(self, doc):
        self.focus(doc)
        self.save(doc)
        self.run('key', 13, 'cmd')
        deadline = time.monotonic() + 10
        while doc.window in self.window_ids():
            require(time.monotonic() < deadline, 'Document did not close')
            time.sleep(.1)
        del self.documents[doc.index]
        self.event('document_close', document=doc.index, window=doc.window)
        self.live()  # The plain sentinel must keep this exact process alive.
        self.event('audit_boundary', offset=self.tail.drain())

    def quit(self):
        self.live()
        self.run('key', 12, 'cmd')
        require(self.process.wait(timeout=20) == 0, 'Application did not quit cleanly')
        deadline = time.monotonic() + 10
        while any(self.processes().values()):
            require(time.monotonic() < deadline, 'Isolated helper/application survived clean quit')
            time.sleep(.1)
        self.event('clean_quit', pid=self.process.pid)
        self.log.close()
        self.log = None
        self.documents.clear()

    def stable_bounds(self, location, length):
        previous, repeats = None, 0
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            bounds = self.run('bounds', location, length)['bounds']
            key = tuple(round(bounds[k], 2) for k in ('x', 'y', 'width', 'height'))
            repeats = repeats + 1 if key == previous else 0
            if repeats >= 2:
                return bounds
            previous = key
            time.sleep(.1)
        raise AssertionError('Formula-neighbor geometry did not settle')

    def cleanup(self):
        # Only the process group created by this runner; never kill by app name.
        try:
            if self.process is None:
                return
            rows = self.processes()
            if self.process.poll() is None or rows['helpers']:
                self.result['forced_cleanup'] = True
                self.result['passed'] = False
                owned = [*rows['app'], *rows['helpers']]
                for pid in owned:
                    try:
                        require(os.getpgid(pid) == self.process.pid,
                                'Isolated child left the owned process group; not signalling an unverified group')
                    except ProcessLookupError:
                        pass
                for signum in (signal.SIGTERM, signal.SIGKILL):
                    try:
                        os.killpg(self.process.pid, signum)
                    except ProcessLookupError:
                        pass
                    deadline = time.monotonic() + 5
                    while any(self.processes().values()) and time.monotonic() < deadline:
                        time.sleep(.1)
                    if not any(self.processes().values()):
                        break
                self.process.wait(timeout=5)
            self.result['remaining_isolated_processes'] = self.processes()
            if any(self.result['remaining_isolated_processes'].values()):
                self.result['passed'] = False
        finally:
            if self.log:
                self.log.close()
            self.journal.close()


def prepare(soak, production, build):
    out = soak.out
    subprocess.run(['swiftc', str(ROOT / 'tools/native-event-driver.swift'), '-o', str(soak.driver)], check=True, timeout=180)
    subprocess.run(['clang', '-Wall', '-Wextra', '-Werror', str(ROOT / 'tools/process-footprint.c'), '-o', str(soak.probe)], check=True, timeout=60)
    preflight = subprocess.run([str(soak.driver), '--preflight'], capture_output=True, text=True, timeout=20)
    (out / 'preflight.json').write_text(preflight.stdout)
    (out / 'preflight.stderr.log').write_text(preflight.stderr)
    require(preflight.returncode == 0, 'Unlocked desktop, event, AX and screen-capture permission are required')
    shutil.copytree(production, soak.app)
    info_path = soak.app / 'Contents/Info.plist'
    info = plistlib.loads(info_path.read_bytes())
    identifier = 'io.github.xiaodou997.yu.resource-soak.' + uuid.uuid4().hex
    info['CFBundleIdentifier'] = identifier
    info_path.write_bytes(plistlib.dumps(info))
    subprocess.run(['codesign', '--force', '--sign', '-', '--identifier', identifier, str(soak.app)], check=True, timeout=60)
    original = production / 'Contents/MacOS/Yu'
    require(code_digest(original) == code_digest(soak.binary), 'Isolation changed executable instructions')
    require(digest(soak.helper) == build['helper_sha256'], 'Isolation changed the bundled helper')
    soak.result.update(isolated_bundle=identifier, test_app_sha256=digest(soak.binary),
                       test_helper_sha256=digest(soak.helper), native_driver_sha256=digest(soak.driver))


def exercise(soak):
    import group4_followup
    out, args, result = soak.out, soak.args, soak.result
    sentinel = Document(-1, out / 'sentinel.md', '# Plain sentinel\r\n\r\nKeep this process alive.\r\n')
    sentinel.path.write_bytes(exact_bytes(sentinel.source))
    soak.launch(sentinel.path)
    soak.open_document(sentinel, initial=True)
    soak.await_frame(sentinel, 'plain')
    require(not soak.live()['helpers'], 'Plain sentinel started a helper')
    soak.sample('sentinel_baseline')
    documents = []
    for index in range(args.documents):
        doc = Document(index, out / f'document-{index:02}.md', fixture_source(index, 0, 'plain'))
        doc.path.write_bytes(exact_bytes(doc.source))
        soak.open_document(doc)
        soak.await_frame(doc, 'plain')
        documents.append(doc)
    soak.sample('all_documents_open')
    started, rounds = time.monotonic(), 0
    while time.monotonic() - started < args.seconds:
        rounds += 1
        for doc in documents:
            valid = fixture_source(doc.index, rounds, 'valid')
            bad = fixture_source(doc.index, rounds, 'error')
            soak.install(doc, valid, 'valid')
            soak.sample('churn_valid')
            soak.install(doc, bad, 'error')
            soak.run('key', 6, 'cmd'); soak.expect(valid)
            soak.run('key', 6, 'cmd+shift'); soak.expect(bad)
            soak.sample('churn_error')
            helpers_before = set(soak.live()['helpers'])
            soak.install(doc, fixture_source(doc.index, rounds, 'plain'), 'plain')
            require(set(soak.live()['helpers']) <= helpers_before, 'Plain source started a new helper')
            soak.save(doc)
            soak.sample('churn_plain')
        result['completed_passes'] = rounds
        result['churn_elapsed_seconds'] = time.monotonic() - started
        soak.progress()
    soak.idle('retained_history_idle')
    # Verify original history AFTER idle, before intentionally closing documents.
    for doc in documents:
        soak.focus(doc)
        soak.run('key', 6, 'cmd'); soak.expect(fixture_source(doc.index, rounds, 'error'))
        soak.run('key', 6, 'cmd+shift'); soak.expect(doc.source)
        soak.save(doc)
        soak.event('retained_history_verified', document=doc.index)
    for doc in documents:
        soak.close_document(doc)
        soak.sample('closing_documents')
    soak.focus(sentinel)
    soak.idle('closed_documents_idle')
    # Same PID, fresh document ownership; this is not the later cold process.
    for old in documents:
        require(old.path.read_bytes() == exact_bytes(old.source), 'Closed file bytes changed')
        doc = Document(old.index, old.path, old.source)
        soak.open_document(doc)
        soak.await_frame(doc, 'plain')
        plain = doc.source
        # A distinct length separates this lifetime's resource frame from all
        # prior churn records even though a reopened editor restarts revision.
        warm = fixture_source(doc.index, rounds + 1, 'valid') + 'WARM-RESOURCE-EPOCH\r\n'
        soak.install(doc, warm, 'valid')
        soak.sample('same_process_resource_reopen')
        soak.run('key', 6, 'cmd'); soak.expect(plain)
        doc.source = plain
        soak.await_frame(doc, 'plain', history=(0, 1))
        soak.close_document(doc)
    soak.install(sentinel, COLD_SOURCE, 'cold')
    soak.save(sentinel)
    old_pid = soak.process.pid
    soak.quit()
    result['same_process_phases_completed'] = True
    soak.session = 'cold_reopen'
    soak.launch(sentinel.path)
    require(soak.process.pid != old_pid, 'Cold reopen did not produce a new PID')
    sentinel = Document(-1, sentinel.path, COLD_SOURCE)
    soak.open_document(sentinel, initial=True)
    soak.await_frame(sentinel, 'cold')
    soak.sample('cold_resource_reopen')
    # Never resize, scroll, edit, or focus the formula to obtain a passing image.
    checker = group4_followup.Checks(soak.run, soak.stable_bounds, out, sentinel.path, result)
    result['cold_pixel_oracle_started'] = True
    checker.restored_preview(COLD_SOURCE)
    soak.run('select', len(COLD_SOURCE.encode('utf-16-le')) // 2, 0)
    soak.run('paste-text', 'REOPEN EDIT 中文🙂\r\n')
    changed = COLD_SOURCE + 'REOPEN EDIT 中文🙂\r\n'
    soak.expect(changed)
    soak.run('key', 6, 'cmd'); soak.expect(COLD_SOURCE)
    soak.run('key', 6, 'cmd+shift'); soak.expect(changed)
    sentinel.source = changed
    soak.save(sentinel)
    soak.quit()
    result['passed'] = True
    result['scope'] = 'This finite native soak only; not Group 4 signoff or a leak verdict'


def main(argv=None):
    args = parse_args(argv)
    if sys.platform != 'darwin':
        raise SystemExit('Native resource soak requires macOS; no desktop actions were performed')
    production = HERE / '.build/Yu.app'
    build = json.loads((HERE / '.build/build-manifest.json').read_text())
    require(build.get('configuration') == 'release', 'An audited release build is required')
    require(digest(production / 'Contents/MacOS/Yu') == build['app_sha256'], 'Production application hash differs')
    require(digest(production / 'Contents/Helpers/yu-document-renderer') == build['helper_sha256'], 'Production helper hash differs')
    # Invalid parameters/platform/build never create the output directory.
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    result = {'schema_version': 1, 'passed': False, 'group4_signoff': False, 'checks': [],
              'build': build, 'options': {k: str(v) if isinstance(v, Path) else v for k, v in vars(args).items()},
              'memory_verdict': 'not_determined', 'real_calendar_events': 'not_run',
              'complete_selection_matrix': 'not_run', 'visual_review_required': True,
              'test_runner_sha256': digest(Path(__file__)),
              'accounting_module_sha256': digest(HERE / 'group4_resource_soak.py'),
              'pixel_oracle_module_sha256': digest(HERE / 'group4_followup.py')}
    soak = NativeSoak(args, out, result)
    exit_code = 1
    try:
        soak.progress()
        prepare(soak, production, build)
        exercise(soak)
        exit_code = 0
    except (Exception, KeyboardInterrupt) as error:
        result['passed'] = False
        result['error'] = f'{type(error).__name__}: {error}'
        (out / 'failure.log').write_text(traceback.format_exc())
        exit_code = 130 if isinstance(error, KeyboardInterrupt) else 1
    finally:
        try:
            soak.cleanup()
            require(digest(production / 'Contents/MacOS/Yu') == build['app_sha256'], 'Production executable changed')
            require(digest(production / 'Contents/Helpers/yu-document-renderer') == build['helper_sha256'], 'Production helper changed')
        except Exception as error:
            result['passed'] = False
            result['cleanup_error'] = f'{type(error).__name__}: {error}'
        for name in ('events.jsonl', 'app.log'):
            if (out / name).exists():
                result[name + '_sha256'] = digest(out / name)
        soak.progress()
    print(json.dumps({'passed': result['passed'], 'result': str(out / 'result.json'),
                      'memory_verdict': result['memory_verdict']}, ensure_ascii=False))
    return exit_code if result['passed'] or exit_code else 1


if __name__ == '__main__':
    raise SystemExit(main())
