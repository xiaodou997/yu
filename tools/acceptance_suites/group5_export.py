"""5A targeted HTML evidence. A successful run does NOT close all 24 groups.

Reuse the global runner. Native calls use the product's bundled executable;
artifact validation is named explicitly and is never labelled visual evidence.
"""
from __future__ import annotations
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
from acceptance_runner import ROOT, RunContext, sha, write_json

APP = ROOT / 'platform/macos/yu-shell-macos/.build/Yu.app'
FIXTURES = ROOT / 'platform/macos/yu-shell-macos/Fixtures'
GROUPS = [f'X{i:02d}' for i in range(1, 9)] + [f'H{i:02d}' for i in range(1, 5)] + [f'F{i:02d}' for i in range(1, 7)] + [f'T{i:02d}' for i in range(1, 4)] + [f'I{i:02d}' for i in range(1, 4)]
CORE_CASES = {
    'html_whole_semantics_and_clipboard_contract_remain_separate': ['H01'],
    'html_active_content_has_readable_fallback_without_execution': ['X08', 'H04'],
    'html_details_frontmatter_and_duplicate_headings': ['X06', 'H02'],
    'html_equation_numbering_and_static_resources_come_from_yu': ['X05'],
    'html_table_math_and_mixed_repeated_footnotes': ['H01', 'H02'],
    'html_local_images_are_data_and_remote_images_warn': ['X04', 'H03', 'H04'],
    'html_lf_crlf_utf8_identity_and_cancel': ['X01', 'X02'],
    'destination_cancel_and_failure_keep_old_output': ['X03', 'X07'],
    'destination_protects_source_resource_and_aliases': ['X03'],
    'image_snapshot_is_fixed_and_change_is_detected': ['X04'],
    'svg_security_rejects_active_and_external_content': ['X08'],
    'destination_commit_gate_preserves_old_file_on_rejection': ['X03', 'X07'],
}

