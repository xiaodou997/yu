"""5A named native history/failure contracts and core resource budgets."""
from __future__ import annotations
import json
import subprocess
from pathlib import Path
from acceptance_runner import ROOT, RunContext, sha, write_json

NATIVE_CASES = ['success-multi', 'success-reverse', 'success-table', 'warning-cancel',
                'write-denied', 'target-changed', 'resource-changed', 'resource-byte-budget']
CORE_CASES = ['frozen_resource_count_is_bounded_before_loading',
              'frozen_resource_single_byte_budget_rejects_sparse_oversize',
              'untitled_relative_base_is_required_and_same_names_stay_distinct',
              'final_checkpoint_cancel_cleans_temp_and_keeps_old_file',
              'target_appearance_parent_swap_and_resource_alias_are_rejected',
              'invalid_style_is_rejected_before_resource_resolution',
              'portable::quota_tests::aggregate_quota_is_checked_before_resource_allocation']


def run_suite(ctx: RunContext) -> int:
    app = ROOT / 'platform/macos/yu-shell-macos/.build/Yu.app'
    binary = app / 'Contents/MacOS/Yu'
    build = json.loads((app.parent / 'build-manifest.json').read_text())
    if build['configuration'] != 'release' or sha(binary.read_bytes()) != build['app_sha256']:
        raise ValueError('Matching audited Release bundle required')
    write_json(ctx.output / 'build-manifest.json', build)
    names = subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard', '--',
        'crates/yu-export', 'crates/yu-storage-ffi', 'platform/macos/yu-shell-macos/Sources',
        'tools/acceptance_suites/group5_safety.py'], cwd=ROOT).decode().split('\0')
    write_json(ctx.output / 'source-lock.json', {
        'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        'source_sha256': {name: sha((ROOT / name).read_bytes()) for name in sorted(set(names)) if name and (ROOT / name).is_file()}})
    core_code = ctx.run('safety-core', ['cargo', 'test', '--locked', '-p', 'yu-export'])
    log = ctx.log('safety-core').read_text()
    for name in CORE_CASES:
        ctx.ledger.add(name, ['core'], metadata={'groups': ['X03', 'X04', 'X08']})
        ctx.ledger.record(name, 'core', 'passed' if core_code == 0 and f'test {name} ... ok' in log else 'failed', 'safety-core')
    native_code = ctx.run('safety-native', [str(binary), '--html-export-safety-self-check', str(ctx.output / 'native')])
    report_path = ctx.output / 'native/report.json'
    report = json.loads(report_path.read_text()) if report_path.exists() else {}
    observed = {row['id']: row for row in report.get('cases', [])}
    for name in NATIVE_CASES:
        ctx.ledger.add(name, ['native'], metadata={'groups': ['X02', 'X03', 'X04', 'X07', 'X08'], 'not_real_window': True})
        row = observed.get(name, {})
        passed = native_code == 0 and report.get('passed') and row.get('passed') and row.get('full_identity_preserved') and row.get('redo_undo_executed') and row.get('continued_edit_undo')
        ctx.ledger.record(name, 'native', 'passed' if passed else 'failed', 'safety-native',
            evidence={'case_result': row, 'report': ctx.artifact(report_path) if report_path.exists() else None})
    rows = ctx.ledger.finish()
    good = all(row['status'] == 'passed' for row in rows)
    write_json(ctx.output / 'reports/cases.json', {'cases': rows, 'groups_closed': 0})
    write_json(ctx.output / 'reports/summary.json', {'passed': good, 'batch_5a': 'partial', 'groups_closed': 0,
        'scope': 'Named resource budgets and native selection/history/failure contracts only'})
    return 0 if good else 1
