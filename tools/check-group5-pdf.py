#!/usr/bin/env python3
"""Test-only independent PDF artifact checks using PyMuPDF.

Usage: python check-group5-pdf.py MANIFEST_JSON NEW_REPORT_JSON
The manifest binds exact PDF bytes and predeclared kind/paper expectations.
This does not certify visual or interactive-viewer behavior. PyMuPDF is an
optional developer check tool, never a Yu runtime or PDF production dependency.
"""
from __future__ import annotations
import hashlib
import json
from pathlib import Path
import re
import sys
import fitz


def inspect_file(spec: dict, root: Path) -> dict:
    path = (root / spec['file']).resolve(strict=True)
    if not path.is_relative_to(root.resolve()) or not path.is_file():
        raise ValueError('PDF path must stay within the manifest directory')
    payload = path.read_bytes()
    digest = hashlib.sha256(payload).hexdigest()
    if digest != spec['sha256']:
        raise ValueError('PDF bytes changed since export')
    document = fitz.open(path)
    errors, pages = [], []
    expected = (612, 792) if spec['paper'] == 'Letter' else (595.2756, 841.8898)
    if spec.get('landscape'): expected = expected[::-1]
    if not 1 <= len(document) <= 1000: errors.append('page count outside budget')
    if document.is_encrypted or document.embfile_count(): errors.append('unexpected encryption/embedded files')
    for n, page in enumerate(document):
        if abs(page.rect.width - expected[0]) > .02 or abs(page.rect.height - expected[1]) > .02:
            errors.append(f'page {n + 1}: wrong paper size')
        text = page.get_text(sort=False)
        # These fixed fixtures use margins >=36 pt and numbered pages. Keep
        # raw extraction, but separate only the geometrically verified footer
        # before comparing a prose fragment that crosses a page boundary.
        footer_box = fitz.Rect(0, page.rect.height - 35, page.rect.width, page.rect.height)
        footer_spans = [span for block in page.get_text('dict', clip=footer_box)['blocks']
                        if 'lines' in block for line in block['lines'] for span in line['spans']]
        if (len(footer_spans) != 1 or footer_spans[0]['text'] != str(n + 1)
                or abs(footer_spans[0]['size'] - 9) > .01):
            errors.append(f'page {n + 1}: unexpected footer content')
        body_text = page.get_text(clip=fitz.Rect(0, 0, page.rect.width, page.rect.height - 35), sort=False)
        links = page.get_links()
        for link in links:
            if link['kind'] == fitz.LINK_GOTO and not 0 <= link.get('page', -1) < len(document):
                errors.append(f'page {n + 1}: unresolved internal link')
            if link['kind'] == fitz.LINK_URI and not link.get('uri', '').startswith(('https:', 'http:', 'mailto:')):
                errors.append(f'page {n + 1}: unsafe URL action')
        for image in page.get_image_info():
            box = fitz.Rect(image['bbox'])
            if box.width > page.rect.width * .8 and box.height > page.rect.height * .8:
                errors.append(f'page {n + 1}: page-sized bitmap')
        pages.append({'page': n + 1, 'width': page.rect.width, 'height': page.rect.height,
                      'text': text, 'body_text': body_text, 'footer_text': ''.join(s['text'] for s in footer_spans),
                      'vector_paths': len(page.get_drawings()),
                      'raster_images': len(page.get_images()), 'links': len(links)})
    joined = '\n'.join(page['text'] for page in pages)
    if 'PDF-LATER-EDIT-NOT-EXPORTED' in joined or 'GROUP5-PRIVATE-METADATA-MUST-NOT-LEAK' in joined:
        errors.append('snapshot or metadata isolation')
    if spec.get('unsaved') and 'PDF-UNSAVED-SNAPSHOT' not in joined:
        errors.append('unsaved snapshot missing')
    kind = spec['kind']
    if kind == 'composite':
        if '~~删除~~' in joined: errors.append('Markdown strikethrough emitted as literal delimiters')
        for marker in ['Yu 内置 HTML 导出', 'GROUP5-DOCUMENT-END', 'GROUP5-DETAILS-OFFSCREEN-CONTENT',
                       '合并后仍完整', '第一条文末注', '第二条文末注', 'println!', 'English']:
            if marker not in joined: errors.append('missing content: ' + marker)
        needle = '这是一份独立的第五组固定语料'
        hits = [(n, rect) for n, page in enumerate(document) for rect in page.search_for(needle)]
        if not hits: errors.append('ordinary Chinese not searchable')
        elif not any(needle in document[n].get_textbox(rect) for n, rect in hits):
            errors.append('Chinese selection text mismatch')
        if sum(p['vector_paths'] for p in pages) < 80: errors.append('missing expected vector content')
        if sum(p['links'] for p in pages) < 15: errors.append('missing expected navigation')
    elif kind == 'table':
        if len(document) < 2 or 'TABLE-END' not in joined: errors.append('table is not complete/multipage')
        for n in range(40):
            markers = [f'GROUP-{n:03}', f'CELL-{n:03}-A', f'CELL-{n:03}-B']
            locations = [[p['page'] for p in pages if marker in p['text']] for marker in markers]
            if any(joined.count(marker) != 1 for marker in markers) or not locations[0] == locations[1] == locations[2]:
                errors.append(f'table merged group {n}: missing, repeated or split')
        if any('CELL-' in p['text'] and any(header not in p['text'] for header in ['HEADER-A', 'HEADER-B', 'HEADER-C']) for p in pages):
            errors.append('table header not repeated')
    elif kind == 'long-code':
        if len(document) < 2 or 'LONG-CODE-END' not in joined: errors.append('long code incomplete')
        if any(joined.count(f'CODE-{n:03}') != 1 for n in range(140)): errors.append('missing or duplicate code line')
        # Exclude the separately asserted page numbers, not arbitrary digits.
        # This is a content-count test, not a space-preserving-copy guarantee.
        prose = '\n'.join(page['body_text'] for page in pages)
        if re.sub(r'\s', '', prose).count('普通中文长段落') != 200: errors.append('long prose content count')
    elif kind == 'warnings':
        if 'GROUP5-WARNING-END' not in joined or "unknownYuCommand" not in joined: errors.append('warning fallback text missing')
        if sum(p['links'] for p in pages): errors.append('warning source became an executable link')
    else:
        errors.append('unknown manifest kind')
    for n in range(1, document.xref_length()):
        obj = document.xref_object(n)
        if re.search(r'/(JavaScript|Launch|SubmitForm|EmbeddedFile)\b', obj):
            errors.append('unexpected active PDF object'); break
    document.close()
    return {'file': spec['file'], 'kind': kind, 'sha256': digest, 'passed': not errors, 'errors': errors, 'pages': pages}


def main() -> int:
    if len(sys.argv) != 3: raise SystemExit(__doc__)
    manifest_path, output = map(Path, sys.argv[1:])
    if output.exists(): raise ValueError('Refusing existing evidence report')
    manifest = json.loads(manifest_path.read_text())
    if not isinstance(manifest.get('files'), list) or not 1 <= len(manifest['files']) <= 10:
        raise ValueError('Expected one to ten fixed PDFs')
    results = [inspect_file(spec, manifest_path.parent) for spec in manifest['files']]
    report = {'passed': all(r['passed'] for r in results), 'evidence': 'independent_pdf_structure_text',
              'pymupdf_version': fitz.VersionBind, 'visual_review': 'not_automatically_passed', 'files': results}
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({'passed': report['passed'], 'files': [{k: r[k] for k in ['file', 'passed', 'errors']} for r in results]}, ensure_ascii=False))
    return 0 if report['passed'] else 1


if __name__ == '__main__': raise SystemExit(main())
