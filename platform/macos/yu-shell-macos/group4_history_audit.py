"""Read bounded, typed history receipt records. Never infer delivery from silence."""
import math
from collections import Counter
from group4_resource_soak import unique_object, reject_constant
import json

PREFIX = 'yu-history-audit '
ENTRIES = {'keyEquivalent', 'keyDown', 'textSystem', 'menuSelector', 'direct'}
COMMON = {'schema', 'pid', 'phase', 'ticket', 'uptime'}
STATE = {'window', 'firstResponder', 'editable', 'composition', 'available', 'revision'}


def integer(value, minimum=0):
    if type(value) is not int or not minimum <= value <= 2**64 - 1:
        raise ValueError('Invalid history integer')


def timestamp(value):
    if type(value) not in (int, float) or not math.isfinite(value) or value < 0:
        raise ValueError('Invalid history timestamp')


def summarize_history(lines, expected_pids):
    expected = set(expected_pids)
    if not expected or len(expected) != len(expected_pids):
        raise ValueError('History requires distinct expected process identities')
    for pid in expected:
        integer(pid, 1)
    pending, last_ticket, last_time = {}, {}, {}
    entries, commands, outcomes = Counter(), Counter(), Counter()
    total = 0
    for line in lines:
        if not line.startswith(PREFIX):
            continue
        if len(line) > 2048 or not line.endswith('\n'):
            raise ValueError('Incomplete or oversized history record')
        record = json.loads(line[len(PREFIX):], object_pairs_hook=unique_object, parse_constant=reject_constant)
        if not isinstance(record, dict) or type(record.get('schema')) is not int or record['schema'] != 1:
            raise ValueError('Invalid history record schema')
        pid, ticket = record.get('pid'), record.get('ticket')
        integer(pid, 1); integer(ticket, 1); timestamp(record.get('uptime'))
        if pid not in expected or ticket > 4096:
            raise ValueError('Unexpected history process or exhausted budget')
        if record['uptime'] < last_time.get(pid, 0):
            raise ValueError('History process clock moved backwards')
        last_time[pid] = record['uptime']
        key = (pid, ticket)
        phase = record.get('phase')
        if phase == 'truncated':
            raise ValueError('History receipt budget exhausted; diagnostic coverage incomplete')
        if phase == 'begin':
            allowed = COMMON | {'entry', 'command', 'eventTimestamp', 'state'}
            if set(record) - allowed or not (allowed - {'eventTimestamp'}) <= set(record):
                raise ValueError('Unexpected history fields')
            if record['entry'] not in ENTRIES or type(record['command']) is not int or record['command'] not in (8, 9):
                raise ValueError('Invalid history entry or command')
            state = record['state']
            if not isinstance(state, dict) or set(state) != STATE:
                raise ValueError('Invalid history state')
            integer(state['revision']); integer(state['window'])
            for name in STATE - {'revision', 'window'}:
                if type(state[name]) is not bool:
                    raise ValueError('Invalid history state boolean')
            if 'eventTimestamp' in record:
                timestamp(record['eventTimestamp'])
            if ticket != last_ticket.get(pid, 0) + 1 or len(pending) >= 64:
                raise ValueError('Nonsequential or unbounded history receipts')
            last_ticket[pid] = ticket
            pending[key] = record
        elif phase == 'end':
            if set(record) != COMMON | {'revisionAfter', 'handled'} or type(record.get('handled')) is not bool:
                raise ValueError('Invalid history completion')
            integer(record['revisionAfter'])
            begin = pending.pop(key, None)
            if begin is None or record['revisionAfter'] < begin['state']['revision']:
                raise ValueError('Unmatched or backwards history completion')
            total += 1
            entries[begin['entry']] += 1
            commands[str(begin['command'])] += 1
            outcome = 'handled' if record['handled'] else 'declined'
            if record['handled'] and record['revisionAfter'] == begin['state']['revision']:
                outcome = 'handled_without_revision_change'
            outcomes[outcome] += 1
        else:
            raise ValueError('Unknown history record phase')
    if pending or set(last_ticket) != expected or total == 0:
        raise ValueError('Missing or incomplete history receipt evidence')
    return {'complete': True, 'pairs': total, 'processes': sorted(expected), 'entries': dict(entries),
            'commands': dict(commands), 'outcomes': dict(outcomes),
            'scope': 'Received history routes only; absence does not prove an event was never delivered'}
