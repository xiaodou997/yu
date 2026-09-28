"""System print panel checks in an isolated Yu bundle; never press physical Print."""
from __future__ import annotations
import json
import time
import os
import shutil
import subprocess
from pathlib import Path
from acceptance_runner import ROOT, RunContext, sha, write_json
from acceptance_suites.group5_window import WindowChecks

class PrintChecks(WindowChecks):
    def setup(self):
        self.temp = self.out / 'private-temp'; self.temp.mkdir()
        # Foundation uses Darwin's user temporary directory, even when the
        # CUPS side of NSPrintPanel honors TMPDIR. Observe only the new Yu-owned
        # directory delta; never remove or inspect another pre-existing task.
        self.foundation_temp = Path(subprocess.check_output(['getconf', 'DARWIN_USER_TEMP_DIR'], text=True).strip())
        self.existing_print_dirs = set(self.foundation_temp.glob('yu-print-*'))
        self.inspector = self.out / 'inspect-pdf'
        assert self.ctx.run('build-pdf-inspector', ['swiftc', str(ROOT / 'tools/inspect-pdf-native.swift'), '-o', str(self.inspector)]) == 0
        before = os.environ.get('TMPDIR'); os.environ['TMPDIR'] = str(self.temp) + '/'
        try: super().setup()
        finally:
            if before is None: os.environ.pop('TMPDIR', None)
            else: os.environ['TMPDIR'] = before
        self.queue_before = subprocess.check_output(['lpstat', '-W', 'all', '-o'], text=True)

    def drained(self):
        deadline = time.monotonic() + 8
        while set(self.foundation_temp.glob('yu-print-*')) - self.existing_print_dirs:
            assert time.monotonic() < deadline, 'Owned print temporary directory remained'
            time.sleep(.08)

    def settled_controls(self, identifiers):
        # AppKit exposes controls before its sheet/window transition has settled.
        # Only observe geometry here; do not repeat user actions or weaken results.
        deadline = time.monotonic() + 8; previous = None; since = time.monotonic()
        while time.monotonic() < deadline:
            controls = self.controls()
            selected = [next((c for c in controls if c.get('AXIdentifier') == identifier), None) for identifier in identifiers]
            state = [(c.get('AXPosition'), c.get('AXSize'), c.get('AXEnabled')) for c in selected if c]
            if len(state) == len(identifiers) and state == previous:
                if time.monotonic() - since >= .8: return
            else: previous = state; since = time.monotonic()
            time.sleep(.15)
        raise AssertionError('Native panel geometry did not settle')

    def range_and_orientation(self, pages=None, landscape=False):
        if pages:
            self.event('print-option', 'PR')
            self.find(lambda c: c.get('AXIdentifier') == 'PR' and c.get('AXValue') == 1)
            # Keep the range valid at each field commit. AppKit can clamp a
            # start of 2 back to 1 while the current end is still 1.
            for identifier, value in [('PREnd', pages[1]), ('PRStart', pages[0])]:

                self.event('print-field', identifier, value)
                self.find(lambda c: c.get('AXIdentifier') == identifier and str(c.get('AXValue')) == str(value))
        if landscape:
            self.event('print-option', 'LandscapeButton')
            self.find(lambda c: c.get('AXIdentifier') == 'LandscapeButton' and c.get('AXValue') == 1)

    def save_pdf(self, name, source, pages=None, landscape=False):
        self.range_and_orientation(pages, landscape)
        owned = set(self.foundation_temp.glob('yu-print-*')) - self.existing_print_dirs
        frozen = [p / 'prepared.pdf' for p in owned if (p / 'prepared.pdf').is_file()]
        assert len(frozen) == 1, 'Exact print snapshot was not observed'
        reference = self.inputs / (name + '-frozen.pdf'); shutil.copy2(frozen[0], reference)
        baseline = self.command([self.inspector, reference])
        self.click(self.find(lambda c: c.get('AXDescription') == 'PDF' and c.get('AXRole') == 'AXButton'))
        self.identifier('saveAsNameTextField')
        target = self.outputs / (name + '.pdf')
        self.settled_controls(['saveAsNameTextField', 'OKButton'])
        existed = target.exists()
        self.choose_target(target)
        if existed: self.click(self.button('替换'))
        deadline = time.monotonic() + 90
        while not target.exists() or not target.read_bytes().startswith(b'%PDF-'):
            assert time.monotonic() < deadline, 'System PDF was not published'
            time.sleep(.1)
        self.drained(); self.source(source)
        if name == 'system-all': self.capture('system-print-completed')
        result = self.command([self.inspector, target])
        wanted = baseline['pages'][pages[0] - 1:pages[1]] if pages else baseline['pages']
        assert len(result['pages']) == len(wanted), 'Page selection was not honored'
        for actual, expected in zip(result['pages'], wanted):
            assert ''.join(actual['text'].split()) == ''.join(expected['text'].split()), 'Printed text differs from the frozen page'
            if landscape: assert actual['width'] > actual['height'], 'Orientation did not change'
        result.update(reference_sha256=baseline['sha256'], matched_frozen_pages=True)
        write_json(self.out / 'reports' / (name + '.json'), result)
        self.sample(name + '-completed')
        return result

    def lifecycle(self):
        self.open_document(self.second, self.second_source)
        rows = []
        for n in range(10):
            long = n % 2 == 1
            path, source, marker = (self.second, self.second_source, 'SECOND-DOCUMENT-END') if long else (self.first, self.first_source, 'GROUP5-DOCUMENT-END')
            self.event('raise-window', path); self.source(source); self.open_print()
            result = self.save_pdf(f'repeat-{n + 1:02}', source)
            text = ''.join(''.join(p['text'].split()) for p in result['pages'])
            assert marker in text
            assert ('GROUP5-DOCUMENT-END' not in text) if long else ('SECOND-DOCUMENT-END' not in text)
            if long: assert all(f'第{i}段中文' in text for i in range(220))
            rows.append({'number': n + 1, 'pages': result['page_count'], 'sha256': result['sha256']})
            write_json(self.out / 'reports/ten-prints.json', rows)
        source = '# 取消打印\n\n' + ''.join(f'第 {n} 项 $x_{{{n}}}^2+1$\n\n' for n in range(1000)) + 'PRINT-CANCEL-END\n'
        path = self.inputs / 'cancel-long.md'; path.write_text(source)
        self.event('raise-window', self.first); self.open_document(path, source)
        retries = []
        for n in range(3):
            self.key(35, 'cmd')
            self.click(self.identifier('yu-print-cancel'))
            self.drained(); self.source(source)
            self.open_print()
            result = self.save_pdf(f'cancel-retry-{n + 1}', source)
            assert 'PRINT-CANCEL-END' in ''.join(p['text'] for p in result['pages'])
            retries.append({'number': n + 1, 'pages': result['page_count'], 'sha256': result['sha256']})
            write_json(self.out / 'reports/three-cancels.json', retries)
        self.key(35, 'cmd'); self.identifier('yu-print-cancel')
        self.key(13, 'cmd'); self.event('raise-window', self.first); self.source(self.first_source); self.drained()
        self.event('raise-window', self.second); self.key(13, 'cmd'); self.source(self.first_source)
        # Continuing to edit after printing must retain ordinary undo behavior.
        self.event('select', len(self.first_source.encode('utf-16-le')) // 2, 0)
        self.event('text', 'PRINT-CONTINUED-EDIT'); self.source(self.first_source + 'PRINT-CONTINUED-EDIT')
        self.key(6, 'cmd'); self.source(self.first_source)
        deadline = time.monotonic() + 75
        while True:
            self.sample('post-print-idle')
            if not self.resources[-1]['children']: break
            assert time.monotonic() < deadline, 'Helper remained after print idle grace'
            time.sleep(5)
        assert subprocess.check_output(['lpstat', '-W', 'all', '-o'], text=True) == self.queue_before

    def choose_target(self, path):
        self.event('save-name', path.name)
        self.find(lambda c: c.get('AXIdentifier') == 'saveAsNameTextField' and c.get('AXValue') == path.name)
        self.go(path.parent)
        self.click(self.identifier('OKButton'))

    def resource_paths(self):
        warning = self.inputs / 'printing-warnings.md'
        text = (ROOT / 'platform/macos/yu-shell-macos/Fixtures/group5-export-warning.md').read_text()
        warning.write_text(text)
        self.open_document(warning, text)
        self.key(35, 'cmd')
        self.button('带诊断继续打印')
        self.click(self.button('取消打印')); self.drained(); self.source(text)
        self.key(35, 'cmd'); self.click(self.button('带诊断继续打印'))
        self.wait_print()
        report = self.save_pdf('confirmed-warnings', text)
        assert 'GROUP5-WARNING-END' in ''.join(p['text'] for p in report['pages'])
        self.key(13, 'cmd'); self.event('raise-window', self.first); self.source(self.first_source)
        self.untitled_path()
        write_json(self.out / 'reports/resources-checks.json', {'passed': True, 'warning_cancel_then_explicit_consent': True, 'untitled_image_base': True})

    def untitled_path(self):
        self.key(45, 'cmd'); self.source('')
        text = '# 未命名打印\n\n![图片](assets/yu-mark.png)\n\nPRINT-UNTITLED-END\n'
        self.event('text', text); self.source(text); self.key(35, 'cmd')
        self.button('选择图片基准目录…'); self.settled_controls(['action-button-2'])
        self.event('print-resource-base'); self.identifier('OKButton')
        self.go(self.inputs); self.click(self.identifier('OKButton')); self.wait_print()
        report = self.save_pdf('untitled-print', text)
        assert 'PRINT-UNTITLED-END' in ''.join(p['text'] for p in report['pages'])
        self.key(13, 'cmd'); self.button('不保存'); self.event('discard-test-document'); self.source(self.first_source)
        assert subprocess.check_output(['lpstat', '-W', 'all', '-o'], text=True) == self.queue_before
        write_json(self.out / 'reports/untitled-checks.json', {'passed': True, 'untitled_image_base': True})

    def alias_refusal(self):
        self.open_print()
        target = self.outputs / 'protected-source.pdf'
        os.link(self.first, target); original = target.read_bytes()
        self.click(self.find(lambda c: c.get('AXDescription') == 'PDF' and c.get('AXRole') == 'AXButton'))
        self.identifier('saveAsNameTextField'); self.choose_target(target); self.click(self.button('替换'))
        self.drained(); self.source(self.first_source)
        assert target.read_bytes() == original == self.first.read_bytes(), 'System panel overwrote protected source alias'
        write_json(self.out / 'reports/source-alias.json', {'passed': True, 'original_sha256': sha(original), 'source_and_alias_preserved': True})

    def system_outputs(self):
        results = []
        for name, pages, landscape in [('system-all', None, False), ('system-range', (2, 3), False), ('system-landscape', None, True)]:
            self.open_print(); results.append(self.save_pdf(name, self.first_source, pages, landscape))
        write_json(self.out / 'reports/system-output-summary.json', [{'pages': r['page_count'], 'sha256': r['sha256']} for r in results])
        assert subprocess.check_output(['lpstat', '-W', 'all', '-o'], text=True) == self.queue_before, 'Printer queue changed during file-only test'

    def click(self, item):
        if item.get('AXRole') == 'AXButton' and item.get('AXTitle') in ['打印', 'Print']:
            raise AssertionError('Physical Print is never admitted by this acceptance suite')
        super().click(item)

    def open_print(self, menu=False):
        if menu:
            self.event('menu-open', '文件')
            self.click(self.find(lambda c: c.get('AXTitle') == '打印…' and c.get('AXRole') == 'AXMenuItem'))
        else:
            self.key(35, 'cmd')
        self.wait_print()

    def wait_print(self):
        # AppKit may keep a previous panel's AX tree while the new snapshot is
        # preparing. Require the new prepared file AND current modal focus;
        # a stale PDPrint button is not proof the new panel is interactive.
        deadline = time.monotonic() + 90
        while True:
            owned = set(self.foundation_temp.glob('yu-print-*')) - self.existing_print_dirs
            ready = len(owned) == 1 and all((p / 'prepared.pdf').is_file() for p in owned)
            if ready and self.event('snapshot').get('focused_window_identifier') == 'PDPrintPanel': break
            assert time.monotonic() < deadline, 'The current print panel did not become ready'
            time.sleep(.1)
        self.find(lambda c: c.get('AXRole') == 'AXButton' and c.get('AXTitle') in ['打印', 'Print'], timeout=90)
        self.settled_controls(['PR', 'PRStart', 'PREnd', 'LandscapeButton', 'PDPrint'])

    def panel_cancel(self):
        before = self.event('snapshot')['AXSelectedTextRanges']
        self.open_print(menu=True)
        controls = self.controls()
        write_json(self.out / 'reports/panel-controls.json', controls)
        self.capture('print-panel')
        self.click(self.button('取消'))
        self.source(self.first_source)
        self.drained()
        assert self.event('snapshot')['AXSelectedTextRanges'] == before
        self.sample('after-panel-cancel')


def run_suite(ctx: RunContext) -> int:
    checks = PrintChecks(ctx)
    try:
        checks.setup()
        phase = os.environ.get('GROUP5_PRINT_PHASE', 'window')
        if phase == 'lifecycle': checks.lifecycle()
        elif phase == 'resources': checks.resource_paths()
        elif phase == 'untitled': checks.untitled_path()
        elif phase == 'save':
            # A separate named path; it never converts a failed window/range
            # phase into a pass or closes T01. Original failures remain recorded.
            checks.panel_cancel(); checks.open_print()
            checks.save_pdf('系统打印 中文 空格', checks.first_source)
            checks.alias_refusal()
        elif phase == 'window': checks.panel_cancel(); checks.system_outputs(); checks.alias_refusal()
        else: raise ValueError('Unknown print phase')
        write_json(ctx.output / 'reports/summary.json', {'passed': True, 'groups_closed': 0, 'physical_jobs_submitted': 0, 'phase': phase, 'scope': 'Only the selected named print software paths; no physical printing'})
        return 0
    except Exception:
        if checks.process and checks.process.poll() is None:
            try: checks.capture('print-failure')
            except Exception: pass
        raise
    finally:
        checks.cleanup()
