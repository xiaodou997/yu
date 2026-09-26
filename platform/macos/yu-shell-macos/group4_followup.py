"""Focused, external-event Group 4 checks. Never call Yu's internal commands.

Used by run-embedded-checks.py so the isolated bundle, clipboard restoration,
foreground safety, exact saved bytes and complete quit/reopen remain shared.
"""
import json
from pathlib import Path
import time


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
        actual = self.text()
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
