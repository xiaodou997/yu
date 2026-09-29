"""Finite real PNG exports and preparation cancellation in an isolated bundle."""
from __future__ import annotations
import time
from acceptance_runner import RunContext, write_json
from acceptance_suites.group5_png_window import PNGChecks

class PNGLifecycle(PNGChecks):
    def export_one(self,path,source,name):
        self.event('raise-window',path);self.source(source)
        self.open_export();self.options(1)
        target=self.inputs/(name+'.png');parts=self.inputs/(name+'-images')
        existed=target.exists()
        self.choose_target(target)
        if existed:self.click(self.button('替换'))
        state=self.find(lambda c:c.get('AXValue')=='PNG 导出完成' or (c.get('AXRole')=='AXButton' and c.get('AXTitle')=='分段导出'),timeout=180)
        if state.get('AXTitle')=='分段导出':
            assert not parts.exists();self.event('export-button','分段导出')
            output=parts
        else:output=target
        rows=self.complete_png(output,source,800)
        if output==parts:
            assert [p.name for p in sorted(parts.glob('*.png'))]==[f'part-{n+1:03}.png' for n in range(len(rows))]
        return rows
    def run_checks(self):
        self.open_document(self.second,self.second_source)
        repeats=[]
        for n in range(10):
            long=n%2==1;path,source=(self.second,self.second_source) if long else (self.first,self.first_source)
            self.sample(f'png-{n+1}-start')
            rows=self.export_one(path,source,f'repeat-{n+1:02}')
            repeats.append({'number':n+1,'document':'long-bom-crlf' if long else 'composite','parts':rows})
            self.sample(f'png-{n+1}-done');write_json(self.out/'reports/ten-pngs.json',repeats)
        source='# PNG 运行取消\n\n'+''.join(f'第{n}项 $x_{{{n}}}^2+1$\n\n' for n in range(1000))+'PNG-CANCEL-END\n'
        path=self.inputs/'png-cancel.md';path.write_text(source);self.open_document(path,source)
        retries=[]
        for n in range(3):
            target=self.inputs/f'cancel-{n+1}.png';target.write_bytes(b'OLD-PNG-KEEP')
            self.open_export();self.options(1);self.choose_target(target);self.click(self.button('替换'))
            self.button('取消导出');self.event('export-button','取消导出');self.source(source)
            time.sleep(.25);assert target.read_bytes()==b'OLD-PNG-KEEP'
            assert not (self.inputs/f'cancel-{n+1}-images').exists()
            rows=self.export_one(path,source,f'cancel-{n+1}')
            retries.append({'number':n+1,'old_output_preserved_on_cancel':True,'retry_parts':rows})
            write_json(self.out/'reports/three-png-cancels.json',retries)
        target=self.inputs/'owner-close.png';target.write_bytes(b'OLD-PNG-KEEP')
        self.open_export();self.options(1);self.choose_target(target);self.click(self.button('替换'));self.button('取消导出')
        self.key(13,'cmd');self.event('raise-window',self.first);self.source(self.first_source)
        time.sleep(.5);assert target.read_bytes()==b'OLD-PNG-KEEP'
        self.event('raise-window',self.second);self.key(13,'cmd');self.source(self.first_source)
        self.event('select',len(self.first_source.encode('utf-16-le'))//2,0)
        self.event('text','PNG-CONTINUE');self.source(self.first_source+'PNG-CONTINUE');self.key(6,'cmd');self.source(self.first_source)
        deadline=time.monotonic()+75
        while True:
            self.sample('png-idle')
            if not self.resources[-1]['children']:break
            assert time.monotonic()<deadline,'Helper did not exit in idle grace';time.sleep(5)
        assert not list(self.inputs.glob('.yu-export-*'))
        write_json(self.out/'reports/summary.json',{'passed':True,'groups_closed':0,'exports':10,'running_cancels':3,'scope':'PNG finite desktop lifecycle; not a permanent memory bound'})

def run_suite(ctx:RunContext)->int:
    checks=PNGLifecycle(ctx)
    try:checks.setup();checks.run_checks();return 0
    except Exception:
        if checks.process and checks.process.poll() is None:
            try:checks.capture('png-lifecycle-failure')
            except Exception:pass
        raise
    finally:checks.cleanup()
