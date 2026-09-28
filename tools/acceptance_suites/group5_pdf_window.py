"""First PDF menu/save/cover check. Reuse the existing isolated desktop driver.
This does not sign off all PDF settings, repeated runs or cancellation cases.
"""
from __future__ import annotations
import time
import json
from pathlib import Path
from acceptance_runner import RunContext, sha, write_json
from acceptance_suites.group5_window import WindowChecks


class PDFWindowChecks(WindowChecks):
    def open_export(self):
        self.event('menu-open', '文件')
        self.click(self.find(lambda c: c.get('AXTitle') == '导出 PDF…'))
        self.identifier('saveAsNameTextField')

    def completed_pdf(self, target):
        self.find(lambda c: c.get('AXValue') == 'PDF 导出完成', timeout=90)
        payload = target.read_bytes()
        assert payload.startswith(b'%PDF-') and len(payload) > 1000
        self.source(self.first_source)
        self.capture('pdf-complete')
        self.click(self.button('关闭'))
        return {'path': str(target.relative_to(self.out)), 'sha256': sha(payload), 'bytes': len(payload),
                'scope': 'Real menu produced PDF; independent page/text/visual check separate'}

    def menus(self):
        before = self.event('snapshot')['AXSelectedTextRanges']
        self.open_export()
        assert self.identifier('yu-pdf-paper')['AXValue'] == 'A4'
        assert self.identifier('yu-pdf-orientation')['AXValue'] == '纵向'
        assert str(self.identifier('yu-pdf-margin')['AXValue']) == '44'
        self.capture('pdf-save-settings')
        self.click(self.identifier('CancelButton'))
        self.source(self.first_source)
        target = self.outputs / 'PDF 中文 空格.pdf'
        self.open_export(); self.choose_target(target)
        result = self.completed_pdf(target)
        original = target.read_bytes()
        self.open_export(); self.choose_target(target)
        self.button('替换'); self.capture('pdf-cover-confirmation')
        # Escape belongs to the frontmost replacement sheet. The save panel's
        # CancelButton remains in AXChildren behind that sheet and is not the
        # sheet's Cancel action; do not click an obscured parent control.
        self.key(53)
        deadline = time.monotonic() + 5
        while any(c.get('AXTitle') == '替换' and c.get('AXEnabled') for c in self.controls()):
            assert time.monotonic() < deadline, 'Replacement sheet did not cancel'
            time.sleep(.1)
        self.click(self.identifier('CancelButton'))
        assert target.read_bytes() == original
        self.open_export(); self.choose_target(target); self.click(self.button('替换'))
        result = self.completed_pdf(target)
        assert self.event('snapshot')['AXSelectedTextRanges'] == before
        assert self.first.read_text() == self.first_source
        assert not list(self.outputs.glob('.yu-export-*'))
        write_json(self.out / 'reports/pdf-output.json', result)
        self.sample('pdf-menu-completed')


def run_suite(ctx: RunContext) -> int:
    checks = PDFWindowChecks(ctx)
    ctx.ledger.add('pdf-menu-save-cancel-overwrite', ['real_window'], metadata={'groups': ['X02', 'X03', 'F06'], 'scope': 'A4 portrait menu path; no whole 5B sign-off'})
    try:
        checks.setup()
        lock = ctx.output / 'source-lock.json'
        identity = json.loads(lock.read_text())
        identity['pdf_suite_sha256'] = sha(Path(__file__).read_bytes())
        write_json(lock, identity)
        checks.menus()
        ctx.ledger.record('pdf-menu-save-cancel-overwrite', 'real_window', 'passed', checks.last_command)
        write_json(ctx.output / 'reports/summary.json', {'passed': True, 'batch_5b': 'partial', 'groups_closed': 0})
        return 0
    except Exception:
        if checks.process and checks.process.poll() is None:
            try: checks.capture('pdf-window-failure')
            except Exception: pass
        if checks.last_command in ctx.commands:
            ctx.ledger.record('pdf-menu-save-cancel-overwrite', 'real_window', 'failed', checks.last_command)
        write_json(ctx.output / 'reports/summary.json', {'passed': False, 'batch_5b': 'partial', 'groups_closed': 0})
        raise
    finally:
        write_json(ctx.output / 'reports/cases.json', {'cases': ctx.ledger.finish(), 'groups_closed': 0})
        checks.cleanup()
