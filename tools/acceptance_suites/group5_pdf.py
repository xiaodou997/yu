"""First 5B native pipeline. This suite never closes all F01-F06 by itself."""
from __future__ import annotations
import json
import shutil
import subprocess
from pathlib import Path
from acceptance_runner import ROOT, RunContext, sha, write_json

SHELL = ROOT / 'platform/macos/yu-shell-macos'
CASES = [('a4', []), ('letter-landscape', ['--pdf-letter', '--pdf-landscape']),
         ('table-pagination', []), ('long-text-code', []),
         ('warnings', ['--pdf-warnings']), ('cancel', ['--pdf-cancel'])]


def report_passed(name: str, code: int, report: dict) -> bool:
    if code != 0 or not report.get('passed') or not report.get('source_identity_preserved') or not report.get('continued_edit_undo'):
        return False
    extracted = '\n'.join(page['text'] for page in report.get('pages', []))
    if name == 'table-pagination':
        return (report.get('page_count', 0) > 1 and 'TABLE-END' in extracted
                and all(extracted.count(f'GROUP-{n:03}') == 1 and extracted.count(f'CELL-{n:03}-A') == 1 and extracted.count(f'CELL-{n:03}-B') == 1 for n in range(40))
                and all('HEADER-A' in page['text'] for page in report.get('pages', []) if 'CELL-' in page['text']))
    if name == 'long-text-code':
        return report.get('page_count', 0) > 1 and 'LONG-CODE-END' in extracted and all(extracted.count(f'CODE-{n:03}') == 1 for n in range(140))
    return True


def run_suite(ctx: RunContext) -> int:
    app = SHELL / '.build/Yu.app'
    binary = app / 'Contents/MacOS/Yu'
    build = json.loads((SHELL / '.build/build-manifest.json').read_text())
    if build['configuration'] != 'release' or sha(binary.read_bytes()) != build['app_sha256']:
        raise ValueError('Matching audited Release required')
    write_json(ctx.output / 'build-manifest.json', build)
    inputs = ctx.output / 'inputs'; inputs.mkdir()
    outputs = ctx.output / 'outputs'; outputs.mkdir()
    (inputs / 'assets').mkdir()
    shutil.copy2(SHELL / 'Fixtures/assets/yu-mark.png', inputs / 'assets/yu-mark.png')
    source = (SHELL / 'Fixtures/group5-export.md').read_text()
    (inputs / 'composite.md').write_text(source)
    (inputs / 'composite-bom-crlf.md').write_bytes(b'\xef\xbb\xbf' + source.replace('\n', '\r\n').encode())
    shutil.copy2(SHELL / 'Fixtures/group5-export-warning.md', inputs / 'warnings.md')
    table = '<table><thead><tr><th>HEADER-A</th><th>HEADER-B</th><th>HEADER-C</th></tr></thead><tbody>'
    for n in range(40):
        table += f'<tr><td rowspan="2">GROUP-{n:03}-合并</td><td>CELL-{n:03}-A</td><td>中文 {n}</td></tr><tr><td colspan="2">CELL-{n:03}-B 完整跨列</td></tr>'
    table += '</tbody></table>\n\nTABLE-END\n'
    (inputs / 'table-pagination.md').write_text('# 跨页合并表格\n\n' + table)
    prose = '# 长段落与代码\n\n' + ('普通中文长段落 English，必须完整换行。' * 200) + '\n\n```rust\n'
    prose += ''.join(f'let line_{n:03} = "CODE-{n:03} 中文内容";\n' for n in range(140))
    prose += '```\n\nLONG-CODE-END\n'
    (inputs / 'long-text-code.md').write_text(prose)
    names = subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard', '--',
        'Cargo.toml', 'Cargo.lock', 'crates/yu-export', 'crates/yu-storage-ffi', 'platform/macos/yu-render-macos',
        'platform/macos/yu-shell-macos/Sources', 'tools/acceptance_suites/group5_pdf.py'], cwd=ROOT).decode().split('\0')
    write_json(ctx.output / 'source-lock.json', {'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        'source_sha256': {n: sha((ROOT / n).read_bytes()) for n in sorted(set(names)) if n and (ROOT / n).is_file()},
        'input_sha256': {str(p.relative_to(inputs)): sha(p.read_bytes()) for p in inputs.rglob('*') if p.is_file()}})
    for name, _ in CASES:
        ctx.ledger.add(name, ['native'], metadata={'groups': ['X01', 'X02', 'X03', 'F01', 'F02', 'F03', 'F04', 'F06'], 'scope': 'Named native path, not whole 5B acceptance'})
    for name, flags in CASES:
        fixture = inputs / (name + '.md' if name in ['warnings', 'table-pagination', 'long-text-code'] else 'composite-bom-crlf.md' if name == 'letter-landscape' else 'composite.md')
        target = outputs / (name + '.pdf')
        if name == 'cancel': target.write_bytes(b'OLD-PDF-KEEP')
        code = ctx.run('pdf-' + name, [str(binary), '--pdf-export-self-check', str(fixture), str(target), *flags])
        report_path = Path(str(target) + '.report.json')
        report = json.loads(report_path.read_text()) if report_path.exists() else {}
        good = report_passed(name, code, report)
        ctx.ledger.record(name, 'native', 'passed' if good else 'failed', 'pdf-' + name, evidence={'report': ctx.artifact(report_path) if report_path.exists() else None})
        if not good: break
    rows = ctx.ledger.finish()
    passed = len(rows) == len(CASES) and all(row['status'] == 'passed' for row in rows)
    write_json(ctx.output / 'reports/cases.json', {'cases': rows, 'groups_closed': 0})
    write_json(ctx.output / 'reports/summary.json', {'passed': passed, 'batch_5b': 'partial', 'scope': 'Native PDF snapshot/paper/warning/cancellation paths; external visual/GUI separate'})
    return 0 if passed else 1
