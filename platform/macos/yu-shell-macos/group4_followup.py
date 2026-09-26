"""Focused, external-event Group 4 checks. Never call Yu's internal commands.

Used by run-embedded-checks.py so the isolated bundle, clipboard restoration,
foreground safety, exact saved bytes and complete quit/reopen remain shared.
"""
import json
import re
import shutil
import subprocess
from pathlib import Path
import time


def option_error(options):
    """Reject conflicting follow-up modes before any file or desktop effects."""
    seconds = options.get('stress_seconds', 0)
    if seconds and not 30 <= seconds <= 1800:
        return '--stress-seconds must be between 30 and 1800'
    groups = [('table_interactions', 'table_resize'), ('smoke_document',), ('stress_seconds',), ('list_gestures',)]
    active = [group for group in groups if any(options.get(key) for key in group)]
    if not active:
        return None  # Preserve existing independent suites.
    if len(active) > 1:
        return 'Choose one follow-up suite (table-interactions and table-resize may combine)'
    allowed = {'output', 'dark', 'reopen', 'resource_audit', *active[0]}
    if any(value for key, value in options.items() if key not in allowed):
        return 'Follow-up suites combine only with --dark and --reopen'
    return None


def resource_records(text):
    """Ignore other log lines and a trailing incomplete write, not bad counters."""
    records = []
    for line in text.splitlines(keepends=True):
        if line.startswith('yu-resource-audit ') and line.endswith('\n'):
            record = json.loads(line[len('yu-resource-audit '):])
            if not isinstance(record, dict):
                raise ValueError('Resource audit record must be an object')
            records.append(record)
    return records


def utf16(text):
    return len(text.encode('utf-16-le')) // 2


def center(rect):
    return rect['x'] + rect['width'] / 2, rect['y'] + rect['height'] / 2


