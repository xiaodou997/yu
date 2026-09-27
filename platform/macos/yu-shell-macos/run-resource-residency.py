#!/usr/bin/env python3
"""Bounded same-process document lifetimes, separate from the 30-minute soak.

One untouched sentinel keeps the process alive. Every lifetime preserves its
original undo history through idle, verifies it, closes normally, and samples
the same sentinel-only state. A finite plateau is not a no-leak verdict.
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import traceback

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
spec = importlib.util.spec_from_file_location('yu_residency_soak', HERE / 'run-resource-soak.py')
native = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = native
spec.loader.exec_module(native)
from group4_resource_soak import COLD_SOURCE, Options, digest, exact_bytes, fixture_source, require, same_process_identity


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--documents', type=int, default=4)
    parser.add_argument('--lifetimes', type=int, default=4, help='2..8 complete open/edit/idle/close/idle cycles')
    parser.add_argument('--passes', type=int, default=3, help='1..10 identical fixed passes per lifetime')
    parser.add_argument('--idle-seconds', type=int, default=70)
    parser.add_argument('--sample-seconds', type=int, default=5)
    parser.add_argument('--dark', action='store_true')
    parser.add_argument('--allocation-stacks', action='store_true', help='Separate intrusive experiment, never baseline memory')
    parser.add_argument('--history-audit', action='store_true', help='Native history receipts plus bounded before/after source/focus identities; diagnostic only')
    args = parser.parse_args(argv)
    try:
        Options(args.documents, 30, args.idle_seconds, args.sample_seconds).validate()
        for key, lower, upper in [('lifetimes', 2, 8), ('passes', 1, 10)]:
            if not lower <= getattr(args, key) <= upper:
                raise ValueError(f'{key} must be {lower}..{upper}')
    except ValueError as error:
        parser.error(str(error))
    return args


def boundary_identity(state):
    """No source, selected text, window title or path in the receipt journal."""
    window = state.get('focused_window_identifier')
    value = state.get('AXValue')
    return {'frontmost_pid': state.get('frontmost_pid'), 'ax_frontmost': state.get('AXFrontmost'),
            'focused_role': state.get('focused_role'), 'editor_focused': state.get('AXFocused'),
            'window_identity': native.text_identity(window) if isinstance(window, str) else None,
            'source_identity': native.text_identity(value) if isinstance(value, str) else None}


class Residency(native.NativeSoak):
    def run(self, *arguments):
        history_key = (len(arguments) >= 3 and arguments[0] == 'key' and arguments[1] == 6)
        if not (self.args.history_audit and history_key):
            return super().run(*arguments)
        count = self.result.get('history_input_keys', 0)
        require(count < 1024, 'History input boundary budget exhausted')
        self.result['history_input_keys'] = count + 1
        before = super().run('snapshot')
        self.event('history_input_boundary', ticket=count + 1, when='before',
                   modifiers=arguments[2], state=boundary_identity(before))
        result = super().run(*arguments)
        after = super().run('snapshot')
        self.event('history_input_boundary', ticket=count + 1, when='after',
                   modifiers=arguments[2], state=boundary_identity(after))
        return result

    def install(self, doc, text, stage):
        self.focus(doc)
        self.run('select', 0, len(doc.source.encode('utf-16-le')) // 2)
        receipt = self.run('paste-document', text)
        require(isinstance(receipt, dict) and receipt.get('verified_document_paste') is True,
                'Whole-document paste lacks a consumption receipt')
        self.event('verified_document_paste', receipt=receipt)
        self.expect(text)
        doc.source = text
        self.run('select', 0, 0)
        self.await_frame(doc, stage)

    def application_environment(self):
        env = super().application_environment()
        removed = sorted(k for k in env if k.startswith('Malloc') or k in (
            'DYLD_INSERT_LIBRARIES', 'NSZombieEnabled', 'NSDeallocateZombies',
            'NSAutoreleaseFreedObjectCheckEnabled', 'YU_HISTORY_AUDIT'))
        for key in removed:
            del env[key]
        self.result['removed_child_environment_keys'] = removed
        if self.args.allocation_stacks:
            env['MallocStackLogging'] = '1'
        if self.args.history_audit:
            env['YU_HISTORY_AUDIT'] = '1'
        return env

    def capture_allocations(self, phase):
        """No content capture; tools are sequential, not an atomic census."""
        require(self.args.allocation_stacks, 'Intrusive tools are forbidden in baseline runs')
        self.live()
        pid = self.process.pid
        identity = dict(self.process_identities[pid])
        commands = [('heap', ['xcrun', 'heap', '-sortBySize', '-noContent', str(pid)]),
                    ('stacks', ['xcrun', 'malloc_history', str(pid), '-allBySize']),
                    ('leaks', ['xcrun', 'leaks', '-noContent', str(pid)]),
                    ('vmmap', ['vmmap', '-summary', str(pid)])]
        for tool, command in commands:
            self.live()
            require(same_process_identity(identity, self.process_identities.get(pid)), 'Capture target changed')
            target = self.out / f'{phase}-{tool}.txt'
            started = time.monotonic()
            with target.open('x') as stream:
                try:
                    response = subprocess.run(command, stdout=stream, stderr=subprocess.STDOUT, timeout=90)
                    status = {'exit_code': response.returncode}
                except subprocess.TimeoutExpired:
                    status = {'timeout': True}
            self.live()
            require(same_process_identity(identity, self.process_identities.get(pid)), 'Capture target changed')
            record = dict(phase=phase, tool=tool, identity=identity, file=target.name,
                          elapsed_seconds=time.monotonic() - started, sha256=digest(target), **status)
            self.result.setdefault('allocation_captures', []).append(record)
            self.event('allocation_capture', **record)
            self.progress()
            allowed = (0, 1) if tool == 'leaks' else (0,)
            require(status.get('exit_code') in allowed, 'Allocation tool failed; evidence retained')
        # A leaks exit 1 stays a nonzero scan, even when native interactions pass.
        self.result['leaks_scans_all_zero'] = all(item.get('exit_code') == 0
            for item in self.result['allocation_captures'] if item['tool'] == 'leaks')


def lifetime(soak, sentinel, old_documents, number):
    """Identical workload; reopen real saved bytes, never clear live history."""
    args = soak.args
    prefix = f'lifetime_{number:02}'
    documents = []
    for old in old_documents:
        require(old.path.read_bytes() == exact_bytes(old.source), 'Closed document bytes changed')
        doc = native.Document(old.index, old.path, old.source)
        soak.open_document(doc)
        soak.await_frame(doc, 'plain')
        documents.append(doc)
    soak.sample(prefix + '_opened')
    for generation in range(1, args.passes + 1):
        for doc in documents:
            valid, bad = (fixture_source(doc.index, generation, stage) for stage in ('valid', 'error'))
            soak.install(doc, valid, 'valid')
            soak.sample(prefix + '_valid')
            soak.install(doc, bad, 'error')
            soak.run('key', 6, 'cmd'); soak.expect(valid)
            soak.run('key', 6, 'cmd+shift'); soak.expect(bad)
            soak.install(doc, fixture_source(doc.index, generation, 'plain'), 'plain')
            soak.save(doc)
            soak.sample(prefix + '_plain')
    soak.idle(prefix + '_retained_idle')
    for doc in documents:
        soak.focus(doc)
        require((doc.audit['undo_entries'], doc.audit['redo_entries']) == (3 * args.passes, 0),
                'Original lifetime history count differs')
        soak.run('key', 6, 'cmd'); soak.expect(fixture_source(doc.index, args.passes, 'error'))
        soak.run('key', 6, 'cmd+shift'); soak.expect(doc.source)
        soak.save(doc)
        soak.event('retained_history_verified', lifetime=number, document=doc.index,
                   undo_entries=doc.audit['undo_entries'], redo_entries=doc.audit['redo_entries'])
    for doc in documents:
        soak.close_document(doc)
    soak.focus(sentinel)
    soak.expect(sentinel.source)
    require(soak.window_ids() == {sentinel.window}, 'Not the same sentinel-only window state')
    soak.idle(prefix + '_closed_idle')
    soak.result['completed_lifetimes'] = number
    soak.event('lifetime_complete', lifetime=number, sentinel_source=native.text_identity(sentinel.source))
    soak.progress()
    return documents


def exercise(soak):
    import group4_followup
    sentinel = native.Document(-1, soak.out / 'sentinel.md', '# Plain sentinel\r\n\r\nKeep this process alive.\r\n')
    sentinel.path.write_bytes(exact_bytes(sentinel.source))
    soak.launch(sentinel.path)
    soak.open_document(sentinel, initial=True)
    soak.await_frame(sentinel, 'plain')
    require(not soak.live()['helpers'], 'Plain sentinel started a helper')
    identity = dict(soak.process_identities[soak.process.pid])
    soak.result['retained_process_identity'] = identity
    soak.sample('sentinel_cold_baseline')
    documents = []
    for index in range(soak.args.documents):
        doc = native.Document(index, soak.out / f'document-{index:02}.md', fixture_source(index, 0, 'plain'))
        doc.path.write_bytes(exact_bytes(doc.source))
        documents.append(doc)
    for number in range(1, soak.args.lifetimes + 1):
        documents = lifetime(soak, sentinel, documents, number)
        soak.live()
        require(same_process_identity(identity, soak.process_identities.get(soak.process.pid)),
                'Lifetimes did not retain the same process birth identity')
        if soak.args.allocation_stacks and number in (1, soak.args.lifetimes):
            soak.capture_allocations(f'lifetime_{number:02}_closed')
    soak.result['same_process_lifetimes_completed'] = True
    # Final cold check is not used to lower the same-process residency baseline.
    soak.install(sentinel, COLD_SOURCE, 'cold')
    soak.save(sentinel)
    old_pid = soak.process.pid
    soak.quit()
    soak.session = 'cold_reopen'
    soak.launch(sentinel.path)
    require(soak.process.pid != old_pid, 'Cold reopen did not create a new PID')
    sentinel = native.Document(-1, sentinel.path, COLD_SOURCE)
    soak.open_document(sentinel, initial=True)
    soak.await_frame(sentinel, 'cold')
    soak.sample('cold_resource_reopen')
    checker = group4_followup.Checks(soak.run, soak.stable_bounds, soak.out, sentinel.path, soak.result)
    soak.result['cold_pixel_oracle_started'] = True
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
    soak.result['passed'] = True


def main(argv=None):
    args = parse_args(argv)
    if sys.platform != 'darwin':
        raise SystemExit('Native residency requires macOS; no desktop actions performed')
    production = HERE / '.build/Yu.app'
    build = json.loads((HERE / '.build/build-manifest.json').read_text())
    require(build.get('configuration') == 'release', 'Audited Release required')
    for path, key in [('Contents/MacOS/Yu', 'app_sha256'), ('Contents/Helpers/yu-document-renderer', 'helper_sha256')]:
        require(digest(production / path) == build[key], 'Product identity differs')
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    result = dict(schema_version=1, passed=False, group4_signoff=False, checks=[], build=build,
        options={k: str(v) if isinstance(v, Path) else v for k, v in vars(args).items()},
        memory_verdict='not_determined', real_calendar_events='not_run', complete_selection_matrix='not_run',
        visual_review_required=True, baseline_comparable=not (args.allocation_stacks or args.history_audit),
        scope='Finite fixed-load lifetimes, not the 30-minute soak or proof of no leaks.',
        source_hashes={p.name:digest(p) for p in [Path(__file__), HERE/'run-resource-soak.py',
            HERE/'group4_resource_soak.py', HERE/'group4_followup.py', HERE/'group4_history_audit.py']})
    soak = Residency(args, out, result)
    code = 1
    try:
        soak.progress()
        native.prepare(soak, production, build)
        exercise(soak)
        code = 0
    except (Exception, KeyboardInterrupt) as error:
        result.update(passed=False, error=f'{type(error).__name__}: {error}')
        (out/'failure.log').write_text(traceback.format_exc())
        code = 130 if isinstance(error, KeyboardInterrupt) else 1
    finally:
        try:
            soak.cleanup()
            for path, key in [('Contents/MacOS/Yu', 'app_sha256'), ('Contents/Helpers/yu-document-renderer', 'helper_sha256')]:
                require(digest(production/path) == build[key], 'Product changed during run')
            for name, expected in result['source_hashes'].items():
                require(digest(HERE/name) == expected, 'Covered source changed during run')
        except Exception as error:
            result.update(passed=False, cleanup_error=f'{type(error).__name__}: {error}')
        if args.history_audit and (out/'app.log').exists():
            from group4_history_audit import summarize_history
            try:
                with (out/'app.log').open() as stream:
                    result['history_audit'] = summarize_history(stream, expected_pids=result.get('application_pids', []))
                result['history_audit_scope'] = 'Pair integrity of recorded product receipts only; absence is not proof of non-delivery.'
            except Exception as error:
                result['history_audit_error'] = str(error)
                result['passed'] = False
        for name in ('events.jsonl', 'app.log'):
            if (out/name).exists():
                result[name+'_sha256'] = digest(out/name)
        soak.progress()
    print(json.dumps({'passed':result['passed'], 'result':str(out/'result.json'),
                      'completed_lifetimes':result.get('completed_lifetimes',0),
                      'memory_verdict':result['memory_verdict']}))
    return code if result['passed'] or code else 1


if __name__ == '__main__':
    raise SystemExit(main())
