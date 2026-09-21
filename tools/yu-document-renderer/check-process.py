#!/usr/bin/env python3
"""Exercise the real helper transport, isolation and 60-second idle lifecycle."""
import hashlib
import json
import selectors
import platform
import subprocess
import sys
import time
from pathlib import Path

binary = Path(sys.argv[1]).resolve()
out = Path(sys.argv[2]).resolve()
out.mkdir(parents=True, exist_ok=False)
result = {'system': platform.platform(), 'architecture': platform.machine(), 'binary': str(binary), 'passed': False, 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'checks': []}
probe = Path(sys.argv[3]).resolve() if len(sys.argv) > 3 else None
result['memory_metric'] = 'proc_pid_rusage RUSAGE_INFO_V4 physical footprint' if probe else None
result['memory_samples'] = []
app_binary = binary.parent.parent/'MacOS'/'Yu'
if app_binary.is_file():
    result['app_sha256'] = hashlib.sha256(app_binary.read_bytes()).hexdigest()
def sample(pid):
    if probe:
        result['memory_samples'].append(json.loads(subprocess.check_output([str(probe),str(pid)],text=True)))
process = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
try:
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    cases = [
        ('math', r'\begin{pmatrix}a&b\\c&d\end{pmatrix}', True),
        ('mermaid', 'flowchart LR\nA[Start]-->B[Done]', True),
        ('math', r'\notARealYuCommand{x}', False),
        ('math', r'\frac{\text{收益}}{\text{成本}}', True),
        ('mermaid', 'flowchart LR\nA-->B\ngarbage ???', False),
        ('mermaid', 'sequenceDiagram\nA->>B: Hi\nunknown instruction', False),
        ('mermaid', 'classDiagram\nclass A\nunknown ???', False),
        ('mermaid', 'flowchart LR; A-->B; click A call external()', False),
        ('mermaid', 'flowchart LR\nA[unclosed', False),
        ('mermaid', '---\ntitle: Example\n---\npie\n"A" : nope', False),
        ('mermaid', '---\ntitle: Example\n---\npie\n"A" : 10', True),
    ]
    for job, (kind, source, succeeds) in enumerate(cases, start=1):
        request = {'id': job, 'document': 71, 'revision': 42, 'kind': kind, 'source': source}
        process.stdin.write(json.dumps(request).encode() + b'\n')
        process.stdin.flush()
        assert selector.select(20), 'Helper did not return a bounded response'
        response = json.loads(process.stdout.readline())
        assert (response['id'], response['document'], response['revision']) == (job, 71, 42)
        if succeeds:
            assert response['status'] == 'ready'
            (out / f'{kind}-{job}.svg').write_text(response['vector']['svg'])
            assert response['vector']['width'] > 0 and response['vector']['height'] > 0
        else:
            assert response['status'] == 'failed' and response['diagnostic']
            assert 'vector' not in response
        sample(process.pid)
        result['checks'].append(f'{kind} job {job}: tagged result and expected success/diagnostic')
    idle_start = time.monotonic()
    deadline = idle_start + 65
    while process.poll() is None and time.monotonic() < deadline:
        try:
            sample(process.pid)
        except subprocess.CalledProcessError:
            if process.poll() is None:
                raise
            break
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            pass
    assert process.poll() == 0, 'Helper did not exit successfully at idle deadline'
    if probe:
        absent = subprocess.run([str(probe),str(process.pid)],capture_output=True,text=True)
        assert absent.returncode != 0, 'Exited helper still has a live footprint record'
        result['post_exit_probe'] = {'returncode': absent.returncode, 'diagnostic': absent.stderr.strip()}
        result['peak_physical_footprint_bytes'] = max(s['lifetime_peak_physical_footprint_bytes'] for s in result['memory_samples'])
    elapsed = time.monotonic() - idle_start
    assert 58 <= elapsed <= 65, f'Unexpected idle shutdown: {elapsed}'
    result['idle_exit_seconds'] = elapsed
    result['checks'].append('real helper exits after 60 idle seconds with stdin still open')
    cancel = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        cancel.stdin.write(json.dumps({'id': 100, 'document': 71, 'revision': 43, 'kind': 'math', 'source': 'x+' * 16000 + 'x'}).encode() + b'\n')
        cancel.stdin.flush()
        cancel.terminate()
        cancel.wait(timeout=3)
        assert cancel.returncode != 0
        result['checks'].append('disposable worker process can be terminated for cancellation')
    finally:
        if cancel.poll() is None:
            cancel.kill(); cancel.wait()
    result['passed'] = True
finally:
    if process.poll() is None:
        process.kill(); process.wait()
    (out/'results.json').write_text(json.dumps(result, indent=2) + '\n')
