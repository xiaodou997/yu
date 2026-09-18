#!/usr/bin/env python3
"""Drive the real macOS input method and mouse against a private Yu bundle.

Requires an unlocked desktop, Accessibility/event-posting/screen-capture access,
and enabled ABC and Simplified Pinyin. Never calls product self-check hooks.
Native pasteboard contents are preserved in the event driver's memory.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import struct
import time
import uuid

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
PINYIN = 'com.apple.inputmethod.SCIM.ITABC'
ABC = 'com.apple.keylayout.ABC'
BASE = ('# Native input\r\n\r\nSTART English words 中文段落 END\r\n\r\n'
        'Second paragraph with emoji 🙂 and é.\r\n\r\n'
        '| Alpha | Beta |\r\n| --- | --- |\r\n| one | two |\r\n| three | four |\r\n\r\n'
        '> | Left | Right |\r\n> | --- | --- |\r\n> | five | six |\r\n\r\n'
        'Tail unchanged.\r\n\r\n') + ''.join(
            f'Paragraph {i}: long document 中文内容 English words emoji 🙂 é. ' * 4 + '\r\n\r\n'
            for i in range(20))


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def u16index(source, term):
    return len(source[:source.index(term)].encode('utf-16-le')) // 2


def insert(source, offset, text):
    raw = source.encode('utf-16-le')
    return (raw[:offset * 2] + text.encode('utf-16-le') + raw[offset * 2:]).decode('utf-16-le')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--dark', action='store_true')
    parser.add_argument('--inspect', action='store_true', help='Launch a private app for independent manual/driver diagnosis; does not mark checks passed')
    parser.add_argument('--keep-open-on-failure', action='store_true', help='Keep only the failed private app alive for diagnosis')
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    build = json.loads((HERE / '.build/build-manifest.json').read_text())
    production = HERE / '.build/Yu.app'
    assert build['configuration'] == 'release' and digest(production / 'Contents/MacOS/Yu') == build['app_sha256']
    driver = out / 'native-event-driver'
    subprocess.run(['swiftc', str(ROOT / 'tools/native-event-driver.swift'), '-o', str(driver)], check=True)
    preflight = subprocess.run([str(driver), '--preflight'], text=True, capture_output=True, timeout=10)
    (out / 'desktop-preflight.json').write_text(preflight.stdout)
    if preflight.returncode != 0:
        raise RuntimeError('Desktop is locked or required native-event permissions are unavailable; see desktop-preflight.json')
    rule_measure = out / 'measure-native-rules'
    subprocess.run(['swiftc', str(ROOT / 'tools/measure-native-rules.swift'), '-o', str(rule_measure)], check=True)
    app = out / 'YuEditingChecks.app'
    shutil.copytree(production, app)
    info_path = app / 'Contents/Info.plist'
    info = plistlib.loads(info_path.read_bytes())
    info['CFBundleIdentifier'] = 'io.github.xiaodou997.yu.editing-check.' + uuid.uuid4().hex
    info_path.write_bytes(plistlib.dumps(info))
    subprocess.run(['codesign', '--force', '--sign', '-', '--identifier', info['CFBundleIdentifier'], str(app)], check=True)
    fixture = out / 'input.md'
    fixture.write_bytes(b'\xef\xbb\xbf' + BASE.encode())
    env = {k: v for k, v in os.environ.items() if not k.startswith('YU_')}
    env.update(YU_DOCUMENT_STATE_DIR=str(out / 'state'), YU_PRESENTATION_STATE_DIR=str(out / 'columns'), YU_NATIVE_INPUT_TRACE='1')
    events, checks = [], []
    result = {'passed': False, 'build': build, 'test_app_sha256': digest(app / 'Contents/MacOS/Yu'),
              'fixture_original_sha256': digest(fixture), 'checks': checks}
    process = None
    original_input = None

    def run(*arguments):
        command = [str(driver), str(process.pid), *map(str, arguments)]
        executed = subprocess.run(command, text=True, capture_output=True, timeout=15)
        events.append({'arguments': list(map(str, arguments)), 'exit_code': executed.returncode, 'stderr': executed.stderr})
        (out / 'events.json').write_text(json.dumps(events, ensure_ascii=False, indent=2))
        if executed.returncode:
            raise RuntimeError(f'{arguments}: {executed.stderr}')
        return executed.stdout

    def snap(name):
        time.sleep(.2)
        value = json.loads(run('snapshot'))
        (out / f'{name}.json').write_text(json.dumps(value, ensure_ascii=False, indent=2))
        return value

    def capture(name):
        value = json.loads(run('capture', out / name))
        (out / f'{name}-windows.json').write_text(json.dumps(value, indent=2))
        return value

    def table_rules(name, top=490, bottom=704):
        captured = capture(name)
        window = next(w for w in captured['windows'] if w['kCGWindowLayer'] == 0 and w['kCGWindowBounds']['Width'] >= 400)
        png = out / f'{name}-{window["kCGWindowNumber"]}.png'
        measured = json.loads(subprocess.check_output([str(rule_measure), str(png), '500', str(top), '1770', str(bottom)], text=True))
        (out / f'{name}-rules.json').write_text(json.dumps(measured, indent=2))
        assert len(measured['vertical_rules']) == 3, 'Cannot identify all three root-table rules'
        return measured['vertical_rules']

    def check(name):
        checks.append(name)
        print('PASS:', name, flush=True)
        (out / 'results.json').write_text(json.dumps(result, ensure_ascii=False, indent=2))

    def key(code, modifiers=''):
        run('key', code, modifiers)

    def undo(): key(6, 'cmd')
    def redo(): key(6, 'cmd+shift')
    def type_pinyin():
        for code in [6, 4, 31, 45, 5, 13, 14, 45]: key(code)
    def select(offset): run('select', offset, 0)

    try:
        log = (out / 'app.log').open('w')
        command = [str(app / 'Contents/MacOS/Yu'), str(fixture)]
        if args.dark: command.append('--dark-mode')
        process = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        deadline = time.monotonic() + 12
        while True:
            if process.poll() is not None:
                raise RuntimeError(f'Application exited before readiness: {process.returncode}')
            try:
                initial = json.loads(run('activate'))
                break
            except RuntimeError as error:
                if time.monotonic() >= deadline or not any(reason in str(error) for reason in
                    ['Usage: native-event-driver', 'No native document AXTextArea found', 'Target app did not become frontmost']):
                    raise
                time.sleep(.3)
        original_input = initial['input_source']
        result['initial_input_source'] = original_input
        result['pid'] = process.pid
        assert initial['AXValue'] == BASE
        if args.inspect:
            result['inspection_ready'] = True
            print('Inspection PID:', process.pid, flush=True)
            return 0
        run('source', PINYIN)
        anchor = u16index(BASE, 'START')
        select(anchor)
        type_pinyin()
        preedit = snap('pinyin-preedit')
        assert preedit['AXValue'] == BASE
        panel = capture('pinyin-preedit')
        assert any(w['kCGWindowLayer'] > 0 for w in panel['windows']), 'No real candidate window'
        key(49)
        expected = insert(BASE, anchor, '中文')
        assert snap('pinyin-commit')['AXValue'] == expected
        undo(); assert snap('pinyin-undo')['AXValue'] == BASE
        redo(); assert snap('pinyin-redo')['AXValue'] == expected
        run('keys', 45, 34, 4, 0, 31); key(53)
        assert snap('pinyin-cancel')['AXValue'] == expected
        run('keys', 16, 31, 51, 51)
        assert snap('pinyin-backspace-empty')['AXValue'] == expected
        undo(); assert snap('pinyin-backspace-undo')['AXValue'] == BASE
        redo(); assert snap('pinyin-backspace-redo')['AXValue'] == expected
        check('system Pinyin commit, candidate window, undo/redo, Escape and final-preedit Backspace cancellation')

        # Editing shortcuts must apply to the field editor, never the document.
        key(3, 'cmd'); assert snap('find-open')['focused_role'] == 'AXTextField'
        run('source', ABC)
        for code in [31, 45, 14]: key(code)
        assert snap('find-query')['focused_value'] == 'one', 'Hardware query did not reach field'
        key(0, 'cmd')
        snap('find-select-all')
        copied = json.loads(run('copy-read')); assert copied['text'] == 'one'
        cut = json.loads(run('cut-read')); assert cut['text'] == 'one'
        assert snap('find-cut')['focused_value'] == ''
        run('paste-text', 'one')
        assert snap('find-paste')['focused_value'] == 'one'
        undo(); undone = snap('find-undo'); assert undone['AXValue'] == expected and undone['focused_value'] == ''
        redo(); assert snap('find-redo')['focused_value'] == 'one'
        key(36); assert snap('find-return')['AXSelectedText'] == 'one'
        key(36, 'shift'); assert snap('find-shift-return')['AXSelectedText'] == 'one'
        key(53); assert snap('find-close')['focused_role'] == 'AXTextArea'
        key(3, 'cmd'); key(0, 'cmd'); key(51)
        run('source', PINYIN); run('keys', 45, 34, 4, 0, 31); key(53)
        assert snap('find-ime-cancel')['focused_role'] == 'AXTextField'
        key(53); assert snap('find-ime-close')['focused_role'] == 'AXTextArea'
        assert snap('find-document-unchanged')['AXValue'] == expected
        check('find clipboard/select-all/undo follow focus; Return navigation and IME-aware Escape')

        run('source', ABC)
        state = json.loads(run('activate'))
        main_window = next(w for w in state['windows'] if w['kCGWindowLayer'] == 0 and w['kCGWindowBounds']['Width'] > 400)
        bounds = main_window['kCGWindowBounds']
        assert bounds['Width'] == 900 and bounds['Height'] == 620
        x, y = bounds['X'], bounds['Y']
        a, b = (x + 360, y + 166), (x + 384, y + 206)
        run('click', *a); first = snap('drag-start')['AXSelectedTextRange']['location']
        run('click', *b); last = snap('drag-end')['AXSelectedTextRange']['location']
        assert first < last and '\r\n\r\n' in expected.encode('utf-16-le')[first*2:last*2].decode('utf-16-le')
        run('drag', *a, *b); forward = snap('drag-forward')
        assert forward['AXSelectedTextRange'] == {'location': first, 'length': last-first}
        run('drag', *b, *a); assert snap('drag-reverse')['AXSelectedTextRange'] == forward['AXSelectedTextRange']
        run('click', *a); run('click', *b, 'shift')
        assert snap('shift-click')['AXSelectedTextRange'] == forward['AXSelectedTextRange']
        check('real forward/reverse cross-paragraph dragging and Shift-click extension')
        run('double-click', *a)
        assert snap('double-word')['AXSelectedText'] == 'English'
        run('word-drag', *a, *b)
        words = snap('word-drag')['AXSelectedText']
        assert words.startswith('English ') and words.endswith('paragraph'), words
        run('triple-click', *a)
        assert snap('triple-paragraph')['AXSelectedText'] == '中文START English words 中文段落 END\r\n'
        run('double-click', x+491, y+206)
        assert snap('double-emoji')['AXSelectedText'] == '🙂'
        run('double-click', x+543, y+206)
        assert snap('double-composed')['AXSelectedText'] == 'é'
        assert json.loads(run('snapshot'))['AXValue'] == expected
        check('native double-click words/emoji/combining sequence, triple-click paragraph and whole-word dragging')


        run('click', x + 285, y + 299)
        offset = snap('table-point')['AXSelectedTextRange']['location']
        cell_start = u16index(expected, 'one')
        assert cell_start <= offset <= cell_start + 3
        run('source', PINYIN); type_pinyin(); key(49)
        typed = insert(expected, offset, '中文')
        assert snap('table-pinyin')['AXValue'] == typed
        key(48); assert snap('table-tab')['AXSelectedTextRange'] == {'location': u16index(typed, 'two'), 'length': 0}
        key(48, 'shift'); assert snap('table-backtab')['AXSelectedTextRange']['location'] == cell_start
        undo(); assert snap('table-undo')['AXValue'] == expected
        redo(); assert snap('table-redo')['AXValue'] == typed
        undo(); assert snap('table-restored')['AXValue'] == expected
        check('table mouse hit, system Pinyin, Tab/Shift-Tab and exact undo/redo')
        run('source', ABC)
        run('click', x+615, y+337)
        root_offset = snap('root-last-cell')['AXSelectedTextRange']['location']
        assert u16index(expected, 'four') <= root_offset <= u16index(expected, 'four')+4
        run('paste-text', '🙂')
        root_emoji = insert(expected, root_offset, '🙂')
        assert snap('root-emoji')['AXValue'] == root_emoji
        key(48)
        root_appended = snap('root-appended')['AXValue']
        assert root_appended.count('\r\n|') == root_emoji.count('\r\n|') + 1
        undo(); assert snap('root-append-undo')['AXValue'] == root_emoji
        undo(); assert snap('root-emoji-undo')['AXValue'] == expected
        redo(); assert snap('root-emoji-redo')['AXValue'] == root_emoji
        redo(); assert snap('root-append-redo')['AXValue'] == root_appended
        undo(); undo(); assert snap('root-reset')['AXValue'] == expected
        check('root-table emoji, last-cell Tab row insertion and two-step undo/redo')


        run('source', ABC)
        run('drag', x+285, y+299, x+615, y+337, 'alt+shift')
        copied = json.loads(run('copy-read'))
        (out / 'grid-copy.json').write_text(json.dumps(copied, ensure_ascii=False, indent=2))
        assert copied['text'].replace('\r\n','\n').strip() == 'one\ttwo\nthree\tfour', copied
        assert snap('grid-selected')['AXValue'] == expected
        capture('grid-selected')
        pasted = json.loads(run('copy-paste', x+305, y+435))
        assert pasted['text'] == copied['text']
        grid_source = snap('grid-pasted')['AXValue']
        assert '> | one | two |\r\n> | three | four |' in grid_source
        undo(); assert snap('grid-undo')['AXValue'] == expected
        redo(); assert snap('grid-redo')['AXValue'] == grid_source
        undo(); assert snap('grid-reset')['AXValue'] == expected
        check('actual rectangular drag, native grid clipboard, quoted-table growth and undo/redo')

        # Quoted-table input remains ordinary Markdown with original CRLF.
        run('click', x+305, y+435)
        quoted_offset = snap('quoted-point')['AXSelectedTextRange']['location']
        assert u16index(expected, 'five') <= quoted_offset <= u16index(expected, 'five') + 4
        run('source', PINYIN); type_pinyin(); key(49)
        quoted = insert(expected, quoted_offset, '中文')
        assert snap('quoted-pinyin')['AXValue'] == quoted
        run('paste-text', '🙂é')
        emoji = insert(quoted, quoted_offset + 2, '🙂é')
        assert snap('quoted-emoji')['AXValue'] == emoji
        key(51)
        assert snap('grapheme-backspace')['AXValue'] == insert(quoted, quoted_offset+2, '🙂'), 'Backspace split a combining sequence'
        key(51)
        assert snap('emoji-backspace')['AXValue'] == quoted, 'Backspace split an emoji'
        undo(); assert snap('grapheme-undo')['AXValue'] == emoji, 'Grouped backward deletion did not undo atomically'

        key(48); assert snap('quoted-tab')['AXSelectedTextRange']['location'] == u16index(emoji, 'six')
        key(48, 'shift'); assert snap('quoted-backtab')['AXSelectedTextRange']['location'] <= quoted_offset
        key(48); key(48)
        appended = snap('quoted-append')['AXValue']
        assert appended.count('> |') == emoji.count('> |') + 1
        for cycle in range(2):
            for number, previous_source in enumerate([emoji, quoted, expected]):
                undo(); assert snap(f'quoted-undo-{cycle}-{number}')['AXValue'] == previous_source
            for number, next_source in enumerate([quoted, emoji, appended]):
                redo(); assert snap(f'quoted-redo-{cycle}-{number}')['AXValue'] == next_source
        for previous_source in [emoji, quoted, expected]:
            undo(); assert json.loads(run('snapshot'))['AXValue'] == previous_source
        check('quoted table Pinyin/emoji, Tab/backtab, last-cell append and repeated three-step undo/redo')

        # Persisted widths prove the native gesture reached the production session.
        key(1, 'cmd'); time.sleep(.3)
        run('source', ABC)
        before_rules = table_rules('resize-before')
        divider_x = x + before_rules[1]/2
        run('drag', divider_x, y+299, divider_x-48, y+299)
        assert snap('resize-finished')['AXValue'] == expected
        width_files = list((out / 'columns').glob('*.yucolumns'))
        assert len(width_files) == 1, 'Native divider drag did not persist width metadata'
        width_bytes = width_files[0].read_bytes()
        assert width_bytes[:8] == b'YUCOLW01'
        offset = 12 + struct.unpack_from('<I', width_bytes, 8)[0] + 16
        count = struct.unpack_from('<I', width_bytes, offset)[0]
        assert count >= 1
        offset += 4 + 16
        columns = struct.unpack_from('<I', width_bytes, offset)[0]
        ratios = struct.unpack_from('<' + 'f'*columns, width_bytes, offset+4)
        assert columns == 2 and abs(sum(ratios)-1) < .001 and abs(ratios[0]-.5) > .03
        resized_rules = table_rules('resize-finished')
        assert abs((resized_rules[1]-before_rules[1])/2 + 48) <= 1, 'Visible divider did not follow the drag'
        run('drag-cancel', divider_x-48, y+299, divider_x-8, y+299)
        assert snap('resize-cancelled')['AXValue'] == expected
        assert width_files[0].read_bytes() == width_bytes, 'Cancelled resize committed metadata'
        cancelled_rules = table_rules('resize-cancelled')
        assert abs(cancelled_rules[1]-resized_rules[1]) <= 1, 'Escape did not restore the visible divider'
        run('click', x+285, y+299)
        resized_offset = snap('resized-click')['AXSelectedTextRange']['location']
        assert u16index(expected,'one') <= resized_offset <= u16index(expected,'one') + 3
        key(7)
        assert snap('resized-edit')['AXValue'] == insert(expected, resized_offset, 'x')
        undo(); assert snap('resized-undo')['AXValue'] == expected
        check('real column drag persists proportions, Escape cancels, subsequent click/edit/undo stays aligned')

        old_height = snap('zoom-before')['AXSize']['height']
        key(24, 'cmd+shift'); key(24, 'cmd+shift')
        time.sleep(.6)
        zoomed = snap('zoom-in')
        assert zoomed['AXValue'] == expected and zoomed['AXSize']['height'] > old_height * 1.05
        zoom_rules = table_rules('zoom-in', top=568, bottom=840)
        run('drag', x+290, y+351, x+560, y+402, 'alt+shift')
        zoom_copy = json.loads(run('copy-read'))
        assert zoom_copy['text'].replace('\r\n','\n').strip() == 'one\ttwo\nthree\tfour'
        run('copy-paste', x+310, y+523)
        assert '> | one | two |\r\n> | three | four |' in snap('zoom-grid-paste')['AXValue']
        undo(); assert snap('zoom-grid-undo')['AXValue'] == expected
        zoom_divider = x + zoom_rules[1]/2
        run('drag', zoom_divider, y+350, zoom_divider-30, y+350)
        zoom_resized = table_rules('zoom-resized', top=568, bottom=840)
        assert abs((zoom_resized[1]-zoom_rules[1])/2+30) <= 1
        run('click', x+290, y+350)
        zoom_offset = snap('zoom-edit-caret')['AXSelectedTextRange']['location']
        assert u16index(expected, 'one') <= zoom_offset <= u16index(expected, 'one')+3
        key(7)
        assert snap('zoom-edit')['AXValue'] == insert(expected, zoom_offset, 'x')
        undo(); assert snap('zoom-edit-undo')['AXValue'] == expected
        check('125% zoom: real grid drag/copy/paste, column drag and immediate editing remain aligned')
        key(29, 'cmd'); time.sleep(.5)
        assert abs(snap('zoom-reset')['AXSize']['height'] - old_height) < 2
        check('native zoom shortcuts reflow the shared geometry and reset without editing source')

        run('resize', 620, 480); time.sleep(.5)
        key(125, 'cmd')  # document end reveals a wrapped paragraph after scrolling
        time.sleep(.5)
        scrolled = snap('narrow-scrolled')
        narrow = next(w['kCGWindowBounds'] for w in scrolled['windows'] if w['kCGWindowLayer']==0 and w['kCGWindowBounds']['Width']>=400)
        point = (narrow['X']+280, narrow['Y']+narrow['Height']-170)
        run('click', *point)
        clicked = snap('narrow-click')['AXSelectedTextRange']['location']
        assert clicked > len(expected.encode('utf-16-le'))//4, 'Click did not reach the scrolled document'
        run('source', PINYIN); type_pinyin()
        candidate = capture('narrow-candidate')
        panels = [w['kCGWindowBounds'] for w in candidate['windows'] if w['kCGWindowLayer']>0]
        assert panels and any(abs(b['Y']-point[1])<90 and abs(b['X']-point[0])<180 for b in panels), 'Candidate panel detached from clicked text'
        key(49)
        assert snap('narrow-commit')['AXValue'] == insert(expected, clicked, '中文')
        undo(); assert snap('narrow-undo')['AXValue'] == expected
        run('resize', 1200, 800); time.sleep(.5)
        run('keys',45,34,4,0,31)
        wide_capture = capture('wide-candidate')
        assert any(w['kCGWindowLayer']==20 for w in wide_capture['windows']), 'Wide-window candidate missing'
        key(53)
        assert snap('wide-cancel')['AXValue'] == expected
        check('narrow-window wrapping, scrolling, click-to-Pinyin candidate geometry and wide-window cancellation')

        key(1, 'cmd'); time.sleep(.4)
        assert fixture.read_bytes() == b'\xef\xbb\xbf' + expected.encode()
        result['final_source_sha256'] = digest(fixture)
        check('exact BOM/CRLF source save and unchanged document regions')
        key(12, 'cmd'); process.wait(timeout=10)
        reopen_log = (out / 'reopen.log').open('w')
        process = subprocess.Popen(command, env=env, stdout=reopen_log, stderr=subprocess.STDOUT, start_new_session=True)
        deadline = time.monotonic()+12
        while True:
            if process.poll() is not None: raise RuntimeError('Reopened app exited before readiness')
            try:
                reopened = json.loads(run('activate'))
                break
            except RuntimeError as error:
                if time.monotonic() >= deadline or not any(reason in str(error) for reason in
                    ['Usage: native-event-driver', 'No native document AXTextArea found', 'Target app did not become frontmost']): raise
                time.sleep(.3)
        (out/'reopened.json').write_text(json.dumps(reopened,ensure_ascii=False,indent=2))
        assert reopened['AXValue'] == expected
        assert fixture.read_bytes() == b'\xef\xbb\xbf' + expected.encode()
        check('new process reopens exactly the saved source with BOM/CRLF untouched')
        result['passed'] = True
    except Exception as error:
        result['error'] = str(error)
        raise
    finally:
        (out / 'results.json').write_text(json.dumps(result, ensure_ascii=False, indent=2))
        if process is not None and process.poll() is None and not args.inspect and not (args.keep_open_on_failure and not result['passed']):
            try:
                if original_input: run('source', original_input)
                key(1, 'cmd'); key(12, 'cmd')
                process.wait(timeout=5)
            except Exception:
                process.kill(); process.wait(timeout=5)
        assert digest(production / 'Contents/MacOS/Yu') == build['app_sha256'], 'Production changed during tests'
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
