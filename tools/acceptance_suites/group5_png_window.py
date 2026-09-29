"""PNG real-menu and explicit split consent; artifacts remain local."""
from __future__ import annotations
import json
from acceptance_runner import ROOT, RunContext, write_json
from acceptance_suites.group5_window import WindowChecks

class PNGChecks(WindowChecks):
    def setup(self):
        self.inspector=self.out/'inspect-png'
        assert self.ctx.run('build-png-inspector',['swiftc',str(ROOT/'tools/inspect-png-native.swift'),'-o',str(self.inspector)])==0
        super().setup()
    def open_document(self,path,source):
        assert path.parent == self.inputs
        self.open_count = getattr(self, 'open_count', 0) + 1
        assert self.ctx.run(f'open-owned-png-fixture-{self.open_count}',['/usr/bin/open','-a',str(self.app),str(path)]) == 0
        self.find(lambda c:c.get('AXRole')=='AXWindow' and c.get('AXIdentifier')==str(path))
        self.event('raise-window',path);self.source(source)
    def open_export(self):
        self.event('menu-open','文件')
        self.click(self.find(lambda c:c.get('AXTitle')=='导出 PNG…'))
        self.identifier('saveAsNameTextField')
    def choose_target(self,path):
        # This suite saves into its own fixture directory already selected by
        # the document's native panel. Folder navigation is not signed off here.
        assert path.parent == self.inputs
        self.event('save-name-value',path.name);self.click(self.identifier('OKButton'))
    def options(self,multiplier=1,current=False):
        self.event('png-setting','style','当前正文主题' if current else '浅色')
        self.event('png-setting','width','800')
        self.event('png-setting','scale',f'{multiplier}×')
    def complete_png(self,target,source,width):
        self.find(lambda c:c.get('AXValue')=='PNG 导出完成',timeout=180)
        self.source(source)
        urls=sorted(target.glob('part-*.png')) if target.is_dir() else [target]
        reports=[self.command([self.inspector,p]) for p in urls]
        assert reports and all(r['decoded'] and r['width']==width for r in reports)
        self.button('关闭');self.event('export-button','关闭')
        return reports
    def normal(self):
        before=self.event('snapshot')['AXSelectedTextRanges']
        self.open_export();self.click(self.identifier('CancelButton'));self.source(self.first_source)
        reports=[]
        for scale,current in [(1,False),(2,True)]:
            self.open_export();self.options(scale,current)
            if scale == 2: self.capture('png-options')
            target=self.inputs/f'menu-{scale}x.png';self.choose_target(target)
            reports.extend(self.complete_png(target,self.first_source,800*scale))
        assert reports[1]['width']==reports[0]['width']*2 and reports[1]['height']==reports[0]['height']*2
        assert self.event('snapshot')['AXSelectedTextRanges']==before
        write_json(self.out/'reports/menu-png.json',reports)
    def splitting(self):
        source='# PNG 分段\n\n'+''.join(f'第{n}段 SEGMENT-LINE-{n} 中文完整内容。\n\n' for n in range(700))+'PNG-LONG-END\n'
        path=self.inputs/'png-long.md';path.write_text(source);self.open_document(path,source)
        target=self.inputs/'menu-split.png';directory=self.inputs/'menu-split-images'
        self.open_export();self.options(2);self.choose_target(target)
        self.button('分段导出');assert not directory.exists() and not target.exists()
        self.event('export-button','取消导出');self.source(source)
        self.open_export();self.options(2);self.choose_target(target)
        self.button('分段导出');assert not directory.exists();self.event('export-button','分段导出')
        reports=self.complete_png(directory,source,1600)
        assert len(reports)>1 and not target.exists()
        write_json(self.out/'reports/menu-split.json',reports)
        self.key(13,'cmd');self.source(self.first_source)
        assert not list(self.inputs.glob('.yu-export-*'))

def run_suite(ctx:RunContext)->int:
    checks=PNGChecks(ctx)
    try:
        checks.setup();checks.normal();checks.splitting()
        write_json(ctx.output/'reports/summary.json',{'passed':True,'groups_closed':0,'scope':'Actual PNG menu/options/save and split cancel/consent'})
        return 0
    except Exception:
        if checks.process and checks.process.poll() is None:
            try:checks.capture('png-failure')
            except Exception:pass
        raise
    finally:checks.cleanup()
