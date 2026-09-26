"""Run explicitly after building Yu.app; these are process error-path checks, not UI acceptance."""
import hashlib
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile
import unittest
import uuid

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(sys.platform == 'darwin', 'Requires the built macOS application')
class BridgeResourceChecks(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = Path(os.environ.get('YU_BRIDGE_TEST_APP',
                       ROOT / 'platform/macos/yu-shell-macos/.build/Yu.app')).resolve()
        cls.binary = cls.app / 'Contents/MacOS/Yu'
        if not cls.binary.is_file():
            raise AssertionError('Build Yu.app before running bridge resource checks')
        cls.before = hashlib.sha256(cls.binary.read_bytes()).hexdigest()

    @classmethod
    def tearDownClass(cls):
        if hashlib.sha256(cls.binary.read_bytes()).hexdigest() != cls.before:
            raise AssertionError('Production executable changed during checks')

    def run_failure(self, *, missing_source):
        with tempfile.TemporaryDirectory(prefix='yu-bridge-resources-') as directory:
            root = Path(directory)
            app = root / 'MissingResources.app'
            binary = app / 'Contents/MacOS/Yu'
            binary.parent.mkdir(parents=True)
            shutil.copy2(self.binary, binary)
            info = plistlib.loads((self.app / 'Contents/Info.plist').read_bytes())
            info['CFBundleIdentifier'] = 'io.github.xiaodou997.yu.resource-init-check.' + uuid.uuid4().hex
            (app / 'Contents/Info.plist').write_bytes(plistlib.dumps(info))
            subprocess.run(['codesign', '--force', '--sign', '-', str(app)],
                           check=True, capture_output=True, timeout=30)
            source = root / 'source.md'
            data = b'\xef\xbb\xbf# Test\r\n\r\n$x^2$\r\n'
            if not missing_source:
                source.write_bytes(data)
            env = {k: v for k, v in os.environ.items() if not k.startswith('YU_')}
            env.update(YU_DOCUMENT_STATE_DIR=str(root / 'state'),
                       YU_PRESENTATION_STATE_DIR=str(root / 'columns'))
            result = subprocess.run([str(binary), '--selection-self-check', str(source)],
                                    env=env, capture_output=True, text=True, timeout=30)
            self.assertEqual(result.returncode, 1,
                             f'Expected a caught initialization error, not a signal: {result.returncode}\n{result.stderr}')
            self.assertIn('Yu Selection self-check failed:', result.stderr)
            if missing_source:
                self.assertFalse(source.exists())
            else:
                self.assertIn('应用缺少 yu_shaders.metallib', result.stderr)
                self.assertEqual(source.read_bytes(), data)

    def test_missing_shader_after_handle_creation_exits_normally(self):
        self.run_failure(missing_source=False)

    def test_missing_document_before_handle_creation_exits_normally(self):
        self.run_failure(missing_source=True)


if __name__ == '__main__':
    unittest.main()
