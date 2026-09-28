"""Remaining 5A warning/failure interactions, using the existing AX runner.

Permission changes apply only to this run's disposable fixtures and are restored
in finally blocks. No system networking, clipboard or user preferences change.
"""
from __future__ import annotations

import os
import time
from pathlib import Path
from acceptance_runner import RunContext, sha, write_json
from acceptance_suites.group5_window import WindowChecks, SHELL


class SafetyChecks(WindowChecks):
    def warning_sheet(self):
        self.find(lambda c: c.get('AXValue') == '部分内容需要占位或诊断，是否继续？')
        return self.button('带占位或诊断继续导出')

    def close_document(self, dirty=False):
        self.key(13, 'cmd')
        if dirty:
            self.click(self.button('不保存'))
        self.source(self.first_source)

    def failure(self, message, target, expected, original=b'OLD-OUTPUT-KEEP'):
        self.find(lambda c: message in str(c.get('AXValue') or ''))
        self.find(lambda c: c.get('AXValue') == '没有提交新的输出文件。原文及已有输出保持不变。')
        assert target.read_bytes() == original, 'Failure changed the old output'
        self.source(expected)
        self.capture('failure-preserves-output-' + target.stem)
        self.click(self.button('关闭'))
        assert not list(target.parent.glob('.yu-export-*'))

    def warnings(self):
        source = (SHELL / 'Fixtures/group5-export-warning.md').read_text()
        fixture = self.inputs / '诊断.md'; fixture.write_text(source)
        self.open_document(fixture, source)
        before = self.event('snapshot')['AXSelectedTextRanges']
        target = self.outputs / '诊断.html'; target.write_bytes(b'OLD-OUTPUT-KEEP')
        self.open_export(); self.choose_target(target); self.click(self.button('替换'))
        self.warning_sheet(); self.capture('warnings-before-consent')
        assert target.read_bytes() == b'OLD-OUTPUT-KEEP', 'Warnings committed without consent'
        # Escape chooses the sheet's default cancellation, not a stale panel coordinate.
        self.key(53)
        self.source(source)
        deadline = time.monotonic() + 5
        while any(c.get('AXValue') == '部分内容需要占位或诊断，是否继续？' for c in self.controls()):
            assert time.monotonic() < deadline, 'Warning sheet stayed open after cancellation'
            time.sleep(.1)
        assert target.read_bytes() == b'OLD-OUTPUT-KEEP'
        self.open_export(); self.choose_target(target); self.click(self.button('替换'))
        self.click(self.warning_sheet())
        self.find(lambda c: c.get('AXValue') == '已带占位或诊断导出，请检查警告')
        self.find(lambda c: '不是完整成功' in str(c.get('AXValue') or ''))
        html = target.read_text()
        for marker in ['GROUP5-WARNING-END', '不得执行', '远程图片', '缺失图片']:
            assert marker in html, marker
        assert '<script' not in html and 'src="https:' not in html
        self.source(source)
        assert self.event('snapshot')['AXSelectedTextRanges'] == before
        assert fixture.read_text() == source
        self.capture('completed-with-warnings')
        self.click(self.button('关闭')); self.close_document()
        write_json(self.out / 'reports/warnings.json', {'passed': True, 'phase': 'completed_with_warnings',
            'warning_cancel_keeps_old': True, 'consent_required': True,
            'html': self.ctx.artifact(target)})

    def missing_base(self):
        self.key(45, 'cmd'); self.source('')
        source = '# 未命名拒绝\n\n![图片](assets/yu-mark.png)\n'
        self.event('text', source); self.source(source)
        target = self.outputs / '没有基准.html'; target.write_bytes(b'OLD-OUTPUT-KEEP')
        self.open_export(); self.choose_target(target); self.click(self.button('替换'))
        self.failure('需要选择资源基准目录', target, source)
        self.key(6, 'cmd'); self.source('')
        self.key(6, 'cmd+shift'); self.source(source)
        self.close_document(dirty=True)

    def unreadable_image(self):
        image = self.inputs / '无权限.png'
        image.write_bytes((SHELL / 'Fixtures/assets/yu-mark.png').read_bytes())
        source = '# 无权限资源\n\n![不可读图片](无权限.png)\n\nREAD-ERROR-END\n'
        fixture = self.inputs / '无权限图片.md'; fixture.write_text(source)
        self.open_document(fixture, source)
        image.chmod(0)
        try:
            try:
                image.read_bytes()
            except PermissionError:
                pass
            else:
                raise AssertionError('Read-denied fixture is readable; no valid negative test')
            target = self.outputs / '无权限图片.html'
            self.open_export(); self.choose_target(target)
            self.click(self.warning_sheet())
            self.find(lambda c: c.get('AXValue') == '已带占位或诊断导出，请检查警告')
            assert '不可读图片' in target.read_text() and 'READ-ERROR-END' in target.read_text()
            self.source(source); self.capture('unreadable-image-diagnostic')
            self.click(self.button('关闭'))
        finally:
            image.chmod(0o600)
        self.close_document()

    def denied_write(self):
        # Warnings hold the real job before publication. Revoke ONLY this task's
        # destination directory, then approve, so the write fails deterministically.
        source = '# 写入失败\n\n![缺失](missing.png)\n\nWRITE-FAIL-END\n'
        fixture = self.inputs / '写入失败.md'; fixture.write_text(source)
        self.open_document(fixture, source)
        directory = self.outputs / '不可写'; directory.mkdir()
        target = directory / '旧输出.html'; target.write_bytes(b'OLD-OUTPUT-KEEP')
        self.open_export(); self.choose_target(target); self.click(self.button('替换'))
        approve = self.warning_sheet()
        directory.chmod(0o500)
        try:
            self.click(approve)
            self.failure('不能在目标目录创建导出临时文件', target, source)
        finally:
            directory.chmod(0o700)
        self.close_document()


def run_suite(ctx: RunContext) -> int:
    checks = SafetyChecks(ctx)
    cases = [('warning-consent-and-cancellation', ['X02', 'X04', 'H04'], checks.warnings),
             ('untitled-base-refusal-history', ['X01', 'X02', 'X04'], checks.missing_base),
             ('unreadable-image-warning', ['X04', 'H04'], checks.unreadable_image),
             ('denied-write-keeps-old', ['X02', 'X03'], checks.denied_write)]
    for name, groups, _ in cases:
        ctx.ledger.add(name, ['real_window'], metadata={'groups': groups})
    good = False
    try:
        checks.setup()
        checks.event('source', 'com.apple.keylayout.ABC')
        for name, _, run in cases:
            try:
                run()
            except Exception as error:
                ctx.ledger.record(name, 'real_window', 'failed', checks.last_command, evidence={'error': repr(error)})
                try: checks.capture('failed-' + name)
                except Exception: pass
                raise
            ctx.ledger.record(name, 'real_window', 'passed', checks.last_command)
            write_json(ctx.output / 'reports/cases.json', {'cases': ctx.ledger.finish(), 'groups_closed': 0})
            print('PASS:', name, flush=True)
        good = True
        return 0
    finally:
        checks.cleanup()
        write_json(ctx.output / 'reports/cases.json', {'cases': ctx.ledger.finish(), 'groups_closed': 0})
        write_json(ctx.output / 'reports/summary.json', {'passed': good, 'batch_5a': 'partial',
            'scope': 'Four named warning/failure interactions, not full 5A sign-off'})
