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


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def tree_hashes(root):
    return {
        str(path.relative_to(root)): sha256(path)
        for path in root.rglob('*')
        if path.is_file()
    }


def audit(app, configuration):
    binary = app / 'Contents/MacOS/Yu'
    helper = app / 'Contents/Helpers/yu-document-renderer'
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
    helper_version = run('xcrun', 'vtool', '-show-build', str(helper))
    if not re.search(r'\bminos 26\.0(?:\.0)?\b', helper_version):
        raise ValueError('Incorrect helper minimum OS')
    helper_dependencies = run('otool', '-L', str(helper))
    if re.search(r'WebKit|Chromium|JavaScriptCore|libnode', helper_dependencies, re.I):
        raise ValueError('Browser dependency in native helper')

    repo_root = Path(__file__).resolve().parent.parent
    source_resources = repo_root / 'platform/macos/yu-shell-macos/AppBundle/Resources'
    shipped_resources = app / 'Contents/Resources'
    for resource_tree in ['Fonts', 'HTMLExportLicenses', 'PDFExportLicenses']:
        expected = tree_hashes(source_resources / resource_tree)
        actual = tree_hashes(shipped_resources / resource_tree)
        if actual != expected:
            raise ValueError(f'{resource_tree} resources differ from audited source tree')

    font_hashes = {
        'OpenSans-Regular.ttf': 'c53aceea2dcf5b4098099c0c4d0a061d17e178a049317b42a422b1a9f7f8eb59',
        'OpenSans-Italic.ttf': '93bc1bb6abf4e6b7c75d7131714061d5b57cc478abcabe4cb3519bb38fb917aa',
        'OpenSans-SemiBold.ttf': '4a413711684a9dd564ef0f1c10cb62b5d9f7eb6df2cff962f5341a6ecd5f64ae',
        'OpenSans-SemiBoldItalic.ttf': 'c646a9aefc0e964f66487260639e375685328a6280856faea3e012c43fe9c56d',
        'OpenSans-Bold.ttf': '27da758f4dcac9a65abe914c13b463b42982b9909bc65713424099f4810bd1e6',
        'OpenSans-BoldItalic.ttf': 'd672a770037104b6af45e1336b3d3c1729c8aea940f81e010f5a8a7319c29a21',
    }
    for name, expected_hash in font_hashes.items():
        if sha256(shipped_resources / 'Fonts' / name) != expected_hash:
            raise ValueError(f'Missing or modified bundled font: {name}')

    native_licenses = shipped_resources / 'NativeRendererLicenses'
    renderer_source = repo_root / 'tools/yu-document-renderer'
    expected_renderer_notices = {
        path.name: sha256(path)
        for path in (renderer_source / 'licenses').glob('*.txt')
    }
    expected_renderer_notices['MiTeX.txt'] = sha256(renderer_source / 'vendor/mitex/LICENSE')
    expected_renderer_notices['xarrow.txt'] = sha256(renderer_source / 'vendor/xarrow/LICENSE')
    for name, expected_hash in expected_renderer_notices.items():
        shipped = native_licenses / name
        if not shipped.is_file() or sha256(shipped) != expected_hash:
            raise ValueError(f'Missing or modified native renderer notice: {name}')

    dependency_manifest_path = native_licenses / 'RustDependencies.json'
    dependency_bundle_path = native_licenses / 'RustDependencyLicenses.txt'
    if not dependency_manifest_path.is_file() or not dependency_bundle_path.is_file():
        raise ValueError('Native renderer locked dependency notices are missing')
    dependency_manifest = json.loads(dependency_manifest_path.read_text(encoding='utf-8'))
    if dependency_manifest.get('root_package') != 'yu-document-renderer':
        raise ValueError('Native renderer dependency manifest has the wrong root')
    if dependency_manifest.get('target') != 'aarch64-apple-darwin':
        raise ValueError('Native renderer dependency manifest has the wrong target')
    if dependency_manifest.get('cargo_lock_sha256') != sha256(repo_root / 'Cargo.lock'):
        raise ValueError('Native renderer dependency manifest is stale')
    packages = dependency_manifest.get('packages', [])
    if not packages or len(packages) != dependency_manifest.get('external_package_count'):
        raise ValueError('Native renderer dependency manifest package count is invalid')
    if any(not package.get('license') for package in packages):
        raise ValueError('Native renderer dependency manifest has an unlicensed package')
    typst_assets = [
        package for package in packages
        if package.get('name') == 'typst-assets' and package.get('version') == '0.15.1'
    ]
    if len(typst_assets) != 1:
        raise ValueError('Expected locked typst-assets 0.15.1 in renderer closure')
    bundle = dependency_manifest.get('license_text_bundle', {})
    if bundle.get('file') != dependency_bundle_path.name or sha256(dependency_bundle_path) != bundle.get('sha256'):
        raise ValueError('Native renderer dependency license text bundle is stale')
    if sha256(native_licenses / 'TypstAssets-NOTICE.txt') != '80ac46fb7d70f1c30bf0c14d3408e6a6f0f9cc9997a00b592aa0d2c8a5243a9d':
        raise ValueError('typst-assets 0.15.1 NOTICE is missing or modified')
    if sha256(native_licenses / 'TypstAssets-Apache-2.0.txt') != '62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a':
        raise ValueError('typst-assets 0.15.1 license is missing or modified')

    expected_native_names = set(expected_renderer_notices) | {
        dependency_manifest_path.name, dependency_bundle_path.name
    }
    actual_native_names = {
        str(path.relative_to(native_licenses))
        for path in native_licenses.rglob('*') if path.is_file()
    }
    if actual_native_names != expected_native_names:
        raise ValueError('Native renderer license directory contains stale or missing files')

    cjk_license = app / 'Contents/Resources/NativeRendererLicenses/NotoSerifCJK-OFL.txt'
    if not cjk_license.is_file() or sha256(cjk_license) != '6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2':
        raise ValueError('Missing or modified bundled Noto Serif CJK license')
    run('codesign', '--verify', '--strict', str(helper))
    run('codesign', '--verify', '--deep', '--strict', str(app))
    return {
        'app': str(app), 'app_sha256': sha256(binary),
        'shader_sha256': sha256(shader),
        'helper_sha256': sha256(helper),
        'helper_build_version': helper_version, 'helper_dependencies': helper_dependencies,
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
