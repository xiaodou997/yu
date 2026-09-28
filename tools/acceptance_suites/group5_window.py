"""Bounded 5A real-menu checks using the existing global evidence runner.

Only isolated test bundles/documents are mutated. Keyboard text injection does
not use the system clipboard. Browser/visual sign-off remains a separate layer.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import time
import uuid

from acceptance_runner import ROOT, RunContext, sha, write_json

SHELL = ROOT / 'platform/macos/yu-shell-macos'


class WindowChecks:
    def __init__(self, ctx: RunContext):
        self.ctx = ctx
        self.out = ctx.output
        self.driver = self.out / 'native-event-driver'
        self.sequence = 0
        self.process = None
        self.input_source = None
        self.resources = []
        self.last_command = 'preflight'
        self.long_formula_count = 1000 if os.environ.get('GROUP5_WINDOW_PHASE') == 'cancellation' else 220

    def command(self, argv):
        self.sequence += 1
        self.last_command = f'event-{self.sequence:04}'
        code = self.ctx.run(self.last_command, list(map(str, argv)))
        text = self.ctx.log(self.last_command).read_text()
        if code:
            raise RuntimeError(f'{self.last_command}: {text[-1200:]}')
        return json.loads(text) if text.lstrip().startswith('{') else None

    def event(self, *args):
        if self.process is None or self.process.poll() is not None:
            raise RuntimeError('The isolated test application is not running')
        return self.command([self.driver, self.process.pid, *args])

    def controls(self):
        return self.event('controls')['controls']

    def find(self, predicate, timeout=10):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for item in self.controls():
                if predicate(item):
                    return item
            time.sleep(.12)
        raise AssertionError('Expected native control was not observed')

    def identifier(self, value):
        return self.find(lambda item: item.get('AXIdentifier') == value)

    def button(self, title):
        return self.find(lambda item: item.get('AXRole') == 'AXButton'
                         and item.get('AXTitle') == title and item.get('AXEnabled'))

    def click(self, item):
        point, size = item['AXPosition'], item['AXSize']
        assert size['width'] > 0 and size['height'] > 0, 'Control is not visible'
        self.event('click', point['x'] + size['width'] / 2,
                   point['y'] + size['height'] / 2)

    def key(self, code, flags=''):
        self.event('key', code, flags)

    def source(self, expected):
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            state = self.event('snapshot')
            if state.get('AXValue') == expected:
                return state
            time.sleep(.12)
        raise AssertionError('Canonical source differs from the exact expected text')

    def capture(self, name):
        self.event('capture', self.out / 'visual' / name)

    def open_export(self):
        self.event('menu-open', '文件')
        self.click(self.find(lambda c: c.get('AXTitle') == '导出 HTML…'))
        self.identifier('saveAsNameTextField')

    def go(self, path):
        self.key(5, 'cmd+shift')
        time.sleep(.25)
        self.event('text', path)
        state = self.event('snapshot')
        assert state.get('focused_value') == str(path), 'Go-to-folder text was not accepted'
        self.key(36)
        time.sleep(.45)

    def choose_target(self, path):
        self.click(self.identifier('saveAsNameTextField'))
        self.key(0, 'cmd')
        self.event('text', path.name)
        assert self.identifier('saveAsNameTextField')['AXValue'] == path.name
        self.go(path.parent)
        self.click(self.identifier('OKButton'))

    def set_export_style(self, current):
        popup = self.find(lambda c: c.get('AXRole') == 'AXPopUpButton'
                          and c.get('AXValue') in ['浅色', '当前正文主题'])
        wanted = '当前正文主题' if current else '浅色'
        if popup['AXValue'] != wanted:
            self.click(popup)
            self.click(self.find(lambda c: c.get('AXRole') == 'AXMenuItem'
                                 and c.get('AXTitle') == wanted))
        assert self.find(lambda c: c.get('AXRole') == 'AXPopUpButton'
                         and c.get('AXValue') == wanted)

    def complete(self, path, source, marker, current=False, screenshot=None):
        self.find(lambda c: c.get('AXValue') == 'HTML 导出完成', timeout=90)
        payload = path.read_text(encoding='utf-8')
        assert marker in payload and '<!doctype html>' in payload
        assert '<script' not in payload and 'src="https:' not in payload
        assert 'GROUP5-PRIVATE-METADATA-MUST-NOT-LEAK' not in payload
        assert ('--bg:#ffffff' not in payload) if current else ('--bg:#ffffff' in payload)
        self.source(source)
        if screenshot:
            self.capture(screenshot)
        self.click(self.button('关闭'))
        return {'path': str(path.relative_to(self.out)), 'sha256': sha(path.read_bytes()),
                'embedded': payload.count('class="yu-embedded"'), 'bytes': path.stat().st_size}

    def open_document(self, path, source):
        self.key(31, 'cmd')
        self.find(lambda c: c.get('AXIdentifier') == 'OKButton')
        self.go(path)
        self.click(self.identifier('OKButton'))
        self.source(source)
        self.event('raise-window', path)

    def sample(self, phase):
        sample = self.command([self.out / 'process-footprint', self.process.pid])
        listing = subprocess.check_output(['ps', '-axo', 'pid=,ppid=,rss=,comm='], text=True)
        children = []
        for line in listing.splitlines():
            columns = line.strip().split(None, 3)
            if len(columns) == 4 and columns[1] == str(self.process.pid):
                children.append({'pid': int(columns[0]), 'rss_kib': int(columns[2]),
                                 'executable': Path(columns[3]).name})
        sample.update(phase=phase, children=children, monotonic_seconds=time.monotonic())
        self.resources.append(sample)
        write_json(self.out / 'reports/resources.json', {
            'metric': 'RUSAGE_INFO_V4 physical footprint; child RSS is separate',
            'samples': self.resources, 'claim': 'bounded observation, not a leak-free guarantee'})

    def setup(self):
        assert self.ctx.run('build-event-driver', ['swiftc', str(ROOT / 'tools/native-event-driver.swift'), '-o', str(self.driver)]) == 0
        assert self.ctx.run('preflight', [str(self.driver), '--preflight']) == 0
        assert self.ctx.run('build-footprint', ['clang', str(ROOT / 'tools/process-footprint.c'), '-o', str(self.out / 'process-footprint')]) == 0
        production = SHELL / '.build/Yu.app'
        build = json.loads((SHELL / '.build/build-manifest.json').read_text())
        assert build['configuration'] == 'release'
        assert sha((production / 'Contents/MacOS/Yu').read_bytes()) == build['app_sha256']
        self.app = self.out / 'YuExportChecks.app'
        shutil.copytree(production, self.app)
        info_path = self.app / 'Contents/Info.plist'
        info = plistlib.loads(info_path.read_bytes())
        self.bundle_id = 'io.github.xiaodou997.yu.export-check.' + uuid.uuid4().hex
        info['CFBundleIdentifier'] = self.bundle_id
        info_path.write_bytes(plistlib.dumps(info))
        assert self.ctx.run('sign-isolated-bundle', ['codesign', '--force', '--sign', '-', '--identifier', self.bundle_id, str(self.app)]) == 0
        for name, arguments in [('disable-autosave', ['Yu.autosaveEnabled', '-bool', 'false']),
                                ('set-isolated-theme', ['Yu.readingTheme', '-int', '2'])]:
            assert self.ctx.run(name, ['defaults', 'write', self.bundle_id, *arguments]) == 0
        self.inputs = self.out / 'inputs'; self.inputs.mkdir()
        self.outputs = self.out / 'outputs'; self.outputs.mkdir()
        (self.out / 'visual').mkdir()
        shutil.copytree(SHELL / 'Fixtures/assets', self.inputs / 'assets')
        self.first = self.inputs / '综合 中文.md'
        self.first_source = (SHELL / 'Fixtures/group5-export.md').read_text()
        self.first.write_text(self.first_source, encoding='utf-8')
        self.second = self.inputs / '长文 BOM.md'
        self.second_source = '# 第二篇长文\r\n\r\n' + ''.join(
            f'第 {i} 段中文 English 🙂。固定公式 $x_{{{i}}}^2+1$。' + '正文不可丢失。' * 40 + '\r\n\r\n'
            for i in range(self.long_formula_count)) + 'SECOND-DOCUMENT-END\r\n'
        self.second.write_bytes(b'\xef\xbb\xbf' + self.second_source.encode())
        write_json(self.out / 'source-lock.json', {
            'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
            'tracked_diff_sha256': sha(subprocess.check_output(['git', 'diff', '--binary'], cwd=ROOT)),
            'suite_sha256': sha(Path(__file__).read_bytes()),
            'input_sha256': {p.name: sha(p.read_bytes()) for p in [self.first, self.second]},
            'driver_source_sha256': sha((ROOT / 'tools/native-event-driver.swift').read_bytes())})
        write_json(self.out / 'build-manifest.json', {'production': build, 'isolated_bundle': self.bundle_id,
            'isolated_executable_sha256': sha((self.app / 'Contents/MacOS/Yu').read_bytes())})
        environment = {k: v for k, v in os.environ.items() if not k.startswith('YU_')}
        environment.update(YU_DOCUMENT_STATE_DIR=str(self.out / 'state'), YU_PRESENTATION_STATE_DIR=str(self.out / 'columns'))
        self.log = (self.out / 'logs/application.log').open('x')
        self.process = subprocess.Popen([str(self.app / 'Contents/MacOS/Yu'), str(self.first)],
                                        stdout=self.log, stderr=subprocess.STDOUT, env=environment)
        # Process creation precedes LaunchServices/AX registration. Observe the
        # same PID with a deadline; do not relaunch or repeat document mutations.
        deadline = time.monotonic() + 12
        while True:
            try:
                initial = self.event('activate')
                break
            except RuntimeError as error:
                transient = any(token in str(error) for token in [
                    'Usage: native-event-driver', 'No native document AXTextArea',
                    'Target app did not become frontmost'])
                if not transient or time.monotonic() >= deadline:
                    raise
                time.sleep(.2)
        self.input_source = initial['input_source']
        self.source(self.first_source)
        self.sample('baseline')

    def ime(self):
        self.event('select', len(self.first_source.encode('utf-16-le')) // 2, 0)
        self.event('source', 'com.apple.inputmethod.SCIM.ITABC')
        self.event('keys', 45, 34)
        self.source(self.first_source)
        self.capture('ime-candidate-before')
        self.event('menu-open', '文件')
        self.click(self.find(lambda c: c.get('AXTitle') == '导出 HTML…'))
        # Existing status chrome is visible in the window capture but is not
        # exposed by this driver's AXChildren walk. Do not equate that with
        # composition loss; assert the input contract and review the capture.
        time.sleep(.3)
        assert not any(c.get('AXIdentifier') == 'save-panel' for c in self.controls())
        assert self.source(self.first_source)['focused_role'] == 'AXTextArea'
        self.capture('ime-nonmodal-refusal')
        # The second request is still refused: the first did not cancel preedit.
        self.event('menu-open', '文件')
        self.click(self.find(lambda c: c.get('AXTitle') == '导出 HTML…'))
        time.sleep(.2)
        assert not any(c.get('AXIdentifier') == 'save-panel' for c in self.controls())
        self.source(self.first_source)
        self.key(53)  # User explicitly cancels their composition.
        self.source(self.first_source)
        self.open_export()
        self.click(self.identifier('CancelButton'))
        self.event('source', 'com.apple.inputmethod.SCIM.ITABC')
        self.event('keys', 45, 34)
        self.key(49)
        self.source(self.first_source + '你')
        self.key(6, 'cmd')
        self.source(self.first_source)
        self.event('source', self.input_source)

    def menus(self):
        initial_selection = self.event('snapshot')['AXSelectedTextRanges']
        self.open_export()
        self.click(self.identifier('CancelButton'))
        self.source(self.first_source)
        target = self.outputs / '中文 空格.html'
        self.open_export(); self.set_export_style(False); self.choose_target(target)
        before = self.complete(target, self.first_source, 'GROUP5-DOCUMENT-END', screenshot='light-complete')
        self.open_export(); self.choose_target(target)
        self.button('替换')
        self.capture('overwrite-confirmation')
        # The overwrite sheet and save panel both have Cancel. Use sheet identity.
        self.click(self.identifier('action-button-2'))
        assert sha(target.read_bytes()) == before['sha256']
        self.click(self.identifier('CancelButton'))
        self.open_export(); self.choose_target(target); self.click(self.button('替换'))
        after = self.complete(target, self.first_source, 'GROUP5-DOCUMENT-END')
        assert before == after
        assert self.event('snapshot')['AXSelectedTextRanges'] == initial_selection
        write_json(self.out / 'reports/menu-output.json', after)
        # A changed viewport/source-mode presentation must not change the
        # whole-document snapshot or leak editor chrome into the artifact.
        self.event('select', len(self.first_source.encode('utf-16-le')) // 2, 0)
        # Use the existing native key-equivalent route. AXPress can return
        # success while AppKit is still tracking the menu; that is not proof
        # that a zoom/source-mode command ran. Verify zoom via its menu state.
        self.event('raise-window', self.first)
        for _ in range(2):
            self.key(24, 'cmd+shift')
        self.event('menu-open', '显示')
        self.find(lambda c: c.get('AXTitle') == '实际大小' and c.get('AXEnabled'))
        self.key(53)
        self.key(46, 'cmd+shift')
        self.source(self.first_source)
        self.capture('presentation-before-export')
        selection = self.event('snapshot')['AXSelectedTextRanges']
        second = self.outputs / 'presentation-isolation.html'
        self.open_export()
        self.set_export_style(False); self.choose_target(second)
        result = self.complete(second, self.first_source, 'GROUP5-DOCUMENT-END')
        assert result['sha256'] == after['sha256'], 'Viewport/source mode changed exported bytes'
        assert self.event('snapshot')['AXSelectedTextRanges'] == selection
        write_json(self.out / 'reports/presentation-isolation.json', {
            'passed': True, 'same_output_sha256': result['sha256'],
            'source_mode_shortcut': 'Command-Shift-M', 'zoom_in_shortcuts': 2,
            'end_of_document_selection_preserved': True})
        self.key(46, 'cmd+shift')
        self.key(29, 'cmd')

    def untitled(self):
        self.key(45, 'cmd')
        self.source('')
        text = '# 未命名导出\n\n![本地图片](assets/yu-mark.png)\n\nUNTITLED-DOCUMENT-END\n'
        self.event('text', text); self.source(text)
        self.open_export()
        self.click(self.button('选择图片基准目录…'))
        self.find(lambda c: c.get('AXIdentifier') == 'OKButton')
        self.go(self.inputs); self.click(self.identifier('OKButton'))
        self.find(lambda c: c.get('AXValue') == '图片基准目录：inputs')
        target = self.outputs / '未命名 含图片.html'
        self.set_export_style(False); self.choose_target(target)
        result = self.complete(target, text, 'UNTITLED-DOCUMENT-END', screenshot='untitled-complete')
        assert target.read_text().count('data:image/png;base64,') == 1
        assert not (self.inputs / '未命名导出.md').exists()
        self.key(13, 'cmd'); self.click(self.button('不保存'))
        self.source(self.first_source)
        write_json(self.out / 'reports/untitled-output.json', result)

    def repeat(self):
        self.open_document(self.second, self.second_source)
        outputs = []
        for number in range(10):
            long = number % 2 == 1
            path, source, marker = ((self.second, self.second_source, 'SECOND-DOCUMENT-END') if long
                                    else (self.first, self.first_source, 'GROUP5-DOCUMENT-END'))
            self.event('raise-window', path); self.source(source)
            self.open_export(); self.set_export_style(long)
            target = self.outputs / f'repeat-{number + 1:02}.html'
            self.choose_target(target); self.sample(f'export-{number + 1:02}-started')
            result = self.complete(target, source, marker, current=long,
                                   screenshot='dark-complete' if number == 9 else None)
            assert result['embedded'] == (220 if long else 14)
            assert ('GROUP5-DOCUMENT-END' not in target.read_text()) if long else ('SECOND-DOCUMENT-END' not in target.read_text())
            outputs.append(result); self.sample(f'export-{number + 1:02}-completed')
            write_json(self.out / 'reports/repeated-outputs.json', outputs)
        assert not list(self.outputs.glob('.yu-export-*')), 'Temporary export files remain'

    def cancellations(self):
        self.event('raise-window', self.second)
        self.source(self.second_source)
        results = []
        for number in range(3):
            target = self.outputs / f'cancel-retry-{number + 1}.html'
            target.write_bytes(b'OLD-OUTPUT-KEEP')
            self.open_export(); self.set_export_style(True); self.choose_target(target); self.click(self.button('替换'))
            self.click(self.button('取消导出'))
            time.sleep(.35)
            assert target.read_bytes() == b'OLD-OUTPUT-KEEP'
            self.source(self.second_source)
            self.sample(f'cancel-{number + 1}')
            self.open_export(); self.set_export_style(True); self.choose_target(target); self.click(self.button('替换'))
            result = self.complete(target, self.second_source, 'SECOND-DOCUMENT-END', current=True)
            assert result['embedded'] == self.long_formula_count
            results.append(result)
            write_json(self.out / 'reports/cancel-retry.json', results)
        target = self.outputs / 'owner-close.html'
        target.write_bytes(b'OLD-OUTPUT-KEEP')
        self.open_export(); self.set_export_style(True); self.choose_target(target); self.click(self.button('替换'))
        self.button('取消导出')  # It must still be a running, cancellable job.
        self.key(13, 'cmd')
        self.source(self.first_source)
        time.sleep(.5)
        assert target.read_bytes() == b'OLD-OUTPUT-KEEP'
        # The ordinary viewport renderer has its own 60-second idle lifecycle.
        # Observe that separately instead of calling every child an export leak.
        deadline = time.monotonic() + 75
        while True:
            self.sample('post-close-idle')
            if not self.resources[-1]['children']:
                break
            assert time.monotonic() < deadline, 'Helper did not exit within idle grace'
            time.sleep(5)
        assert not list(self.outputs.glob('.yu-export-*'))
        write_json(self.out / 'reports/owner-close.json', {'passed': True,
            'cancelled_output_unchanged': True, 'other_document_source_unchanged': True})

    def cleanup(self):
        if self.process and self.process.poll() is None:
            try:
                if self.input_source:
                    self.event('source', self.input_source)
                self.key(12, 'cmd')
                deadline = time.monotonic() + 4
                while self.process.poll() is None and time.monotonic() < deadline:
                    choices = [c for c in self.controls() if c.get('AXTitle') == '不保存' and c.get('AXRole') == 'AXButton']
                    if choices:
                        self.click(choices[0])
                    else:
                        time.sleep(.1)
                self.process.wait(timeout=5)
            except Exception:
                # Only the exact process created by this suite is terminated.
                self.process.terminate(); self.process.wait(timeout=5)
        if hasattr(self, 'log'):
            self.log.close()


def run_suite(ctx: RunContext) -> int:
    checks = WindowChecks(ctx)
    cases = [('ime-preedit-refusal-and-recovery', ['X01', 'X02'], checks.ime),
             ('native-save-cancel-overwrite', ['X02', 'X03'], checks.menus),
             ('untitled-image-base', ['X01', 'X04'], checks.untitled),
             ('ten-alternating-exports', ['X05', 'X06', 'X07', 'H04'], checks.repeat),
             ('cancel-retry-and-owner-close', ['X02', 'X03', 'X07'], checks.cancellations)]
    # Separate bounded phases: the 220-formula repeat fixture can finish while
    # AX is locating Cancel. Running cancellation uses its fixed 1000-formula
    # fixture instead of treating a completed publication as a cancelled job.
    phase = os.environ.get('GROUP5_WINDOW_PHASE', 'window')
    if phase == 'cancellation':
        cases = cases[-1:]
    elif phase == 'window':
        cases = cases[:-1]
    else:
        raise ValueError('GROUP5_WINDOW_PHASE must be window or cancellation')
    for name, groups, _ in cases:
        ctx.ledger.add(name, ['real_window'], metadata={'groups': groups})
    good = False
    try:
        checks.setup()
        if phase == 'cancellation':
            checks.open_document(checks.second, checks.second_source)
        for name, _, execute in cases:
            try:
                execute()
            except Exception as error:
                ctx.ledger.record(name, 'real_window', 'failed', checks.last_command,
                                  evidence={'error': str(error)})
                try:
                    checks.capture('failure-' + name)
                except Exception:
                    pass
                raise
            ctx.ledger.record(name, 'real_window', 'passed', checks.last_command)
            write_json(ctx.output / 'reports/cases.json', {'cases': ctx.ledger.finish(), 'groups_closed': 0})
            print('PASS:', name, flush=True)
        good = True
        return 0
    finally:
        checks.cleanup()
        write_json(ctx.output / 'reports/cases.json', {'cases': ctx.ledger.finish(), 'groups_closed': 0})
        write_json(ctx.output / 'reports/summary.json', {'passed': good, 'batch_5a': 'partial', 'phase': phase,
            'scope': 'Named real-window subcases only; screenshots await independent visual review',
            'groups_closed': 0})
