#!/usr/bin/env python3
"""Real native settings events in a private bundle; expanded as writing features land."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
from urllib.parse import quote, unquote
import shutil
import subprocess
import time
import uuid

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--dark', action='store_true')
    parser.add_argument('--multiwindow-preferences-only', action='store_true', help='Run three-window theme and actual autosave transitions')
    parser.add_argument('--multiwindow-only', action='store_true', help='Run real existing/new-window reading preference synchronization')
    parser.add_argument('--spelling-only', action='store_true', help='Run only the native spelling interaction scenario')
    parser.add_argument('--image-retry-only', action='store_true', help='Run missing-image right-click retry and source/history checks')
    parser.add_argument('--image-panels-only', action='store_true', help='Run native image picker and untitled save/cancel scenarios')
    parser.add_argument('--image-save-as-only', action='store_true', help='Run parent-relative image relocation through native Save As')
    parser.add_argument('--image-drag-first-save-only', action='store_true', help='Run actual file drop and first-save modal cancellation/confirmation')
    parser.add_argument('--image-drag-only', action='store_true', help='Run real external AppKit file dragging scenarios')
    parser.add_argument('--table-only', action='store_true', help='Run native table menu and history scenarios')
    parser.add_argument('--table-context-only', action='store_true', help='Run table right-click interaction scenarios')
    parser.add_argument('--table-boundaries-only', action='store_true', help='Run sequential first/last column and row edits')
    parser.add_argument('--table-action', help='Restrict table checks to one native menu title (requires --table-only)')
    args = parser.parse_args()
    if args.multiwindow_preferences_only: args.multiwindow_only = True
    if args.table_context_only or args.table_boundaries_only: args.table_only = True
    if args.table_context_only and args.table_boundaries_only: parser.error('Choose one table scenario')
    if args.table_only and args.spelling_only: parser.error('Choose only one isolated scenario')
    if args.image_retry_only and (args.table_only or args.spelling_only): parser.error('Choose only one isolated scenario')
    if args.image_panels_only and (args.table_only or args.spelling_only or args.image_retry_only): parser.error('Choose only one isolated scenario')
    if args.image_drag_only and (args.table_only or args.spelling_only or args.image_retry_only or args.image_panels_only): parser.error('Choose only one isolated scenario')
    if args.image_save_as_only and any([args.table_only,args.spelling_only,args.image_retry_only,args.image_panels_only,args.image_drag_only]): parser.error('Choose only one isolated scenario')
    if args.image_drag_first_save_only and any([args.table_only,args.spelling_only,args.image_retry_only,args.image_panels_only,args.image_drag_only,args.image_save_as_only]): parser.error('Choose only one isolated scenario')
    if args.multiwindow_only and any([args.table_only,args.spelling_only,args.image_retry_only,args.image_panels_only,args.image_drag_only,args.image_save_as_only,args.image_drag_first_save_only]): parser.error('Choose only one isolated scenario')
    if args.table_action and not args.table_only: parser.error('--table-action requires --table-only')
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    driver = out / 'native-event-driver'
    subprocess.run(['swiftc', str(ROOT/'tools/native-event-driver.swift'), '-o', str(driver)], check=True)
    preflight = subprocess.run([str(driver), '--preflight'], text=True, capture_output=True, timeout=10)
    (out/'preflight.json').write_text(preflight.stdout)
    if preflight.returncode:
        raise RuntimeError('Desktop or native permissions unavailable; see preflight.json')
    build = json.loads((HERE/'.build/build-manifest.json').read_text())
    production = HERE/'.build/Yu.app'
    digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
    assert build['configuration'] == 'release' and digest(production/'Contents/MacOS/Yu') == build['app_sha256']
    app = out/'YuWritingChecks.app'
    shutil.copytree(production, app)
    info_path = app/'Contents/Info.plist'
    info = plistlib.loads(info_path.read_bytes())
    info['CFBundleIdentifier'] = identifier = 'io.github.xiaodou997.yu.writing-check.'+uuid.uuid4().hex
    info_path.write_bytes(plistlib.dumps(info))
    subprocess.run(['codesign','--force','--sign','-','--identifier',identifier,str(app)],check=True)
    if args.image_save_as_only or args.image_drag_first_save_only or args.multiwindow_only:
        subprocess.run(['defaults','write',identifier,'Yu.autosaveEnabled','-bool','false'],check=True)
    fixture = out/'settings.md'
    source = '# 写作设置\r\n\r\n正文不能被设置快捷键修改。\r\n\r\n| A | B |\r\n| --- | --- |\r\n| one | two |\r\n\r\n尾段\r\n\r\n'
    if args.image_save_as_only:
        (out/'original/notes').mkdir(parents=True)
        (out/'original/shared').mkdir()
        fixture = out/'original/notes/原文.md'
        shutil.copyfile(HERE/'Fixtures/assets/yu-mark.png',out/'original/shared/图 %.png')
        source = '![图](../shared/%E5%9B%BE%20%25.png)\r\n\r\n正文🪶保持\r\n'
    original = b'\xef\xbb\xbf'+source.encode()
    fixture.write_bytes(original)
    env = {k:v for k,v in os.environ.items() if not k.startswith('YU_')}
    env.update(YU_DOCUMENT_STATE_DIR=str(out/'state'), YU_PRESENTATION_STATE_DIR=str(out/'columns'))
    if args.table_only: env['YU_NATIVE_INPUT_TRACE'] = '1'
    command = [str(app/'Contents/MacOS/Yu'),str(fixture)]+(['--dark-mode'] if args.dark else [])
    result = {'passed':False,'isolated_bundle':identifier,'build':build,'test_app_sha256':digest(app/'Contents/MacOS/Yu'),'checks':[]}
    process = None
    sequence = 0

    def run(*arguments):
        nonlocal sequence
        try:
            execution = subprocess.run([str(driver),str(process.pid),*map(str,arguments)],text=True,capture_output=True,timeout=15)
        except subprocess.TimeoutExpired as error:
            sequence += 1
            decode = lambda value: value.decode(errors='replace') if isinstance(value,bytes) else value
            (out/f'event-{sequence:03}.json').write_text(json.dumps({'arguments':arguments,'timed_out':True,
                'stdout':decode(error.stdout),'stderr':decode(error.stderr)},ensure_ascii=False,indent=2))
            raise
        sequence += 1
        (out/f'event-{sequence:03}.json').write_text(json.dumps({'arguments':arguments,'exit_code':execution.returncode,'stdout':execution.stdout,'stderr':execution.stderr},ensure_ascii=False,indent=2))
        if execution.returncode: raise RuntimeError(str(arguments)+': '+execution.stderr)
        return json.loads(execution.stdout) if execution.stdout.strip().startswith('{') else None

    def key(code, modifiers=''): run('key',code,modifiers)
    def controls(): return run('controls')['controls']
    def directory(): return next(c for c in controls() if c.get('AXIdentifier')=='yu-image-directory')
    def click(control):
        p,s = control['AXPosition'],control['AXSize']
        run('click',p['x']+(9 if control.get('AXRole')=='AXCheckBox' else s['width']/2),p['y']+s['height']/2)
    def check(name):
        result['checks'].append(name)
        (out/'results.json').write_text(json.dumps(result,ensure_ascii=False,indent=2))
        print('PASS:',name,flush=True)
    def expect_source(expected, label):
        # AppKit menu actions may be dispatched after the closing animation.
        # Observe completion without repeating a mutation, and retain evidence
        # when the expected document state never arrives.
        started = time.monotonic()
        observations = 0
        while True:
            state = run('snapshot')
            observations += 1
            if state['AXValue'] == expected:
                if observations > 1:
                    result.setdefault('delayed_source_observations',[]).append({
                        'action':label,'observations':observations,'seconds':time.monotonic()-started})
                return state
            if time.monotonic()-started >= 3:
                run('capture',str(out/f'source-failure-{sequence}'))
                raise AssertionError(label + ': source did not reach expected state')
            time.sleep(.05)
    def launch():
        nonlocal process
        log = (out/f'app-{sequence}.log').open('w')
        process = subprocess.Popen(command,env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        deadline = time.monotonic()+12
        while True:
            if process.poll() is not None: raise RuntimeError('Application exited before readiness')
            try: return run('activate')
            except RuntimeError as error:
                if time.monotonic()>=deadline or not any(x in str(error) for x in ['Usage: native-event-driver','No native document AXTextArea','Target app did not become frontmost']): raise
                time.sleep(.2)
    def quit_app():
        key(12,'cmd')
        process.wait(timeout=10)

    try:
        assert launch()['AXValue']==source
        run('resize',1200,800)
        if args.multiwindow_only:
            second = out/'第二篇.md'
            third = out/'第三篇.md'
            for path in [second,third]: path.write_bytes(original)
            mode_titles = ['专注模式（突出当前段落）','打字机模式（输入时保持活动行居中）']
            documents = [fixture,second]
            def open_document(path):
                key(31,'cmd'); time.sleep(.3)
                key(5,'cmd+shift'); time.sleep(.2)
                run('paste-text',str(path))
                assert run('snapshot')['focused_value']==str(path)
                key(36); time.sleep(.3)
                deadline = time.monotonic()+5
                while True:
                    button = next((c for c in controls() if c.get('AXIdentifier')=='OKButton' and c.get('AXEnabled') is True),None)
                    if button is not None: break
                    assert time.monotonic()<deadline, 'Open panel did not select fixture'
                    time.sleep(.1)
                click(button)
                expect_source(source,'open another document')
                run('raise-window',str(path))
                run('resize',1200,800)
            def settings():
                key(43,'cmd'); time.sleep(.2)
                return controls()
            def close_settings():
                key(13,'cmd'); time.sleep(.2)
                assert not any(c.get('AXTitle')=='Yu 设置' for c in controls())
            open_document(second)
            if args.multiwindow_preferences_only:
                for theme_index,theme_name in [(1,'Github'),(2,'Night'),(0,'Yu（跟随系统）')]:
                    values = settings()
                    popup = next(c for c in values if c.get('AXDescription')=='正文主题')
                    click(popup)
                    deadline = time.monotonic()+3
                    while True:
                        option = next((c for c in controls() if c.get('AXRole')=='AXMenuItem' and c.get('AXTitle')==theme_name),None)
                        if option is not None: break
                        if time.monotonic()>=deadline:
                            run('capture',str(out/'theme-menu-missing'))
                            raise AssertionError('Native theme popup did not expose its visible option')
                        time.sleep(.05)
                    click(option)
                    deadline = time.monotonic()+3
                    while True:
                        selected = next(c for c in controls() if c.get('AXDescription')=='正文主题')['AXValue']
                        if selected==theme_name: break
                        if time.monotonic()>=deadline:
                            run('capture',str(out/'theme-selection-failed'))
                            raise AssertionError(f'Theme selection remained {selected!r}, expected {theme_name!r}')
                        time.sleep(.05)
                    close_settings()
                    if theme_index==1:
                        open_document(third); documents.append(third)
                    for index,path in enumerate(documents):
                        run('raise-window',str(path)); expect_source(source,'theme preserves document source')
                        run('capture',str(out/f'theme-{theme_index}-window-{index}'))
                        assert path.read_bytes()==original
                    check(theme_name+' theme updates existing windows and is inherited by the new window without source/disk edits; appearance captured')
                edited = {}
                for index,path in enumerate(documents):
                    run('raise-window',str(path)); run('select',len(source.encode('utf-16-le'))//2,0)
                    run('paste-text',f'窗口{index}关闭自动保存')
                    edited[path] = source+f'窗口{index}关闭自动保存'
                    expect_source(edited[path],'edit with autosave disabled')
                time.sleep(1.3)
                assert all(path.read_bytes()==original for path in documents)
                values = settings()
                auto = next(c for c in values if c.get('AXRole')=='AXCheckBox' and '自动保存' in (c.get('AXTitle') or ''))
                assert int(auto['AXValue'])==0
                click(auto); close_settings()
                for path in documents:
                    run('raise-window',str(path)); run('select',len(edited[path].encode('utf-16-le'))//2,0)
                    run('paste-text',' 已启用')
                    edited[path]+=' 已启用'
                    expect_source(edited[path],'edit after enabling autosave')
                deadline = time.monotonic()+6
                while not all(path.read_bytes()==b'\xef\xbb\xbf'+edited[path].encode() for path in documents):
                    assert time.monotonic()<deadline, 'Enabled autosave did not reach every existing window'
                    time.sleep(.1)
                check('autosave disabled preserves all three original files; enabling it saves new edits in every existing window with exact BOM/CRLF bytes')
                values = settings()
                click(next(c for c in values if c.get('AXRole')=='AXCheckBox' and '自动保存' in (c.get('AXTitle') or '')))
                close_settings()
                saved = {path:path.read_bytes() for path in documents}
                for path in documents:
                    run('raise-window',str(path)); run('select',len(edited[path].encode('utf-16-le'))//2,0)
                    run('paste-text',' 再次关闭')
                    edited[path]+=' 再次关闭'
                    expect_source(edited[path],'edit after disabling autosave again')
                time.sleep(1.3)
                assert all(path.read_bytes()==saved[path] for path in documents)
                for path in documents:
                    run('raise-window',str(path)); key(1,'cmd'); time.sleep(.1)
                    assert path.read_bytes()==b'\xef\xbb\xbf'+edited[path].encode()
                check('disabling autosave again stops automatic writes in all windows while manual save remains correct')
                quit_app(); result['passed']=True
                return 0
            for index,path in enumerate(documents):
                run('raise-window',str(path)); expect_source(source,'distinct existing document')
                run('select',len(source.encode('utf-16-le'))//2,0)
                run('paste-text',f'窗口 {index+1} 编辑')
                expect_source(source+f'窗口 {index+1} 编辑','individual history')
                run('select',len(source[:source.index('正文')].encode('utf-16-le'))//2,0)
                key(115,'cmd')
                run('capture',str(out/f'window-{index}-before'))
            settings()
            click(next(c for c in controls() if c.get('AXIdentifier')=='yu-body-font-size'))
            key(125); key(125); key(36)
            click(next(c for c in controls() if c.get('AXIdentifier')=='yu-reading-column-width'))
            key(125); key(125); key(36)
            for title in mode_titles: click(next(c for c in controls() if c.get('AXTitle')==title))
            click(next(c for c in controls() if c.get('AXTitle')=='系统拼写检查'))
            close_settings()
            for index,path in enumerate(documents):
                run('raise-window',str(path))
                expected = source+f'窗口 {index+1} 编辑'
                expect_source(expected,'settings preserve each source')
                run('capture',str(out/f'window-{index}-configured'))
                key(6,'cmd'); expect_source(source,'settings add no history in existing window')
                key(6,'cmd+shift'); expect_source(expected,'existing window independent redo')
                key(1,'cmd'); time.sleep(.2)
                assert path.read_bytes()==b'\xef\xbb\xbf'+expected.encode()
                values = settings()
                assert next(c for c in values if c.get('AXIdentifier')=='yu-body-font-size')['AXValue']=='20 pt'
                assert next(c for c in values if c.get('AXIdentifier')=='yu-reading-column-width')['AXValue']=='600 pt'
                for title in mode_titles: assert int(next(c for c in values if c.get('AXTitle')==title)['AXValue'])==1
                assert int(next(c for c in values if c.get('AXTitle')=='系统拼写检查')['AXValue'])==0
                close_settings()
            check('two existing native windows retain distinct source/history and save exact bytes while sharing font/column/mode/spelling preferences; appearance captured')
            open_document(third)
            run('capture',str(out/'window-new-configured'))
            image = out/'多窗口.png'
            shutil.copyfile(HERE/'Fixtures/assets/yu-mark.png',image)
            settings()
            click(directory()); key(0,'cmd'); run('paste-text','共享 图片'); key(36)
            close_settings()
            for reference in [False,True]:
                if reference:
                    values = settings()
                    policy = next(c for c in values if c.get('AXRole')=='AXPopUpButton' and c.get('AXDescription')=='图片导入方式')
                    click(policy); key(125); key(36)
                    close_settings()
                previous_files = set((out/'共享 图片').glob('*'))
                for index,path in enumerate([*documents,third]):
                    run('raise-window',str(path))
                    expected = source+(f'窗口 {index+1} 编辑' if index<2 else '')
                    run('select',len(expected.encode('utf-16-le'))//2,0)
                    run('paste-file',str(image))
                    after = run('snapshot')['AXValue']
                    match = re.fullmatch(re.escape(expected)+r'!\[多窗口\]\(([^)]+)\)',after)
                    assert match, 'Image policy insertion changed unrelated source'
                    uri = unquote(match.group(1))
                    if reference:
                        assert uri==image.as_uri() or uri==str(image) or uri=='file://'+str(image)
                        assert set((out/'共享 图片').glob('*'))==previous_files
                    else:
                        assert uri.startswith('共享 图片/image-')
                        assert (out/uri).read_bytes()==image.read_bytes()
                    key(6,'cmd'); expect_source(expected,'shared image policy single undo')
                    key(6,'cmd+shift'); expect_source(after,'shared image policy redo')
                    key(6,'cmd'); expect_source(expected,'restore document after image verification')
                    key(1,'cmd'); time.sleep(.1)
                    assert path.read_bytes()==b'\xef\xbb\xbf'+expected.encode()
                check(('reference' if reference else 'copy')+' image policy applies in all existing/new windows with exact resource bytes, isolated undo/redo and original source restored')
            values = settings()
            assert next(c for c in values if c.get('AXIdentifier')=='yu-body-font-size')['AXValue']=='20 pt'
            assert next(c for c in values if c.get('AXIdentifier')=='yu-reading-column-width')['AXValue']=='600 pt'
            click(next(c for c in values if c.get('AXTitle')=='恢复默认设置'))
            close_settings()
            for index,path in enumerate([*documents,third]):
                run('raise-window',str(path))
                expected = source+(f'窗口 {index+1} 编辑' if index<2 else '')
                expect_source(expected,'reset preserves every source')
                key(115,'cmd'); time.sleep(.2)
                run('capture',str(out/f'window-{index}-reset'))
                assert path.read_bytes()==b'\xef\xbb\xbf'+expected.encode()
                values = settings()
                assert next(c for c in values if c.get('AXIdentifier')=='yu-body-font-size')['AXValue']=='16 pt'
                assert next(c for c in values if c.get('AXIdentifier')=='yu-reading-column-width')['AXValue']=='随主题'
                for title in mode_titles: assert int(next(c for c in values if c.get('AXTitle')==title)['AXValue'])==0
                assert int(next(c for c in values if c.get('AXTitle')=='系统拼写检查')['AXValue'])==1
                close_settings()
            check('new native window inherits configured reading preferences and reset reaches all three windows without source or disk edits; appearance captured')
            quit_app(); result['passed']=True
            return 0
        if args.image_save_as_only:
            destination = out/'迁移目录/notes'
            destination.mkdir(parents=True)
            saved_document = destination/'迁移副本.md'
            target_image = destination.parent/'shared/图 %.png'
            original_image = out/'original/shared/图 %.png'
            run('select',len(source.encode('utf-16-le'))//2,0)
            run('paste-text','新增尾段')
            edited = source+'新增尾段'
            expect_source(edited,'edit before Save As')
            def save_panel():
                run('menu-open','文件')
                item = next(c for c in controls() if c.get('AXTitle')=='另存为…')
                assert item['AXEnabled'] is True
                click(item)
                deadline = time.monotonic()+5
                while True:
                    found = controls()
                    if any(c.get('AXIdentifier')=='save-panel' and c.get('AXTitle')=='另存为' for c in found): return
                    if time.monotonic()>=deadline:
                        run('capture',str(out/'save-panel-missing'))
                        run('snapshot')
                        raise AssertionError('Native Save As panel did not appear')
                    time.sleep(.1)
            save_panel(); key(53)
            expect_source(edited,'cancel Save As preserves source')
            assert not target_image.exists() and not saved_document.exists()
            assert fixture.read_bytes()==original
            check('cancel actual Save As leaves parent-relative resources, old document and editing history untouched')
            save_panel()
            key(5,'cmd+shift'); time.sleep(.2)
            run('paste-text',str(destination))
            assert run('snapshot')['focused_value']==str(destination)
            key(36); time.sleep(.3)
            field = next(c for c in controls() if c.get('AXIdentifier')=='saveAsNameTextField')
            click(field); key(0,'cmd'); run('paste-text',saved_document.name)
            assert run('snapshot')['focused_value']==saved_document.name
            run('capture',str(out/'parent-save-confirm'))
            click(next(c for c in controls() if c.get('AXIdentifier')=='OKButton' and c.get('AXTitle')=='保存'))
            deadline = time.monotonic()+5
            while not saved_document.exists():
                assert time.monotonic()<deadline, 'Save As did not publish destination'
                time.sleep(.1)
            expect_source(edited,'Save As preserves exact source')
            assert saved_document.read_bytes()==b'\xef\xbb\xbf'+edited.encode()
            assert target_image.read_bytes()==original_image.read_bytes()
            assert fixture.read_bytes()==original
            key(6,'cmd'); expect_source(source,'undo original edit after Save As')
            key(6,'cmd+shift'); expect_source(edited,'redo original edit after Save As')
            key(1,'cmd'); time.sleep(.2)
            assert saved_document.read_bytes()==b'\xef\xbb\xbf'+edited.encode()
            assert fixture.read_bytes()==original
            check('actual Save As copies Unicode parent-relative resource to sibling folder and preserves BOM/CRLF, original bytes and one undo/redo')
            quit_app(); command[1]=str(saved_document)
            assert launch()['AXValue']==edited
            time.sleep(1)
            run('capture',str(out/'parent-save-reopened'))
            check('relocated image document reopens with exact saved source')
            quit_app(); result['passed']=True
            return 0
        if args.image_drag_first_save_only:
            key(45,'cmd'); time.sleep(.3)
            expect_source('','new untitled document for drop')
            baseline = 'DROP_HERE\n\n未命名中文🪶正文\n'
            run('paste-text',baseline)
            state = expect_source(baseline,'untitled drop baseline')
            run('select',len(baseline.encode('utf-16-le'))//2,0)
            prior_selection = run('snapshot')['AXSelectedTextRange']
            position,size = state['AXPosition'],state['AXSize']
            x = position['x']+max(24,(size['width']-760)/2)+1
            y = position['y']+43
            images = [out/'首次 一.png',out/'首次 二.png']
            for image in images: shutil.copyfile(HERE/'Fixtures/assets/yu-mark.png',image)
            destination = out/'拖放首次保存'
            destination.mkdir()
            saved_document = destination/'新文章.md'
            def drafts():
                return {str(p):p.read_bytes() for p in (out/'state/Drafts').rglob('*') if p.is_file()}
            time.sleep(.8)
            original_drafts = drafts()
            for cancel in [True,False]:
                drag_arguments = ('drag-files',x,y,*map(str,images))
                drag_job = subprocess.Popen([str(driver),str(process.pid),*map(str,drag_arguments)],
                    text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
                try:
                    deadline = time.monotonic()+10
                    while True:
                        found = controls()
                        if any(c.get('AXIdentifier')=='save-panel' and c.get('AXTitle')=='保存文档' for c in found): break
                        assert time.monotonic()<deadline, 'Untitled external drop did not request first save'
                        time.sleep(.1)
                    expect_source(baseline,'drop must not insert before first-save decision')
                    run('capture',str(out/('drop-first-save-cancel' if cancel else 'drop-first-save-panel')))
                    if cancel:
                        key(53)
                    else:
                        key(5,'cmd+shift'); time.sleep(.2)
                        run('paste-text',str(destination))
                        assert run('snapshot')['focused_value']==str(destination)
                        key(36); time.sleep(.3)
                        field = next(c for c in controls() if c.get('AXIdentifier')=='saveAsNameTextField')
                        click(field); key(0,'cmd'); run('paste-text',saved_document.name)
                        assert run('snapshot')['focused_value']==saved_document.name
                        click(next(c for c in controls() if c.get('AXIdentifier')=='OKButton' and c.get('AXTitle')=='保存'))
                    stdout,stderr = drag_job.communicate(timeout=10)
                    sequence += 1
                    (out/f'event-{sequence:03}.json').write_text(json.dumps({'arguments':drag_arguments,'asynchronous':True,
                        'exit_code':drag_job.returncode,'stdout':stdout,'stderr':stderr},ensure_ascii=False,indent=2))
                    assert drag_job.returncode==0, stderr
                    operation = json.loads(stdout)
                    assert operation['started'] and operation['copy_accepted']==(not cancel)
                finally:
                    if drag_job.poll() is None:
                        drag_job.kill(); drag_job.wait(timeout=5)
                if cancel:
                    unchanged = expect_source(baseline,'cancel first save during drop')
                    assert unchanged['AXSelectedTextRange']==prior_selection
                    assert drafts()==original_drafts
                    assert not saved_document.exists() and not (destination/'assets').exists()
                    check('cancel actual first-save panel during external file drop preserves source, prior caret and draft/resource bytes')
            after = run('snapshot')['AXValue']
            paths = re.findall(r'!\[首次 [一二]\]\((assets/image-[a-z0-9-]+\.png)\)',after)
            assert len(paths)==2 and len(set(paths))==2
            inserted = '![首次 一]('+paths[0]+') ![首次 二]('+paths[1]+')'
            assert after==inserted+baseline, 'First-save drop changed hit or source order'
            for relative,image in zip(paths,images): assert (destination/relative).read_bytes()==image.read_bytes()
            key(6,'cmd'); undone = expect_source(baseline,'one undo after first-save drop')
            assert undone['AXSelectedTextRange']==prior_selection
            for relative in paths: assert (destination/relative).exists()
            key(6,'cmd+shift'); expect_source(after,'redo first-save drop')
            key(1,'cmd'); time.sleep(.2)
            assert saved_document.read_bytes()==after.encode()
            assert fixture.read_bytes()==original
            check('confirm actual first-save drop preserves hit/order and original caret on undo, copies resources beside chosen file, and saves exact UTF8')
            quit_app(); command[1]=str(saved_document)
            assert launch()['AXValue']==after
            time.sleep(.5)
            run('capture',str(out/'drop-first-save-reopened'))
            check('first-save external drop reopens with both resources and exact source')
            quit_app(); result['passed']=True
            return 0
        if args.image_drag_only:
            baseline = 'DROP_HERE\r\n\r\n原文  中文🪶保持\r\n'
            key(0,'cmd'); run('paste-text',baseline); key(115,'cmd')
            state = expect_source(baseline,'prepare drop fixture')
            position, size = state['AXPosition'], state['AXSize']
            # Drop just before the first rendered character in the fixed Yu
            # theme. Exact resulting source validates the canonical hit.
            x = position['x'] + max(24,(size['width']-760)/2) + 1
            y = position['y'] + 43
            originals = [out/'拖放 一.png',out/'拖放 二.png']
            for original_image in originals:
                shutil.copyfile(HERE/'Fixtures/assets/yu-mark.png',original_image)
            run('capture',str(out/'drag-before'))
            prior_selection = run('snapshot')['AXSelectedTextRange']
            cancelled = run('drag-files-cancel',x,y,*map(str,originals))
            assert cancelled['started'] and cancelled['cancel_requested'] and not cancelled['copy_accepted']
            unchanged = expect_source(baseline,'cancelled native file drag')
            assert unchanged['AXSelectedTextRange']==prior_selection
            assert not (out/'assets').exists()
            check('Escape cancels the actual external file drag without source/selection changes or copied resources')
            run('select',len(baseline.encode('utf-16-le'))//2,0)
            selection_before_drop = run('snapshot')['AXSelectedTextRange']
            operation = run('drag-files',x,y,*map(str,originals))
            assert operation['started'] and operation['copy_accepted'] and operation['files']==2
            deadline = time.monotonic()+5
            while True:
                after = run('snapshot')['AXValue']
                if after != baseline: break
                assert time.monotonic()<deadline, 'Accepted drag did not update source'
                time.sleep(.1)
            paths = re.findall(r'!\[拖放 [一二]\]\((assets/image-[a-z0-9-]+\.png)\)',after)
            assert len(paths)==2 and len(set(paths))==2, 'Both dragged files need distinct resources'
            inserted = '![拖放 一]('+paths[0]+') ![拖放 二]('+paths[1]+')'
            assert after==inserted+baseline, 'External file drop hit/order/source mismatch'
            for relative, original_image in zip(paths,originals):
                assert (out/relative).read_bytes()==original_image.read_bytes()
            run('capture',str(out/'drag-inserted'))
            key(6,'cmd'); undone = expect_source(baseline,'one undo for external batch drag')
            assert undone['AXSelectedTextRange']==selection_before_drop, 'Undo did not restore the original selection before drop'
            for relative in paths: assert (out/relative).exists()
            key(6,'cmd+shift'); expect_source(after,'external batch drag redo')
            run('select',len(after.encode('utf-16-le'))//2,0)
            before_failure = run('snapshot')
            resource_bytes = {p.name:p.read_bytes() for p in (out/'assets').iterdir() if p.is_file()}
            broken = out/'损坏.png'; broken.write_bytes(b'not an image')
            drag_arguments = ('drag-files',x,y,str(originals[0]),str(broken))
            # The destination presents a modal error before returning from
            # performDragOperation. Keep the external source alive while a
            # second driver dismisses that real alert, then inspect its result.
            drag_job = subprocess.Popen([str(driver),str(process.pid),*map(str,drag_arguments)],
                text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
            try:
                deadline = time.monotonic()+10
                while True:
                    alert = controls()
                    dismiss = next((c for c in alert if c.get('AXRole')=='AXButton' and c.get('AXTitle') in ['好','OK']),None)
                    if dismiss is not None: break
                    assert time.monotonic()<deadline, 'Invalid file drop did not report its native error'
                    time.sleep(.1)
                run('capture',str(out/'drag-invalid-batch'))
                expect_source(after,'invalid batch drag source rollback')
                click(dismiss)
                stdout,stderr = drag_job.communicate(timeout=10)
                sequence += 1
                (out/f'event-{sequence:03}.json').write_text(json.dumps({'arguments':drag_arguments,'asynchronous':True,
                    'exit_code':drag_job.returncode,'stdout':stdout,'stderr':stderr},ensure_ascii=False,indent=2))
                assert drag_job.returncode==0, stderr
                assert not json.loads(stdout)['copy_accepted'], 'Invalid batch reported successful drag'
            finally:
                if drag_job.poll() is None:
                    drag_job.kill(); drag_job.wait(timeout=5)
            unchanged = expect_source(after,'invalid batch leaves source unchanged')
            assert {p.name:p.read_bytes() for p in (out/'assets').iterdir() if p.is_file()}==resource_bytes
            assert unchanged['AXSelectedTextRange']==before_failure['AXSelectedTextRange'], 'Rejected drop changed the prior selection'
            check('invalid second dragged file reports failure and preserves source, prior selection and all resource bytes')
            key(1,'cmd'); time.sleep(.1)
            assert fixture.read_bytes()==b'\xef\xbb\xbf'+after.encode()
            quit_app(); assert launch()['AXValue']==after
            run('capture',str(out/'drag-reopen'))
            check('real cross-process AppKit two-file drag preserves hit/order/Unicode source and resource bytes, one undo/redo, BOM/CRLF save and reopen')
            quit_app(); result['passed']=True
            return 0
        if args.image_panels_only:
            image = out/'图 100%.png'
            shutil.copyfile(HERE/'Fixtures/assets/yu-mark.png',image)
            def open_image_picker():
                run('menu-open','文件')
                item = next(c for c in controls() if c.get('AXTitle')=='插入图片…')
                assert item['AXEnabled'] is True
                click(item); time.sleep(.4)
                controls()
            def choose_image():
                key(5,'cmd+shift'); time.sleep(.2)
                controls()
                run('paste-text',str(image))
                path_state = run('snapshot'); controls()
                run('capture',str(out/f'image-picker-path-{sequence}'))
                assert path_state['focused_value']==str(image), 'Native file-panel path paste did not reach its field'
                key(36); time.sleep(.3)
                deadline = time.monotonic()+5
                while True:
                    panel_controls = controls()
                    button = next((c for c in panel_controls if c.get('AXIdentifier')=='OKButton' and c.get('AXEnabled') is True),None)
                    if button is not None: break
                    assert time.monotonic()<deadline, 'Image picker did not enable Open for the selected fixture'
                    time.sleep(.1)
                click(button)
            def first_save_controls():
                deadline = time.monotonic()+5
                while True:
                    found = controls()
                    if any(c.get('AXIdentifier')=='save-panel' and c.get('AXTitle')=='保存文档' for c in found): return found
                    assert time.monotonic()<deadline, 'Image import did not request an actual first-save panel'
                    time.sleep(.1)
            run('select',len(source.encode('utf-16-le'))//2,0)
            open_image_picker()
            run('capture',str(out/'image-picker-cancel'))
            key(53); expect_source(source,'image picker cancellation')
            assert not (out/'assets').exists()
            assert fixture.read_bytes()==original
            check('cancelling the actual image picker leaves document bytes and resources untouched')
            open_image_picker(); choose_image()
            deadline = time.monotonic()+5
            while True:
                resources = list((out/'assets').glob('image-*.png'))
                if resources or time.monotonic()>=deadline: break
                time.sleep(.1)
            assert len(resources)==1, 'Picker must produce one uniquely named resource'
            copied = resources[0]
            expected = source + '![图 100\\%](assets/' + copied.name + ')'
            expect_source(expected,'native picker image insertion')
            assert copied.read_bytes()==image.read_bytes()
            run('capture',str(out/'image-picker-inserted'))
            key(6,'cmd'); expect_source(source,'native image insertion undo')
            key(6,'cmd+shift'); expect_source(expected,'native image insertion redo')
            key(1,'cmd'); time.sleep(.1)
            assert fixture.read_bytes()==b'\xef\xbb\xbf'+expected.encode()
            quit_app(); assert launch()['AXValue']==expected
            check('native picker handles Unicode/space/percent paths, exact resource copies, one undo/redo and BOM/CRLF save/reopen')
            key(45,'cmd'); time.sleep(.3)
            expect_source('','new untitled document')
            before = {str(p):p.read_bytes() for p in (out/'state/Drafts').rglob('*') if p.is_file()}
            open_image_picker(); choose_image()
            first_save_controls()
            run('capture',str(out/'image-first-save-cancel'))
            key(53); expect_source('','cancel first save after selecting image')
            after = {str(p):p.read_bytes() for p in (out/'state/Drafts').rglob('*') if p.is_file()}
            assert after == before, 'Cancelling first save copied resources or changed draft bytes'
            assert fixture.read_bytes()==b'\xef\xbb\xbf'+expected.encode()
            check('cancelling the actual untitled first-save panel after choosing an image leaves draft bytes and resources untouched')
            destination = out/'首次保存 子目录'
            destination.mkdir()
            saved_document = destination/'第一篇.md'
            open_image_picker(); choose_image(); first_save_controls()
            key(5,'cmd+shift'); time.sleep(.2)
            run('paste-text',str(destination))
            assert run('snapshot')['focused_value']==str(destination), 'Save panel directory paste'
            key(36); time.sleep(.3)
            field = next(c for c in controls() if c.get('AXIdentifier')=='saveAsNameTextField')
            initial_name = field['AXValue']
            click(field); key(0,'cmd'); run('paste-text',saved_document.name)
            assert run('snapshot')['focused_value']==saved_document.name
            key(6,'cmd'); assert run('snapshot')['focused_value']==initial_name, 'Native save-field undo must not reach the document'
            key(6,'cmd+shift'); assert run('snapshot')['focused_value']==saved_document.name
            assert run('snapshot')['AXValue']=='', 'Save-field edits changed the untitled document'
            check('save panel directory/name paste, field undo and redo follow the native responder without editing the document')
            run('capture',str(out/'image-first-save-confirm'))
            click(next(c for c in controls() if c.get('AXIdentifier')=='OKButton' and c.get('AXTitle')=='保存'))
            deadline = time.monotonic()+5
            while True:
                draft_resources = list((destination/'assets').glob('image-*.png'))
                if saved_document.exists() and draft_resources: break
                assert time.monotonic()<deadline, 'First save did not publish document and image resource'
                time.sleep(.1)
            assert len(draft_resources)==1 and draft_resources[0].read_bytes()==image.read_bytes()
            draft_source = '![图 100\\%](assets/'+draft_resources[0].name+')'
            expect_source(draft_source,'image insertion after actual first save')
            key(6,'cmd'); expect_source('','first-save image insertion undo')
            assert draft_resources[0].exists(), 'Undo must retain the resource'
            key(6,'cmd+shift'); expect_source(draft_source,'first-save image insertion redo')
            key(1,'cmd'); time.sleep(.2)
            assert saved_document.read_bytes()==draft_source.encode()
            assert fixture.read_bytes()==b'\xef\xbb\xbf'+expected.encode()
            quit_app(); command[1]=str(saved_document)
            assert launch()['AXValue']==draft_source
            run('capture',str(out/'image-first-save-reopen'))
            check('actual untitled first save chooses a Unicode directory, copies image beside the new document, preserves undo and reopens exact saved source')
            quit_app()
            result['passed']=True
            return 0
        if args.image_retry_only:
            missing_source = '![missing](restored.png)\r\n\r\n尾部保持\r\n'
            key(0,'cmd'); run('paste-text',missing_source); key(115,'cmd')
            key(1,'cmd'); time.sleep(3)
            state = expect_source(missing_source, 'missing image source')
            position, size = state['AXPosition'], state['AXSize']
            x = position['x'] + max(24,(size['width']-760)/2) + 12
            y = position['y'] + 43
            run('capture',str(out/'image-failed'))
            run('right-click',x,y)
            run('capture',str(out/'image-failed-menu'))
            assert run('snapshot')['AXSelectedTextRange']['location'] < len('![missing](restored.png)')
            image = HERE/'Fixtures/assets/yu-mark.png'
            restored = out/'restored.png'
            shutil.copyfile(image,restored)
            key(125); key(36); time.sleep(1)
            expect_source(missing_source,'image retry does not edit source')
            run('capture',str(out/'image-restored'))
            # The failed-image retry item must disappear after success. The
            # first enabled image action is then Properties, which opens the
            # real native sheet. This is separate from screenshot inspection.
            run('right-click',x,y)
            run('capture',str(out/'image-restored-menu'))
            key(125); key(36); time.sleep(.5)
            sheet = controls()
            assert any(c.get('AXIdentifier')=='yu-image-width' for c in sheet), 'Recovered image did not expose its Properties sheet'
            run('capture',str(out/'image-restored-properties'))
            click(next(c for c in sheet if c.get('AXTitle')=='取消' and c.get('AXRole')=='AXButton'))
            expect_source(missing_source,'image properties cancel')
            key(6,'cmd'); expect_source(source,'retry must not add an undo step')
            key(6,'cmd+shift'); expect_source(missing_source,'original edit redo')
            key(1,'cmd'); time.sleep(.1)
            assert fixture.read_bytes()==b'\xef\xbb\xbf'+missing_source.encode()
            assert restored.read_bytes()==image.read_bytes()
            quit_app(); assert launch()['AXValue']==missing_source
            time.sleep(.5); run('capture',str(out/'image-restored-reopen'))
            check('real failed-image right-click retry, restored Properties sheet, unchanged history/source and exact resource/save/reopen; screenshots require inspection')
            quit_app(); result['passed']=True
            return 0
        if args.table_only:
            def row(cells): return '| ' + ' | '.join(cells) + ' |'
            header = ['A','B','C']; delimiter = ['---','---','---']
            body = ['left\\|pipe','middle','中文🪶']
            cases = [
                ('在左侧插入列', [row(['A','','B','C']), row(['---']*4), row([body[0],'',body[1],body[2]])]),
                ('在右侧插入列', [row(['A','B','','C']), row(['---']*4), row([body[0],body[1],'',body[2]])]),
                ('删除列', [row(['A','C']),row(['---']*2),row([body[0],body[2]])]),
                ('在上方插入正文行', [row(header),row(delimiter),row(['']*3),row(body)]),
                ('在下方插入正文行', [row(header),row(delimiter),row(body),row(['']*3)]),
                ('删除正文行', [row(header),row(delimiter)]),
                ('列左对齐', [row(header),row(['---',':---','---']),row(body)]),
                ('列居中', [row(header),row(['---',':---:','---']),row(body)]),
                ('列右对齐', [row(header),row(['---','---:','---']),row(body)]),
                ('列默认对齐', [row(header),row(delimiter),row(body)]),
            ]
            if args.table_action:
                cases = [case for case in cases if case[0] == args.table_action]
                if not cases: raise ValueError('Unknown table action: ' + args.table_action)
            if args.table_context_only or args.table_boundaries_only: cases = []
            for prefix in ['', '> ']:
                def document(rows):
                    return 'KEEP  中文\r\n\r\n' + ''.join(prefix + line + '\r\n' for line in rows) + '\r\n尾部不变\r\n'
                baseline = document([row(header),row(delimiter),row(body)])
                if args.table_boundaries_only:
                    key(0,'cmd'); run('paste-text',baseline)
                    states = [baseline]
                    # Each edit acts on the result of the previous one. This
                    # catches stale cell/column identities after a structural
                    # change; the final undo chain must recover exact bytes.
                    steps = [
                        ('在右侧插入列','中文🪶', [row(['A','B','C','']),row(['---']*4),row(body+[''])]),
                        ('删除列','left\\|pipe', [row(['B','C','']),row(['---']*3),row(body[1:]+[''])]),
                        ('在左侧插入列','middle', [row(['','B','C','']),row(['---']*4),row(['']+body[1:]+[''])]),
                        ('在上方插入正文行','middle', [row(['','B','C','']),row(['---']*4),row(['']*4),row(['']+body[1:]+[''])]),
                        ('删除正文行','middle', [row(['','B','C','']),row(['---']*4),row(['']*4)]),
                    ]
                    for action, needle, expected_rows in steps:
                        current = states[-1]
                        run('select',len(current[:current.index(needle)].encode('utf-16-le'))//2,0)
                        run('menu-open','表格')
                        item = next(c for c in controls() if c.get('AXTitle')==action and c.get('AXRole')=='AXMenuItem')
                        assert item['AXEnabled'] is True, action
                        click(item)
                        expected = document(expected_rows)
                        expect_source(expected, action + ': sequential boundary edit')
                        states.append(expected)
                    for expected in reversed(states[:-1]):
                        key(6,'cmd'); assert run('snapshot')['AXValue']==expected, 'sequential undo'
                    for expected in states[1:]:
                        key(6,'cmd+shift'); assert run('snapshot')['AXValue']==expected, 'sequential redo'
                    key(1,'cmd'); time.sleep(.1)
                    assert fixture.read_bytes()==b'\xef\xbb\xbf'+states[-1].encode()
                    quit_app(); assert launch()['AXValue']==states[-1]
                    check(('quoted' if prefix else 'plain') + ' table first/last column and row boundary sequence, full undo/redo and exact save/reopen pass')
                    single = document([row(['A']),row(['---']),row(['only'])])
                    key(0,'cmd'); run('paste-text',single)
                    run('select',len(single[:single.index('only')].encode('utf-16-le'))//2,0)
                    run('menu-open','表格')
                    item = next(c for c in controls() if c.get('AXTitle')=='删除列')
                    assert item['AXEnabled'] is True
                    click(item)
                    # The established editor contract removes a table when
                    # its last column is deleted, with one-step undo.
                    expect_source(document([]), 'last-column table removal')
                    key(6,'cmd'); expect_source(single, 'undo last-column removal')
                    key(6,'cmd+shift'); expect_source(document([]), 'redo last-column removal')
                    key(1,'cmd'); time.sleep(.1)
                    assert fixture.read_bytes()==b'\xef\xbb\xbf'+document([]).encode()
                    quit_app(); assert launch()['AXValue']==document([])
                    check(('quoted' if prefix else 'plain') + ' deleting the last table column removes only the table, remains undoable and saves/reopens exactly')
                for action, expected_rows in cases:
                    before = document([row(header),row(['---','---:','---']),row(body)]) if action == '列默认对齐' else baseline
                    key(0,'cmd'); run('paste-text',before)
                    run('select',len(before[:before.index('middle')].encode('utf-16-le'))//2,0)
                    run('menu-open','表格')
                    item = next(c for c in controls() if c.get('AXTitle')==action and c.get('AXRole')=='AXMenuItem')
                    assert item['AXEnabled'] is True, action
                    if args.table_action: run('capture',str(out/f'table-menu-{sequence}'))
                    click(item)
                    expected = document(expected_rows)
                    expect_source(expected, action)
                    key(6,'cmd'); assert run('snapshot')['AXValue']==before, action + ': undo'
                    key(6,'cmd+shift'); assert run('snapshot')['AXValue']==expected, action + ': redo'
                    key(1,'cmd'); time.sleep(.1)
                    assert fixture.read_bytes()==b'\xef\xbb\xbf'+expected.encode(), action + ': saved bytes'
                if args.table_context_only:
                    run('resize',1200,800)
                    key(0,'cmd'); run('paste-text',baseline)
                    key(115,'cmd'); time.sleep(.3)
                    state = run('snapshot')
                    position, size = state['AXPosition'], state['AXSize']
                    # Fixed default typography, verified against the table
                    # screenshot. Assert the source hit before invoking a menu
                    # action so a shifted layout cannot silently edit a column.
                    x = position['x'] + max(24,(size['width']-760)/2) + 320
                    y = position['y'] + 137
                    run('right-click',x,y)
                    state = run('snapshot')
                    start = len(baseline[:baseline.index('middle')].encode('utf-16-le'))//2
                    assert start <= state['AXSelectedTextRange']['location'] <= start+len('middle'), 'Context click missed middle cell'
                    run('capture',str(out/f'table-context-{sequence}'))
                    # The contextual menu is a separate native surface. Send
                    # actual navigation keys; the source assertion identifies
                    # the selected action independently of its visual label.
                    for _ in range(8): key(125)
                    run('capture',str(out/f'table-context-selected-{sequence}'))
                    key(36)
                    expected = document([row(header),row(['---',':---:','---']),row(body)])
                    expect_source(expected, 'Context center alignment')
                    key(6,'cmd'); assert run('snapshot')['AXValue']==baseline
                    key(6,'cmd+shift'); assert run('snapshot')['AXValue']==expected
                    key(1,'cmd'); time.sleep(.1)
                    assert fixture.read_bytes()==b'\xef\xbb\xbf'+expected.encode()
                    quit_app(); assert launch()['AXValue']==expected
                    check(('quoted' if prefix else 'plain') + ' table right-click hit, keyboard context action, isolated undo/redo and exact save/reopen pass')
                # Header deletion must remain disabled rather than removing
                # the table delimiter or the surrounding prose.
                key(0,'cmd'); run('paste-text',baseline)
                run('select',len(baseline[:baseline.index('A |')].encode('utf-16-le'))//2,0)
                run('menu-open','表格')
                assert next(c for c in controls() if c.get('AXTitle')=='删除正文行')['AXEnabled'] is False
                key(53)
                assert run('snapshot')['AXValue']==baseline
                key(1,'cmd'); time.sleep(.1)
                quit_app(); assert launch()['AXValue']==baseline
                check(('quoted' if prefix else 'plain') + (' table header guard and reopen pass' if args.table_context_only or args.table_boundaries_only else ' table native row/column/alignment menu edits preserve surrounding CRLF source, isolated undo/redo and saved bytes; header guard and reopen pass'))
            run('capture',str(out/'table-menus-final'))
            quit_app()
            result['passed']=True
            return 0
        if not args.spelling_only:
            run('resize',1200,800)
            run('capture',str(out/'reading-before'))
            run('select',len(source[:source.index('one')].encode('utf-16-le'))//2,0)
            key(43,'cmd'); time.sleep(.3)
            run('menu-open','表格')
            table_action = next(c for c in controls() if c.get('AXTitle')=='在上方插入正文行')
            assert table_action['AXEnabled'] is False, 'Settings exposed the background table mutation'
            key(53)
            check('background table mutations are disabled while settings is the key window')
            field = directory()
            assert field['AXValue']=='assets'
            click(field); key(0,'cmd'); run('paste-text','素材/图片')
            state = run('snapshot')
            assert state['focused_value']=='素材/图片' and state['AXValue']==source
            key(0,'cmd'); assert run('copy-read')['text']=='素材/图片'
            key(6,'cmd')
            state = run('snapshot')
            assert state['focused_value']=='assets' and state['AXValue']==source, 'Settings undo reached the document or lost field history'
            key(6,'cmd+shift'); key(36)
            assert directory()['AXValue']=='素材/图片'
            assert run('snapshot')['AXValue']==source
            check('settings field clipboard and undo/redo target the field, never the background document')
            font_popup = next(c for c in controls() if c.get('AXIdentifier')=='yu-body-font-size')
            click(font_popup); key(125); key(125); key(36)
            column_popup = next(c for c in controls() if c.get('AXIdentifier')=='yu-reading-column-width')
            click(column_popup); key(125); key(125); key(36)

            captured = run('capture',str(out/'settings'))
            (out/'settings-windows.json').write_text(json.dumps(captured,indent=2))
            key(13,'cmd'); time.sleep(.2)
            assert not any(c.get('AXTitle')=='Yu 设置' for c in controls()), 'Command-W closed the document instead of settings'
            assert run('snapshot')['AXValue']==source
            check('Command-comma opens settings; Command-W closes only settings')
            run('capture',str(out/'reading-after'))
            check('native font and column controls apply without source edits; captured appearance requires visual inspection')
            key(43,'cmd'); time.sleep(.2)
            mode_titles = ['专注模式（突出当前段落）', '打字机模式（输入时保持活动行居中）']
            for title in mode_titles:
                control = next(c for c in controls() if c.get('AXTitle')==title)
                assert not int(control['AXValue'])
                click(control)
                assert int(next(c for c in controls() if c.get('AXTitle')==title)['AXValue'])==1
            key(13,'cmd'); time.sleep(.2)
            assert run('snapshot')['AXValue']==source
            run('capture',str(out/'focus-table'))
            run('select',len(source[:source.index('正文')].encode('utf-16-le'))//2,0)
            run('capture',str(out/'focus-paragraph'))
            check('native focus/typewriter settings toggle without source edits; focus captures require visual inspection')
            quit_app()
            assert launch()['AXValue']==source
            key(43,'cmd'); time.sleep(.2)
            assert directory()['AXValue']=='素材/图片'
            assert next(c for c in controls() if c.get('AXIdentifier')=='yu-body-font-size')['AXValue']=='20 pt'
            assert next(c for c in controls() if c.get('AXIdentifier')=='yu-reading-column-width')['AXValue']=='600 pt'
            for title in mode_titles:
                assert int(next(c for c in controls() if c.get('AXTitle')==title)['AXValue'])==1
            check('image directory, font size and column width preferences survive a fresh process')
            reset = next(c for c in controls() if c.get('AXTitle')=='恢复默认设置')
            click(reset)
            assert directory()['AXValue']=='assets'
            assert next(c for c in controls() if c.get('AXIdentifier')=='yu-body-font-size')['AXValue']=='16 pt'
            assert next(c for c in controls() if c.get('AXIdentifier')=='yu-reading-column-width')['AXValue']=='随主题'
            for title in mode_titles:
                assert not int(next(c for c in controls() if c.get('AXTitle')==title)['AXValue'])
            assert fixture.read_bytes()==original
            check('restore defaults returns the image directory to assets without modifying BOM/CRLF document bytes')
            key(13,'cmd'); key(13,'cmd'); time.sleep(.3)
            assert run('snapshot')['AXValue'] is None
            key(43,'cmd'); time.sleep(.2)
            click(directory()); key(0,'cmd'); run('paste-text','resources')
            assert run('snapshot')['focused_value']=='resources'
            key(0,'cmd'); assert run('copy-read')['text']=='resources'
            key(6,'cmd'); assert run('snapshot')['focused_value']=='assets'
            key(13,'cmd'); time.sleep(.2)
            assert not any(c.get('AXTitle')=='Yu 设置' for c in controls())
            check('settings clipboard, undo and close work with no open document windows')
            quit_app()
            assert launch()['AXValue']==source
            key(43,'cmd'); time.sleep(.2)
            click(directory()); key(0,'cmd'); run('paste-text','素材/图片'); key(36); key(13,'cmd')
            image = out/'图 % 原图.png'
            image.write_bytes((HERE/'Fixtures/assets/yu-mark.png').read_bytes())
            key(125,'cmd'); run('paste-image',str(image))
            pasted = run('snapshot')['AXValue']
            assert pasted.startswith(source)
            match = re.search(r'!\[图片\]\(([^)]+)\)',pasted)
            assert match and unquote(match[1]).startswith('素材/图片/')
            copied_image = out/unquote(match[1])
            assert copied_image.read_bytes()==image.read_bytes()
            key(6,'cmd'); assert run('snapshot')['AXValue']==source
            assert copied_image.is_file(), 'Undo removed a resource that may have other references'
            key(6,'cmd+shift'); assert run('snapshot')['AXValue']==pasted
            check('native bitmap paste uses configured Unicode resource directory and one-step undo/redo without deleting the image')
            key(43,'cmd'); time.sleep(.2)
            policy = next(c for c in controls() if c.get('AXDescription')=='图片导入方式')
            click(policy); key(125); key(36); key(13,'cmd')
            key(125,'cmd'); run('paste-file',str(image))
            referenced = run('snapshot')['AXValue']
            assert quote(str(image),safe='/-._~') in referenced, 'Reference policy did not preserve the original absolute file path'
            key(6,'cmd'); assert run('snapshot')['AXValue']==pasted
            assert image.read_bytes()==(HERE/'Fixtures/assets/yu-mark.png').read_bytes()
            key(6,'cmd+shift'); assert run('snapshot')['AXValue']==referenced
            key(1,'cmd'); time.sleep(.3)
            assert fixture.read_bytes()==b'\xef\xbb\xbf'+referenced.encode()
            key(126,'cmd'); time.sleep(.5)
            captured = run('capture',str(out/'images'))
            (out/'images-windows.json').write_text(json.dumps(captured,indent=2))
            check('native file paste honors reference policy; undo/redo and save preserve original image and BOM/CRLF')
            quit_app()
            assert launch()['AXValue']==referenced
            assert copied_image.read_bytes()==image.read_bytes()
            check('fresh process reopens copied and referenced image source with exact saved bytes')

            def image_field(name):
                return next(c for c in controls() if c.get('AXIdentifier')=='yu-image-'+name)
            def replace_field(name, value):
                click(image_field(name)); key(0,'cmd'); run('paste-text',value)
            def open_image_properties(text):
                at = text.index('![图片]') if '![图片]' in text else text.index('<img')
                run('select',len(text[:at].encode('utf-16-le'))//2,0)
                run('menu-open','文件')
                item = next(c for c in controls() if c.get('AXTitle')=='图片属性…')
                assert item['AXEnabled'], 'Image properties menu disabled at an image'
                click(item); time.sleep(.25)
                return image_field('alternative')
            assert open_image_properties(referenced)['AXValue']=='图片'
            assert image_field('destination')['AXValue']==unquote(match[1])
            assert image_field('width')['AXValue']=='' and image_field('height')['AXValue']==''
            replace_field('alternative','取消的文字'); key(53)
            assert run('snapshot')['AXValue']==referenced
            check('image properties cancel leaves source and undo history untouched')
            open_image_properties(referenced)
            replace_field('width','0')
            click(next(c for c in controls() if c.get('AXTitle')=='应用'))
            assert image_field('width')['AXValue']=='0' and run('snapshot')['AXValue']==referenced
            replace_field('width','128'); key(48)
            assert image_field('height')['AXValue']=='128', 'Aspect lock did not use image intrinsic ratio'
            replace_field('alternative','羽毛 <图>')
            captured = run('capture',str(out/'image-properties'))
            (out/'image-properties-windows.json').write_text(json.dumps(captured,indent=2))
            click(next(c for c in controls() if c.get('AXTitle')=='应用'))
            resized = run('snapshot')['AXValue']
            assert 'alt="羽毛 &lt;图&gt;"' in resized and 'width="128" height="128"' in resized
            assert resized.startswith(source) and quote(str(image),safe='/-._~') in resized
            key(6,'cmd'); assert run('snapshot')['AXValue']==referenced
            key(6,'cmd+shift'); assert run('snapshot')['AXValue']==resized
            check('image sheet validates dimensions; aspect-locked resize and alternative text form one undo step')
            open_image_properties(resized)
            assert image_field('width')['AXValue']=='128' and image_field('height')['AXValue']=='128'
            click(next(c for c in controls() if c.get('AXTitle')=='锁定纵横比'))
            assert next(c for c in controls() if c.get('AXTitle')=='锁定纵横比')['AXValue']==0
            replacement = out/'替换 图%.png'
            replacement.write_bytes(image.read_bytes())
            replace_field('destination',replacement.name)
            replace_field('width','160'); replace_field('height','80')
            click(next(c for c in controls() if c.get('AXTitle')=='应用'))
            stretched = run('snapshot')['AXValue']
            assert 'width="160" height="80"' in stretched
            assert quote(replacement.name,safe='/-._~') in stretched
            key(6,'cmd'); assert run('snapshot')['AXValue']==resized
            key(6,'cmd+shift'); assert run('snapshot')['AXValue']==stretched
            key(1,'cmd'); time.sleep(.3)
            assert fixture.read_bytes()==b'\xef\xbb\xbf'+stretched.encode()
            assert copied_image.read_bytes()==image.read_bytes()
            run('capture',str(out/'resized-images'))
            quit_app()
            assert launch()['AXValue']==stretched
            open_image_properties(stretched)
            assert image_field('width')['AXValue']=='160' and image_field('height')['AXValue']=='80'
            assert image_field('destination')['AXValue']==replacement.name
            assert replacement.read_bytes()==image.read_bytes()
            key(53)
            check('unlocked dimensions and readable Unicode/percent path edits survive undo, save/reopen without altering resource bytes')
            key(43,'cmd'); time.sleep(.2)
            policy = next(c for c in controls() if c.get('AXDescription')=='图片导入方式')
            click(policy); key(126); key(36); key(13,'cmd')
            one, two = out/'batchone.png', out/'batchtwo.png'
            one.write_bytes(image.read_bytes()); two.write_bytes(image.read_bytes())
            key(125,'cmd'); run('paste-files',str(one),str(two))
            batch = run('snapshot')['AXValue']
            assert batch.startswith(stretched)
            inserted = batch[len(stretched):]
            matched = re.fullmatch(r'!\[batchone\]\(([^)]+)\) !\[batchtwo\]\(([^)]+)\)',inserted)
            assert matched, 'Multi-image paste did not preserve order as one batch'
            resources = [out/unquote(matched[i]) for i in [1,2]]
            assert resources[0] != resources[1]
            for index, resource in enumerate(resources):
                assert unquote(matched[index+1]).startswith('素材/图片/')
                assert resource.read_bytes()==image.read_bytes()
            key(6,'cmd'); assert run('snapshot')['AXValue']==stretched
            key(6,'cmd+shift'); assert run('snapshot')['AXValue']==batch
            key(1,'cmd'); time.sleep(.2)
            assert fixture.read_bytes()==b'\xef\xbb\xbf'+batch.encode()
            check('real multi-file clipboard insertion preserves order and copies each image; one undo/redo covers the complete batch')
            before_files = set((out/'素材/图片').iterdir())
            broken = out/'broken.png'; broken.write_text('not an image')
            run('paste-files',str(one),str(broken)); time.sleep(.2)
            run('capture',str(out/'batch-failure'))
            assert run('snapshot')['AXValue']==batch
            key(36); time.sleep(.2)
            assert run('snapshot')['AXValue']==batch
            assert set((out/'素材/图片').iterdir())==before_files
            quit_app()
            assert launch()['AXValue']==batch
            for resource in resources: assert resource.read_bytes()==image.read_bytes()
            run('capture',str(out/'batch-images'))
            check('invalid second file leaves source and resources untouched; successful batch reopens with exact saved bytes')
        spelling_source = 'speling and `codde` <https://exampel.com>\r\n'
        key(0,'cmd'); run('paste-text',spelling_source); key(115,'cmd'); time.sleep(.3)
        state = run('snapshot')
        assert state['AXValue']==spelling_source
        time.sleep(1)
        run('capture',str(out/'spelling-before'))
        key(43,'cmd'); time.sleep(.2)
        spelling_control = next(c for c in controls() if c.get('AXTitle')=='系统拼写检查')
        assert int(spelling_control['AXValue'])==1
        click(spelling_control); key(13,'cmd'); time.sleep(.5)
        run('capture',str(out/'spelling-disabled'))
        assert run('snapshot')['AXValue']==spelling_source
        key(43,'cmd'); time.sleep(.2)
        spelling_control = next(c for c in controls() if c.get('AXTitle')=='系统拼写检查')
        assert int(spelling_control['AXValue'])==0
        click(spelling_control); key(13,'cmd'); time.sleep(1)
        run('capture',str(out/'spelling-enabled'))
        assert run('snapshot')['AXValue']==spelling_source
        check('native spelling setting toggles without source edits; underline captures require visual inspection')
        position, size = state['AXPosition'], state['AXSize']
        # Fixed Yu default: 760pt reading column, 30pt top padding. The
        # ensuing source-selection assertion proves this hit reached the word.
        x = position['x'] + max(24, (size['width']-760)/2) + 14
        y = position['y'] + 30 + 13
        run('right-click',x,y); time.sleep(.3)
        state = run('snapshot')
        assert state['AXSelectedTextRange']['location'] < 7, 'Context click missed the spelling word'
        run('capture',str(out/'spelling-menu'))
        # AppKit's contextual menu is a separate accessibility surface on
        # this host. Select its first candidate with real menu key events;
        # the exact corrected source below proves which action executed.
        key(125); key(36); time.sleep(.2)
        corrected = spelling_source.replace('speling','spelling',1)
        assert run('snapshot')['AXValue']==corrected
        key(6,'cmd'); assert run('snapshot')['AXValue']==spelling_source
        key(6,'cmd+shift'); assert run('snapshot')['AXValue']==corrected
        key(1,'cmd'); time.sleep(.2)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+corrected.encode()
        quit_app()
        assert launch()['AXValue']==corrected
        check('real right-click system spelling suggestion replaces only prose in one undo step and survives save/reopen')
        key(43,'cmd'); time.sleep(.2)
        click(next(c for c in controls() if c.get('AXTitle')=='系统拼写检查'))
        quit_app()
        assert launch()['AXValue']==corrected
        key(43,'cmd'); time.sleep(.2)
        assert int(next(c for c in controls() if c.get('AXTitle')=='系统拼写检查')['AXValue'])==0
        click(next(c for c in controls() if c.get('AXTitle')=='恢复默认设置'))
        assert int(next(c for c in controls() if c.get('AXTitle')=='系统拼写检查')['AXValue'])==1
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+corrected.encode()
        check('spelling preference persists across processes and reset restores enabled without source edits')
        quit_app()
        result['passed']=True
    except Exception as error:
        result['error']=str(error)
        raise
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            try: process.wait(timeout=5)
            except subprocess.TimeoutExpired: process.kill(); process.wait(timeout=5)
        # Keep results/events/screenshots, not a redundant signed app copy.
        # This happens only after the private process is confirmed terminated.
        shutil.rmtree(app)
        if args.image_save_as_only or args.image_drag_first_save_only or args.multiwindow_only:
            subprocess.run(['defaults','delete',identifier],capture_output=True,check=False)
        result['test_bundle_removed'] = True
        (out/'results.json').write_text(json.dumps(result,ensure_ascii=False,indent=2))
        assert digest(production/'Contents/MacOS/Yu')==build['app_sha256']
    return 0


if __name__=='__main__':
    raise SystemExit(main())
