"""Tests for the feature-independent acceptance execution contract."""

from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).parent))
from acceptance_runner import RunContext


class AcceptanceRunnerTests(unittest.TestCase):
    def test_partial_requires_all_layers_for_full_pass(self):
        with tempfile.TemporaryDirectory() as temporary:
            ctx = RunContext(Path(temporary) / "run", "example")
            self.assertEqual(ctx.run("core", [sys.executable, "-c", "print('ok')"]), 0)
            ctx.ledger.add("example/one", ["core", "real_window"])
            ctx.ledger.record("example/one", "core", "passed", "core")
            self.assertEqual(ctx.ledger.finish()[0]["status"], "partial")
            self.assertEqual(ctx.run("window", [sys.executable, "-c", "print('window ok')"]), 0)
            ctx.ledger.record("example/one", "real_window", "passed", "window")
            self.assertEqual(ctx.ledger.finish()[0]["status"], "passed")
            self.assertTrue(Path(ctx.commands["core"]["log"]).is_file())
            self.assertEqual(len(ctx.artifact(ctx.log("core"))["sha256"]), 64)

    def test_failed_command_cannot_produce_pass(self):
        with tempfile.TemporaryDirectory() as temporary:
            ctx = RunContext(Path(temporary) / "run", "example")
            self.assertEqual(ctx.run("failure", [sys.executable, "-c", "exit(4)"]), 4)
            ctx.ledger.add("example/one", ["core"])
            with self.assertRaises(ValueError):
                ctx.ledger.record("example/one", "core", "passed", "failure")
            with self.assertRaises(ValueError):
                ctx.ledger.record("example/one", "core", "failed", "failure",
                                  evidence={"log_sha256": "forged"})
            ctx.ledger.record("example/one", "core", "failed", "failure")
            self.assertEqual(ctx.ledger.finish()[0]["status"], "failed")

    def test_output_and_step_names_cannot_be_reused(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "run"
            ctx = RunContext(output, "example")
            ctx.run("check", [sys.executable, "-c", "print('ok')"])
            with self.assertRaises(ValueError):
                ctx.run("check", [sys.executable, "-c", "print('again')"])
            with self.assertRaises(FileExistsError):
                RunContext(output, "example")


if __name__ == "__main__":
    unittest.main()
