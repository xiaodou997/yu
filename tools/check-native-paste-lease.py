#!/usr/bin/env python3
"""Real delayed AppKit clipboard-consumption regression, not a Yu acceptance run."""
from pathlib import Path
import argparse
import hashlib
import json
import plistlib
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    if sys.platform != 'darwin':
        parser.error('Requires an unlocked macOS desktop')
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    driver = out / 'native-event-driver'
    app = out / 'DelayedPaste.app'
    binary = app / 'Contents/MacOS/DelayedPaste'
    binary.parent.mkdir(parents=True)
    identifier = 'io.github.xiaodou997.yu.editing-check.' + uuid.uuid4().hex
    (app / 'Contents/Info.plist').write_bytes(plistlib.dumps({
        'CFBundleIdentifier': identifier, 'CFBundleExecutable': 'DelayedPaste',
        'CFBundleName': 'DelayedPaste', 'CFBundlePackageType': 'APPL', 'NSPrincipalClass': 'NSApplication'}))
    sources = [ROOT/'tools/native-event-driver.swift', ROOT/'tools/fixtures/NativeDelayedPaste.swift']
    subprocess.run(['swiftc', str(sources[0]), '-o', str(driver)], check=True, timeout=180)
    subprocess.run(['swiftc', str(sources[1]), '-o', str(binary)], check=True, timeout=180)
    subprocess.run(['codesign', '--force', '--sign', '-', str(app)], check=True, timeout=30)
    subprocess.run([str(driver), '--preflight'], check=True, timeout=20)
    result = {'passed': False, 'scope': 'Synthetic driver regression, not product acceptance',
              'source_hashes': {str(p.relative_to(ROOT)): sha(p) for p in sources}, 'cases': []}
    try:
        for action, expected in [('paste-text', 'PRIOR_PAYLOAD'), ('paste-document', 'AFTER')]:
            case = out/action
            case.mkdir()
            with (case/'app.log').open('w') as log:
                child = subprocess.Popen([str(binary), str(case)], stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                try:
                    def run(*arguments):
                        response = subprocess.run([str(driver), str(child.pid), *map(str,arguments)],
                            capture_output=True, text=True, timeout=20)
                        if response.returncode:
                            raise AssertionError(response.stderr)
                        return json.loads(response.stdout) if response.stdout.strip().startswith('{') else None
                    deadline = time.monotonic() + 10
                    while True:
                        try:
                            state = run('snapshot')
                            if state.get('AXValue') == 'BEFORE':
                                break
                        except AssertionError:
                            pass
                        if child.poll() is not None or time.monotonic() >= deadline:
                            raise AssertionError('Fixture did not expose its editor')
                        time.sleep(0.1)
                    run('select',0,6)
                    receipt = run(action,'AFTER')
                    deadline = time.monotonic()+5
                    while not (case/'receipt.json').exists():
                        if time.monotonic() >= deadline:
                            raise AssertionError('Delayed paste did not execute')
                        time.sleep(0.05)
                    consumer = json.loads((case/'receipt.json').read_text())
                    assert consumer == {'pastes':1,'result':expected}, consumer
                    assert run('snapshot')['AXValue'] == expected
                    if action == 'paste-document':
                        assert receipt['clipboard_held_until_expected_source'] is True
                    result['cases'].append({'action':action,'expected_observed':True,
                        'paste_invocations':consumer['pastes'],'consumer_result':expected,'driver_receipt':receipt})
                finally:
                    # Own fixture restores the original clipboard before exiting;
                    # cleanup uses a local stop file, not keys into another app.
                    (case/'stop').touch()
                    try:
                        assert child.wait(timeout=10) == 0
                    except subprocess.TimeoutExpired:
                        child.terminate()
                        child.wait(timeout=5)
                        raise AssertionError('Fixture did not restore and exit normally')
        result['passed'] = True
    finally:
        (out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result,indent=2))


if __name__ == '__main__':
    main()
