"""Independent offline file opening of exact native-menu HTML outputs.

Use GROUP5_WINDOW_EVIDENCE to select one completed window evidence directory.
A moved byte-identical copy is used; original evidence files are never removed.
"""
from __future__ import annotations
import json
import os
from pathlib import Path
import shutil

from acceptance_runner import ROOT, RunContext, sha, write_json


def run_suite(ctx: RunContext) -> int:
    selected = os.environ.get('GROUP5_WINDOW_EVIDENCE')
    if not selected:
        raise ValueError('GROUP5_WINDOW_EVIDENCE must name the exact window run')
    source = Path(selected).resolve(strict=True)
    resources = json.loads((source / 'reports/resources.json').read_text())
    producer = resources['samples'][0]['pid']
    menu = json.loads((source / 'reports/menu-output.json').read_text())
    repeated = json.loads((source / 'reports/repeated-outputs.json').read_text())
    untitled = json.loads((source / 'reports/untitled-output.json').read_text())
    if len(repeated) != 10:
        raise ValueError('The selected run has not completed ten outputs')
    specs = [('light-composite', menu, 15, 'GROUP5-DOCUMENT-END', True),
             ('dark-long', repeated[-1], 220, 'SECOND-DOCUMENT-END', False),
             ('untitled-image', untitled, 1, 'UNTITLED-DOCUMENT-END', False)]
    transit = ctx.output / 'transit'; transit.mkdir()
    moved = ctx.output / 'moved'; moved.mkdir()
    cases = []
    for identifier, artifact, count, marker, formulas in specs:
        original = (source / artifact['path']).resolve(strict=True)
        if not original.is_relative_to(source):
            raise ValueError('Output path escapes the selected evidence directory')
        payload = original.read_bytes()
        if sha(payload) != artifact['sha256']:
            raise ValueError('Original output no longer matches native evidence')
        copy = transit / (identifier + '.html')
        copy.write_bytes(payload)
        destination = moved / (identifier + ' 分享.html')
        copy.rename(destination)
        cases.append({'id': identifier, 'path': str(destination), 'sha256': artifact['sha256'],
                      'images': count, 'marker': marker, 'formulas': formulas})
        ctx.ledger.add(identifier, ['cold_reopen', 'visual'], metadata={
            'groups': ['H01', 'H02', 'H03', 'H04'],
            'scope': 'External Chrome direct file URL; screenshot review is separate'})
    manifest = ctx.output / 'browser-inputs.json'
    write_json(manifest, {'producer_pid': producer, 'cases': cases})
    write_json(ctx.output / 'build-manifest.json', json.loads((source / 'build-manifest.json').read_text()))
    node = shutil.which('node')
    if not node:
        raise RuntimeError('A test-side Node.js with built-in WebSocket is required')
    code = ctx.run('browser-file-offline', [node, str(ROOT / 'tools/check-group5-browser.mjs'),
                                          str(manifest), str(ctx.output / 'viewer')])
    report_path = ctx.output / 'viewer/report.json'
    report = json.loads(report_path.read_text()) if report_path.exists() else {}
    observed = {case['id']: case for case in report.get('cases', [])}
    for case in cases:
        passed = code == 0 and report.get('producer_exited') and observed.get(case['id'], {}).get('passed')
        ctx.ledger.record(case['id'], 'cold_reopen', 'passed' if passed else 'failed',
                          'browser-file-offline', evidence={
                              'html': ctx.artifact(Path(case['path'])),
                              'report': ctx.artifact(report_path) if report_path.exists() else None})
    write_json(ctx.output / 'reports/cases.json', {'cases': ctx.ledger.finish(), 'groups_closed': 0})
    write_json(ctx.output / 'reports/summary.json', {'automation_passed': code == 0,
        'visual_review': 'pending', 'groups_closed': 0, 'batch_5a': 'partial'})
    return code
