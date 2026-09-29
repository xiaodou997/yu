#!/usr/bin/env python3
"""Local macOS release stages; CI must call this same entry point.

No credentials are accepted on the command line except a Keychain profile name.
Every stage preserves logs and refuses modified artifacts from previous stages.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys
import uuid

ROOT = Path(__file__).resolve().parent.parent
SHELL = ROOT / 'platform/macos/yu-shell-macos'
BUNDLE_ID = 'io.github.xiaodou997.yu'
REQUIRED_ACCEPTANCE = ('macos_27', 'external_ax', 'local_install',
                       'software_recovery', 'upgrade')


def checked_acceptance(evidence, manifest):
    for key in ('git_commit', 'version', 'build_number', 'dmg_sha256'):
        if evidence.get(key) != manifest.get(key):
            raise ValueError(f'Acceptance evidence has wrong {key}')
    checks = evidence.get('checks')
    if not isinstance(checks, dict):
        raise ValueError('Acceptance evidence must contain checks')
    for key in REQUIRED_ACCEPTANCE:
        item = checks.get(key)
        if not isinstance(item, dict) or item.get('status') != 'passed' \
                or not isinstance(item.get('evidence'), str) or not item['evidence'].strip():
            raise ValueError(f'Acceptance check is missing or not passed: {key}')
    return {key: checks[key] for key in REQUIRED_ACCEPTANCE}


def sha(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def inventory(app):
    result = {}
    for path in sorted(app.rglob('*')):
        if path.is_symlink():
            raise ValueError(f'Unexpected bundle symlink: {path}')
        if path.is_file():
            result[str(path.relative_to(app))] = {
                'sha256': sha(path), 'mode': path.stat().st_mode & 0o777,
            }
    return result


def dmg_capacity_mib(staging):
    # hdiutil's automatic estimate can under-size the intermediate filesystem.
    # Count logical bytes (including sparse files), then reserve metadata/slack.
    logical = sum(path.stat().st_size for path in staging.rglob('*')
                  if not path.is_symlink() and path.is_file())
    mib = 1024 * 1024
    return max(256, (logical + logical // 4 + 32 * mib + mib - 1) // mib)


def capture(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True, stderr=subprocess.STDOUT).strip()


def source_state():
    return {
        'git_commit': capture('git', 'rev-parse', 'HEAD'),
        'git_status': capture('git', 'status', '--porcelain', '--untracked-files=normal'),
        'diff_sha256': hashlib.sha256(subprocess.check_output(
            ['git', 'diff', '--binary', 'HEAD'], cwd=ROOT)).hexdigest(),
        'pipeline_sha256': sha(Path(__file__)),
    }


def gatekeeper_evidence(policy, app_result, dmg_result):
    enforced = policy.strip() == 'assessments enabled'
    bypass = 'override=security disabled' in app_result + dmg_result
    return {
        'system_policy': policy.strip(),
        'enforced_assessment': enforced and not bypass,
        'default_policy_launch_acceptance': 'pending',
    }


def gatekeeper_policy():
    # --status returns 1 for disabled assessments, which is evidence, not an
    # execution error. Unknown output must never be interpreted as enabled.
    result = subprocess.run(['spctl', '--status'], text=True, capture_output=True)
    return (result.stdout + result.stderr).strip()


def signature_details(target, team, bundle=False):
    capture('codesign', '--verify', '--strict', *(['--deep'] if bundle else []), str(target))
    details = capture('codesign', '-d', '--verbose=4', str(target))
    if f'TeamIdentifier={team}' not in details.splitlines():
        raise ValueError(f'Wrong signing team: {target}')
    if not re.search(r'^Authority=Developer ID Application: .+ \(' + re.escape(team) + r'\)$', details, re.M):
        raise ValueError(f'Not a Developer ID Application signature: {target}')
    if not re.search(r'^CodeDirectory .*flags=.*\bruntime\b', details, re.M):
        raise ValueError(f'Hardened Runtime missing: {target}')
    if not re.search(r'^Timestamp=.+', details, re.M):
        raise ValueError(f'Secure timestamp missing: {target}')
    ent = subprocess.check_output(['codesign', '-d', '--entitlements', ':-', str(target)],
                                  stderr=subprocess.DEVNULL)
    if ent.strip() and plistlib.loads(ent):
        raise ValueError(f'Release currently allows no entitlements: {target}')
    return details


class Release:
    def __init__(self, output):
        self.output = output.resolve()
        self.app = self.output / 'Yu.app'
        self.manifest_path = self.output / 'release-manifest.json'
        self.data = {}

    def save(self):
        temporary = self.manifest_path.with_suffix('.tmp')
        temporary.write_text(json.dumps(self.data, indent=2, ensure_ascii=False) + '\n')
        temporary.replace(self.manifest_path)

    def run(self, name, *args, env=None, check=True):
        print(f'[{name}]', flush=True)
        log = self.output / f'{name}.log'
        with log.open('wb') as stream:
            result = subprocess.run([str(arg) for arg in args], cwd=ROOT, env=env,
                                    stdout=stream, stderr=subprocess.STDOUT)
        self.data.setdefault('commands', []).append({
            'name': name, 'exit_code': result.returncode, 'log': log.name,
            'log_sha256': sha(log),
        })
        self.save()
        if result.returncode and check:
            raise RuntimeError(f'{name} failed ({result.returncode}); see {log}')
        return result.returncode

    def record_app(self):
        self.data['bundle_files'] = inventory(self.app)
        for label, relative in [('yu', 'Contents/MacOS/Yu'),
                                ('helper', 'Contents/Helpers/yu-document-renderer'),
                                ('metal', 'Contents/Resources/yu_shaders.metallib')]:
            self.data[f'{label}_sha256'] = sha(self.app / relative)
        self.save()

    def load(self, expected):
        self.data = json.loads(self.manifest_path.read_text())
        if self.data['stage'] != expected:
            raise ValueError(f"Expected {expected}, found {self.data['stage']}")
        if inventory(self.app) != self.data['bundle_files']:
            raise ValueError('Bundle changed after preceding stage; rebuild in a new directory')

    def build(self, candidate):
        state = source_state()
        if state['git_status'] and not candidate:
            raise ValueError('Official builds require a clean checkout; use --candidate for local development')
        # Never overwrite a release or reuse an old bundle containing stale resources.
        self.output.mkdir(parents=True, exist_ok=False)
        self.data = dict(state, schema_version=1, release_id=str(uuid.uuid4()),
                         created_at=datetime.now(timezone.utc).isoformat(),
                         candidate=candidate, stage='building',
                         channel='Developer ID / notarized DMG / GitHub Releases',
                         notarization={}, acceptance={
                             **{key: {'status': 'pending'} for key in REQUIRED_ACCEPTANCE},
                             'os_logout_reboot': {'status': 'excluded-by-user'},
                             'voiceover_speech': {'status': 'not-tested'},
                             'default_gatekeeper_launch': {'status': 'not-tested'},
                             'clean_user_install': {'status': 'not-tested'},
                             'ci_runner': {'status': 'deferred'},
                         })
        self.save()
        env = dict(os.environ, YU_APP_OUTPUT=str(self.app),
                   YU_BUILD_MANIFEST=str(self.output / 'artifact-audit.json'))
        self.run('release-gates', sys.executable, '-m', 'unittest', 'discover',
                 '-s', 'tools', '-p', 'test_release_macos.py', env=env)
        self.run('rust-fmt', 'cargo', 'fmt', '--all', '--check', env=env)
        self.run('rust-clippy', 'cargo', 'clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings', env=env)
        self.run('rust-tests', 'cargo', 'test', '--workspace', '--locked', env=env)
        self.run('build', SHELL / 'build-app.sh', '--release', env=env)
        info = plistlib.loads((self.app / 'Contents/Info.plist').read_bytes())
        if info['CFBundleIdentifier'] != BUNDLE_ID or info['LSMinimumSystemVersion'] != '26.0':
            raise ValueError('Release bundle identity or deployment target changed')
        if not re.fullmatch(r'\d+\.\d+\.\d+', info['CFBundleShortVersionString']):
            raise ValueError('Release version must be major.minor.patch')
        if not re.fullmatch(r'[1-9]\d*', info['CFBundleVersion']):
            raise ValueError('Build number must be a positive integer')
        icon = self.app / 'Contents/Resources' / info['CFBundleIconFile']
        if icon.read_bytes()[:4] != b'icns' or not info.get('NSHumanReadableCopyright'):
            raise ValueError('Release icon or copyright is missing')
        if sha(self.app / 'Contents/Resources/Yu-LICENSE.txt') != sha(ROOT / 'LICENSE'):
            raise ValueError('Application license missing or changed')
        executables = []
        for file in self.app.rglob('*'):
            if file.is_file() and 'Mach-O' in capture('file', '-b', str(file)):
                executables.append(str(file.relative_to(self.app)))
        if sorted(executables) != ['Contents/Helpers/yu-document-renderer', 'Contents/MacOS/Yu']:
            raise ValueError(f'Unreviewed nested code: {executables}')
        self.data.update(bundle_id=BUNDLE_ID, version=info['CFBundleShortVersionString'],
                         build_number=info['CFBundleVersion'], minimum_os='26.0', architecture='arm64',
                         validation_host=capture('sw_vers'),
                         rustc=capture('rustc', '--version'), swift=capture('swift', '--version'))
        self.run('native-checks', SHELL / 'run-self-checks.sh',
                 env=dict(env, YU_SELF_CHECK_BINARY=str(self.app / 'Contents/MacOS/Yu')))
        if source_state() != state:
            raise ValueError('Source changed during build; rebuild from a stable checkout')
        self.data['stage'] = 'built'
        self.record_app()

    def verify_signatures(self):
        team = self.data['team_id']
        details = {}
        for name, target, bundle in [('helper', self.app / 'Contents/Helpers/yu-document-renderer', False),
                                     ('app', self.app, True)]:
            details[name] = signature_details(target, team, bundle)
        self.data['signature_details'] = details
        self.save()

    def sign(self, identity, team):
        self.load('built')
        if not re.fullmatch(r'[A-Z0-9]{10}', team):
            raise ValueError('Expected a 10-character Team ID')
        self.data.update(stage='signing', signing_identity=identity, team_id=team,
                         entitlements={})
        self.save()
        for label, path in [('helper', self.app / 'Contents/Helpers/yu-document-renderer'), ('app', self.app)]:
            self.run(f'sign-{label}', 'codesign', '--force', '--sign', identity,
                     '--options', 'runtime', '--timestamp', path)
        self.verify_signatures()
        self.run('signed-native-checks', SHELL / 'run-self-checks.sh',
                 env=dict(os.environ, YU_SELF_CHECK_BINARY=str(self.app / 'Contents/MacOS/Yu')))
        self.data['stage'] = 'signed'
        self.record_app()

    def notarize_file(self, label, path, profile):
        # A submission ID is persisted before waiting, so a network interruption
        # can be recovered with notarytool info/log without reuploading.
        submission = self.output / f'{label}-submission.json'
        self.run(f'{label}-submit', 'xcrun', 'notarytool', 'submit', path,
                 '--keychain-profile', profile, '--output-format', 'json')
        submission.write_bytes((self.output / f'{label}-submit.log').read_bytes())
        result = json.loads(submission.read_text())
        identifier = result['id']
        self.data['notarization'][label] = dict(submission_id=identifier, status='Submitted',
                                               submitted_sha256=sha(path))
        self.save()
        wait_code = self.run(f'{label}-wait', 'xcrun', 'notarytool', 'wait', identifier,
                            '--keychain-profile', profile, '--output-format', 'json',
                            '--timeout', '30m', check=False)
        # Invalid submissions can return a nonzero code; still retrieve Apple's log.
        try:
            result = json.loads((self.output / f'{label}-wait.log').read_text())
        except json.JSONDecodeError:
            result = {'status': 'Unknown'}
        status = result.get('status', 'Unknown')
        self.data['notarization'][label]['status'] = status
        self.save()
        self.run(f'{label}-notary-log', 'xcrun', 'notarytool', 'log', identifier,
                 '--keychain-profile', profile, self.output / f'{label}-apple-log.json', check=False)
        if status != 'Accepted' or wait_code:
            raise ValueError(f'{label} notarization is {status}; inspect saved logs and submission ID')

    def package(self, preview=False, attempt_prefix=''):
        prefix = attempt_prefix + ('preview-' if preview else '')
        staging = self.output / f'{prefix}dmg-root'
        staging.mkdir()
        self.run(f'{prefix}copy-to-dmg', 'ditto', self.app, staging / 'Yu.app')
        (staging / 'Applications').symlink_to('/Applications')
        (staging / '安装说明.txt').write_text(
            '将 Yu.app 拖入 Applications 后启动。升级前退出 Yu，再替换应用。\n'
            '支持 Apple Silicon，最低系统 macOS 26；本版实机验收使用 macOS 27。\n'
            '删除应用不会删除你的文档；请勿清理尚未恢复的恢复副本。\n')
        suffix = '-UNNOTARIZED' if preview else ''
        dmg = self.output / f"Yu-{self.data['version']}-{self.data['build_number']}-arm64{suffix}.dmg"
        if dmg.exists():
            raise ValueError('DMG already exists; preserve it and use a new release directory')
        capacity = dmg_capacity_mib(staging)
        self.data['dmg_capacity_mib'] = capacity
        self.save()
        self.run(f'{prefix}dmg-create', 'hdiutil', 'create', '-volname', 'Yu', '-srcfolder', staging,
                 '-fs', 'HFS+', '-size', f'{capacity}m', '-format', 'UDZO', dmg)
        self.run(f'{prefix}dmg-sign', 'codesign', '--sign', self.data['signing_identity'], '--timestamp', dmg)
        self.run(f'{prefix}dmg-verify', 'codesign', '--verify', '--strict', dmg)
        if preview:
            self.data['unnotarized_preview'] = {'file': dmg.name, 'sha256': sha(dmg)}
        else:
            self.data.update(dmg=dmg.name, dmg_sha256=sha(dmg))
        self.save()
        return dmg

    def distribute(self, profile):
        self.load('signed')
        self.verify_signatures()
        # Check credentials before modifying the signed app or submitting software.
        self.run('notary-auth', 'xcrun', 'notarytool', 'history', '--keychain-profile', profile,
                 '--output-format', 'json')
        self.data['stage'] = 'notarizing'
        self.save()
        archive = self.output / 'Yu-notarization.zip'
        self.run('app-zip', 'ditto', '-c', '-k', '--keepParent', self.app, archive)
        self.notarize_file('app', archive, profile)
        self.run('app-staple', 'xcrun', 'stapler', 'staple', self.app)
        self.run('app-ticket', 'xcrun', 'stapler', 'validate', self.app)
        self.verify_signatures()
        self.record_app()
        self.finish_dmg(profile)

    def finish_dmg(self, profile, resume=False):
        prefix = ''
        if resume:
            self.load('notarizing')
            if self.data['notarization'].get('app', {}).get('status') != 'Accepted':
                raise ValueError('Cannot resume packaging without Accepted app notarization')
            if 'dmg' in self.data['notarization']:
                raise ValueError('DMG already submitted; inspect its existing submission before retrying')
            self.verify_signatures()
            prefix = f'retry-{uuid.uuid4().hex[:8]}-'
            self.run(f'{prefix}app-ticket', 'xcrun', 'stapler', 'validate', self.app)
            self.data.setdefault('packaging_retries', []).append({
                'prefix': prefix, 'pipeline_sha256': sha(Path(__file__)),
            })
            self.save()
        dmg = self.package(attempt_prefix=prefix)
        self.notarize_file('dmg', dmg, profile)
        self.run('dmg-staple', 'xcrun', 'stapler', 'staple', dmg)
        self.run('dmg-ticket', 'xcrun', 'stapler', 'validate', dmg)
        self.run('dmg-final-signature', 'codesign', '--verify', '--strict', dmg)
        self.run('gatekeeper-app', 'spctl', '--assess', '--type', 'execute', '--verbose=4', self.app)
        self.run('gatekeeper-dmg', 'spctl', '--assess', '--type', 'open',
                 '--context', 'context:primary-signature', '--verbose=4', dmg)
        self.data['gatekeeper'] = gatekeeper_evidence(
            gatekeeper_policy(),
            (self.output / 'gatekeeper-app.log').read_text(),
            (self.output / 'gatekeeper-dmg.log').read_text())
        # A locally disabled Gatekeeper can return success through an override.
        # Never count that as default-policy first-launch acceptance.
        self.data.update(stage='notarized-awaiting-acceptance', dmg_sha256=sha(dmg))
        (self.output / 'SHA256SUMS').write_text(f"{sha(dmg)}  {dmg.name}\n")
        self.save()

    def finalize(self, evidence_path):
        self.load('notarized-awaiting-acceptance')
        if self.data['candidate']:
            raise ValueError('A dirty-checkout candidate cannot be ready to publish')
        dmg = self.output / self.data['dmg']
        if sha(dmg) != self.data['dmg_sha256']:
            raise ValueError('DMG changed after notarization')
        if any(self.data['notarization'].get(label, {}).get('status') != 'Accepted'
               for label in ('app', 'dmg')):
            raise ValueError('Both notarization results must be Accepted')
        evidence = json.loads(evidence_path.read_text())
        checks = checked_acceptance(evidence, self.data)
        self.verify_signatures()
        self.run('final-app-ticket', 'xcrun', 'stapler', 'validate', self.app)
        self.run('final-dmg-ticket', 'xcrun', 'stapler', 'validate', dmg)
        self.run('final-dmg-integrity', 'hdiutil', 'verify', dmg)
        saved_evidence = self.output / 'acceptance-evidence.json'
        if evidence_path.resolve() != saved_evidence.resolve():
            shutil.copyfile(evidence_path, saved_evidence)
        self.data['acceptance'].update(checks)
        self.data['acceptance_evidence_sha256'] = sha(saved_evidence)
        self.data['stage'] = 'ready-to-publish'
        self.save()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    build = commands.add_parser('build')
    build.add_argument('--candidate', action='store_true', help='Allow dirty source; never an official release')
    sign = commands.add_parser('sign')
    sign.add_argument('--identity', required=True)
    sign.add_argument('--team', required=True)
    distribute = commands.add_parser('notarize')
    distribute.add_argument('--profile', required=True, help='notarytool Keychain profile name')
    preview = commands.add_parser('preview-dmg', help='Signed but NOT notarized DMG for packaging checks')
    resume = commands.add_parser('resume-dmg', help='Retry packaging after app ticket validation; preserve failed logs')
    resume.add_argument('--profile', required=True)
    finalize = commands.add_parser('finalize', help='Verify final artifact and attach release acceptance evidence')
    finalize.add_argument('--evidence', type=Path, required=True)
    for command in (build, sign, distribute, preview, resume, finalize):
        command.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    release = Release(args.output)
    if args.command == 'build':
        release.build(args.candidate)
    elif args.command == 'sign':
        release.sign(args.identity, args.team)
    elif args.command == 'preview-dmg':
        release.load('signed')
        release.verify_signatures()
        release.package(preview=True)
    elif args.command == 'resume-dmg':
        release.finish_dmg(args.profile, resume=True)
    elif args.command == 'finalize':
        release.finalize(args.evidence)
    else:
        release.distribute(args.profile)
    print(f"{release.data['stage']}: {release.manifest_path}")


if __name__ == '__main__':
    try:
        main()
    except (ValueError, RuntimeError, OSError, subprocess.CalledProcessError) as error:
        print(f'Release stopped: {error}', file=sys.stderr)
        sys.exit(1)
