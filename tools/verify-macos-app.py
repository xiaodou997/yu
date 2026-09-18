#!/usr/bin/env python3
"""Audit the shipped artifact, rather than trusting build command exit status."""
import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import re
import subprocess


def run(*command):
    return subprocess.check_output(command, text=True, stderr=subprocess.STDOUT).strip()


def audit(app, configuration):
    binary = app / 'Contents/MacOS/Yu'
    shader = app / 'Contents/Resources/yu_shaders.metallib'
    info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
    archs = run('lipo', '-archs', str(binary)).split()
    if archs != ['arm64']:
        raise ValueError(f'Expected arm64 only, found {archs}')
    version = run('xcrun', 'vtool', '-show-build', str(binary))
    if not re.search(r'\bminos 26\.0(?:\.0)?\b', version):
        raise ValueError(f'Incorrect minimum OS: {version}')
    if not re.search(r'\bsdk 27\.', version):
        raise ValueError(f'Expected SDK 27: {version}')
    if info.get('LSMinimumSystemVersion') != '26.0':
        raise ValueError('Info.plist minimum OS must be 26.0')
    if 'UIDesignRequiresCompatibility' in info:
        raise ValueError('Legacy design compatibility must be removed')
    if not shader.read_bytes().startswith(b'MTLB'):
        raise ValueError('Compiled Metal library is missing or invalid')
    dependencies = run('otool', '-L', str(binary))
    if re.search(r'WebKit|Chromium|JavaScriptCore|libnode', dependencies, re.I):
        raise ValueError('Browser runtime dependency found')
    for file in app.rglob('*'):
        if file.is_file() and file.name.lower() in {'node', 'node.exe', 'chromium'}:
            raise ValueError(f'Unexpected runtime {file}')
        if file.is_file():
            kind = run('file', '-b', str(file))
            if 'Mach-O' in kind and run('lipo', '-archs', str(file)).split() != ['arm64']:
                raise ValueError(f'Non-arm64 bundled Mach-O: {file}')
    run('codesign', '--verify', '--deep', '--strict', str(app))
    return {
        'app': str(app), 'app_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
        'shader_sha256': hashlib.sha256(shader.read_bytes()).hexdigest(),
        'configuration': configuration, 'architectures': archs,
        'minimum_os': '26.0', 'build_version': version,
        'xcode': run('xcodebuild', '-version'), 'sdk': run('xcrun', '--sdk', 'macosx', '--show-sdk-version'),
        'host_os': run('sw_vers', '-productVersion'), 'dependencies': dependencies,
        'artifact_audit_passed': True, 'runtime_26_verified': False,
        'runtime_27_verified': False, 'visual_acceptance': False,
        'real_input_acceptance': False, 'performance_acceptance': False,
    }


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('app', type=Path)
    parser.add_argument('--configuration', choices=['debug', 'release'], required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = audit(args.app.resolve(), args.configuration)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(f"Yu artifact audit passed: {result['app_sha256']}")
