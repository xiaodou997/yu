#!/usr/bin/env python3
"""Inspect actual HTML bytes/structure. Not a browser/visual acceptance result."""
from __future__ import annotations
import argparse
import base64
from collections import Counter
from html.parser import HTMLParser
import json
from pathlib import Path
import struct
import xml.etree.ElementTree as ET

class HTMLInspection(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.ids = []
        self.fragments = []
        self.images = []
        self.tags = Counter()
        self.data = []
        self.errors = []
        self.csp = False
    def handle_starttag(self, tag, attrs):
        attributes = dict(attrs)
        self.tags[tag] += 1
        if 'id' in attributes: self.ids.append(attributes['id'])
        if attributes.get('href', '').startswith('#'): self.fragments.append(attributes['href'][1:])
        if tag in {'script', 'iframe', 'object', 'embed', 'link', 'base'}: self.errors.append('active tag: ' + tag)
        if any(name.startswith('on') for name in attributes): self.errors.append('event attribute')
        if tag == 'meta' and attributes.get('http-equiv', '').lower() == 'content-security-policy': self.csp = True
        if tag == 'details' and 'open' not in attributes: self.errors.append('closed details')
        if tag == 'img':
            uri = attributes.get('src', '')
            if not uri.startswith(('data:image/png;base64,', 'data:image/svg+xml;base64,')):
                self.errors.append('non-portable image'); return
            header, encoded = uri.split(',', 1)
            payload = base64.b64decode(encoded, validate=True)
            if 'image/png' in header:
                if payload[:8] != b'\x89PNG\r\n\x1a\n': self.errors.append('invalid PNG signature')
                width, height = struct.unpack('>II', payload[16:24])
                if not width or not height: self.errors.append('empty PNG')
                self.images.append({'kind': 'png', 'bytes': len(payload), 'width': width, 'height': height})
            else:
                root = ET.fromstring(payload)
                if root.tag.split('}')[-1] != 'svg': self.errors.append('invalid SVG root')
                self.images.append({'kind': 'svg', 'bytes': len(payload), 'viewBox': root.attrib.get('viewBox')})
    def handle_data(self, data): self.data.append(data)

def inspect(path: Path, warnings: bool) -> dict:
    source = path.read_text(encoding='utf-8')
    parser = HTMLInspection(); parser.feed(source); parser.close()
    duplicate = [value for value, count in Counter(parser.ids).items() if count != 1]
    missing = sorted(set(parser.fragments) - set(parser.ids))
    if duplicate: parser.errors.append('duplicate ids: ' + repr(duplicate))
    if missing: parser.errors.append('unresolved fragment links: ' + repr(missing))
    if not parser.csp: parser.errors.append('missing CSP')
    for secret in ['GROUP5-PRIVATE-METADATA-MUST-NOT-LEAK', 'GROUP5-LATER-EDIT-NOT-IN-EXPORT', 'file:///', '/Users/']:
        if secret in source: parser.errors.append('unexpected private/later content: ' + secret)
    text = ''.join(parser.data)
    if warnings:
        for marker in ['GROUP5-WARNING-END', '不得执行', '远程图片', '缺失图片']:
            if marker not in text: parser.errors.append('missing diagnostic content: ' + marker)
    else:
        if len(parser.images) != 15: parser.errors.append('expected 14 SVG resources and 1 local PNG')
        if Counter(image['kind'] for image in parser.images) != {'svg': 14, 'png': 1}: parser.errors.append('resource types/counts differ')
        for marker in ['GROUP5-DOCUMENT-END', 'GROUP5-DETAILS-OFFSCREEN-CONTENT', '第一条文末注', '第二条文末注', 'GROUP5-UNSAVED-SNAPSHOT-中文🙂']:
            if marker not in text: parser.errors.append('missing complete content: ' + marker)
    return {'passed': not parser.errors, 'evidence_kind': 'artifact_structure_not_visual',
            'errors': parser.errors, 'images': parser.images, 'ids': len(parser.ids),
            'fragment_links': len(parser.fragments), 'tags': dict(parser.tags)}

def main():
    args = argparse.ArgumentParser(description=__doc__)
    args.add_argument('html', type=Path); args.add_argument('report', type=Path)
    args.add_argument('--warnings', action='store_true')
    options = args.parse_args()
    report = inspect(options.html, options.warnings)
    with options.report.open('x', encoding='utf-8') as output: json.dump(report, output, ensure_ascii=False, indent=2)
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report['passed'] else 1
if __name__ == '__main__': raise SystemExit(main())