def run_suite(ctx: RunContext) -> int:
    out = ctx.output
    inputs = out / 'inputs'; inputs.mkdir()
    outputs = out / 'outputs'; outputs.mkdir()
    images = inputs / '中文 图片'; images.mkdir()
    shutil.copy2(FIXTURES / 'assets/yu-mark.png', images / 'yu 标志.png')
    source = (FIXTURES / 'group5-export.md').read_text(encoding='utf-8').replace('assets/yu-mark.png', '中文%20图片/yu%20标志.png')
    (inputs / 'composite.md').write_text(source, encoding='utf-8')
    (inputs / 'composite-bom-crlf.md').write_bytes(b'\xef\xbb\xbf' + source.replace('\n', '\r\n').encode())
    shutil.copy2(FIXTURES / 'group5-export-warning.md', inputs / 'warning.md')
    binary = APP / 'Contents/MacOS/Yu'
    build = {str(path.relative_to(APP)): sha(path.read_bytes()) for path in [binary, APP / 'Contents/Helpers/yu-document-renderer', APP / 'Contents/Resources/yu_shaders.metallib']}
    write_json(out / 'build-manifest.json', {'app_files_sha256': build, 'minimum_os': '26.0', 'note': 'Existing bundle; source/build identity must be confirmed before accepting a changed-source rerun.'})
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    diff = subprocess.check_output(['git', 'diff', '--binary'], cwd=ROOT)
    # git diff alone omits the new, untracked implementation before its first
    # commit. Bind every in-scope source byte, but never read unrelated files.
    candidate_names = subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard', '--',
        'Cargo.toml', 'Cargo.lock', 'crates/yu-export', 'crates/yu-markdown', 'crates/yu-storage-ffi',
        'platform/macos/yu-render-macos', 'platform/macos/yu-shell-macos/Sources',
        'tools/acceptance_suites/group5_export.py', 'tools/check-group5-html.py'], cwd=ROOT).decode().split('\0')
    write_json(out / 'source-lock.json', {'head': head, 'tracked_diff_sha256': sha(diff),
        'candidate_source_sha256': {name: sha((ROOT / name).read_bytes()) for name in sorted(set(candidate_names)) if name and (ROOT / name).is_file()},
        'input_sha256': {str(path.relative_to(inputs)): sha(path.read_bytes()) for path in inputs.rglob('*') if path.is_file()}})
    core_code = ctx.run('html-core', ['cargo', 'test', '--locked', '-p', 'yu-export'])
    core_log = ctx.log('html-core').read_text()
    for test, groups in CORE_CASES.items():
        identifier = 'core/' + test
        ctx.ledger.add(identifier, ['core'], metadata={'groups': groups, 'scope': 'named core contract; mock resource payloads are not rendered output proof'})
        passed = core_code == 0 and f'test {test} ... ok' in core_log
        ctx.ledger.record(identifier, 'core', 'passed' if passed else 'failed', 'html-core', evidence={'test_name': test})
    scenarios = [
        ('light', 'composite.md', [], ['X01', 'X02', 'X04', 'X05', 'X06', 'H01', 'H02', 'H03']),
        ('dark-bom-crlf', 'composite-bom-crlf.md', ['--export-dark'], ['X01', 'X02', 'H04']),
        ('history', 'composite.md', ['--export-history'], ['X02']),
        ('warnings', 'warning.md', ['--export-warnings'], ['X04', 'X05', 'X08', 'H04']),
        ('cancel', 'composite.md', ['--export-cancel'], ['X02', 'X03', 'X07']),
        ('retry', 'composite.md', [], ['X07']),
    ]
    for name, fixture, flags, groups in scenarios:
        destination = outputs / f'{name}.html'
        if name == 'cancel': destination.write_text('OLD-OUTPUT-KEEP', encoding='utf-8')
        command = 'native-' + name
        code = ctx.run(command, [str(binary), '--html-export-self-check', str(inputs / fixture), str(destination), *flags])
        report_path = Path(str(destination) + '.report.json')
        report = json.loads(report_path.read_text()) if report_path.exists() else {}
        identifier = 'native/' + name
        ctx.ledger.add(identifier, ['native'], metadata={'groups': groups, 'scope': 'native production task, no menu/real-window assertion'})
        evidence = {'report': ctx.artifact(report_path)} if report_path.exists() else {}
        ctx.ledger.record(identifier, 'native', 'passed' if code == 0 and report.get('passed') else 'failed', command, evidence=evidence)
        if code == 0 and name != 'cancel':
            check_name = 'artifact-' + name
            check_report = out / 'reports' / f'{check_name}.json'
            check = [sys.executable, str(ROOT / 'tools/check-group5-html.py'), str(destination), str(check_report)]
            if name == 'warnings': check.append('--warnings')
            check_code = ctx.run(check_name, check)
            ctx.ledger.add('artifact/' + name, ['native'], metadata={'groups': ['H01', 'H02', 'H03', 'H04'], 'scope': 'artifact structure/bytes; not independent visual inspection'})
            ctx.ledger.record('artifact/' + name, 'native', 'passed' if check_code == 0 else 'failed', check_name,
                              evidence={'html': ctx.artifact(destination), 'structure_report': ctx.artifact(check_report)})
    rows = ctx.ledger.finish()
    group_rows = []
    for group in GROUPS:
        linked = [row for row in rows if group in row['fixture_manifest_case']['groups']]
        failed = any(row['status'] == 'failed' for row in linked)
        group_rows.append({'id': group, 'status': 'failed' if failed else 'partial' if linked else 'not_run',
                           'evidence_subcases': [row['id'] for row in linked],
                           'closed': False,
                           'remaining': 'Full group subcases, real-window/independent-viewer evidence and fixed repeat/resource observations remain.' if linked else 'Format/batch not implemented in this run.'})
    write_json(out / 'reports/cases.json', {'suite': ctx.suite, 'groups': group_rows, 'subcases': rows, 'full_groups_closed': 0})
    good = all(item['exit_code'] == 0 for item in ctx.commands.values())
    write_json(out / 'reports/summary.json', {'targeted_checks_passed': good, 'group5_status': 'in_progress', 'batch_5a_status': 'partial', 'full_groups_closed': 0})
    print(f'5A targeted checks: {"passed" if good else "failed"}; full acceptance groups closed: 0/24')
    return 0 if good else 1
