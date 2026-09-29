"""Negative tests for release signing and artifact handoff boundaries."""
import importlib.util
import json
from pathlib import Path
import plistlib
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('release', Path(__file__).with_name('release-macos.py'))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)
DETAILS = '''Authority=Developer ID Application: Example (ABCDEFGHIJ)
TeamIdentifier=ABCDEFGHIJ
CodeDirectory v=20500 size=123 flags=0x10000(runtime) hashes=1+2
Timestamp=29 Sep 2026 at 14:00:00
'''


class ReleaseTests(unittest.TestCase):
    def test_final_acceptance_requires_matching_artifact_and_each_software_gate(self):
        manifest = {'git_commit': 'abc', 'version': '0.1.2',
                    'build_number': '3', 'dmg_sha256': 'digest'}
        evidence = dict(manifest, checks={key: {'status': 'passed', 'evidence': key + '.json'}
                                          for key in release.REQUIRED_ACCEPTANCE})
        self.assertEqual(release.checked_acceptance(evidence, manifest), evidence['checks'])
        for changed in [dict(evidence, dmg_sha256='wrong'),
                        dict(evidence, checks={**evidence['checks'],
                                               'external_ax': {'status': 'pending'}})]:
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                release.checked_acceptance(changed, manifest)

    def test_candidate_cannot_be_finalized(self):
        with tempfile.TemporaryDirectory() as directory:
            work = release.Release(Path(directory))
            work.app.mkdir()
            work.data = {'stage': 'notarized-awaiting-acceptance', 'candidate': True,
                         'bundle_files': {}, 'dmg': 'Yu.dmg', 'dmg_sha256': 'none'}
            work.save()
            with self.assertRaisesRegex(ValueError, 'candidate'):
                work.finalize(Path(directory) / 'missing.json')

    def test_packaging_resume_rejects_unaccepted_app_or_existing_dmg_submission(self):
        for notarization in [{'app': {'status': 'Invalid'}},
                             {'app': {'status': 'Accepted'}, 'dmg': {'status': 'Submitted'}}]:
            with self.subTest(notarization=notarization), tempfile.TemporaryDirectory() as directory:
                work = release.Release(Path(directory))
                work.app.mkdir()
                work.data = {'stage': 'notarizing', 'bundle_files': {}, 'notarization': notarization}
                work.save()
                with self.assertRaises(ValueError):
                    work.finish_dmg('unused-profile', resume=True)

    def test_dmg_capacity_counts_sparse_logical_bytes_and_excludes_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            staging = Path(directory)
            self.assertGreaterEqual(release.dmg_capacity_mib(staging), 256)
            with (staging / 'large-helper').open('wb') as stream:
                stream.truncate(400 * 1024 * 1024)
            (staging / 'link').symlink_to(staging / 'large-helper')
            self.assertEqual(release.dmg_capacity_mib(staging), 532)

    def test_disabled_gatekeeper_cannot_be_counted_as_enforced_acceptance(self):
        for policy, result in [('assessments disabled', 'accepted'),
                               ('assessments enabled', 'accepted\noverride=security disabled')]:
            evidence = release.gatekeeper_evidence(policy, result, 'accepted')
            self.assertFalse(evidence['enforced_assessment'])
            self.assertEqual(evidence['default_policy_launch_acceptance'], 'pending')

    def validate(self, details, entitlements=b''):
        with patch.object(release, 'capture', side_effect=['', details]), patch.object(
                release.subprocess, 'check_output', return_value=entitlements):
            return release.signature_details(Path('Yu.app'), 'ABCDEFGHIJ', True)

    def test_developer_id_runtime_timestamp_empty_entitlements(self):
        self.assertEqual(self.validate(DETAILS), DETAILS)

    def test_rejects_development_adhoc_wrong_team_missing_runtime_or_timestamp(self):
        for details in [DETAILS.replace('Developer ID Application', 'Apple Development'),
                        DETAILS.replace('ABCDEFGHIJ', 'OTHERTEAM0'),
                        DETAILS.replace('0x10000(runtime)', '0x0(none)'),
                        DETAILS.replace('Timestamp=29 Sep 2026 at 14:00:00\n', ''),
                        'Signature=adhoc\n']:
            with self.subTest(details=details), self.assertRaises(ValueError):
                self.validate(details)

    def test_rejects_any_entitlement_including_debug_and_runtime_exceptions(self):
        for key in ['com.apple.security.get-task-allow', 'com.apple.security.cs.allow-jit',
                    'com.apple.security.cs.disable-library-validation',
                    'com.apple.security.cs.allow-unsigned-executable-memory',
                    'com.apple.security.cs.allow-dyld-environment-variables']:
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.validate(DETAILS, plistlib.dumps({key: True}))

    def test_rejects_bundle_mutation_between_stages(self):
        with tempfile.TemporaryDirectory() as directory:
            work = release.Release(Path(directory))
            work.app.mkdir()
            file = work.app / 'resource'
            file.write_text('original')
            work.data = {'stage': 'built', 'bundle_files': release.inventory(work.app)}
            work.save()
            work.load('built')
            file.write_text('changed')
            with self.assertRaisesRegex(ValueError, 'Bundle changed'):
                work.load('built')

    def test_rejects_symlink_in_shipped_app(self):
        with tempfile.TemporaryDirectory() as directory:
            app = Path(directory)
            (app / 'outside').symlink_to('/tmp')
            with self.assertRaises(ValueError):
                release.inventory(app)

    def test_dirty_checkout_cannot_produce_official_build(self):
        with patch.object(release, 'source_state', return_value={'git_status': ' M source'}):
            with self.assertRaisesRegex(ValueError, 'clean checkout'):
                release.Release(Path('/unused')).build(False)


if __name__ == '__main__':
    unittest.main()
