#!/usr/bin/env python3
"""Validate a packaged release, then upload it using local, hidden credentials."""
import argparse
from datetime import datetime, timezone
import getpass
import hashlib
import json
from pathlib import Path
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('--check-only', action='store_true')
    args = parser.parse_args()
    directory = args.directory.resolve()
    manifest = json.loads((directory / 'appstore-manifest.json').read_text())
    package = directory / 'Yu-macOS-AppStore.pkg'
    digest = hashlib.sha256(package.read_bytes()).hexdigest()
    if (manifest.get('stage') != 'packaged' or manifest.get('candidate') is not False
            or manifest.get('git_status') or digest != manifest.get('package_sha256')
            or manifest.get('app_store_connect_id') != '6817272771'
            or manifest.get('bundle_id') != 'io.github.xiaodou997.yu'):
        raise ValueError('Release manifest or package verification failed')
    subprocess.run(['pkgutil', '--check-signature', str(package)], check=True)
    print(f"Verified Yu {manifest['version']} ({manifest['build_number']}): {digest}")
    if args.check_only:
        return
    if not sys.stdin.isatty():
        raise ValueError('Run interactively in your local terminal')
    username = input('App Store Connect Apple ID: ').strip()
    password = getpass.getpass('Existing App-specific password (hidden): ')
    if not username or not password or '\n' in password or '\r' in password:
        raise ValueError('Missing or invalid credentials')
    record = {'version': manifest['version'], 'build_number': manifest['build_number'],
              'package_sha256': digest, 'commands': []}
    for operation in ('validate-app', 'upload-package'):
        # Omitting -p makes altool read stdin. Never put the secret in argv or env.
        command = ['xcrun', 'altool', '--' + operation, str(package),
                   '-u', username, '--output-format', 'json']
        print(f'Apple {operation} in progress…', flush=True)
        result = subprocess.run(command, input=password + '\n', text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        safe_output = result.stdout.replace(password, '[redacted]').replace(username, '[redacted]')
        log = directory / (operation + '.log')
        log.write_text(safe_output)
        record['commands'].append({'operation': operation, 'exit_code': result.returncode,
                                   'log': log.name, 'finished_at': datetime.now(timezone.utc).isoformat()})
        (directory / 'upload-result.json').write_text(json.dumps(record, indent=2) + '\n')
        print(safe_output)
        if result.returncode:
            raise RuntimeError(f'Apple {operation} failed; upload stopped. See {log}')
    print('Upload finished. Select the processed build in App Store Connect. No review was submitted.')


if __name__ == '__main__':
    try:
        main()
    except (ValueError, RuntimeError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
