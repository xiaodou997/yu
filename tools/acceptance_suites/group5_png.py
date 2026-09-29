"""Actual PNG snapshot/layout/encode/publication through the production bundle."""
from __future__ import annotations
import json
import shutil
import subprocess
from acceptance_runner import ROOT, RunContext, sha, write_json

CASES = {'light-1x', 'light-2x', 'dark-1x', 'split-cancel', 'split-confirm', 'directory-exists', 'source-alias', 'cancel'}
def report_passed(code, report):
    rows = report.get('cases', [])
    return code == 0 and report.get('passed') is True and len(rows) == len(CASES) and {r.get('id') for r in rows} == CASES and all(r.get('passed') is True for r in rows)

def run_suite(ctx: RunContext) -> int:
    shell = ROOT / 'platform/macos/yu-shell-macos'
    binary = shell / '.build/Yu.app/Contents/MacOS/Yu'
    build = json.loads((shell / '.build/build-manifest.json').read_text())
    if build['configuration'] != 'release' or sha(binary.read_bytes()) != build['app_sha256']:
        raise ValueError('Matching audited Release required')
    write_json(ctx.output / 'build-manifest.json', build)
    inputs = ctx.output / 'inputs'; inputs.mkdir()
    shutil.copytree(shell / 'Fixtures/assets', inputs / 'assets')
    shutil.copy2(shell / 'Fixtures/group5-export.md', inputs / 'source.md')
    paths = subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard', '--',
        'Cargo.lock', 'crates/yu-export', 'crates/yu-storage-ffi', 'platform/macos/yu-render-macos', 'platform/macos/yu-shell-macos/Sources'], cwd=ROOT).decode().split('\0')
    write_json(ctx.output / 'source-lock.json', {'source_sha256': {p: sha((ROOT / p).read_bytes()) for p in sorted(set(paths)) if p and (ROOT / p).is_file()}})
    code = ctx.run('native-png', [str(binary), '--png-export-self-check', str(inputs / 'source.md'), str(ctx.output / 'native')])
    path = ctx.output / 'native/report.json'
    report = json.loads(path.read_text()) if path.exists() else {}
    passed = report_passed(code, report)
    write_json(ctx.output / 'reports/summary.json', {'passed': passed, 'groups_closed': 0, 'scope': 'Eight PNG native paths; real menu, visuals, and repeated tasks separate'})
    return 0 if passed else 1
