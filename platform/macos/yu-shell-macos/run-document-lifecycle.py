#!/usr/bin/env python3
"""Verify native document workflows in an isolated bundle and private fixtures.

Panels use injected user decisions. Crash recovery is tested by SIGKILL after
an atomic ready record, followed by a new process. Real IME remains separate.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import time
import uuid

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def code_digest(binary):
    commands = subprocess.check_output(['xcrun', 'otool', '-l', str(binary)], text=True)
    match = re.search(r'sectname __text\s+segname __TEXT\s+addr \S+\s+size (0x[0-9a-fA-F]+)\s+offset (\d+)', commands)
    if not match:
        raise RuntimeError('Cannot verify executable code identity')
    size, offset = int(match[1], 16), int(match[2])
    return hashlib.sha256(binary.read_bytes()[offset:offset + size]).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--case', choices=['all', 'lifecycle', 'crash'], default='all')
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    source = HERE / '.build/Yu.app'
    build = json.loads((HERE / '.build/build-manifest.json').read_text())
    binary = source / 'Contents/MacOS/Yu'
    if build['configuration'] != 'release' or digest(binary) != build['app_sha256']:
        raise RuntimeError('A matching audited release build is required')
    app = out / 'YuLifecycleChecks.app'
    shutil.copytree(source, app)
    info_path = app / 'Contents/Info.plist'
    info = plistlib.loads(info_path.read_bytes())
    identifier = 'io.github.xiaodou997.yu.lifecycle-check.' + uuid.uuid4().hex
    info['CFBundleIdentifier'] = identifier
    info_path.write_bytes(plistlib.dumps(info, sort_keys=False))
    subprocess.run(['codesign', '--force', '--sign', '-', '--identifier', identifier, str(app)], check=True)
    executable = app / 'Contents/MacOS/Yu'
    if code_digest(executable) != code_digest(binary):
        raise RuntimeError('Isolation changed executable instructions')
    subprocess.run(['python3', str(ROOT / 'tools/verify-macos-app.py'), str(app),
                    '--configuration', 'release', '--output', str(out / 'test-build-manifest.json')], check=True)
    (out / 'production-build-manifest.json').write_text(json.dumps(build, indent=2))
    (out / 'isolation.json').write_text(json.dumps({'test_bundle_id': identifier,
        'production_app_sha256': build['app_sha256'], 'test_app_sha256': digest(executable),
        'matching_text_section_sha256': code_digest(executable)}, indent=2))
    base_env = {k: v for k, v in os.environ.items() if not k.startswith('YU_')}
    results = {}

    def environment(directory, result_name):
        env = dict(base_env)
        env['YU_DOCUMENT_STATE_DIR'] = str(directory / 'state')
        env['YU_PRESENTATION_STATE_DIR'] = str(directory / 'columns')
        env['YU_DOCUMENT_CHECK_RESULT'] = str(directory / result_name)
        return env

    def launch(name, directory, flag, fixture, dark=False):
        result_path = directory / f'{name}.json'
        command = [str(executable), flag, str(fixture)] + (['--dark-mode-self-check'] if dark else [])
        log = directory / f'{name}.log'
        with log.open('w') as stream:
            try:
                code = subprocess.run(command, env=environment(directory, result_path.name),
                    stdout=stream, stderr=subprocess.STDOUT, timeout=120).returncode
            except subprocess.TimeoutExpired:
                code = 124
        result = {'exit_code': code, 'passed': code == 0 and result_path.exists(), 'log_sha256': digest(log)}
        if result_path.exists():
            result['measurement'] = json.loads(result_path.read_text())
            result['passed'] = result['passed'] and result['measurement'].get('passed', False)
        results[str(directory.name) + '/' + name] = result
        (out / 'results.json').write_text(json.dumps(results, ensure_ascii=False, indent=2))
        print(directory.name, name, 'PASS' if result['passed'] else 'FAIL', flush=True)
        return result['passed']

    if args.case in ('all', 'lifecycle'):
        for dark in (False, True):
            directory = out / ('dark' if dark else 'light')
            directory.mkdir()
            fixture = directory / 'original.md'
            original = b'\xef\xbb\xbf' + '原稿\r\n'.encode()
            fixture.write_bytes(original)
            if not launch('lifecycle', directory, '--document-lifecycle-self-check', fixture, dark):
                break
            if fixture.read_bytes() != original:
                raise RuntimeError('Lifecycle modified the original fixture')
            if not launch('recent-reader', directory, '--document-recent-reader-self-check', fixture, dark):
                break

    if args.case in ('all', 'crash'):
        directory = out / 'crash'
        directory.mkdir()
        fixture = directory / 'original.md'
        fixture.write_bytes(b'\xef\xbb\xbf' + '原稿\r\n'.encode())
        ready = directory / 'writer-ready.json'
        with (directory / 'writer.log').open('w') as stream:
            process = subprocess.Popen([str(executable), '--document-recovery-writer-self-check', str(fixture)],
                env=environment(directory, ready.name), stdout=stream, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + 60
                while not ready.exists() and process.poll() is None and time.monotonic() < deadline:
                    time.sleep(.1)
                if not ready.exists() or process.poll() is not None:
                    raise RuntimeError('Crash writer did not reach a live, persisted checkpoint')
                checkpoint = json.loads(ready.read_text())
                process.kill()
                exit_code = process.wait(timeout=10)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=10)
        (directory / 'kill.json').write_text(json.dumps({'pid': process.pid, 'exit_code': exit_code,
            'ready_sha256': digest(ready), 'signal': 'SIGKILL'}, indent=2))
        if exit_code != -9:
            raise RuntimeError('Writer was not terminated by SIGKILL')
        Path(checkpoint['named_path']).write_text('外部版本\r\n', encoding='utf-8', newline='')
        Path(checkpoint['deleted_path']).unlink()
        corrupt = directory / 'state/Recovery/000-corrupt.yurecovery'
        corrupt.write_bytes(b'invalid recovery; preserve for diagnostics')
        reader = directory / 'reader.md'
        reader.write_text('reader\n')
        if launch('recovery-reader', directory, '--document-recovery-reader-self-check', reader):
            if Path(checkpoint['named_path']).read_bytes() != '外部版本\r\n'.encode():
                raise RuntimeError('Recovery overwrote externally modified source')
            if Path(checkpoint['deleted_path']).exists() or Path(checkpoint['draft_path']).exists():
                raise RuntimeError('Recovery silently created an original/draft file')
            if corrupt.read_bytes() != b'invalid recovery; preserve for diagnostics':
                raise RuntimeError('Corrupt record was discarded')
            if (directory / 'state/Files/recovered-draft.md').read_bytes() != '未命名恢复中文🙂\n'.encode():
                raise RuntimeError('Recovered draft save bytes mismatch')
            results['crash/post-exit-disk-checks'] = {'passed': True}
            (out / 'results.json').write_text(json.dumps(results, ensure_ascii=False, indent=2))
    if digest(binary) != build['app_sha256']:
        raise RuntimeError('Production executable changed during checks')
    return 0 if results and all(value['passed'] for value in results.values()) else 1


if __name__ == '__main__':
    raise SystemExit(main())