class Checks:
    def __init__(self, run, stable_bounds, out, fixture, result):
        self.run, self.bounds = run, stable_bounds
        self.out, self.fixture, self.result = out, fixture, result

    def text(self):
        return self.run('snapshot')['AXValue']

    def expect(self, text):
        # Native menu/clipboard dispatch can settle after the input driver
        # returns. Observe only; never resend a mutation or relax exact bytes.
        deadline = time.monotonic()+3
        actual = self.text()
        polls = 0
        while actual != text and time.monotonic() < deadline:
            time.sleep(.05)
            polls += 1
            actual = self.text()
        if polls:
            self.result.setdefault('source_observation_waits', []).append(polls)
        if actual != text:
            (self.out/'mismatch-expected.txt').write_bytes(text.encode())
            (self.out/'mismatch-actual.txt').write_bytes(actual.encode())
            raise AssertionError('Exact document source mismatch; evidence retained')

    def install(self, text):
        self.run('select', 0, utf16(self.text()))
        self.run('paste-text', text)
        self.expect(text)
        self.run('select', 0, 0)
        time.sleep(.3)

    def point(self, text, label):
        if text.count(label) != 1:
            raise AssertionError('A source label must have one identity: '+label)
        return center(self.bounds(utf16(text[:text.index(label)]), utf16(label)))

    def capture(self, name):
        self.run('capture', str(self.out/name))

    def save(self, text):
        self.run('key', 1, 'cmd')
        deadline = time.monotonic()+3
        expected = b'\xef\xbb\xbf'+text.encode()
        while self.fixture.read_bytes() != expected:
            if time.monotonic() >= deadline:
                raise AssertionError('Saved BOM/CRLF bytes differ')
            time.sleep(.05)

    def record(self, message):
        self.result['checks'].append(message)

    def table_interactions(self, root):
        fixtures = root/'crates/yu-editor/tests/fixtures/group4-paste'
        target = (fixtures/'row-groups-whole-target.md').read_text()
        valid = (fixtures/'row-groups-rows-payload.md').read_text().strip()
        invalid = (fixtures/'merged-payload.md').read_text().strip()
        for reverse in (False, True):
            for accept in (True, False):
                donor = valid if accept else invalid
                labels = ['新表头中文🙂', '新表体中文🙂'] if accept else ['传入中文🙂']
                base = ('DONOR\n\n'+donor+'\n\n'+target+'\n\n$y^2$\n\nTAIL-END\n').replace('\n','\r\n')
                self.install(base)
                self.run('select',utf16(base),0); self.run('paste-text',' HISTORY-A')
                before = base+' HISTORY-A'
                self.run('select',0,0); self.run('select',utf16(before),0)
                self.run('paste-text',' HISTORY-B')
                self.run('key',6,'cmd'); self.expect(before)
                self.run('select',0,0)
                # All points come from the source-backed live AX projection.
                first = self.point(before,labels[0])
                last = self.point(before,labels[-1])
                start = self.point(before,'目标中文🙂')
                end = self.point(before,'矩形终点🙂')
                self.run('click',*first)
                first = self.point(before,labels[0]); last = self.point(before,labels[-1])
                self.run('drag',*first,*last,'alt+shift')
                target_points = (*end,*start) if reverse else (*start,*end)
                copied = self.run('copy-paste',*target_points)
                payload = json.loads(copied['source_fragments'])
                assert payload['version'] == 2 and payload['columns'] == 2
                assert all(label in payload['tableSource'] for label in labels)
                # The UI must actually select both owners, not paste at a caret.
                selected = copied['target_selected_ranges']
                assert len(selected) == 2, selected
                for label in ['目标中文🙂','矩形终点🙂']:
                    at = utf16(before[:before.index(label)])
                    assert any(r['location'] <= at < r['location']+r['length'] for r in selected), selected
                after = self.text()
                name = 'whole-{}-{}'.format('reverse' if reverse else 'forward','accept' if accept else 'reject')
                if accept:
                    assert after != before
                    assert after.startswith(('DONOR\n\n'+donor+'\n\n').replace('\n','\r\n'))
                    assert after.count('<table') == 2
                    for token in ["<thead id='head'>", "<tbody id='empty'></tbody>", "<tbody id='body'>"]:
                        assert after.count(token) == 1, token
                    for label in labels:
                        assert after.count(label) == 2
                    assert '目标中文🙂' not in after and '矩形终点🙂' not in after
                    self.run('key',6,'cmd'); self.expect(before)
                    assert self.run('snapshot')['AXSelectedTextRanges'] == selected
                    self.run('key',6,'cmd+shift'); self.expect(after)
                else:
                    self.expect(before)
                    snapshot = self.run('snapshot')
                    assert snapshot.get('focused_description') == '警告', snapshot.get('focused_description')
                    self.capture(name+'-alert'); self.run('key',36)
                    self.expect(before)
                    assert self.run('snapshot')['AXSelectedTextRanges'] == selected
                    # Replay both old branches, not only one redundant undo.
                    for modifiers, expected in [('cmd+shift',before+' HISTORY-B'),('cmd',before),('cmd',base),('cmd+shift',before),('cmd+shift',before+' HISTORY-B')]:
                        self.run('key',6,modifiers); self.expect(expected)
                    self.run('key',6,'cmd'); self.expect(before)
                    after = before
                self.run('select',0,0); self.capture(name); self.save(after)
                self.record(name+': actual native donor copy and whole-target Option+Shift drag; exact history/ranges, groups, empty group and saved bytes')
        return after, None

    def divider(self):
        # Distinct source-backed splitter identities, not guessed pixel borders.
        deadline = time.monotonic()+5
        previous = None
        while time.monotonic() < deadline:
            snapshot = self.run('snapshot')
            items = snapshot.get('splitters') or [item for item in snapshot['children'] if item.get('AXRole') == 'AXSplitter']
            if items:
                items.sort(key=lambda item:item['AXPosition']['x'])
                item = items[0]
                key = (item['AXPosition']['x'],item['AXPosition']['y'],item['AXSize']['height'])
                if key == previous:
                    return item, snapshot
                previous = key
            time.sleep(.1)
        raise AssertionError('Visible table splitter not observed')

    def merged_resize(self):
        source = ('# Resize merged table\n\n'
                  '<table><tr><th colspan="2">Joined header</th><th>Right</th></tr>'
                  '<tr><td rowspan="2">Rowspan 中文🙂</td><td>Cell A</td><td>Cell B</td></tr>'
                  '<tr><td>Cell C</td><td>Cell D</td></tr></table>\n\n$x^2$\n\nTAIL\n').replace('\n','\r\n')
        self.install(source)
        item, snapshot = self.divider()
        start_x = item['AXPosition']['x'] + item['AXSize']['width']/2
        cell = self.bounds(utf16(source[:source.index('Cell A')]),6)
        y = cell['y']+cell['height']/2
        self.run('drag',start_x,y,start_x+55,y)
        after, state = self.divider()
        after_x = after['AXPosition']['x']+after['AXSize']['width']/2
        assert abs(after_x-start_x-55) <= 2, (start_x,after_x)
        self.expect(source)
        self.capture('merged-column-drag')
        self.run('drag-cancel',after_x,y,after_x-35,y)
        cancelled, _ = self.divider()
        cancelled_x = cancelled['AXPosition']['x']+cancelled['AXSize']['width']/2
        assert abs(cancelled_x-after_x) <= 1, (cancelled_x,after_x)
        self.expect(source); self.save(source)
        self.capture('merged-column-cancel')
        relative_x = after_x-state['AXPosition']['x']
        self.result['merged_column_resize'] = {'before_x':start_x,'after_x':after_x,'relative_x':relative_x,'delta':55,'cancel_restored':True}
        self.record('merged column: actual mouse drag changes geometry by 55pt, Escape restores preview; source and exact saved bytes do not change')
        def reopened():
            # A reopened window may use the default width. Compare geometry at
            # the same viewport; persisted column proportions reflow normally.
            self.run('resize',1200,800)
            self.run('select',0,0)
            time.sleep(.4)
            divider, snapshot = self.divider()
            x = divider['AXPosition']['x']+divider['AXSize']['width']/2-snapshot['AXPosition']['x']
            assert abs(x-relative_x) <= 1, (x,relative_x)
            self.result['merged_column_resize']['reopened_relative_x'] = x
            self.record('merged column presentation width survives full application quit/reopen without source rewriting')
        return source, reopened

    def smoke_document(self, root):
        original = root/'platform/macos/yu-shell-macos/Fixtures/group4-smoke.md'
        source = original.read_bytes().decode('utf-8').replace('\r\n','\n').replace('\n','\r\n')
        assets = self.fixture.parent/'assets'
        assets.mkdir(exist_ok=True)
        shutil.copyfile(original.parent/'assets/yu-mark.png', assets/'yu-mark.png')
        self.install(source)
        marker = utf16(source[:source.index('[TOC]')])
        # The fixed sample's short, unwrapped headings occupy one row each.
        # Measure the generated block through its canonical marker, never OCR.
        self.run('select',marker-2,0)
        rect = self.bounds(marker,5)
        headings = list(re.finditer(r'^#{1,6} (.+)\r?$',source,re.M))
        self.result['smoke_toc_geometry'] = {'rect':rect,'headings':len(headings)}
        self.capture('smoke-toc')
        self.run('click',rect['x']+40,rect['y']+rect['height']/len(headings)*1.5,'cmd')
        target = utf16(source[:headings[1].start(1)])
        state = self.run('snapshot')
        assert state['AXValue'] == source and state['AXSelectedTextRange']['location'] == target, (state['AXSelectedTextRange'],target,rect)
        self.record('fixed smoke: real Command-click in generated TOC reaches canonical second heading without source edits')
        self.capture('smoke-toc-jump')
        # Native find brings a deeply off-screen summary into view, then the
        # actual disclosure pointer must change only the opening details tag.
        summary = '展开查看内容'
        at = utf16(source[:source.index(summary)])
        self.run('select',at,0)
        rect = self.bounds(at,utf16(summary))
        self.capture('smoke-details-closed')
        self.run('click',rect['x']-10,rect['y']+rect['height']/2)
        opened = source.replace('<details>','<details open>')
        self.expect(opened); self.capture('smoke-details-open')
        self.run('key',6,'cmd'); self.expect(source)
        self.run('key',6,'cmd+shift'); self.expect(opened)
        self.run('key',6,'cmd'); self.expect(source)
        self.record('fixed smoke: actual disclosure click opens body; one undo/redo changes only the details open attribute')
        self.run('key',3,'cmd'); self.run('paste-text','折叠容器的正文。'); self.run('key',36)
        found = self.run('snapshot')
        assert found['AXSelectedText'] == '折叠容器的正文。' and found['AXValue'] == source
        self.run('key',53); self.capture('smoke-hidden-find')
        self.run('paste-text','折叠正文已编辑🙂。')
        changed = source.replace('折叠容器的正文。','折叠正文已编辑🙂。')
        self.expect(changed); self.save(changed)
        self.run('key',6,'cmd'); self.expect(source)
        self.run('key',6,'cmd+shift'); self.expect(changed)
        self.run('key',6,'cmd'); self.expect(source)
        self.record('fixed smoke: real Find reveals closed body without rewriting source; body edit, exact history and save preserve all other content')
        # Capture each heading viewport rather than infer off-screen geometry.
        for i, heading in enumerate(headings):
            self.run('select',utf16(source[:heading.start(1)]),0)
            time.sleep(.35)
            self.capture('smoke-section-{:02}'.format(i))
        self.expect(source); self.save(source)
        return source

    def resource_stress(self, root, app_pid, helpers, duration):
        probe = self.out/'process-footprint'
        subprocess.run(['clang','-Wall','-Wextra','-Werror',str(root/'tools/process-footprint.c'),'-o',str(probe)],check=True)
        def footprint(pid):
            return json.loads(subprocess.check_output([str(probe),str(pid)],text=True))
        started = time.monotonic()
        samples, pids, rounds = [], set(), 0
        stats = {'requested_seconds':duration,'rounds':0,'samples':samples,'metric':'RUSAGE_INFO_V4 physical footprint','scope':'single document; repeated valid/error/plain source, not multi-document or overnight stress'}
        self.result['resource_stress'] = stats
        audit = self.result.get('resource_audit', False)
        def plain_residency(plain, after_revision):
            deadline = time.monotonic()+10
            while time.monotonic() < deadline:
                records = resource_records((self.out/'app.log').read_text())
                if records:
                    record = records[-1]
                    if record['revision'] > after_revision and record['source_bytes'] == len(plain.encode()):
                        assert record['embedded_gpu_textures'] == 0, record
                        assert record['embedded_gpu_rgba_bytes'] == 0, record
                        return record
                time.sleep(.1)
            raise AssertionError('Current plain frame did not publish resource counters')

        while time.monotonic()-started < duration:
            rounds += 1
            valid = ('# Stress {}\r\n\r\n$x_{}^2$\r\n\r\n```mermaid\r\nflowchart LR\r\nA[轮次 {}] --> B[Done]\r\n```\r\n\r\nTAIL\r\n').format(rounds,rounds,rounds)
            self.install(valid)
            deadline = time.monotonic()+15
            while not helpers():
                assert time.monotonic() < deadline, 'No helper for current valid document'
                time.sleep(.1)
            live = helpers(); assert len(live) == 1, live; pids.update(live)
            bad = valid.replace('flowchart LR','yuUnsupportedGraph')
            self.install(bad)
            self.run('key',6,'cmd'); self.expect(valid)
            self.run('key',6,'cmd+shift'); self.expect(bad)
            plain = '# Plain {}\r\n\r\n中文🙂\r\n'.format(rounds)
            before_plain = set(helpers())
            records_before = resource_records((self.out/'app.log').read_text()) if audit else []
            revision_before = records_before[-1]['revision'] if records_before else -1
            self.install(plain)
            # A completed helper may legitimately remain until its idle timer.
            # Plain text must not accumulate processes or start a new one.
            self.expect(plain)
            time.sleep(.15)
            remaining = helpers()
            assert len(remaining) <= 1 and set(remaining) <= before_plain, remaining
            samples.append({'seconds':time.monotonic()-started,'round':rounds,'app':footprint(app_pid),'helpers_after_plain':len(remaining)})
            if audit:
                samples[-1]['residency'] = plain_residency(plain, revision_before)
            stats['rounds'] = rounds
            (self.out/'stress-progress.json').write_text(json.dumps(stats,indent=2)+'\n')
        final = '# Stress restored\r\n\r\n$x^2$\r\n\r\nTAIL\r\n'
        self.install(final); self.save(final); self.capture('stress-restored')
        stats.update(elapsed_seconds=time.monotonic()-started,helper_pids=sorted(pids),min_app_bytes=min(s['app']['physical_footprint_bytes'] for s in samples),max_app_bytes=max(s['app']['physical_footprint_bytes'] for s in samples))
        self.record('resource stress: {} completed valid/error/plain cycles in {:.1f}s; exact source/history, one helper while rendering and no new process for plain text; measured memory, no leak-proof claim'.format(rounds,stats['elapsed_seconds']))
        return final

    def list_gestures(self):
        parent = "<li id='parent'>父项中文<ul><li>子项🙂</li><li>后子</li></ul></li>"
        body = '<ul><li>前项</li>'+parent+'<li>尾项</li></ul>'
        indented = '<ul><li>前项<ul>'+parent+'</ul></li><li>尾项</li></ul>'
        for in_cell in (False, True):
            for multicaret in (False, True):
                def wrap(content):
                    if in_cell:
                        content = '<table><tr><td>'+content+'</td><td>NEIGHBOR</td></tr></table>'
                    return '# List gestures\r\n\r\n'+content+'\r\n\r\n$x^2$\r\n\r\nTAIL'
                base = wrap(body)
                self.install(base)
                self.run('select',utf16(base),0); self.run('paste-text',' HISTORY-A')
                before = base+' HISTORY-A'
                self.run('select',0,0)
                # A whole parent label's AX range may span a hidden boundary
                # and two visual rows. Hit a single visible CJK glyph instead
                # of the empty midpoint of that multi-line rectangle.
                def glyph(label):
                    return center(self.bounds(utf16(before[:before.index(label)]),1))
                child = glyph('子项🙂'); parent_point = glyph('父项中文')
                self.capture('list-before-{}-{}'.format(in_cell,multicaret))
                if multicaret:
                    self.run('click',*parent_point)
                    child = glyph('子项🙂')
                    self.run('click',*child,'alt')
                else:
                    self.run('click',*child)
                    self.capture('list-child-active-{}'.format(in_cell))
                    child = glyph('子项🙂'); parent_point = glyph('父项中文')
                    self.run('drag',*child,*parent_point)
                selected = self.run('snapshot')['AXSelectedTextRanges']
                self.capture('list-selection-{}-{}'.format(in_cell,multicaret))
                self.expect(before)
                if multicaret:
                    assert len(selected) == 2 and all(s['length'] == 0 for s in selected), selected
                else:
                    assert len(selected) == 1 and selected[0]['length'] > 0, selected
                self.run('key',30,'cmd')
                expected = wrap(indented)+' HISTORY-A'
                self.expect(expected)
                self.run('key',6,'cmd'); self.expect(before)
                assert self.run('snapshot')['AXSelectedTextRanges'] == selected
                self.run('key',6,'cmd+shift'); self.expect(expected)
                self.run('select',0,0); self.save(expected)
                name = 'list-{}-{}'.format('cell' if in_cell else 'standalone','multi' if multicaret else 'reverse')
                self.capture(name)
                self.record(name+': actual pointer range or two Option-click carets move parent subtree once; exact undo/ranges, redo, neighbor and saved bytes')
        return expected
