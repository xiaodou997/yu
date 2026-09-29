"""PNG real-menu and explicit split consent; artifacts remain local."""
from __future__ import annotations
import json
import os
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

    def resource_paths(self):
        # Untitled document: choose an explicit relative-image base in the
        # native PNG accessory without forcing a Markdown save.
        self.key(45,'cmd');self.source('')
        text='# PNG 未命名\n\n![本地图片](assets/yu-mark.png)\n\nPNG-UNTITLED-END\n'
        self.event('text',text);self.source(text)
        self.open_export();self.options(1)
        self.click(self.button('选择图片基准目录…'))
        self.identifier('OKButton');self.go(self.inputs);self.click(self.identifier('OKButton'))
        self.find(lambda c:c.get('AXValue')=='图片基准目录：inputs')
        self.go(self.inputs)
        target=self.inputs/'untitled-menu.png';self.choose_target(target)
        rows=self.complete_png(target,text,800)
        assert len(rows)==1 and rows[0]['decoded']
        assert not (self.inputs/'未命名.md').exists()
        self.key(13,'cmd');self.click(self.button('不保存'));self.source(self.first_source)

        # Missing resource: first cancel the warning, then explicitly continue.
        warning='# PNG 资源告警\n\n![缺失图片](missing-image.png)\n\nPNG-WARNING-END\n'
        path=self.inputs/'png-warning.md';path.write_text(warning)
        self.open_document(path,warning)
        target=self.inputs/'warning-menu.png';target.write_bytes(b'OLD-PNG-KEEP')
        self.open_export();self.options(1);self.choose_target(target);self.click(self.button('替换'))
        self.button('带占位或诊断继续导出')
        assert target.read_bytes()==b'OLD-PNG-KEEP'
        self.event('export-button','取消导出');self.source(warning)
        assert target.read_bytes()==b'OLD-PNG-KEEP'
        self.open_export();self.options(1);self.choose_target(target);self.click(self.button('替换'))
        proceed=self.button('带占位或诊断继续导出');self.capture('png-warning-confirmation');self.click(proceed)
        self.find(lambda c:c.get('AXValue')=='已带占位或诊断导出，请检查警告',timeout=180)
        report=self.command([self.inspector,target]);assert report['decoded'] and report['width']==800
        self.source(warning);self.event('export-button','关闭')
        self.key(13,'cmd');self.source(self.first_source)
        write_json(self.out/'reports/png-resources.json',{'passed':True,'untitled':rows[0],'warning':report,
            'warning_cancel_preserved_old_output':True,'warning_requires_explicit_continue':True})

def run_suite(ctx:RunContext)->int:
    checks=PNGChecks(ctx)
    try:
        checks.setup()
        phase=os.environ.get('GROUP5_PNG_PHASE','window')
        if phase=='window': checks.normal();checks.splitting()
        elif phase=='resources': checks.resource_paths()
        else: raise ValueError('GROUP5_PNG_PHASE must be window or resources')
        write_json(ctx.output/'reports/summary.json',{'passed':True,'groups_closed':0,'phase':phase,'scope':'Actual PNG menu/options/save, split consent, or resource paths'})
        return 0
    except Exception:
        if checks.process and checks.process.poll() is None:
            try:checks.capture('png-failure')
            except Exception:pass
        raise
    finally:checks.cleanup()
