"""Check percentile inputs: idle time and other windows must not hide or add stalls."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("metrics", Path(__file__).parents[1] / "summarize-render-metrics.py")
metrics = importlib.util.module_from_spec(spec)
spec.loader.exec_module(metrics)


class MetricsChecks(unittest.TestCase):
    def test_real_presents_only_within_completed_gestures(self):
        lines = [
            "yu-render-metric event=live_begin surface=0x1 time_s=1\n",
            "yu-render-metric event=present surface=0x1 time_s=1.2\n",
            "yu-render-metric event=present surface=0x1 time_s=1.1\n",
            "yu-render-metric event=cpu_submit duration_ms=0.5\n",
            "yu-render-metric event=present surface=0x2 time_s=1.15\n",
            "yu-render-metric event=live_end surface=0x1 time_s=2\n",
            "yu-render-metric event=present surface=0x1 time_s=8\n",
            "yu-render-metric event=live_begin surface=0x1 time_s=10\n",
            "yu-render-metric event=present surface=0x1 time_s=10.1\n",
            "yu-render-metric event=present surface=0x1 time_s=10.12\n",
            "yu-render-metric event=live_end surface=0x1 time_s=11\n",
        ]
        result = metrics.summarize(lines)
        frames = result["presented_frame_interval_ms"]
        self.assertEqual(frames["samples"], 2)
        self.assertAlmostEqual(frames["p50"], 20)
        self.assertAlmostEqual(frames["p95"], 100)  # Never filter the long stall.
        self.assertEqual(result["stage_duration_ms"]["cpu_submit"]["samples"], 1)

    def test_gpu_queue_counts_and_pointer_normalization(self):
        result = metrics.summarize([
            "yu-render-metric event=gpu_submit surface=0x0001 time_s=1 duration_ms=0",
            "yu-render-metric event=gpu_submit surface=0x1 time_s=2 duration_ms=0",
            "yu-render-metric event=gpu_complete surface=0x1 time_s=3 duration_ms=0",
            "yu-render-metric event=gpu_complete surface=0x1 time_s=4 duration_ms=0",
        ])
        self.assertEqual(result["gpu_in_flight_max_by_surface"], {"0x1": 2})
        self.assertNotIn("gpu_submit", result["stage_duration_ms"])
        self.assertEqual(result["counts"]["stale_publication"], 0)

    def test_no_scroll_does_not_claim_zero_frame_time(self):
        result = metrics.summarize(["yu-render-metric event=present surface=0x1 time_s=3"])
        self.assertEqual(result["presented_frame_interval_ms"]["samples"], 0)
        self.assertIsNone(result["presented_frame_interval_ms"]["p95"])


if __name__ == "__main__":
    unittest.main()
