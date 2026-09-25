#!/usr/bin/env python3
"""Prepare source-exact native inputs; this does not run Yu or mark tests passed."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / 'crates/yu-editor/tests/fixtures/group4-list'
MARKER = re.compile(r'«([AF])(\d+)»')
A, B = ' HISTORY-A', ' HISTORY-B'
NAMES = (
    'parent-child.indent', 'child-next.outdent', 'root-cross.outdent',
    'multi-parent.indent', 'multi-parent.outdent', 'late-ancestor.outdent',
    'repeated-root.outdent', 'div-cross.outdent',
    'cross-cells.reject-outdent', 'cross-disclosure.reject-outdent',
    'raw-endpoint.reject-indent', 'first-ancestor.reject-indent', 'mixed-markdown.reject-outdent',
)


def marked(text: str) -> tuple[str, list[dict[str, int]]]:
    parts: list[str] = []
    points: dict[int, dict[str, int]] = {}
    previous, utf8, utf16 = 0, 0, 0
    for match in MARKER.finditer(text):
        fragment = text[previous:match.start()]
        parts.append(fragment)
        utf8 += len(fragment.encode('utf-8'))
        utf16 += len(fragment.encode('utf-16-le')) // 2
        kind, identity = match.group(1), int(match.group(2))
        pair = points.setdefault(identity, {})
        key = 'anchor' if kind == 'A' else 'focus'
        if f'{key}_utf8' in pair:
            raise ValueError(f'duplicate marker: {match.group()}')
        pair[f'{key}_utf8'], pair[f'{key}_utf16'] = utf8, utf16
        previous = match.end()
    parts.append(text[previous:])
    source = ''.join(parts)
    if not points or set(points) != set(range(len(points))) or '«' in source or '»' in source:
        raise ValueError('invalid or missing selection markers')
    result = []
    for identity in range(len(points)):
        pair = points[identity]
        if set(pair) != {'anchor_utf8', 'focus_utf8', 'anchor_utf16', 'focus_utf16'}:
            raise ValueError('incomplete selection markers')
        result.append({'index': identity, **pair})
    return source, result


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def prepare(output: Path, fixtures: Path = FIXTURES) -> dict:
    # Validate every input before creating output, so damaged fixtures do not
    # leave a directory that can be confused with a prepared run.
    parsed = []
    for name in NAMES:
        raw = (fixtures / f'{name}.case').read_bytes()
        text = raw.decode('utf-8').replace('\r\n', '\n').rstrip('\n')
        if text.count('\n@@AFTER@@\n') != 1:
            raise ValueError(f'{name}: missing/duplicate before/after delimiter')
        before, after = text.split('\n@@AFTER@@\n')
        _, old = marked(before)
        _, new = marked(after)
        if len(old) != len(new):
            raise ValueError(f'{name}: endpoint count changed')
        if '.reject-' in name and before != after:
            raise ValueError(f'{name}: rejected command cannot change source or endpoints')
        parsed.append((name, before, after, sha(raw)))
    output.mkdir(parents=True, exist_ok=False)
    manifest = {'schema_version': 1, 'kind': 'input_manifest', 'product_verdict': 'not_run',
                'base_commit': 'b8e68f23ecfd97d75b272dce3cf5d8355b940318', 'cases': []}
    for name, before, after, fixture_sha in parsed:
        locations = ['document'] if name.startswith(('cross-cells.', 'mixed-markdown.')) else ['standalone', 'cell']
        for location in locations:
            for ending_name, ending in [('lf', '\n'), ('crlf', '\r\n')]:
                for bom in (False, True):
                    case_id = f'{name}-{location}-{ending_name}' + ('-bom' if bom else '')
                    directory = output / case_id
                    directory.mkdir()
                    def wrap(body: str) -> str:
                        if location == 'cell':
                            body = f'<table><tr><td>{body}</td><td>NEIGHBOR</td></tr></table>'
                        return ('PREFIX中文🙂\n\n' + body + '\n\nTAIL').replace('\n', ending)
                    source, old_points = marked(wrap(before))
                    expected, new_points = marked(wrap(after))
                    def encoded(text: str) -> bytes:
                        return (b'\xef\xbb\xbf' if bom else b'') + text.encode('utf-8')
                    values = {'input.md': source, 'expected-0.md': source,
                              'expected-a.md': source + A, 'expected-b.md': source + A + B,
                              'expected-list-a.md': expected + A}
                    hashes = {}
                    for filename, value in values.items():
                        data = encoded(value)
                        (directory / filename).write_bytes(data)
                        hashes[filename] = sha(data)
                    manifest['cases'].append({'id': case_id, 'fixture': f'{name}.case',
                        'fixture_sha256': fixture_sha, 'location': location, 'line_ending': ending_name,
                        'bom': bom, 'command': 'indent' if name.endswith('indent') else 'outdent',
                        'expected_changed': '.reject-' not in name, 'status': 'not_run',
                        'selection_offsets_exclude_file_bom': True,
                        'before_selections': old_points, 'after_selections': new_points,
                        'sha256': hashes})
    (output / 'manifest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    try:
        manifest = prepare(args.output)
    except (OSError, ValueError, UnicodeError) as error:
        parser.exit(1, f'Preparation failed: {error}\n')
    print(f'Prepared {len(manifest["cases"])} input variants; no product tests have run.')


if __name__ == '__main__':
    main()
