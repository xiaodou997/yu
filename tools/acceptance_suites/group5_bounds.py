"""Fixed HTML budgets: actual source/SVG/occurrence limits, guard arithmetic,
real ImageIO normalization, task admission and deterministic deadline checks.
Synthetic guard tests do not claim a 256 MiB artifact or 300-second wall wait.
"""
from __future__ import annotations
import json
import struct
import subprocess
import zlib
from pathlib import Path
from acceptance_runner import ROOT, RunContext, sha, write_json

CORE = ['source_eight_mib_boundary_and_overflow', 'resource_occurrence_limit_counts_repeated_references',
        'svg_byte_budget_exact_and_one_over_is_fatal', 'svg_node_budget_exact_and_one_over_is_fatal',
        'repeated_toc_expansion_is_rejected_before_cloning_large_output',
        'document::bounds::output_and_resource_byte_guards_accept_limit_reject_next_and_overflow',
        'document::bounds::slot_preflight_includes_repeated_values_and_literal_tail',
        'document::bounds::final_join_preflights_all_parts_before_allocation',
        'real_write_failure_preserves_old_output_and_cleans_temp']
NATIVE = ['pixels-below', 'pixels-exact', 'pixels-over', 'svg-bytes-over', 'svg-nodes-over']
TASK = 'html_export::bounds::html_budget_lifecycle_two_jobs_deadline_and_source_refusal'


def png(width: int, height: int) -> bytes:
    # A uniform RGBA test image: stream rows to zlib instead of allocating a
    # full bitmap in the Python harness. ImageIO still decodes the real image.
    def chunk(tag, data):
        return struct.pack('>I', len(data)) + tag + data + struct.pack('>I', zlib.crc32(tag + data))
    encoder = zlib.compressobj()
    row = b'\0' + bytes([48, 96, 144, 255]) * width
    encoded = bytearray()
    for _ in range(height):
        encoded.extend(encoder.compress(row))
    encoded.extend(encoder.flush())
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', bytes(encoded)) + chunk(b'IEND', b''))


def run_suite(ctx: RunContext) -> int:
    app = ROOT / 'platform/macos/yu-shell-macos/.build/Yu.app'
    binary = app / 'Contents/MacOS/Yu'
    build = json.loads((app.parent / 'build-manifest.json').read_text())
    if build['configuration'] != 'release' or sha(binary.read_bytes()) != build['app_sha256']:
        raise ValueError('Matching audited Release required')
    write_json(ctx.output / 'build-manifest.json', build)
    inputs = ctx.output / 'inputs'; inputs.mkdir()
    for name, width, height in [('below', 8192, 4095), ('exact', 8192, 4096), ('over', 8193, 4096)]:
        (inputs / f'pixels-{name}.png').write_bytes(png(width, height))
    (inputs / 'svg-bytes-over.svg').write_text('<svg>' + ' ' * (4 * 1024 * 1024 - 10) + '</svg>')
    (inputs / 'svg-nodes-over.svg').write_text('<svg>' + '<g/>' * 99_999 + '</svg>')
    names = subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard', '--',
        'crates/yu-export', 'crates/yu-storage-ffi', 'platform/macos/yu-render-macos',
        'platform/macos/yu-shell-macos/Sources', 'tools/acceptance_suites/group5_bounds.py'], cwd=ROOT).decode().split('\0')
    write_json(ctx.output / 'source-lock.json', {'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
        'source_sha256': {name: sha((ROOT/name).read_bytes()) for name in sorted(set(names)) if name and (ROOT/name).is_file()},
        'input_sha256': {p.name: sha(p.read_bytes()) for p in inputs.iterdir()}})
    code = ctx.run('bounds-core', ['cargo', 'test', '--locked', '-p', 'yu-export', '--', '--test-threads=1'])
    log = ctx.log('bounds-core').read_text()
    for name in CORE:
        groups = ['X03'] if name.startswith('real_write_failure_') else ['X08']
        ctx.ledger.add(name, ['core'], metadata={'groups': groups,
            'scope': 'actual source/SVG limits, named preallocation guard, or process-local kernel write failure'})
        ctx.ledger.record(name, 'core', 'passed' if code == 0 and f'test {name} ... ok' in log else 'failed', 'bounds-core')
    code = ctx.run('bounds-task', ['cargo', 'test', '--locked', '-p', 'yu-storage-ffi', 'html_budget_lifecycle', '--', '--test-threads=1'])
    ctx.ledger.add(TASK, ['core'], metadata={'groups': ['X05', 'X07', 'X08'], 'scope': 'actual task admission, deterministic deadline checkpoint; not wall-clock wait'})
    ctx.ledger.record(TASK, 'core', 'passed' if code == 0 and f'test {TASK} ... ok' in ctx.log('bounds-task').read_text() else 'failed', 'bounds-task')
    code = ctx.run('bounds-native', [str(binary), '--html-export-budget-self-check', str(inputs), str(ctx.output / 'native')])
    report_path = ctx.output / 'native/report.json'
    report = json.loads(report_path.read_text()) if report_path.exists() else {}
    observed = {row['id']: row for row in report.get('cases', [])}
    for name in NATIVE:
        ctx.ledger.add(name, ['native'], metadata={'groups': ['X02', 'X03', 'X08']})
        row = observed.get(name, {})
        passed = code == 0 and report.get('passed') and row.get('passed') and row.get('source_selection_unchanged')
        ctx.ledger.record(name, 'native', 'passed' if passed else 'failed', 'bounds-native', evidence={'case_result': row})
    rows = ctx.ledger.finish()
    good = all(row['status'] == 'passed' for row in rows)
    write_json(ctx.output / 'reports/cases.json', {'cases': rows, 'groups_closed': 0})
    write_json(ctx.output / 'reports/summary.json', {'passed': good, 'batch_5a': 'pending_final_audit',
        'scope': 'Fixed budget boundaries only; 64/256 MiB guard tests are not full-size output stress; deadline is deterministic'})
    return 0 if good else 1
