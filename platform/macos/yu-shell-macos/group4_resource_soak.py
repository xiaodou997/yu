"""Bounded, platform-independent accounting for the native Group 4 soak.

Physical footprint and logical resource counters are deliberately separate.
Neither their difference nor a finite plateau is a leak verdict.
"""
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path

PREFIX = b'yu-resource-audit '
COLD_SOURCE = '# Stress restored\r\n\r\n$x^2$\r\n\r\nTAIL\r\n'
COUNTERS = ('revision', 'source_bytes', 'embedded_gpu_textures',
            'embedded_gpu_rgba_bytes', 'embedded_failures', 'embedded_cache_entries',
            'undo_entries', 'redo_entries', 'gpu_evictions')
STAGES = ('plain', 'valid', 'error')


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def digest(path):
    result = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda: stream.read(65536), b''):
            result.update(chunk)
    return result.hexdigest()


def text_identity(text):
    data = text.encode('utf-8')
    return {'source_bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}


def exact_bytes(text):
    return b'\xef\xbb\xbf' + text.encode('utf-8')


@dataclass(frozen=True)
class Options:
    documents: int = 4
    seconds: int = 600
    idle_seconds: int = 70
    sample_seconds: int = 5

    def validate(self):
        for name, lower, upper in (('documents', 2, 12), ('seconds', 30, 86400),
                                   ('idle_seconds', 70, 3600), ('sample_seconds', 1, 60)):
            value = getattr(self, name)
            if type(value) is not int or not lower <= value <= upper:
                raise ValueError(f'{name} must be an integer between {lower} and {upper}')
        return self


def fixture_source(document, generation, stage):
    """Disjoint byte-length slots identify this controlled corpus, not user docs.

    Length alone is NOT a general document identity. The native runner also
    binds an AX window, checks the entire source and requires a newer revision.
    Padding is after the two visible resources; it never pushes them off screen.
    """
    if type(document) is not int or not 0 <= document < 12:
        raise ValueError('document index must be 0..11')
    if type(generation) is not int or not 0 <= generation <= 999999:
        raise ValueError('generation must be 0..999999')
    if stage not in STAGES:
        raise ValueError('unknown fixture stage')
    source = f'# Soak document {document:02} generation {generation:06}\r\n\r\n'
    if stage != 'plain':
        diagram = 'flowchart LR' if stage == 'valid' else 'yuUnsupportedGraph'
        source += (f'$x_{{{document},{generation}}}^2$\r\n\r\n```mermaid\r\n{diagram}\r\n'
                   f'A[Document {document} round {generation}] --> B[Done]\r\n```\r\n\r\n')
    source += 'TAIL 中文🙂\r\n\r\n'
    target = (document + 1) * 4096 + STAGES.index(stage) * 512
    padding = target - len(source.encode('utf-8')) - 2
    require(padding >= 0, 'Fixture exceeded its identity slot')
    source += 'p' * padding + '\r\n'
    require(len(source.encode('utf-8')) == target, 'Fixture length drift')
    return source


def reject_constant(value):
    raise ValueError('Nonfinite JSON number: ' + value)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('Duplicate JSON field: ' + key)
        result[key] = value
    return result


def validate_record(record):
    if not isinstance(record, dict):
        raise ValueError('Resource audit record must be an object')
    for name in COUNTERS:
        value = record.get(name)
        if type(value) is not int or value < 0:
            raise ValueError('Missing or invalid resource counter: ' + name)
    return record


class AuditTail:
    """Read each log byte once; retain at most one bounded partial line.

    A partial final write is not a record. Completed bad audit JSON fails
    closed. Rotation/truncation fails rather than reusing old document frames.
    A per-poll byte budget bounds memory even when the app produces a backlog.
    """
    def __init__(self, path, *, start=0, read_bytes=262144, max_line_bytes=1048576):
        if min(read_bytes, max_line_bytes) <= 0 or start < 0:
            raise ValueError('Invalid audit cursor bounds')
        self.path = Path(path)
        stat = self.path.stat()
        if start > stat.st_size:
            raise ValueError('Audit cursor is beyond EOF')
        self.identity = (stat.st_dev, stat.st_ino)
        self.offset = start
        self.pending = b''
        self.read_bytes = read_bytes
        self.max_line_bytes = max_line_bytes
        self.records = 0

    def poll(self):
        with self.path.open('rb') as stream:
            stat = os.fstat(stream.fileno())
            if (stat.st_dev, stat.st_ino) != self.identity or stat.st_size < self.offset:
                raise ValueError('Audit log rotated or truncated during observation')
            stream.seek(self.offset)
            chunk = stream.read(self.read_bytes)
        self.offset += len(chunk)
        parts = (self.pending + chunk).split(b'\n')
        self.pending = parts.pop()
        if len(self.pending) > self.max_line_bytes:
            raise ValueError('Audit log line exceeds bounded reader capacity')
        result = []
        for line in parts:
            if len(line) > self.max_line_bytes:
                raise ValueError('Audit log line exceeds bounded reader capacity')
            if not line.startswith(PREFIX):
                continue
            record = json.loads(line[len(PREFIX):].decode('utf-8'),
                                parse_constant=reject_constant, object_pairs_hook=unique_object)
            result.append(validate_record(record))
            self.records += 1
        return result

    def drain(self, max_bytes=8388608):
        """Validate all existing output before changing document ownership.

        Fail on a perpetually growing backlog instead of silently seeking past
        records, hiding malformed output, or accepting old reopen frames.
        """
        start = self.offset
        while True:
            self.poll()
            if self.caught_up:
                require(not self.pending, 'Incomplete audit write at ownership boundary')
                return self.offset
            require(self.offset - start < max_bytes, 'Audit backlog exceeded ownership-boundary budget')

    @property
    def caught_up(self):
        return self.offset == self.path.stat().st_size


def matches_frame(record, text, after_revision=-1):
    validate_record(record)
    return (record['source_bytes'] == len(text.encode('utf-8'))
            and record['revision'] > after_revision)


def settled(record, stage):
    validate_record(record)
    require(record['embedded_cache_entries'] <= 32, 'Embedded cache exceeded its existing 32-entry bound')
    if stage == 'plain':
        return (record['embedded_gpu_textures'] == 0
                and record['embedded_gpu_rgba_bytes'] == 0
                and record['embedded_failures'] == 0)
    if stage == 'valid':
        return record['embedded_gpu_textures'] >= 2 and record['embedded_failures'] == 0
    if stage == 'error':
        return record['embedded_gpu_textures'] >= 1 and record['embedded_failures'] >= 1
    if stage == 'cold':
        return record['embedded_gpu_textures'] >= 1 and record['embedded_failures'] == 0
    raise ValueError('Unknown observation stage')


class FootprintSummary:
    """Constant-space endpoint/range statistics; never add logical GPU bytes."""
    def __init__(self):
        self.phases = {}

    def add(self, session, phase, seconds, footprint):
        if not isinstance(session, str) or not session or not isinstance(phase, str) or not phase:
            raise ValueError('Named process session and phase are required')
        if not isinstance(footprint, dict):
            raise ValueError('Footprint sample must be an object')
        value, pid = footprint.get('physical_footprint_bytes'), footprint.get('pid')
        if type(value) is not int or value < 0 or type(pid) is not int or pid <= 0:
            raise ValueError('Invalid process footprint sample')
        import math
        if type(seconds) not in (int, float) or not math.isfinite(seconds) or seconds < 0:
            raise ValueError('Invalid monotonic sample time')
        key = (session, phase)
        if key not in self.phases:
            self.phases[key] = {'session': session, 'phase': phase, 'pid': pid, 'samples': 0,
                                'first_seconds': seconds, 'last_seconds': seconds,
                                'first_bytes': value, 'last_bytes': value,
                                'min_bytes': value, 'max_bytes': value}
        item = self.phases[key]
        require(item['pid'] == pid, 'Process changed inside a footprint phase')
        require(seconds >= item['last_seconds'], 'Monotonic sample time went backwards')
        item.update(samples=item['samples'] + 1, last_seconds=seconds, last_bytes=value,
                    min_bytes=min(item['min_bytes'], value), max_bytes=max(item['max_bytes'], value))

    def report(self):
        return {'metric': 'RUSAGE_INFO_V4 physical footprint bytes',
                'interpretation': 'Finite observations only; no leak or long-term stability verdict. '
                                  'Logical history/cache/GPU counters are not additive physical footprint.',
                'phases': [dict(item, delta_bytes=item['last_bytes'] - item['first_bytes'])
                           for item in self.phases.values()]}


def process_rows(text, app_binary, helper_binary):
    """Match full isolated executable paths, never another Yu installation."""
    found = {'app': [], 'helpers': []}
    for line in text.splitlines():
        fields = line.strip().split(None, 1)
        if len(fields) != 2 or not fields[0].isdigit():
            continue
        for key, path in (('app', str(app_binary)), ('helpers', str(helper_binary))):
            if fields[1] == path or fields[1].startswith(path + ' '):
                found[key].append(int(fields[0]))
    return found
