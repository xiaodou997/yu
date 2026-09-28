"""Bounded PDF settings, untitled resources, ten exports and three running cancels.
Reuse the existing isolated Yu desktop driver; PDFs stay in local evidence.
"""
from __future__ import annotations
import json
import time
from pathlib import Path
from acceptance_runner import ROOT, RunContext, sha, write_json
from acceptance_suites.group5_pdf_window import PDFWindowChecks


class PDFLifecycle(PDFWindowChecks):
    def setup(self):
        self.inspector = self.out / 'inspect-pdf'
        assert self.ctx.run('build-pdf-inspector', ['swiftc', str(ROOT / 'tools/inspect-pdf-native.swift'), '-o', str(self.inspector)]) == 0
        super().setup()

    def options(self, paper='A4', landscape=False, margin=44, numbers=True):
        for identifier, wanted in [('yu-pdf-paper', paper), ('yu-pdf-orientation', '横向' if landscape else '纵向')]:
            c = self.identifier(identifier)
            if c['AXValue'] != wanted:
                self.click(c); self.click(self.find(lambda c: c.get('AXRole') == 'AXMenuItem' and c.get('AXTitle') == wanted))
            # AppKit closes the popup asynchronously. Observe the selected
            # value; the first AX sample can still describe the outgoing menu.
            self.find(lambda c: c.get('AXIdentifier') == identifier and c.get('AXValue') == wanted)
        self.click(self.identifier('yu-pdf-margin')); self.key(0, 'cmd'); self.event('text', str(margin))
        self.find(lambda c: c.get('AXIdentifier') == 'yu-pdf-margin' and str(c.get('AXValue')) == str(margin))
        c = self.identifier('yu-pdf-page-numbers')
        if bool(int(c['AXValue'])) != numbers: self.click(c)
        self.find(lambda c: c.get('AXIdentifier') == 'yu-pdf-page-numbers' and c.get('AXValue') is not None and bool(int(c['AXValue'])) == numbers)

    def finish(self, target, source, marker, paper='A4', landscape=False, margin=44, numbers=True, embedded=None):
        self.find(lambda c: c.get('AXValue') == 'PDF 导出完成', timeout=180)
        if embedded is not None:
            self.find(lambda c: f'{embedded} 项公式/图表' in str(c.get('AXValue')))
        self.source(source)
        result = self.command([self.inspector, target, margin])
        expected = (612, 792) if paper == 'Letter' else (595.2756, 841.8898)
        if landscape: expected = expected[::-1]
        for p in result['pages']:
            assert abs(p['width'] - expected[0]) < .02 and abs(p['height'] - expected[1]) < .02
            assert str(p['page']) in p['footer'] if numbers else not p['footer'].strip()
        text = '\n'.join(p['text'] for p in result['pages'])
        assert marker in text and 'GROUP5-PRIVATE-METADATA-MUST-NOT-LEAK' not in text
        result['path'] = str(target.relative_to(self.out))
        write_json(self.out / 'reports' / (target.stem + '.json'), result)
        self.click(self.button('关闭'))
        return result

    def settings(self):
        before = self.event('snapshot')['AXSelectedTextRanges']
        for index, (paper, landscape, margin, numbers) in enumerate([('Letter', False, 54, False), ('A4', True, 36, True)]):
            self.open_export(); self.options(paper, landscape, margin, numbers)
            target = self.outputs / f'settings-{index + 1}.pdf'; self.choose_target(target)
            self.finish(target, self.first_source, 'GROUP5-DOCUMENT-END', paper, landscape, margin, numbers, 14)
        self.open_export()
        assert self.identifier('yu-pdf-paper')['AXValue'] == 'A4'
        assert self.identifier('yu-pdf-orientation')['AXValue'] == '横向'
        assert str(self.identifier('yu-pdf-margin')['AXValue']) == '36'
        self.click(self.identifier('CancelButton'))
        assert self.event('snapshot')['AXSelectedTextRanges'] == before
        # End-of-document/source mode and zoom must not alter PDF page text.
        self.event('select', len(self.first_source.encode('utf-16-le')) // 2, 0)
        self.key(24, 'cmd+shift'); self.key(46, 'cmd+shift')
        self.open_export(); self.options('A4', True, 36, True)
        target = self.outputs / 'presentation.pdf'; self.choose_target(target)
        current = self.finish(target, self.first_source, 'GROUP5-DOCUMENT-END', 'A4', True, 36, True, 14)
        prior = json.loads((self.out / 'reports/settings-2.json').read_text())
        assert [p['text'] for p in current['pages']] == [p['text'] for p in prior['pages']]
        self.key(46, 'cmd+shift'); self.key(29, 'cmd')

    def untitled_pdf(self):
        self.key(45, 'cmd'); self.source('')
        source = '# 未命名 PDF\n\n![图片](assets/yu-mark.png)\n\nUNTITLED-PDF-END\n'
        self.event('text', source); self.source(source); self.open_export()
        self.click(self.button('选择图片基准目录…')); self.identifier('OKButton')
        self.go(self.inputs); self.click(self.identifier('OKButton'))
        self.find(lambda c: c.get('AXValue') == '图片基准目录：inputs')
        self.options(); target = self.outputs / 'untitled.pdf'; self.choose_target(target)
        self.finish(target, source, 'UNTITLED-PDF-END', embedded=0)
        self.key(13, 'cmd'); self.click(self.button('不保存')); self.source(self.first_source)

    def repeats(self):
        self.open_document(self.second, self.second_source)
        rows = []
        for n in range(10):
            long = n % 2 == 1
            path, source, marker, count = (self.second, self.second_source, 'SECOND-DOCUMENT-END', 220) if long else (self.first, self.first_source, 'GROUP5-DOCUMENT-END', 14)
            self.event('raise-window', path); self.source(source); self.open_export(); self.options()
            target = self.outputs / f'repeat-{n + 1:02}.pdf'; self.choose_target(target); self.sample(f'repeat-{n + 1}-started')
            report = self.finish(target, source, marker, embedded=count)
            text = '\n'.join(p['text'] for p in report['pages'])
            assert ('GROUP5-DOCUMENT-END' not in text) if long else ('SECOND-DOCUMENT-END' not in text)
            if long:
                import re
                compact = re.sub(r'\s', '', text)
                assert all(f'第{i}段中文' in compact for i in range(220))
            rows.append({'number': n + 1, 'sha256': report['sha256'], 'pages': report['page_count'], 'embedded': count})
            self.sample(f'repeat-{n + 1}-completed'); write_json(self.out / 'reports/ten-exports.json', rows)

    def running_cancels(self):
        source = '# 运行中取消 PDF\n\n' + ''.join(f'第 {n} 项 $x_{{{n}}}^2+1$\n\n' for n in range(1000)) + 'CANCEL-PDF-END\n'
        path = self.inputs / 'cancel-long.md'; path.write_text(source)
        self.event('raise-window', self.first); self.open_document(path, source)
        rows = []
        for n in range(3):
            target = self.outputs / f'cancel-retry-{n + 1}.pdf'; target.write_bytes(b'OLD-PDF-KEEP')
            self.open_export(); self.options(); self.choose_target(target); self.click(self.button('替换'))
            self.click(self.button('取消导出')); time.sleep(.4)
            assert target.read_bytes() == b'OLD-PDF-KEEP'; self.source(source)
            self.sample(f'cancel-{n + 1}')
            self.open_export(); self.choose_target(target); self.click(self.button('替换'))
            report = self.finish(target, source, 'CANCEL-PDF-END', embedded=1000)
            rows.append({'number': n + 1, 'retry_sha256': report['sha256'], 'pages': report['page_count'], 'old_file_preserved_on_cancel': True})
            write_json(self.out / 'reports/three-cancels.json', rows)
        target = self.outputs / 'owner-close.pdf'; target.write_bytes(b'OLD-PDF-KEEP')
        self.open_export(); self.choose_target(target); self.click(self.button('替换')); self.button('取消导出')
        self.key(13, 'cmd'); self.event('raise-window', self.first); self.source(self.first_source)
        time.sleep(.4); assert target.read_bytes() == b'OLD-PDF-KEEP'
        self.event('raise-window', self.second); self.key(13, 'cmd'); self.source(self.first_source)
        deadline = time.monotonic() + 75
        while True:
            self.sample('closed-long-documents-idle')
            if not self.resources[-1]['children']: break
            assert time.monotonic() < deadline, 'Helper remained past idle grace'; time.sleep(5)
        assert not list(self.outputs.glob('.yu-export-*'))


def run_suite(ctx: RunContext) -> int:
    checks = PDFLifecycle(ctx)
    cases = [('paper-options-and-presentation', checks.settings), ('untitled-resource-base', checks.untitled_pdf),
             ('ten-alternating-pdfs', checks.repeats), ('three-running-cancels-and-close', checks.running_cancels)]
    for name, _ in cases: ctx.ledger.add(name, ['real_window'], metadata={'scope': 'Named bounded PDF path, no automatic whole-group sign-off'})
    try:
        checks.setup()
        for name, check in cases:
            try: check()
            except Exception:
                ctx.ledger.record(name, 'real_window', 'failed', checks.last_command); raise
            ctx.ledger.record(name, 'real_window', 'passed', checks.last_command)
        write_json(ctx.output / 'reports/summary.json', {'passed': True, 'groups_closed': 0})
        return 0
    finally:
        write_json(ctx.output / 'reports/cases.json', {'cases': ctx.ledger.finish(), 'groups_closed': 0})
        checks.cleanup()
