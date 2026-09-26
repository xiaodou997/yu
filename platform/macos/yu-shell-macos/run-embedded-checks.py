#!/usr/bin/env python3
"""External native checks for the bundled math/diagram helper, using isolated data."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
from urllib.parse import unquote
import shutil
import signal
import subprocess
import time
import uuid
import group4_followup

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
parser = argparse.ArgumentParser()
parser.add_argument('output', type=Path)
parser.add_argument('--dark', action='store_true')
parser.add_argument('--resource-audit', action='store_true', help='Read-only per-frame CPU cache/history/GPU residency counters')
parser.add_argument('--stress-idle-seconds', type=int, default=0, help='Observe post-stress plain-document idle memory, 0..120 seconds')
parser.add_argument('--list-gestures', action='store_true', help='Actual reverse mouse range and parent/child Option-click multicursors')
parser.add_argument('--smoke-document', action='store_true', help='Fixed combined document TOC/disclosure/find/save interaction without OCR')
parser.add_argument('--stress-seconds', type=int, default=0, help='Bounded real-window resource churn, 30..1800 seconds; not a leak proof')
parser.add_argument('--table-interactions', action='store_true', help='Real forward/reverse whole grouped-table paste with complete history')
parser.add_argument('--table-resize', action='store_true', help='Real merged-column drag/cancel and persisted geometry')
parser.add_argument('--math-suite', action='store_true', help='Exercise aligned, cases and multiline formula editing in the actual application')
parser.add_argument('--diagram-suite', action='store_true', help='Exercise seven Mermaid families in the actual application')
parser.add_argument('--recent-diagrams', action='store_true', help='Exercise the three central-connection and four Gantt-calendar fixtures with real caption edits')
parser.add_argument('--promotion-suite', action='store_true', help='Copy real native merged cells and paste into math/footnote/grouped targets')
parser.add_argument('--ime', action='store_true', help='Verify real system Pinyin hardware-key composition, commit, cancellation and history')
parser.add_argument('--list-inputs', type=Path, help='Prepared group4 list manifest; run single-range standalone/cell CRLF cases through native commands')
parser.add_argument('--reopen', action='store_true', help='Quit and relaunch the isolated application; verify disk source, resources and editing')
parser.add_argument('--cjk-math', action='store_true')
parser.add_argument('--multiline-diagram', action='store_true')
parser.add_argument('--cancel-helper', action='store_true', help='Suspend an isolated helper response and verify document-change cancellation')
parser.add_argument('--lifecycle', action='store_true', help='Measure real application helper idle exit, memory, and restart')
parser.add_argument('--themes', action='store_true')
parser.add_argument('--failures', action='store_true')
parser.add_argument('--dollars', action='store_true')
parser.add_argument('--equations', action='store_true')
parser.add_argument('--highlight', action='store_true')
parser.add_argument('--scripts', action='store_true')
parser.add_argument('--footnotes', action='store_true')
parser.add_argument('--footnote-errors', action='store_true')
parser.add_argument('--toc', action='store_true')
parser.add_argument('--html', action='store_true')
parser.add_argument('--html-blocks', action='store_true')
parser.add_argument('--anchors', action='store_true')
parser.add_argument('--alignment', action='store_true')
parser.add_argument('--html-tables', action='store_true')
parser.add_argument('--merged-tables', action='store_true')
parser.add_argument('--html-lines', action='store_true')
parser.add_argument('--html-details', action='store_true')
parser.add_argument('--html-lists', action='store_true')
args = parser.parse_args()
option_error = group4_followup.option_error(vars(args))
if option_error:
    parser.error(option_error)
if args.math_suite and (args.html_blocks or args.diagram_suite or args.recent_diagrams):
    parser.error('--math-suite cannot be combined with --html-blocks or --diagram-suite')
if (args.lifecycle or args.cancel_helper) and any((args.html_blocks,args.dollars,args.equations,args.failures)):
    parser.error('--lifecycle uses the standard math/diagram fixture')
if (args.merged_tables or args.anchors or args.alignment or args.html_tables or args.html_lines or args.html_details or args.html_lists) and not args.html_blocks:
    parser.error('--anchors/--alignment/--html-tables/--merged-tables/--html-lines/--html-details/--html-lists requires --html-blocks')
out = args.output.resolve()
out.mkdir(parents=True, exist_ok=False)
production = HERE / '.build/Yu.app'
manifest = json.loads((HERE/'.build/build-manifest.json').read_text())
sha = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
assert sha(production/'Contents/MacOS/Yu') == manifest['app_sha256']
assert sha(production/'Contents/Helpers/yu-document-renderer') == manifest['helper_sha256']
driver = out/'native-event-driver'
subprocess.run(['swiftc', str(ROOT/'tools/native-event-driver.swift'), '-o', str(driver)], check=True)
preflight = subprocess.run([str(driver), '--preflight'], capture_output=True, text=True)
(out/'preflight.json').write_text(preflight.stdout)
if preflight.returncode:
    (out/'preflight.stderr.log').write_text(preflight.stderr)
    raise SystemExit(f'Native desktop preflight failed ({preflight.returncode}):\n'
                     f'{preflight.stdout}{preflight.stderr}')
app = out/'YuEmbeddedChecks.app'
shutil.copytree(production, app)
info_path = app/'Contents/Info.plist'
info = plistlib.loads(info_path.read_bytes())
identifier = 'io.github.xiaodou997.yu.embedded-check.' + uuid.uuid4().hex
info['CFBundleIdentifier'] = identifier
info_path.write_bytes(plistlib.dumps(info))
subprocess.run(['codesign', '--force', '--sign', '-', '--identifier', identifier, str(app)], check=True)
fixture = out/'input.md'
initial = '# Native resources\r\n\r\nOrdinary document 中文🙂.\r\n'
source = '# Native resources\r\n\r\n```math\r\n\\begin{pmatrix}a&b\\\\c&d\\end{pmatrix}\r\n```\r\n\r\n```mermaid\r\nflowchart LR\r\nA[Start] --> B[Done]\r\n```\r\n\r\nAFTER RESOURCES ANCHOR\r\n'
if args.multiline_diagram:
    source = source.replace('flowchart LR\r\nA[Start] --> B[Done]', 'flowchart LR\r\nsubgraph S["Group\r\n中文"]\r\nA["First\r\nend\r\n%% literal"] -->|"Yes\r\n继续"| B[Done]\r\nend')
if args.cjk_math:
    source = source.replace('# Native resources\r\n\r\n', '# Native resources\r\n\r\nInline $\\text{中文}$ end.\r\n\r\n$$\r\n\\frac{\\text{收益}}{\\text{成本}}\r\n$$\r\n\r\n')
if args.dollars:
    source = source.replace('# Native resources\r\n\r\n', '# Native resources\r\n\r\nInline 中文 $x_2$ and $\\frac{a}{b}$ end.\r\n\r\n$$\r\n\\sqrt{x^2+y^2}\r\n$$\r\n\r\n$$E=mc^2$$\r\n\r\n')
if args.equations:
    source = '# Equation $E=mc^2$\r\n\r\nReferences \\eqref{energy}, \\ref{custom}. Missing \\eqref{missing}.\r\n\r\n$$\r\nE=mc^2\\label{energy}\r\n$$\r\n\r\n$$a=b\\tag{A}\\label{custom}$$\r\n\r\nMixed **bold** $x_2$ and $\\unknownYuCommand{x}$ end.\r\n\r\n```mermaid\r\nflowchart LR\r\nA[Start] --> B[Done]\r\n```\r\n\r\nAFTER RESOURCES ANCHOR\r\n'
if args.highlight:
    source = 'Highlight ==中文 **bold** [link](https://example.com)== end.\r\n\r\n| ==head== | B |\r\n| --- | --- |\r\n| ==cell== | text |\r\n\r\n' + source
if args.scripts:
    metadata = "---\r\ntitle: 中文写作\r\nliteral: '$x^2$ ==not highlight=='\r\n# metadata comment\r\n---\r\n\r\n"
    initial = metadata + initial
    source = metadata + 'Scripts 中文 H~2~O x^3^ **y^n^** ==a^2^== שלום^2^ end.\r\n\r\n# Heading x^2^\r\n\r\n' + source
if args.footnotes:
    source = 'Footnotes 中文🙂[^first][^second] repeat[^first].\r\n\r\n[^first]: First definition.\r\n\r\n[^second]: Second **definition**.\r\n\r\n' + source
if args.toc:
    source = '[toc]\r\n\r\n# First chapter\r\n\r\n## Second chapter\r\n\r\n' + source
if args.html:
    source = 'Inline <b>HTML bold</b> <em>italic</em> <mark>highlight</mark> H<sub>2</sub>O x<sup>2</sup>.\r\n\r\nUnknown <b class="keep">original</b> <custom>中文</custom>.\r\n\r\n' + source
if args.html_blocks:
    source = '<div>\r\n<h2>HTML chapter</h2>\r\n<p>First paragraph 中文 &amp; <b>bold</b>.</p>\r\n<ol start="3"><li>First item<ul><li>Nested item</li></ul></li><li>Second item</li></ol>\r\n<p>Final paragraph.</p>\r\n</div>\r\n\r\n' + initial
if args.alignment:
    source = ("<div align='center'>\r\n<h2>HTML alignment</h2>\r\n<p>Center edge</p>\r\n<p>First paragraph "
              + ('中文 English words for wrapping. ' * 7)
              + "</p>\r\n<p align='right'>Right edge שלום</p>\r\n<p align='left'>Left edge 中文</p>\r\n"
              + "<p align='justify'>Justified paragraph " + ('中文 English words for native justification. ' * 10)
              + "<br>Short final line.</p>\r\n</div>\r\n\r\n" + initial)
if args.anchors:
    source = "[Go to target](#target) and <a href='#html-chapter'>Go to heading</a>.\r\n\r\n" + source.replace('Ordinary document', "Ordinary <a id='target'>document</a>")
if args.merged_tables:
    source = "<table><tr><td colspan='2' rowspan='2'>Merged alpha 中文🙂</td><td>Cell right</td></tr><tr><td>Cell lower</td></tr></table>\r\n\r\n" + source
if args.html_tables:
    source = source.replace('</div>\r\n\r\n', '</div>\r\n\r\n<table><tr><th>Table head</th><th>中文</th></tr><tr><td>Cell alpha</td><td>Cell beta</td></tr></table>\r\n\r\n')
if args.html_tables and args.alignment:
    source += ("\r\n<table align='justify'><tr><td>Justified cell "
               + ('中文 English words for cell wrapping. ' * 6)
               + "</td><td align='right'>Right cell</td></tr></table>\r\n")
if args.html_lines:
    source = source.replace('<p>Final paragraph.</p>', '<p><u>Underlined 中文 text</u> and <del>Deleted 中文 text</del>.</p>\r\n<p><u><b>Bold underlined</b></u> <u><s>Both lines</s></u>.</p>')
    source += "\r\n<table><tr><td><u>Underlined table words wrap across several lines and remain editable 中文</u></td><td><strike>Deleted table words wrap across several lines and remain editable 中文</strike></td></tr></table>\r\n\r\nTail.\r\n"
if args.html_details:
    source += "\r\n<details><summary>Disclosure summary</summary><h3>Hidden chapter</h3><p>Secret body</p></details>\r\n\r\n[Jump hidden](#hidden-chapter)\r\n\r\nEnd.\r\n"
if args.html_lists:
    source += "\r\n<table><tr><td><ul><li>Alpha</li><li><b>Beta</b></li></ul></td><td>KEEP</td></tr></table>\r\n"
fixture.write_bytes(b'\xef\xbb\xbf'+initial.encode())
env = {k:v for k,v in os.environ.items() if not k.startswith('YU_')}
env.update(YU_DOCUMENT_STATE_DIR=str(out/'state'), YU_PRESENTATION_STATE_DIR=str(out/'columns'), YU_NATIVE_INPUT_TRACE='1')
if args.resource_audit:
    env['YU_RESOURCE_AUDIT'] = '1'
command = [str(app/'Contents/MacOS/Yu'), str(fixture)] + (['--dark-mode'] if args.dark else [])
log = (out/'app.log').open('w')
process = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT)
result = {'passed':False, 'build':manifest, 'checks':[], 'visual_review_required':True,
          'resource_audit':args.resource_audit,
          'stress_idle_seconds':args.stress_idle_seconds,
          'isolated_bundle':identifier,
          'recent_diagrams':args.recent_diagrams,
          'promotion_suite':args.promotion_suite,
          'ime':args.ime,
          'list_inputs':str(args.list_inputs) if args.list_inputs else None,
          'test_app_sha256':sha(app/'Contents/MacOS/Yu'),
          'test_helper_sha256':sha(app/'Contents/Helpers/yu-document-renderer'),
          'test_runner_sha256':sha(Path(__file__)),
          'options':{'multiline_diagram':args.multiline_diagram,'cjk_math':args.cjk_math,'cancel_helper':args.cancel_helper,'lifecycle':args.lifecycle,'dark':args.dark,'themes':args.themes,'failures':args.failures,'dollars':args.dollars,'equations':args.equations,'highlight':args.highlight,'scripts':args.scripts,'footnotes':args.footnotes,'footnote_errors':args.footnote_errors,'toc':args.toc,'html':args.html,'html_blocks':args.html_blocks,'anchors':args.anchors,'alignment':args.alignment,'html_tables':args.html_tables,'merged_tables':args.merged_tables,'html_lines':args.html_lines,'html_details':args.html_details,'html_lists':args.html_lists,'reopen':args.reopen,'diagram_suite':args.diagram_suite,'math_suite':args.math_suite}}
after_reopen_check = None
if args.table_interactions or args.table_resize or args.smoke_document or args.stress_seconds or args.list_gestures:
    result['followup_module_sha256'] = sha(Path(group4_followup.__file__))
    result['table_interactions'] = args.table_interactions
    result['table_resize'] = args.table_resize
    result['smoke_document'] = args.smoke_document
    result['list_gestures'] = args.list_gestures
    result['stress_seconds'] = args.stress_seconds
sequence = 0
suspended_helpers = set()

def run(*arguments):
    global sequence
    response = subprocess.run([str(driver), str(process.pid), *map(str,arguments)], text=True, capture_output=True, timeout=15)
    sequence += 1
    (out/f'event-{sequence:03}.json').write_text(json.dumps({'arguments':arguments, 'code':response.returncode, 'stdout':response.stdout, 'stderr':response.stderr},ensure_ascii=False,indent=2))
    if response.returncode: raise RuntimeError(response.stderr)
    return json.loads(response.stdout) if response.stdout.strip().startswith('{') else None

def stable_bounds(location, length):
    # AX selection can enqueue scrolling/refinement. Do not click a rectangle
    # read before that work settles, or an off-screen source range.
    previous = None
    stable = 0
    deadline = time.monotonic()+5
    time.sleep(.2)
    while time.monotonic() < deadline:
        bounds = run('bounds',location,length)['bounds']
        state = run('snapshot')
        window = next(w for w in state['windows'] if w.get('kCGWindowLayer') == 0 and w['kCGWindowBounds']['Width'] >= 400)
        frame = window['kCGWindowBounds']
        x, y = bounds['x']+bounds['width']/2, bounds['y']+bounds['height']/2
        visible = frame['X'] <= x <= frame['X']+frame['Width'] and frame['Y']+40 <= y <= frame['Y']+frame['Height']-8
        key = tuple(round(bounds[k],2) for k in ('x','y','width','height'))
        key += tuple(round(state['AXPosition'][k],2) for k in ('x','y'))
        stable = stable+1 if key == previous and visible else 0
        if stable >= 2:
            return bounds
        previous = key
        time.sleep(.1)
    raise AssertionError('Native range bounds did not become stable and visible')

def helpers():
    lines = subprocess.check_output(['ps','-axo','pid=,command='],text=True).splitlines()
    return [int(line.strip().split(None,1)[0]) for line in lines if str(app) in line and '/Helpers/yu-document-renderer' in line]

try:
    deadline = time.monotonic()+12
    while True:
        try:
            state = run('activate'); break
        except RuntimeError:
            if process.poll() is not None or time.monotonic()>deadline: raise
            time.sleep(.2)
    run('resize',1200,800)
    assert state['AXValue']==initial, 'Initial document changed before scripted input; inspect activation event AXValue'
    time.sleep(1)
    assert not helpers(), 'Ordinary document launched heavy helper'
    result['checks'].append('ordinary document does not launch the native helper')
    run('select',0,len(initial.encode('utf-16-le'))//2)
    run('paste-text',source)
    run('select',len(source.encode('utf-16-le'))//2,0)
    if args.html_blocks:
        assert not helpers(), 'HTML-only document launched heavy helper'
        result['helper_pids']=[]
    else:
        deadline = time.monotonic()+20
        while not helpers():
            if time.monotonic()>deadline: raise AssertionError('No bundled helper launched')
            time.sleep(.1)
        result['helper_pids']=helpers()
        assert len(result['helper_pids'])==1
    time.sleep(2)
    run('capture',str(out/'native-resources'))
    assert run('snapshot')['AXValue']==source
    run('key',1,'cmd')
    time.sleep(.4)
    assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
    result['checks'].append('HTML-only document preserves exact saved source without launching the helper; screenshot captured' if args.html_blocks else 'math and Mermaid share a bundled helper and preserve exact saved source; screenshot captured')
    run('key',6,'cmd')
    assert run('snapshot')['AXValue']==initial
    run('key',6,'cmd+shift')
    assert run('snapshot')['AXValue']==source
    run('key',1,'cmd')
    time.sleep(.3)
    assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
    result['checks'].append('resource insertion undo/redo restores exact source without renderer edits')
    if args.html:
        start = source.index('HTML bold')
        run('select',len(source[:start].encode('utf-16-le'))//2,len('HTML bold'))
        run('capture',str(out/'html-editing'))
        run('paste-text','HTML changed')
        changed = source[:start]+'HTML changed'+source[start+len('HTML bold'):]
        run('select',len(changed.encode('utf-16-le'))//2,0)
        assert run('snapshot')['AXValue']==changed
        run('capture',str(out/'html-updated'))
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',6,'cmd+shift'); assert run('snapshot')['AXValue']==changed
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
        result['checks'].append('inline HTML body edit/undo/redo/save preserves tags, unknown attributes, and exact BOM/CRLF bytes; screenshots require review')

    if args.html_blocks:
        start = source.index('First paragraph')
        run('select',len(source[:start].encode('utf-16-le'))//2,len('First paragraph'))
        run('capture',str(out/'html-block-editing'))
        run('paste-text','Updated paragraph')
        changed = source[:start]+'Updated paragraph'+source[start+len('First paragraph'):]
        run('select',len(changed.encode('utf-16-le'))//2,0)
        assert run('snapshot')['AXValue']==changed
        run('capture',str(out/'html-block-updated'))
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',6,'cmd+shift'); assert run('snapshot')['AXValue']==changed
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
        result['checks'].append('structural HTML paragraph edit/undo/redo/save preserves container tags, list start, nested structure and BOM/CRLF; screenshots require review')

    if args.html_details:
        locator=out/'locate-native-text'
        subprocess.run(['swiftc',str(ROOT/'tools/locate-native-text.swift'),'-o',str(locator)],check=True)
        start=source.index('Disclosure summary')
        run('select',len(source.encode('utf-16-le'))//2,0)
        captured=run('capture',str(out/'details-closed'))
        window=next(w for w in captured['windows'] if w['kCGWindowLayer']==0 and w['kCGWindowBounds']['Width']>=400)
        png=out/f'details-closed-{window["kCGWindowNumber"]}.png'
        located=json.loads(subprocess.check_output([str(locator),str(png),'Disclosure summary'],text=True))
        assert len(located['matches'])==1,located
        box=located['matches'][0]; bounds=window['kCGWindowBounds']
        x=bounds['X']+box['x']*bounds['Width']/located['image_width']-10
        y=bounds['Y']+(box['y']+box['height']/2)*bounds['Height']/located['image_height']
        run('click',x,y)
        changed=source.replace('<details>','<details open>')
        assert run('snapshot')['AXValue']==changed
        run('select',len(changed.encode('utf-16-le'))//2,0)
        run('capture',str(out/'details-open'))
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',6,'cmd+shift'); assert run('snapshot')['AXValue']==changed
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
        result['checks'].append('pixel-located native disclosure arrow opens details; undo/redo and exact BOM/CRLF save preserve hidden body')
        run('select',len(source[:start].encode('utf-16-le'))//2,0)
        run('key',2,'cmd+alt+ctrl')
        assert run('snapshot')['AXValue']==changed
        run('key',2,'cmd+alt+ctrl')
        closed=source.replace('<details>','<details >')
        assert run('snapshot')['AXValue']==closed
        run('capture',str(out/'details-keyboard-closed'))
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==changed
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
        result['checks'].append('real disclosure keyboard shortcut opens/closes summary; two independent undos and exact save preserve original source')
        run('key',3,'cmd')
        run('paste-text','Secret body')
        run('key',36)
        found=run('snapshot')
        assert found['AXSelectedText']=='Secret body',found
        assert found['AXValue']==source
        run('key',53)
        run('capture',str(out/'details-search-revealed'))
        run('paste-text','Edited body')
        edited=source.replace('Secret body','Edited body')
        assert run('snapshot')['AXValue']==edited
        run('capture',str(out/'details-search-edited'))
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('select',len(source[:start].encode('utf-16-le'))//2,0)
        run('key',2,'cmd+alt+ctrl')
        assert run('snapshot')['AXValue']==source
        run('select',len(source.encode('utf-16-le'))//2,0)
        run('capture',str(out/'details-search-closed'))
        closed_png=out/f'details-search-closed-{window["kCGWindowNumber"]}.png'
        hidden_check=json.loads(subprocess.check_output([str(locator),str(closed_png),'Secret body'],text=True))
        summary_check=json.loads(subprocess.check_output([str(locator),str(closed_png),'Disclosure summary'],text=True))
        assert not hidden_check['matches'] and summary_check['matches'], (hidden_check,summary_check)
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
        result['checks'].append('native find reveals hidden result without source edits; body typing/undo and temporary close preserve exact saved source')
        captured=run('capture',str(out/'hidden-anchor-before'))
        anchor_png=out/f'hidden-anchor-before-{window["kCGWindowNumber"]}.png'
        located=json.loads(subprocess.check_output([str(locator),str(anchor_png),'hidden'],text=True))
        assert len(located['matches'])==1,located
        box=located['matches'][0]
        run('click',bounds['X']+(box['x']+box['width']/2)*bounds['Width']/located['image_width'],
            bounds['Y']+(box['y']+box['height']/2)*bounds['Height']/located['image_height'],'cmd')
        state=run('snapshot')
        assert state['AXValue']==source
        assert state['AXSelectedTextRange']['location']==len(source[:source.index('Hidden chapter')].encode('utf-16-le'))//2,state
        run('capture',str(out/'hidden-anchor-after'))
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
        result['checks'].append('pixel-located Command-click resolves a generated anchor inside a closed disclosure and reveals its heading without changing saved source')

    if args.html_lines:
        label='Underlined table words'
        start=source.index(label)
        run('select',len(source[:start].encode('utf-16-le'))//2,len(label))
        run('paste-text','Changed line')
        changed=source[:start]+'Changed line'+source[start+len(label):]
        assert run('snapshot')['AXValue']==changed
        run('select',len(changed.encode('utf-16-le'))//2,0)
        run('capture',str(out/'html-lines-edited'))
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',6,'cmd+shift'); assert run('snapshot')['AXValue']==changed
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('select',len(source.encode('utf-16-le'))//2,0)
        run('capture',str(out/'html-lines-preview'))
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
        result['checks'].append('HTML underline/strike cell edit, undo/redo and exact BOM/CRLF save preserve formatting tags; preview screenshots require review')

    if args.html_lists:
        utf16 = lambda text: len(text.encode('utf-16-le'))//2
        def expect_list(text, caret=None):
            state = run('snapshot')
            assert state['AXValue'] == text, 'HTML list keyboard command changed unexpected source'
            if caret is not None:
                assert state['AXSelectedTextRange'] == {'location': utf16(text[:caret]), 'length': 0}, state['AXSelectedTextRange']
        beta = source.index('Beta')
        run('select', utf16(source[:beta+2]), 0)
        run('key', 36)
        split = source.replace('Beta', 'Be</b></li><li><b>ta')
        expect_list(split, split.index('ta</b>'))
        run('capture', str(out/'list-split'))
        run('key', 6, 'cmd'); expect_list(source)
        run('select', utf16(source[:beta]), 0)
        run('key', 30, 'cmd')  # Command-]
        nested = source.replace('</li><li><b>Beta</b></li>', '<ul><li><b>Beta</b></li></ul></li>')
        expect_list(nested, nested.index('Beta'))
        run('capture', str(out/'list-indented'))
        run('key', 33, 'cmd')  # Command-[
        expect_list(source, beta)
        run('key', 51)
        exited = source.replace('<li><b>Beta</b></li></ul>', '</ul><p><b>Beta</b></p>')
        expect_list(exited, exited.index('Beta'))
        run('capture', str(out/'list-backspace'))
        run('key', 6, 'cmd'); expect_list(source)
        run('select', utf16(source[:beta+4]), 0)
        run('key', 36)
        empty = source.replace('Beta</b></li>', 'Beta</b></li><li><b></b></li>')
        expect_list(empty, empty.index('<li><b></b>')+len('<li><b>'))
        run('key', 36)
        paragraph = source.replace('</ul></td><td>KEEP', '</ul><p><b></b></p></td><td>KEEP')
        expect_list(paragraph, paragraph.index('<p><b></b>')+len('<p><b>'))
        run('paste-text', '中文<&')
        typed = paragraph.replace('<p><b></b>', '<p><b>中文&lt;&amp;</b>')
        expect_list(typed)
        run('capture', str(out/'list-empty-exit'))
        run('key', 1, 'cmd'); time.sleep(.3)
        assert fixture.read_bytes() == b'\xef\xbb\xbf'+typed.encode()
        for expected in [paragraph, empty, source]:
            run('key', 6, 'cmd'); expect_list(expected)
        for expected in [empty, paragraph, typed]:
            run('key', 6, 'cmd+shift'); expect_list(expected)
        for expected in [paragraph, empty, source]:
            run('key', 6, 'cmd'); expect_list(expected)
        run('key', 1, 'cmd'); time.sleep(.3)
        assert fixture.read_bytes() == b'\xef\xbb\xbf'+source.encode()
        result['checks'].append('real Enter split/empty exit, Command-bracket indent/outdent, Backspace, exact caret, literal paste, consecutive undo/redo and BOM/CRLF save pass; not an IME test')

    if args.merged_tables:
        utf16 = lambda text: len(text.encode('utf-16-le'))//2
        start = source.index('Merged alpha')
        right = source.index('Cell right')
        lower = source.index('Cell lower')
        run('select', utf16(source[:start]), 0)
        run('key', 48)
        assert run('snapshot')['AXSelectedTextRange']['location'] == utf16(source[:right])
        run('key', 48)
        assert run('snapshot')['AXSelectedTextRange']['location'] == utf16(source[:lower])
        run('key', 48, 'shift')
        run('key', 48, 'shift')
        assert run('snapshot')['AXSelectedTextRange']['location'] == utf16(source[:start])
        run('paste-text', '新<&')
        changed = source[:start] + '新&lt;&amp;' + source[start:]
        assert run('snapshot')['AXValue'] == changed
        run('capture', str(out/'merged-table-edited'))
        run('key', 1, 'cmd'); time.sleep(.3)
        assert fixture.read_bytes() == b'\xef\xbb\xbf' + changed.encode()
        run('key', 6, 'cmd'); assert run('snapshot')['AXValue'] == source
        run('key', 6, 'cmd+shift'); assert run('snapshot')['AXValue'] == changed
        run('key', 6, 'cmd'); assert run('snapshot')['AXValue'] == source
        run('key', 1, 'cmd'); time.sleep(.3)
        assert fixture.read_bytes() == b'\xef\xbb\xbf' + source.encode()
        result['checks'].append('merged table real Tab/Shift-Tab skips covered slots, escaped text insertion, exact BOM/CRLF save and undo/redo pass; screenshot requires review; no mouse drag claim')

    if args.html_tables:
        utf16 = lambda text: len(text.encode('utf-16-le'))//2
        start = source.index('Cell alpha')
        run('select',utf16(source[:start]),0)
        run('key',48)
        assert run('snapshot')['AXSelectedTextRange']['location']==utf16(source[:source.index('Cell beta')])
        run('key',48,'shift')
        assert run('snapshot')['AXSelectedTextRange']['location']==utf16(source[:start])
        run('paste-text','新<&')
        changed = source[:start]+'新&lt;&amp;'+source[start:]
        assert run('snapshot')['AXValue']==changed
        run('capture',str(out/'html-table-input'))
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('select',utf16(source[:source.index('Cell beta')]),0)
        run('key',48)
        appended = source.replace('Cell beta</td></tr></table>', 'Cell beta</td></tr><tr><td></td><td></td></tr></table>')
        assert run('snapshot')['AXValue']==appended
        run('capture',str(out/'html-table-appended'))
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
        result['checks'].append('HTML table native Tab/Shift-Tab, literal paste, last-cell append, undo and exact BOM/CRLF save pass; screenshots require review')
        run('select',utf16(source[:start]),0)
        image = HERE/'Fixtures/assets/yu-mark.png'
        run('paste-image',str(image))
        deadline = time.monotonic()+5
        while True:
            imported = run('snapshot')['AXValue']
            if '<img ' in imported: break
            if time.monotonic()>deadline: raise AssertionError('HTML cell bitmap paste did not insert an image element')
            time.sleep(.1)
        match = re.search(r'<img[^>]*src="([^"]+)"[^>]*>', imported)
        assert match, imported
        markup = match[0]
        assert imported == source[:start]+markup+source[start:]
        assert (fixture.parent/unquote(match[1])).read_bytes()==image.read_bytes()
        run('select',utf16(imported),0)
        time.sleep(.5)
        run('capture',str(out/'html-table-image'))
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+imported.encode()
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',6,'cmd+shift'); assert run('snapshot')['AXValue']==imported
        for caret, keycode in [(start+len(markup), 51), (start, 117)]:
            run('select',utf16(imported[:caret]),0)
            run('key',keycode)
            assert run('snapshot')['AXValue']==source, 'Image deletion changed adjacent text or retained the image'
            run('key',6,'cmd'); assert run('snapshot')['AXValue']==imported
        run('select',utf16(imported[:start]),utf16(markup))
        run('paste-text','替换<&')
        assert run('snapshot')['AXValue']==source[:start]+'替换&lt;&amp;'+source[start:]
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==imported
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
        result['checks'].append('HTML cell native bitmap paste creates an image element and matching resource, preserves other bytes, supports Backspace/Delete/text replacement, exact save and undo/redo; image screenshot requires review')


    if args.alignment:
        utf16 = lambda text: len(text.encode('utf-16-le'))//2
        boxes = {}
        for label in ['Center edge','Right edge','Left edge']:
            at = utf16(source[:source.index(label)])
            run('select',utf16(source),0)
            boxes[label] = run('bounds',at,utf16(label))['bounds']
            for fraction in [.25, .75]:
                run('select',utf16(source),0)
                bounds = run('bounds',at,utf16(label))['bounds']
                run('click',bounds['x']+bounds['width']*fraction,bounds['y']+bounds['height']/2)
                state = run('snapshot')
                assert state['AXValue']==source
                assert at <= state['AXSelectedTextRange']['location'] <= at+utf16(label), f'Aligned click missed {label}'
        assert boxes['Left edge']['x'] < boxes['Center edge']['x'] < boxes['Right edge']['x']
        run('select',utf16(source),0)
        run('capture',str(out/'alignment-preview'))
        result['checks'].append('real clicks at both edges of left/center/right labels select canonical source; native bounds follow alignment and source remains exact')
        at = source.index('Justified paragraph')
        run('select',utf16(source[:at]),0)
        run('paste-text','新增')
        changed = source[:at]+'新增'+source[at:]
        assert run('snapshot')['AXValue']==changed
        run('select',utf16(changed),0)
        run('capture',str(out/'justified-preview'))
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+changed.encode()
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',6,'cmd+shift'); assert run('snapshot')['AXValue']==changed
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
        result['checks'].append('justified HTML paragraph edit, source reveal/preview, undo/redo and exact save pass; native justification screenshot requires review')


    if args.anchors:
        utf16 = lambda text: len(text.encode('utf-16-le'))//2
        for label, target in [('Go to target', "<a id='target'>"), ('Go to heading', 'HTML chapter')]:
            at = utf16(source[:source.index(label)])
            expected = utf16(source[:source.index(target)])
            for fraction in [.25, .75]:
                run('select',utf16(source),0)
                bounds = run('bounds',at,utf16(label))['bounds']
                assert bounds['width'] > 0 and bounds['height'] > 0
                run('click',bounds['x']+bounds['width']*fraction,bounds['y']+bounds['height']/2,'cmd')
                state = run('snapshot')
                assert state['AXValue']==source
                assert state['AXSelectedTextRange']['location']==expected, f'Anchor {label} missed target: {state["AXSelectedTextRange"]}'
        run('capture',str(out/'anchor-navigation'))
        result['checks'].append('real Command-click at both label edges navigates Markdown and HTML links to inline id and heading targets without modifying source')

    if args.toc:
        locator = out/'locate-native-text'
        subprocess.run(['swiftc',str(ROOT/'tools/locate-native-text.swift'),'-o',str(locator)],check=True)
        for label in ['First chapter', 'Second chapter']:
            run('select',len(source.encode('utf-16-le'))//2,0)
            captured = run('capture',str(out/('toc-before-'+label)))
            window = next(window for window in captured['windows'] if window.get('kCGWindowLayer')==0 and window['kCGWindowBounds']['Width']>=400)
            screenshot = out/('toc-before-'+label+'-'+str(window['kCGWindowNumber'])+'.png')
            located = json.loads(subprocess.check_output([str(locator),str(screenshot),label,'--accurate'],text=True))
            (out/('toc-ocr-'+label+'.json')).write_text(json.dumps(located,indent=2))
            bounds = window['kCGWindowBounds']
            editor = run('snapshot')
            left = editor['AXPosition']['x']
            right = left + editor['AXSize']['width']
            matches = [item for item in located['matches'] if left <=
                       bounds['X']+(item['x']+item['width']/2)*bounds['Width']/located['image_width'] <= right]
            assert len(matches)>=2, 'TOC entry and heading must both appear inside the editor pixels'
            match = min(matches,key=lambda item:item['y'])
            run('click',bounds['X']+(match['x']+match['width']*.75)*bounds['Width']/located['image_width'],
                bounds['Y']+(match['y']+match['height']/2)*bounds['Height']/located['image_height'],'cmd')
            state = run('snapshot')
            assert state['AXValue']==source
            expected = len(source[:source.index(label)].encode('utf-16-le'))//2
            assert state['AXSelectedTextRange']['location']==expected, 'TOC Command-click missed heading source'
            assert state.get('focused_role')=='AXTextArea', 'TOC click must focus the document editor'
        result['checks'].append('pixel-located TOC entries Command-click to distinct canonical headings without source edits')
        start = source.index('Second chapter')
        run('select',len(source[:start].encode('utf-16-le'))//2,len('Second chapter'))
        run('paste-text','Updated chapter')
        changed = source[:start]+'Updated chapter'+source[start+len('Second chapter'):]
        run('select',len(changed.encode('utf-16-le'))//2,0)
        assert run('snapshot')['AXValue']==changed
        run('capture',str(out/'toc-updated'))
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',6,'cmd+shift'); assert run('snapshot')['AXValue']==changed
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
        result['checks'].append('TOC heading edit/undo/redo/save preserves the original marker and exact BOM/CRLF source')

    if args.footnotes:
        utf16 = lambda text: len(text.encode('utf-16-le'))//2
        for label in ['first', 'second']:
            marker = '[^'+label+']'
            reference = utf16(source[:source.index(marker)])
            definition = utf16(source[:source.index(marker+':')])
            for fraction in [.25, .75]:
                paragraph = source.rfind('\n',0,source.index(marker))+1
                run('select',utf16(source[:paragraph]),0)
                bounds = stable_bounds(reference,utf16(marker))
                assert bounds['width'] > 0 and bounds['height'] > 0
                run('click',bounds['x']+bounds['width']*fraction,
                    bounds['y']+bounds['height']/2,'cmd')
                state = run('snapshot')
                assert state['AXValue']==source
                assert state['AXSelectedTextRange']['location']==definition, 'Footnote click missed definition'
            run('select',definition+utf16(marker+': '),0)
            bounds = stable_bounds(definition,utf16(marker+':'))
            run('click',bounds['x']+bounds['width']/2,bounds['y']+bounds['height']/2,'cmd')
            assert run('snapshot')['AXSelectedTextRange']['location']==reference, 'Footnote backlink missed first reference'
        run('select',utf16(source),0)
        run('capture',str(out/'footnotes-navigation'))
        result['checks'].append('real Command-click on both sides of adjacent footnotes and definition backlinks preserves source')
        start = source.index('First definition')
        run('select',utf16(source[:start]),len('First'))
        run('paste-text','Updated')
        changed = source[:start]+'Updated'+source[start+len('First'):]
        assert run('snapshot')['AXValue']==changed
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',6,'cmd+shift'); assert run('snapshot')['AXValue']==changed
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
        run('select',utf16(source),0)
        result['checks'].append('footnote definition edit/undo/redo/save preserves exact BOM/CRLF source')

    if args.equations:
        captured = run('capture',str(out/'equation-reference-before-jump'))
        locator = out/'locate-native-text'
        subprocess.run(['swiftc',str(ROOT/'tools/locate-native-text.swift'),'-o',str(locator)],check=True)
        window = next(window for window in captured['windows'] if window.get('kCGWindowLayer')==0 and window['kCGWindowBounds']['Width']>=400)
        screenshot = out/('equation-reference-before-jump-'+str(window['kCGWindowNumber'])+'.png')
        located = json.loads(subprocess.check_output([str(locator),str(screenshot),'(1)'],text=True))
        (out/'equation-reference-ocr.json').write_text(json.dumps(located,indent=2))
        assert located['matches'], 'Automatic equation reference is absent from actual pixels'
        match = min(located['matches'],key=lambda item:item['y'])
        bounds = window['kCGWindowBounds']
        run('click',bounds['X']+(match['x']+match['width']/2)*bounds['Width']/located['image_width'],
            bounds['Y']+(match['y']+match['height']/2)*bounds['Height']/located['image_height'],'cmd')
        state = run('snapshot')
        assert state['AXValue']==source
        assert state['AXSelectedTextRange']['location']==len(source[:source.index('$$')].encode('utf-16-le'))//2, 'Command-click did not jump to referenced equation source'
        run('capture',str(out/'equation-reference-after-jump'))
        run('select',len(source.encode('utf-16-le'))//2,0)
        result['checks'].append('pixel-located equation reference Command-click jumps to its canonical source without editing')
        marker = source.index('tag{A}')+4
        run('select',len(source[:marker].encode('utf-16-le'))//2,1)
        run('paste-text','B')
        changed = source[:marker]+'B'+source[marker+1:]
        run('select',len(changed.encode('utf-16-le'))//2,0)
        time.sleep(1.5)
        assert run('snapshot')['AXValue']==changed
        run('capture',str(out/'equation-tag-updated'))
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',6,'cmd+shift'); assert run('snapshot')['AXValue']==changed
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
        run('select',len(source.encode('utf-16-le'))//2,0)
        result['checks'].append('equation tag edit/undo/redo preserves canonical source; reference update and isolated inline failure screenshots captured')

    if args.dollars:
        formula_start = source.index('x_2')
        formula_utf16 = len(source[:formula_start].encode('utf-16-le'))//2
        run('select',formula_utf16,3)
        run('capture',str(out/'inline-formula-editing'))
        assert run('snapshot')['AXValue']==source
        changed = source[:formula_start]+'x_3'+source[formula_start+3:]
        run('paste-text','x_3')
        run('select',len(changed.encode('utf-16-le'))//2,0)
        time.sleep(1.5)
        assert run('snapshot')['AXValue']==changed
        run('capture',str(out/'inline-formula-updated'))
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',6,'cmd+shift'); assert run('snapshot')['AXValue']==changed
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
        run('select',len(source.encode('utf-16-le'))//2,0)
        result['checks'].append('inline formula selection/edit and undo/redo preserve exact source; editing and updated previews captured')

    if args.highlight:
        start = source.index('==中文') + 2
        position = len(source[:start].encode('utf-16-le'))//2
        run('select',position,2)
        run('capture',str(out/'highlight-editing'))
        changed = source[:start]+'高亮文字'+source[start+2:]
        run('paste-text','高亮文字')
        run('select',len(changed.encode('utf-16-le'))//2,0)
        assert run('snapshot')['AXValue']==changed
        run('capture',str(out/'highlight-updated'))
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',6,'cmd+shift'); assert run('snapshot')['AXValue']==changed
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
        run('select',len(source.encode('utf-16-le'))//2,0)
        result['checks'].append('highlight selection/edit/undo/redo preserves exact source; paragraph/table preview requires screenshot review')

    if args.scripts:
        start = source.index('x^3^') + 2
        position = len(source[:start].encode('utf-16-le'))//2
        run('select',position,1)
        run('capture',str(out/'script-editing'))
        changed = source[:start]+'4'+source[start+1:]
        run('paste-text','4')
        run('select',len(changed.encode('utf-16-le'))//2,0)
        assert run('snapshot')['AXValue']==changed
        run('capture',str(out/'script-updated'))
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',6,'cmd+shift'); assert run('snapshot')['AXValue']==changed
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==source
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
        run('select',len(source.encode('utf-16-le'))//2,0)
        result['checks'].append('script editing and metadata preserve exact source; baseline/source reveal screenshots require review')

    if args.themes:
        def click(control):
            point, size = control['AXPosition'], control['AXSize']
            run('click',point['x']+size['width']/2,point['y']+size['height']/2)
        for name in ['Night','Github','Yu（跟随系统）']:
            run('key',43,'cmd'); time.sleep(.2)
            values=run('controls')['controls']
            click(next(item for item in values if item.get('AXDescription')=='正文主题'))
            deadline=time.monotonic()+3
            while True:
                options=run('controls')['controls']
                option=next((item for item in options if item.get('AXRole')=='AXMenuItem' and item.get('AXTitle')==name),None)
                if option is not None: break
                if time.monotonic()>deadline: raise AssertionError('Theme menu unavailable')
                time.sleep(.05)
            click(option)
            run('key',13,'cmd'); time.sleep(1.5)
            assert run('snapshot')['AXValue']==source
            assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
            run('capture',str(out/('theme-'+name)))
        result['checks'].append('Night/Github/Yu native theme transitions preserve source and saved bytes; screenshots captured')
        run('key',24,'cmd'); run('key',24,'cmd'); time.sleep(1.5)
        run('capture',str(out/'zoomed-native-resources'))
        assert run('snapshot')['AXValue']==source
        assert len(helpers())==1
        run('key',29,'cmd'); time.sleep(.4)
        result['checks'].append('zoom regenerates native resources without source edits and retains one helper')

    if args.failures:
        for kind, invalid, marker in [('math', r'\unknownYuCommand{x}', 'unknownYuCommand'), ('mermaid', 'yuUnsupportedGraph', 'yuUnsupportedGraph')]:
            bad = '# Native resources\r\n\r\n```'+kind+'\r\n'+invalid+'\r\n```\r\n\r\nAFTER FAILURE\r\n'
            run('select',0,len(source.encode('utf-16-le'))//2)
            run('paste-text',bad)
            run('select',len(bad.encode('utf-16-le'))//2,0)
            time.sleep(2)
            captured = run('capture',str(out/('failed-resource-source-'+kind)))
            locator = out/'locate-native-text'
            subprocess.run(['swiftc',str(ROOT/'tools/locate-native-text.swift'),'-o',str(locator)],check=True)
            window = next(window for window in captured['windows'] if window.get('kCGWindowLayer')==0 and window['kCGWindowBounds']['Width']>=400)
            screenshot = out/('failed-resource-source-'+kind+'-'+str(window['kCGWindowNumber'])+'.png')
            located = json.loads(subprocess.check_output([str(locator),str(screenshot),marker],text=True))
            (out/('failed-source-ocr-'+kind+'.json')).write_text(json.dumps(located,indent=2))
            assert located['matches'], 'Failed resource source is not visible in actual pixels'
            match = located['matches'][0]
            bounds = window['kCGWindowBounds']
            run('right-click',bounds['X']+(match['x']+match['width']/2)*bounds['Width']/located['image_width'],
                bounds['Y']+(match['y']+match['height']/2)*bounds['Height']/located['image_height'])
            run('capture',str(out/('failed-resource-menu-'+kind)))
            selected = run('snapshot')['AXSelectedTextRange']['location']
            assert bad.index(invalid) <= selected < bad.index(invalid)+len(invalid), 'Right click missed failed source'
            # AppKit context menus are not reliably exposed under AXChildren;
            # invoke the first native context action with real keyboard events.
            run('key',125); run('key',36)
            deadline = time.monotonic()+3
            while True:
                controls = run('controls')['controls']
                if any(item.get('AXTitle')=='继续编辑' for item in controls): break
                if time.monotonic()>deadline: raise AssertionError('Native diagnostic sheet did not open')
                time.sleep(.1)
            assert '原始源码已保留' in json.dumps(controls,ensure_ascii=False), 'Native diagnostic unavailable'
            if kind=='math':
                assert 'unknownYuCommand' in json.dumps(controls,ensure_ascii=False), 'Native diagnostic lost helper error'
            run('capture',str(out/('failed-resource-diagnostic-'+kind)))
            button = next(item for item in controls if item.get('AXTitle')=='继续编辑')
            point,size=button['AXPosition'],button['AXSize']
            run('click',point['x']+size['width']/2,point['y']+size['height']/2)
            assert run('snapshot')['AXValue']==bad
            run('key',1,'cmd'); time.sleep(.3)
            assert fixture.read_bytes()==b'\xef\xbb\xbf'+bad.encode()
            result['checks'].append(kind+' failure displays actual editable source and native diagnostic; BOM/CRLF save remains exact')
            run('select',0,len(bad.encode('utf-16-le'))//2)
            run('paste-text',source)
            run('select',len(source.encode('utf-16-le'))//2,0)
            time.sleep(2)
            run('capture',str(out/('repaired-resource-'+kind)))
            assert run('snapshot')['AXValue']==source
            run('key',6,'cmd'); assert run('snapshot')['AXValue']==bad
            run('key',6,'cmd+shift'); assert run('snapshot')['AXValue']==source
            run('key',1,'cmd'); time.sleep(.3)
            assert fixture.read_bytes()==b'\xef\xbb\xbf'+source.encode()
            result['checks'].append('replacing unsupported source restores native preview and undo/redo preserves both source versions')

    if args.footnote_errors:
        bad = 'Missing reference[^missing].\r\n\r\n' + source
        current = run('snapshot')['AXValue']
        run('select',0,len(current.encode('utf-16-le'))//2)
        run('paste-text',bad)
        run('select',0,0)
        marker = bad.index('[^missing]')
        bounds = stable_bounds(marker,len('[^missing]'))
        run('right-click',bounds['x']+bounds['width']/2,bounds['y']+bounds['height']/2)
        run('capture',str(out/'footnote-error-menu'))
        run('key',125); run('key',36)
        deadline = time.monotonic()+3
        while True:
            controls = run('controls')['controls']
            if any(item.get('AXTitle')=='继续编辑' for item in controls): break
            if time.monotonic()>deadline: raise AssertionError('Footnote diagnostic did not open')
            time.sleep(.1)
        details = json.dumps(controls,ensure_ascii=False)
        assert '找不到此脚注的定义' in details and '原始源码已保留' in details
        run('capture',str(out/'footnote-error-diagnostic'))
        button = next(item for item in controls if item.get('AXTitle')=='继续编辑')
        point,size = button['AXPosition'],button['AXSize']
        run('click',point['x']+size['width']/2,point['y']+size['height']/2)
        assert run('snapshot')['AXValue']==bad
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+bad.encode()
        addition = '\r\n[^missing]: Repaired definition 中文🙂.\r\n'
        run('select',len(bad.encode('utf-16-le'))//2,0)
        run('paste-text',addition)
        corrected = bad + addition
        assert run('snapshot')['AXValue']==corrected
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==bad
        run('key',6,'cmd+shift'); assert run('snapshot')['AXValue']==corrected
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+corrected.encode()
        run('select',0,0)
        bounds = stable_bounds(marker,len('[^missing]'))
        run('click',bounds['x']+bounds['width']/2,bounds['y']+bounds['height']/2,'cmd')
        assert run('snapshot')['AXSelectedTextRange']['location']==len(bad.encode('utf-16-le'))//2+2
        run('capture',str(out/'footnote-error-repaired'))
        source = corrected
        result['checks'].append('missing footnote native diagnostic, exact invalid save, definition repair, undo/redo, exact corrected save and Command-click navigation pass')

    if args.lifecycle:
        probe=out/'process-footprint'
        subprocess.run(['clang','-Wall','-Wextra','-Werror',str(ROOT/'tools/process-footprint.c'),'-o',str(probe)],check=True)
        def footprint(pid):
            return json.loads(subprocess.check_output([str(probe),str(pid)],text=True))
        live=helpers()
        assert len(live)==1,live
        first_pid=live[0]
        samples=[]
        idle_start=time.monotonic()
        result['lifecycle']={'first_pid':first_pid,'metric':'proc_pid_rusage RUSAGE_INFO_V4 physical footprint','samples':samples}
        while helpers():
            assert process.poll() is None, 'App exited during idle observation'
            elapsed=time.monotonic()-idle_start
            assert elapsed<70, 'Helper remained alive past idle deadline'
            try:
                helper_sample=footprint(first_pid)
            except subprocess.CalledProcessError:
                if helpers(): raise
                break
            samples.append({'seconds':elapsed,'app':footprint(process.pid),'helper':helper_sample})
            time.sleep(1)
        result['lifecycle']={'first_pid':first_pid,'idle_observed_seconds':time.monotonic()-idle_start,
                             'metric':'proc_pid_rusage RUSAGE_INFO_V4 physical footprint','samples':samples,
                             'app_after_helper_exit':footprint(process.pid)}
        assert samples, 'Missing process memory observations'
        reap_deadline=time.monotonic()+2
        while True:
            state=subprocess.run(['ps','-p',str(first_pid),'-o','stat='],capture_output=True,text=True)
            if not state.stdout.strip(): break
            assert time.monotonic()<reap_deadline, f'Exited helper was not reaped: {state.stdout.strip()}'
            time.sleep(.02)
        result['lifecycle']['exited_helper_reaped']=True
        run('capture',str(out/'resources-after-helper-exit'))
        assert run('snapshot')['AXValue']==source
        changed=source.replace('pmatrix}a','pmatrix}e')
        assert changed!=source
        run('select',0,len(source.encode('utf-16-le'))//2)
        run('paste-text',changed)
        run('select',len(changed.encode('utf-16-le'))//2,0)
        deadline=time.monotonic()+15
        while not helpers():
            assert time.monotonic()<deadline, 'New render did not restart helper'
            time.sleep(.1)
        restarted=helpers()
        assert len(restarted)==1 and restarted[0]!=first_pid,restarted
        result['lifecycle']['restarted_pid']=restarted[0]
        time.sleep(2)
        run('capture',str(out/'resources-after-helper-restart'))
        assert run('snapshot')['AXValue']==changed
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+changed.encode()
        result['checks'].append('real app helper exits idle while cached resources remain visible; edited formula starts a new helper and saves exact source; physical footprint samples recorded')

    if args.cancel_helper:
        live=helpers()
        assert len(live)==1,live
        blocked_pid=live[0]
        os.kill(blocked_pid,signal.SIGSTOP)
        suspended_helpers.add(blocked_pid)
        paused=subprocess.check_output(['ps','-p',str(blocked_pid),'-o','stat='],text=True).strip()
        assert 'T' in paused,paused
        pending=source.replace('pmatrix}a','pmatrix}f')
        assert pending!=source
        run('select',0,len(source.encode('utf-16-le'))//2)
        run('paste-text',pending)
        run('select',len(pending.encode('utf-16-le'))//2,0)
        time.sleep(.5)
        assert blocked_pid in helpers(), 'Stopped helper disappeared before cancellation trigger'
        assert run('snapshot')['AXValue']==pending
        run('capture',str(out/'helper-response-suspended'))
        cancellation_start=time.monotonic()
        run('select',0,len(pending.encode('utf-16-le'))//2)
        run('paste-text',initial)
        run('select',len(initial.encode('utf-16-le'))//2,0)
        deadline=time.monotonic()+5
        while blocked_pid in helpers():
            assert time.monotonic()<deadline, 'Document revision change failed to cancel stopped helper'
            time.sleep(.05)
        result['cancellation']={'blocked_pid':blocked_pid,'suspended_state':paused,
            'observed_seconds_including_input_events':time.monotonic()-cancellation_start,
            'scenario':'real packaged helper suspended before response; newer document replaces pending formula'}
        assert run('snapshot')['AXValue']==initial
        time.sleep(1)
        assert not helpers(), 'Plain document restarted cancelled resource work'
        run('capture',str(out/'after-helper-cancellation'))
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+initial.encode()
        recovered=source.replace('pmatrix}a','pmatrix}g')
        run('select',0,len(initial.encode('utf-16-le'))//2)
        run('paste-text',recovered)
        run('select',len(recovered.encode('utf-16-le'))//2,0)
        deadline=time.monotonic()+15
        while not helpers():
            assert time.monotonic()<deadline, 'Helper did not restart after cancellation'
            time.sleep(.1)
        assert blocked_pid not in helpers()
        time.sleep(2)
        run('capture',str(out/'after-cancellation-recovery'))
        assert run('snapshot')['AXValue']==recovered
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+recovered.encode()
        result['checks'].append('document revision cancels an actual suspended helper response; plain replacement stays free of stale resources; next valid resources restart and save exactly')

    if args.diagram_suite or args.recent_diagrams:
        cases = [
            ('flowchart', 'flowchart LR\nA[Alpha] --> B[Beta]'),
            ('sequence', 'sequenceDiagram\nactor A as Alpha\nparticipant B as Beta\nA->>B: Request\nB-->>A: Reply\nA-|/B: Half head\nA//--B: Reverse stick\ncreate actor C as Worker\nA->>C: Create\ndestroy C\nC-->>A: Finish'),
            ('class', 'classDiagram\nclass Alpha {\n+String name\n+run()\n}\nAlpha --> Beta'),
            ('state', 'stateDiagram-v2\n[*] --> Alpha\nAlpha --> Beta\nBeta --> [*]'),
            ('er', 'erDiagram\nCUSTOMER["Alpha 客户"] { int id PK string name }\nORDER["Beta 订单"] { int id PK int owner FK }\nCUSTOMER ||--o{ ORDER : owns'),
            ('gantt', 'gantt\ndateFormat YYYY-MM-DD\ntitle Alpha schedule\nsection Work\nAlpha :a, 2026-01-01, 2d\nBeta :after a, 3d'),
            ('pie', 'pie title Alpha share\n"One" : 40\n"Two" : 60'),
        ]
        if args.recent_diagrams:
            cases = []
            for filename, family, labels in [
                ('group4-sequence-central.md','central',['接收端中心连接','右向左','交接']),
                ('group4-gantt-calendar.md','calendar',['工作中文🙂','提交中文🙂','午前','任务']),
            ]:
                fixture_text = (HERE/'Fixtures'/filename).read_text(encoding='utf-8')
                diagrams = re.findall(r'```mermaid\n(.*?)\n```', fixture_text, flags=re.S)
                assert len(diagrams) >= len(labels)
                for index, (diagram, label) in enumerate(zip(diagrams, labels), 1):
                    assert label in diagram
                    # Prefix one visible caption to reuse the exact edit/history
                    # assertions below without changing participant identities.
                    cases.append((f'{family}-{index}',diagram.replace(label,'Alpha'+label,1)))
        for family, diagram in cases:
            text = '# '+family+'\r\n\r\n```mermaid\r\n'+diagram.replace('\n','\r\n')+'\r\n```\r\n\r\nAFTER DIAGRAM\r\n'
            current = run('snapshot')['AXValue']
            run('select',0,len(current.encode('utf-16-le'))//2)
            run('paste-text',text)
            run('select',len(text.encode('utf-16-le'))//2,0)
            time.sleep(2)
            assert run('snapshot')['AXValue']==text
            assert len(helpers())==1
            run('capture',str(out/('diagram-'+family)))
            at = text.index('Alpha')
            run('select',len(text[:at].encode('utf-16-le'))//2,len('Alpha'))
            run('paste-text','Updated')
            changed = text[:at]+'Updated'+text[at+len('Alpha'):]
            run('select',len(changed.encode('utf-16-le'))//2,0)
            time.sleep(1.5)
            assert run('snapshot')['AXValue']==changed
            run('capture',str(out/('diagram-'+family+'-edited')))
            run('key',1,'cmd'); time.sleep(.3)
            assert fixture.read_bytes()==b'\xef\xbb\xbf'+changed.encode()
            run('key',6,'cmd'); assert run('snapshot')['AXValue']==text
            run('key',6,'cmd+shift'); assert run('snapshot')['AXValue']==changed
            source = changed
            result['checks'].append(family+' diagram insert, label edit, undo/redo and exact save pass; actual diagram screenshots require review')
        run('key',1,'cmd'); time.sleep(.3)

    if args.math_suite:
        formulas = [('aligned', '\\begin{aligned}\\text{收益}&=a+b\\\\\\text{成本}&=c+d\\end{aligned}'), ('cases', 'f(x)=\\begin{cases}x&x>0\\\\-x&x<0\\end{cases}'), ('multiline', '\\begin{aligned}\nx&=1+2\\\\\ny&=3+4\\\\\nz&=5+6\n\\end{aligned}')]
        for family, formula in formulas:
            text = '# '+family+'\r\n\r\n$$\r\n'+formula.replace('\n','\r\n')+'\r\n$$\r\n\r\nAFTER FORMULA\r\n'
            current = run('snapshot')['AXValue']
            run('select',0,len(current.encode('utf-16-le'))//2)
            run('paste-text',text)
            run('select',len(text.encode('utf-16-le'))//2,0)
            time.sleep(2)
            assert run('snapshot')['AXValue']==text
            assert len(helpers())==1
            run('capture',str(out/('math-'+family)))
            needle, replacement = ('收益','净收益') if family=='aligned' else (('x>0','x>1') if family=='cases' else ('1+2','1+9'))
            at = text.index(needle)
            run('select',len(text[:at].encode('utf-16-le'))//2,len(needle.encode('utf-16-le'))//2)
            run('paste-text',replacement)
            changed = text[:at]+replacement+text[at+len(needle):]
            run('select',len(changed.encode('utf-16-le'))//2,0)
            time.sleep(1.5)
            assert run('snapshot')['AXValue']==changed
            run('capture',str(out/('math-'+family+'-edited')))
            run('key',1,'cmd'); time.sleep(.3)
            assert fixture.read_bytes()==b'\xef\xbb\xbf'+changed.encode()
            run('key',6,'cmd'); assert run('snapshot')['AXValue']==text
            run('key',6,'cmd+shift'); assert run('snapshot')['AXValue']==changed
            source = changed
            result['checks'].append(family+' formula edit, undo/redo and exact save pass; screenshots require review')
        run('key',1,'cmd'); time.sleep(.3)

    if args.promotion_suite:
        utf16 = lambda text: len(text.encode('utf-16-le'))//2
        fixtures = ROOT/'crates/yu-editor/tests/fixtures/group4-paste'
        cases = [
            ('math-target','merged-payload',True),
            ('footnote-target','merged-payload',True),
            ('row-groups-target','row-groups-merged-payload',True),
            ('cross-groups','merged-payload',False),
            ('row-groups-target','row-groups-conflict-payload',False),
        ]
        for case_index, (name,payload_name,accept) in enumerate(cases):
            donor = (fixtures/(payload_name+'.md')).read_text(encoding='utf-8').strip()
            target = (fixtures/(name+'.md')).read_text(encoding='utf-8')
            text = ('DONOR\n\n'+donor+'\n\n'+target).replace('\n','\r\n')
            current = run('snapshot')['AXValue']
            run('select',0,utf16(current)); run('paste-text',text)
            run('select',utf16(text),0); run('paste-text',' HISTORY-A')
            before = text+' HISTORY-A'
            run('select',0,0); run('select',utf16(before),0)
            run('paste-text',' HISTORY-B'); run('key',6,'cmd')
            assert run('snapshot')['AXValue'] == before
            donor_labels = re.findall(r'>([^<>]+)</t[dh]>',donor)
            assert donor_labels
            def point(label):
                at = before.index(label)
                b = stable_bounds(utf16(before[:at]),utf16(label))
                return (b['x']+b['width']/2,b['y']+b['height']/2)
            run('select',0,0)
            destination = point('目标中文🙂')
            first = point(donor_labels[0]); last = point(donor_labels[-1])
            if first != last:
                run('drag',*first,*last,'alt+shift')
            else:
                run('click',*first,'alt+shift')
            copied = run('copy-read')
            payload = json.loads(copied['source_fragments'])
            assert payload['version'] == 2 and payload['tableSource']
            assert 'rowspan' in payload['tableSource'] and payload.get('columns') == 2
            assert all(label in payload['tableSource'] for label in donor_labels)
            run('copy-paste',*destination)
            after = run('snapshot')['AXValue']
            case_id = f'promotion-{case_index}-{name}'
            if accept:
                assert after != before and after.count('<table') == 2
                assert after.startswith(('DONOR\n\n'+donor+'\n\n').replace('\n','\r\n'))
                assert after.count(donor_labels[0]) == 2
                if name == 'math-target':
                    assert 'data-math-style' in after and '>x^2</span>' in after
                if name == 'footnote-target':
                    assert 'data-yu-footnote' in after and '[^note]: 保留脚注中文🙂。' in after
                if name == 'row-groups-target':
                    for mark in ["<thead id='head'>","<tbody id='body'>",'保留甲','保留乙','保留丙','保留丁',"<span data-math-style='inline'>x^2</span>","<span data-yu-footnote='reference'>[^note]</span>"]:
                        assert mark in after,mark
                run('key',6,'cmd'); assert run('snapshot')['AXValue'] == before
                run('key',6,'cmd+shift'); assert run('snapshot')['AXValue'] == after
                run('select',0,0); time.sleep(.3)
                if name == 'math-target':
                    at = after.index('x^2'); offset = utf16(after[:at])
                    b = stable_bounds(offset,3)
                    run('click',b['x']+b['width']*.25,b['y']+b['height']/2)
                    selected = run('snapshot')['AXSelectedTextRange']
                    assert selected['length'] == 0 and offset <= selected['location'] <= offset+3
                    position = at + selected['location']-offset
                    edited = after[:position]+'q'+after[position:]
                    run('paste-text','q'); assert run('snapshot')['AXValue'] == edited
                    run('key',6,'cmd'); assert run('snapshot')['AXValue'] == after
                    run('key',6,'cmd+shift'); assert run('snapshot')['AXValue'] == edited
                    run('key',6,'cmd'); assert run('snapshot')['AXValue'] == after
                    result['checks'].append(case_id+': native formula mouse hit, TeX body edit and exact undo/redo')
                if name == 'footnote-target':
                    at = after.index('[^note]'); b = stable_bounds(utf16(after[:at]),utf16('[^note]'))
                    run('click',b['x']+b['width']/2,b['y']+b['height']/2,'cmd')
                    assert run('snapshot')['AXSelectedTextRange']['location'] == utf16(after[:after.index('[^note]:')])
                    result['checks'].append(case_id+': converted HTML footnote Command-click reaches its document definition')
                run('select',0,0); time.sleep(.3)
                run('capture',str(out/case_id))
                run('key',1,'cmd'); time.sleep(.3)
                assert fixture.read_bytes() == b'\xef\xbb\xbf'+after.encode()
                source = after
            else:
                assert after == before, 'Rejected native rowspan changed document'
                state = run('snapshot')
                if state.get('focused_description') == '警告':
                    run('controls')
                    run('capture',str(out/(case_id+'-rejection-alert')))
                    run('key',36)
                    time.sleep(.2)
                    assert run('snapshot')['AXValue'] == before
                run('key',6,'cmd+shift'); assert run('snapshot')['AXValue'] == before+' HISTORY-B'
                run('key',6,'cmd'); assert run('snapshot')['AXValue'] == before
                run('select',0,0); time.sleep(.3)
                run('capture',str(out/case_id))
                run('key',1,'cmd'); time.sleep(.3)
                assert fixture.read_bytes() == b'\xef\xbb\xbf'+before.encode()
                source = before
            result['checks'].append(case_id+': real Option+Shift donor selection and version-2 native clipboard; '+('conversion/owner preservation, undo/redo and saved bytes' if accept else 'cross-group rejection preserves source and pre-existing redo branch'))

    if args.ime:
        utf16 = lambda text: len(text.encode('utf-16-le'))//2
        original_input = run('snapshot')['input_source']
        try:
            run('source','com.apple.inputmethod.SCIM.ITABC')
            current = run('snapshot')['AXValue']
            run('select',utf16(current),0); run('paste-text','\r\nIME: ')
            before = current+'\r\nIME: '
            run('keys',6,4,31,45,5,13,14,45)  # zhongwen, actual hardware keys
            run('capture',str(out/'ime-composing'))
            run('key',49)  # Commit the native candidate with Space.
            committed = before+'中文'
            assert run('snapshot')['AXValue'] == committed
            run('key',6,'cmd'); assert run('snapshot')['AXValue'] == before
            run('key',6,'cmd+shift'); assert run('snapshot')['AXValue'] == committed
            run('select',utf16(committed),0)
            run('keys',6,4); run('key',53)  # Cancel a second marked-text session.
            assert run('snapshot')['AXValue'] == committed
            run('key',1,'cmd'); time.sleep(.3)
            assert fixture.read_bytes() == b'\xef\xbb\xbf'+committed.encode()
            run('capture',str(out/'ime-committed'))
            source = committed
            result['checks'].append('System Pinyin hardware-key composition commits 中文, one undo/redo restores exact source, Escape cancels a second composition, and saved BOM/CRLF bytes are exact')
        finally:
            run('source',original_input)

    if args.list_inputs:
        utf16 = lambda text: len(text.encode('utf-16-le'))//2
        manifest_path = args.list_inputs/'manifest.json'
        list_manifest = json.loads(manifest_path.read_text(encoding='utf-8'))
        selected_cases = [case for case in list_manifest['cases']
            if case['line_ending'] == 'crlf' and not case['bom'] and len(case['before_selections']) == 1]
        assert len(selected_cases) == 8
        result['list_manifest_sha256'] = sha(manifest_path)
        prefix = '# List audit\r\n\r\n```math\r\nx^2\r\n```\r\n\r\n'
        for case in selected_cases:
            directory = args.list_inputs/case['id']
            fixture_source = ROOT/'crates/yu-editor/tests/fixtures/group4-list'/case['fixture']
            assert sha(fixture_source) == case['fixture_sha256']
            values = {name:prefix+(directory/name).read_bytes().decode('utf-8-sig') for name in ['expected-0.md','expected-a.md','expected-b.md','expected-list-a.md']}
            base, before, seeded, expected = [values[name] for name in ['expected-0.md','expected-a.md','expected-b.md','expected-list-a.md']]
            current = run('snapshot')['AXValue']
            run('select',0,utf16(current)); run('paste-text',base)
            run('select',utf16(base),0); run('paste-text',before[len(base):])
            run('select',0,0); run('select',utf16(before),0); run('paste-text',seeded[len(before):])
            run('key',6,'cmd'); assert run('snapshot')['AXValue'] == before
            selection = case['before_selections'][0]
            anchor,focus = selection['anchor_utf16'],selection['focus_utf16']
            run('select',utf16(prefix)+min(anchor,focus),abs(anchor-focus))
            prior_selection = run('snapshot')['AXSelectedTextRange']
            assert case['command'] in ('indent','outdent') and case['expected_changed']
            run('key',30 if case['command']=='indent' else 33,'cmd')
            assert run('snapshot')['AXValue'] == expected,case['id']
            run('key',6,'cmd')
            state = run('snapshot'); assert state['AXValue'] == before and state['AXSelectedTextRange'] == prior_selection
            run('key',6,'cmd+shift'); assert run('snapshot')['AXValue'] == expected
            run('select',0,0); time.sleep(.25)
            run('capture',str(out/('list-'+case['id'])))
            run('key',1,'cmd'); time.sleep(.2)
            assert fixture.read_bytes() == b'\xef\xbb\xbf'+expected.encode()
            source = expected
            result['checks'].append('list-'+case['id']+': native bracket command matches fixed fixture; undo restores exact source and primary AX range; redo and BOM/CRLF save pass')

    if args.table_interactions:
        audit = group4_followup.Checks(run, stable_bounds, out, fixture, result)
        source, after_reopen_check = audit.table_interactions(ROOT)

    if args.table_resize:
        audit = group4_followup.Checks(run, stable_bounds, out, fixture, result)
        source, after_reopen_check = audit.merged_resize()

    if args.smoke_document:
        audit = group4_followup.Checks(run, stable_bounds, out, fixture, result)
        source = audit.smoke_document(ROOT)
    if args.stress_seconds:
        audit = group4_followup.Checks(run, stable_bounds, out, fixture, result)
        source = audit.resource_stress(ROOT, process.pid, helpers, args.stress_seconds, args.stress_idle_seconds)

    if args.list_gestures:
        audit = group4_followup.Checks(run, stable_bounds, out, fixture, result)
        source = audit.list_gestures()

    saved_before_reopen = fixture.read_bytes()
    old_helpers = helpers()
    run('key',12,'cmd'); process.wait(timeout=10)
    if args.reopen:
        expected = saved_before_reopen.decode('utf-8-sig')
        deadline = time.monotonic()+8
        while set(old_helpers).intersection(helpers()):
            assert time.monotonic()<deadline, 'Helper survived application exit'
            time.sleep(.1)
        assert fixture.read_bytes()==saved_before_reopen, 'Quit changed saved bytes'
        first_pid = process.pid
        process = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT)
        deadline = time.monotonic()+12
        while True:
            try:
                reopened = run('activate'); break
            except RuntimeError:
                if process.poll() is not None or time.monotonic()>deadline: raise
                time.sleep(.2)
        assert process.pid != first_pid
        assert reopened['AXValue']==expected, 'Relaunch did not restore exact disk source'
        if args.html_blocks:
            time.sleep(1)
            assert not helpers(), 'Reopened HTML document launched heavy helper'
        else:
            deadline = time.monotonic()+20
            while not helpers():
                assert time.monotonic()<deadline, 'Reopened resources did not launch helper'
                time.sleep(.1)
            assert len(helpers())==1 and not set(old_helpers).intersection(helpers())
        time.sleep(2)
        if after_reopen_check:
            after_reopen_check()
        run('capture',str(out/'reopened-document'))
        if args.resource_audit and args.stress_seconds:
            audit.restored_preview(expected)
        assert fixture.read_bytes()==saved_before_reopen
        end = len(expected.encode('utf-16-le'))//2
        run('select',end,0)
        suffix = '\r\nREOPEN CHECK 中文🙂\r\n'
        run('paste-text',suffix)
        assert run('snapshot')['AXValue']==expected+suffix
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==b'\xef\xbb\xbf'+(expected+suffix).encode()
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==expected
        run('key',6,'cmd+shift'); assert run('snapshot')['AXValue']==expected+suffix
        run('key',6,'cmd'); assert run('snapshot')['AXValue']==expected
        run('key',1,'cmd'); time.sleep(.3)
        assert fixture.read_bytes()==saved_before_reopen
        result['reopen']={'previous_pid':first_pid,'new_pid':process.pid,'old_helpers':old_helpers,
                          'new_helpers':helpers(),'saved_sha256':sha(fixture)}
        run('key',12,'cmd'); process.wait(timeout=10)
        assert fixture.read_bytes()==saved_before_reopen
        result['checks'].append('full application quit/relaunch restores exact BOM/CRLF disk bytes, restarts resources on demand, permits new input/undo/redo/save and preserves original bytes after second quit; screenshot requires review')
    result['passed']=True
finally:
    for pid in suspended_helpers:
        if pid in helpers():
            try: os.kill(pid,signal.SIGCONT)
            except ProcessLookupError: pass
    if process.poll() is None:
        process.kill(); process.wait()
    log.close()
    subprocess.run(['defaults','delete',identifier],capture_output=True)
    shutil.rmtree(app)
    assert sha(production/'Contents/MacOS/Yu')==manifest['app_sha256']
    assert sha(production/'Contents/Helpers/yu-document-renderer')==manifest['helper_sha256']
    (out/'results.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
