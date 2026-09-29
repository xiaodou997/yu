#!/usr/bin/env python3
"""Build, sign, and package Yu for the Mac App Store.

This channel is independent of the Developer ID / notarized DMG pipeline.
It never uploads a build or changes the Git repository's tags.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
SHELL = ROOT / 'platform/macos/yu-shell-macos'
BUNDLE_ID = 'io.github.xiaodou997.yu'
HELPER_ID = BUNDLE_ID + '.renderer'
ENTITLEMENTS = SHELL / 'AppBundle/AppStore.entitlements'
HELPER_ENTITLEMENTS = SHELL / 'AppBundle/AppStoreHelper.entitlements'


def output(*args):
    return subprocess.check_output([str(a) for a in args], cwd=ROOT,
                                   stderr=subprocess.STDOUT, text=True).strip()


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def inventory(app):
    files = {}
    for path in sorted(app.rglob('*')):
        if path.is_symlink():
            raise ValueError(f'Unexpected app bundle symlink: {path}')
        if path.is_file():
            files[str(path.relative_to(app))] = digest(path)
    return files


class StoreRelease:
    def __init__(self, directory):
        self.directory = directory.resolve()
        self.app = self.directory / 'Yu.app'
        self.pkg = self.directory / 'Yu-macOS-AppStore.pkg'
        self.manifest = self.directory / 'appstore-manifest.json'
        self.data = {}

    def save(self):
        temporary = self.manifest.with_suffix('.tmp')
        temporary.write_text(json.dumps(self.data, indent=2, ensure_ascii=False) + '\n')
        temporary.replace(self.manifest)

    def load(self, expected):
        self.data = json.loads(self.manifest.read_text())
        if self.data['stage'] != expected:
            raise ValueError(f"Expected stage {expected}, found {self.data['stage']}")
        if inventory(self.app) != self.data['app_files']:
            raise ValueError('App changed since preceding stage; start a new build')

    def run(self, name, *args, env=None):
        log = self.directory / f'{name}.log'
        with log.open('wb') as stream:
            result = subprocess.run([str(a) for a in args], cwd=ROOT,
                                    env=env, stdout=stream, stderr=subprocess.STDOUT)
        self.data.setdefault('commands', []).append({
            'name': name, 'exit_code': result.returncode,
            'log': log.name, 'sha256': digest(log),
        })
        self.save()
        if result.returncode:
            raise RuntimeError(f'{name} failed ({result.returncode}); see {log}')

    def record_app(self):
        self.data['app_files'] = inventory(self.app)
        self.data['yu_sha256'] = digest(self.app / 'Contents/MacOS/Yu')
        self.data['helper_sha256'] = digest(self.app / 'Contents/Helpers/yu-document-renderer')
        self.save()

    def build(self, candidate):
        status = output('git', 'status', '--porcelain', '--untracked-files=normal')
        if status and not candidate:
            raise ValueError('Official App Store builds require a clean checkout')
        self.directory.mkdir(parents=True, exist_ok=False)
        self.data = {
            'schema_version': 1,
            'channel': 'Mac App Store',
            'stage': 'building',
            'candidate': candidate,
            'git_commit': output('git', 'rev-parse', 'HEAD'),
            'git_status': status,
            'pipeline_sha256': digest(Path(__file__)),
            'created_at': datetime.now(timezone.utc).isoformat(),
            'bundle_id': BUNDLE_ID,
            'app_store_connect_id': '6817272771',
        }
        self.save()
        env = dict(os.environ, YU_APP_OUTPUT=str(self.app),
                   YU_BUILD_MANIFEST=str(self.directory / 'artifact-audit.json'))
        self.run('build', SHELL / 'build-app.sh', '--release', env=env)
        info = plistlib.loads((self.app / 'Contents/Info.plist').read_bytes())
        if info['CFBundleIdentifier'] != BUNDLE_ID or info['LSMinimumSystemVersion'] != '26.0':
            raise ValueError('Unexpected bundle identity or minimum macOS version')
        if info.get('LSApplicationCategoryType') != 'public.app-category.productivity':
            raise ValueError('Mac App Store category must match Productivity')
        if not re.fullmatch(r'\d+(?:\.\d+){1,2}', info['CFBundleShortVersionString']):
            raise ValueError('Invalid App Store version string')
        if not re.fullmatch(r'[1-9]\d*', info['CFBundleVersion']):
            raise ValueError('Invalid App Store build number')
        self.data.update(stage='built', version=info['CFBundleShortVersionString'],
                         build_number=info['CFBundleVersion'], minimum_os='26.0',
                         architecture=output('lipo', '-archs', self.app / 'Contents/MacOS/Yu'),
                         swift=output('swift', '--version'), rustc=output('rustc', '--version'))
        if self.data['architecture'] != 'arm64':
            raise ValueError('Yu App Store build must be arm64 only')
        self.record_app()

    @staticmethod
    def verify_code(target, expected_id, expected_entitlements, team):
        details = output('codesign', '-d', '--verbose=4', target)
        if f'Identifier={expected_id}' not in details.splitlines():
            raise ValueError(f'Wrong code identifier: {target}')
        if f'TeamIdentifier={team}' not in details.splitlines():
            raise ValueError(f'Wrong team: {target}')
        if not re.search(r'^Authority=Apple Distribution: .+ \(' + re.escape(team) + r'\)$', details, re.M):
            raise ValueError(f'Wrong App Store signing identity: {target}')
        if not re.search(r'^Timestamp=.+', details, re.M):
            raise ValueError(f'Secure timestamp missing: {target}')
        actual = subprocess.check_output(['codesign', '-d', '--entitlements', ':-', str(target)],
                                         stderr=subprocess.DEVNULL)
        expected = plistlib.loads(expected_entitlements.read_bytes())
        if plistlib.loads(actual) != expected:
            raise ValueError(f'Unexpected entitlements: {target}')
        return details

    def sign(self, identity, team):
        self.load('built')
        if not re.fullmatch(r'[A-Z0-9]{10}', team):
            raise ValueError('Expected 10-character Apple Team ID')
        self.data.update(stage='signing', app_signing_identity=identity, team_id=team)
        self.save()
        helper = self.app / 'Contents/Helpers/yu-document-renderer'
        self.run('sign-helper', 'codesign', '--force', '--sign', identity,
                 '--identifier', HELPER_ID, '--options', 'runtime', '--timestamp',
                 '--entitlements', HELPER_ENTITLEMENTS, helper)
        self.run('sign-app', 'codesign', '--force', '--sign', identity,
                 '--options', 'runtime', '--timestamp', '--entitlements', ENTITLEMENTS, self.app)
        self.run('verify-bundle', 'codesign', '--verify', '--deep', '--strict', self.app)
        self.data['signature_details'] = {
            'helper': self.verify_code(helper, HELPER_ID, HELPER_ENTITLEMENTS, team),
            'app': self.verify_code(self.app, BUNDLE_ID, ENTITLEMENTS, team),
        }
        self.data['stage'] = 'signed'
        self.record_app()

    def package(self, identity):
        self.data = json.loads(self.manifest.read_text())
        stage = self.data['stage']
        if stage not in {'signed', 'packaging'}:
            raise ValueError(f'Expected stage signed or packaging, found {stage}')
        self.load(stage)
        if stage == 'signed':
            self.data.update(stage='packaging', installer_signing_identity=identity)
            self.save()
            self.run('package', 'productbuild', '--component', self.app, '/Applications',
                     '--sign', identity, '--timestamp', self.pkg)
        elif self.data.get('installer_signing_identity') != identity or not self.pkg.is_file():
            raise ValueError('Cannot resume packaging with a different identity or missing package')
        self.run('verify-package', 'pkgutil', '--check-signature', self.pkg)
        details = (self.directory / 'verify-package.log').read_text()
        trusted_statuses = (
            'Status: signed by a certificate trusted by Mac OS X',
            'Status: signed by a developer certificate issued by Apple (Development)',
        )
        if not any(status in details for status in trusted_statuses):
            raise ValueError('Installer signature was not Apple-trusted')
        if 'Signed with a trusted timestamp on:' not in details:
            raise ValueError('Installer signature has no trusted timestamp')
        if 'Apple Root CA' not in details:
            raise ValueError('Installer signature has no Apple root')
        if self.data['team_id'] not in details:
            raise ValueError('Installer signature belongs to the wrong team')
        self.data.update(stage='packaged', package_sha256=digest(self.pkg),
                         package_signature=details)
        self.save()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage', choices=['build', 'sign', 'package'])
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--candidate', action='store_true')
    parser.add_argument('--identity')
    parser.add_argument('--team-id')
    args = parser.parse_args()
    release = StoreRelease(args.output)
    if args.stage == 'build':
        release.build(args.candidate)
    elif args.stage == 'sign':
        if not args.identity or not args.team_id:
            parser.error('sign requires --identity and --team-id')
        release.sign(args.identity, args.team_id)
    else:
        if not args.identity:
            parser.error('package requires --identity')
        release.package(args.identity)
    print(release.manifest)


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f'App Store release failed: {error}', file=sys.stderr)
        sys.exit(1)
