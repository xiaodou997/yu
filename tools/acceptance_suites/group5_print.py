"""Print software chain only: system save job, snapshot and guarded publication.
Never sends physical jobs. Evidence stays in a new local output directory.
"""
from __future__ import annotations
import json
import shutil
import subprocess
from pathlib import Path
from acceptance_runner import ROOT, RunContext, sha, write_json


def report_passed(code: int, report: dict) -> bool:
    required = {'whole', 'range', 'landscape', 'source-alias', 'image-alias', 'write-denied', 'target-change', 'publication-cancel'}
    rows = report.get('cases', [])
    return (code == 0 and report.get('passed') is True
            and report.get('physical_jobs_submitted') == 0
            and len(rows) == len(required) and {r.get('id') for r in rows} == required
            and all(r.get('passed') is True for r in rows))


def run_suite(ctx: RunContext) -> int:
    shell = ROOT / 'platform/macos/yu-shell-macos'
    binary = shell / '.build/Yu.app/Contents/MacOS/Yu'
    build = json.loads((shell / '.build/build-manifest.json').read_text())
    if build['configuration'] != 'release' or sha(binary.read_bytes()) != build['app_sha256']:
        raise ValueError('Audited matching Release required')
    write_json(ctx.output / 'build-manifest.json', build)
    inputs = ctx.output / 'inputs'; inputs.mkdir()
    shutil.copytree(shell / 'Fixtures/assets', inputs / 'assets')
    shutil.copy2(shell / 'Fixtures/group5-export.md', inputs / 'source.md')
    paths = subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard', '--',
        'Cargo.lock', 'crates/yu-export', 'crates/yu-storage-ffi', 'platform/macos/yu-render-macos',
        'platform/macos/yu-shell-macos/Sources', 'tools/acceptance_suites/group5_print.py'], cwd=ROOT).decode().split('\0')
    write_json(ctx.output / 'source-lock.json', {'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        'source_sha256': {p: sha((ROOT / p).read_bytes()) for p in sorted(set(paths)) if p and (ROOT / p).is_file()}})
    code = ctx.run('native-print', [str(binary), '--printing-self-check', str(inputs / 'source.md'), str(ctx.output / 'native')])
    path = ctx.output / 'native/report.json'
    report = json.loads(path.read_text()) if path.exists() else {}
    passed = report_passed(code, report)
    write_json(ctx.output / 'reports/summary.json', {'passed': passed, 'groups_closed': 0, 'scope': 'Eight native software print paths; actual panel and visuals separate'})
    return 0 if passed else 1
