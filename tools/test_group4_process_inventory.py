"""Kernel process-identity regressions; no product windows or user processes changed."""
import importlib.util
import json
import os
from pathlib import Path
import select
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
HOST = ROOT / 'platform/macos/yu-shell-macos'
sys.path.insert(0, str(HOST))
SPEC = importlib.util.spec_from_file_location('soak_inventory_runner', HOST / 'run-resource-soak.py')
RUNNER = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = RUNNER
SPEC.loader.exec_module(RUNNER)


@unittest.skipUnless(sys.platform == 'darwin', 'requires macOS libproc; not UI acceptance')
class KernelInventoryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.build = tempfile.TemporaryDirectory(prefix='yu-kernel-build-')
        cls.inventory = Path(cls.build.name) / 'process-inventory'
        subprocess.run(['clang', '-Wall', '-Wextra', '-Werror',
                        str(ROOT / 'tools/process-inventory.c'), '-o', str(cls.inventory)],
                       check=True, timeout=60)
        cls.fixture = Path(cls.build.name) / 'wait-fixture'
        subprocess.run(['clang', '-Wall', '-Wextra', '-Werror', '-x', 'c', '-', '-o', str(cls.fixture)],
                       input='#include <stdio.h>\n#include <unistd.h>\nint main(void) { puts("ready"); fflush(stdout); for (;;) pause(); }\n',
                       text=True, check=True, timeout=60)

    @classmethod
    def tearDownClass(cls):
        cls.build.cleanup()

    def setUp(self):
        # Space, Unicode, apostrophe and newline would break naive command splitting.
        self.directory = tempfile.TemporaryDirectory(prefix="yu 身份 ' \n")
        self.out = Path(self.directory.name).resolve()
        self.app = self.out / 'YuResourceSoak.app/Contents/MacOS/Yu'
        self.helper = self.out / 'YuResourceSoak.app/Contents/Helpers/yu-document-renderer'
        self.other = self.out / 'other/Yu'
        for path in (self.app, self.helper, self.other):
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(self.fixture, path)
        self.children = []

    def tearDown(self):
        for child in reversed(self.children):
            if child.poll() is None:
                child.terminate()
            child.wait(timeout=5)
            child.stdout.close()
        self.directory.cleanup()

    def launch(self, executable, argv0=None):
        child = subprocess.Popen([str(argv0 or executable)], executable=str(executable),
                                 stdout=subprocess.PIPE, text=True, start_new_session=True)
        self.children.append(child)
        self.assertTrue(select.select([child.stdout], [], [], 5)[0], 'fixture not ready')
        self.assertEqual(child.stdout.readline().strip(), 'ready')
        return child

    def inventory_result(self, app=None, helper=None, owner=0):
        response = subprocess.check_output([str(self.inventory), str(app or self.app),
                                            str(helper or self.helper), str(owner)], text=True, timeout=10)
        value = json.loads(response)
        self.assertEqual(value['schema_version'], 1)
        self.assertGreaterEqual(value['unreadable_unrelated'], 0)
        return value['processes']

    def test_relative_helper_spelling_and_non_ascii_paths_are_real_processes(self):
        app = self.launch(self.app)
        helper = self.launch(self.app.parent / '../Helpers/yu-document-renderer')
        values = {item['pid']: item for item in self.inventory_result(owner=app.pid)}
        self.assertEqual(set(values), {app.pid, helper.pid})
        self.assertEqual(values[app.pid]['kind'], 'app')
        self.assertEqual(values[helper.pid]['kind'], 'helpers')
        self.assertEqual(values[app.pid]['pgid'], app.pid)
        self.assertEqual(values[helper.pid]['ppid'], os.getpid())
        self.assertGreater(values[helper.pid]['start_seconds'], 0)
        self.assertLess(values[helper.pid]['start_microseconds'], 1_000_000)

    def test_runner_counts_helper_using_actual_product_launch_spelling(self):
        app = self.launch(self.app)
        helper = self.launch(self.app.parent / '../Helpers/yu-document-renderer')
        soak = RUNNER.NativeSoak(RUNNER.parse_args([str(self.out)]), self.out, {})
        soak.inventory = self.inventory
        soak.process = app
        try:
            values = soak.processes()
            self.assertEqual(values, {'app': [app.pid], 'helpers': [helper.pid]})
        finally:
            soak.process = None  # teardown owns only these two explicit fixtures
            soak.cleanup()

    def test_spoofed_argv0_is_not_an_executable_identity(self):
        self.launch(self.other, argv0=self.app)
        self.assertEqual(self.inventory_result(), [])

    def test_same_basename_in_another_directory_does_not_match(self):
        self.launch(self.other)
        self.assertEqual(self.inventory_result(), [])

    def test_symlinked_target_and_launch_resolve_to_the_same_kernel_path(self):
        link = self.out / 'linked bundle'
        link.symlink_to(self.out / 'YuResourceSoak.app', target_is_directory=True)
        app = self.launch(link / 'Contents/MacOS/Yu')
        values = self.inventory_result(app=link / 'Contents/MacOS/Yu')
        self.assertEqual([item['pid'] for item in values], [app.pid])

    def test_exited_fixture_is_absent_not_an_alive_zero_footprint_process(self):
        app = self.launch(self.app)
        self.assertEqual(len(self.inventory_result()), 1)
        app.terminate(); app.wait(timeout=5)
        self.assertEqual(self.inventory_result(), [])

    def test_invalid_targets_fail_without_an_empty_successful_inventory(self):
        for app, helper, owner in ((self.out / 'missing', self.helper, '0'),
                                   (self.app, self.app, '0'),
                                   (self.app, self.helper, '-1'),
                                   (self.app, self.helper, 'not-a-pid')):
            result = subprocess.run([str(self.inventory), str(app), str(helper), owner],
                                    text=True, capture_output=True, timeout=10)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, '')


if __name__ == '__main__':
    unittest.main()
