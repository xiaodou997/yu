#!/usr/bin/env python3
"""Summarize measured stages and actual drawable presentation timestamps."""
import collections
import json
import math
import pathlib
import sys


def distribution(values):
    values = sorted(values)
    if not values:
        return {"samples": 0, "p50": None, "p95": None, "p99": None}
    return {"samples": len(values), **{
        f"p{percent}": values[max(0, math.ceil(len(values) * percent / 100) - 1)]
        for percent in (50, 95, 99)
    }}


def summarize(lines):
    events = []
    for line in lines:
        if not line.startswith("yu-render-metric "):
            continue
        event = dict(field.split("=", 1) for field in line.split()[1:] if "=" in field)
        if "event" in event:
            events.append(event)
    counts = collections.Counter(event["event"] for event in events)
    for name in ("drawable_unavailable", "gpu_busy", "render_busy", "stale_publication", "gpu_submit", "gpu_complete", "present"):
        counts.setdefault(name, 0)
    stages = collections.defaultdict(list)
    presents = collections.defaultdict(list)
    gestures = collections.defaultdict(list)
    gpu_events = collections.defaultdict(list)
    for event in events:
        if "duration_ms" in event and event["event"] not in {"gpu_submit", "gpu_complete", "gpu_busy", "drawable_unavailable"}:
            value = float(event["duration_ms"])
            if math.isfinite(value) and value >= 0:
                stages[event["event"]].append(value)
        if "time_s" not in event:
            continue
        timestamp = float(event["time_s"])
        if not math.isfinite(timestamp) or timestamp <= 0:
            continue
        surface = event.get("surface", "unknown")
        if surface.startswith("0x"):
            surface = hex(int(surface, 16))
        if event["event"] in ("gpu_submit", "gpu_complete"):
            gpu_events[surface].append((timestamp, event["event"]))
        if event["event"] == "present":
            presents[surface].append(timestamp)
        elif event["event"] in ("live_begin", "live_end"):
            gestures[surface].append((timestamp, event["event"]))
    intervals = []
    gesture_count = 0
    for surface, boundaries in gestures.items():
        start = None
        for timestamp, kind in sorted(boundaries):
            if kind == "live_begin":
                start = timestamp
            elif start is not None:
                # Callback delivery can be reordered. Use the presentation clock,
                # never callback arrival order; never bridge idle gaps or windows.
                frames = sorted(t for t in presents[surface] if start <= t <= timestamp)
                intervals.extend((b - a) * 1000 for a, b in zip(frames, frames[1:]) if b > a)
                gesture_count += 1
                start = None
    in_flight_max = {}
    for surface, samples in gpu_events.items():
        outstanding = peak = 0
        for _, kind in sorted(samples):
            outstanding = max(0, outstanding + (1 if kind == "gpu_submit" else -1))
            peak = max(peak, outstanding)
        in_flight_max[surface] = peak
    return {
        "gpu_in_flight_max_by_surface": in_flight_max,
        "counts": dict(sorted(counts.items())),
        "stage_duration_ms": {key: distribution(values) for key, values in sorted(stages.items())},
        "completed_live_scroll_gestures": gesture_count,
        "presented_frame_interval_ms": distribution(intervals),
        "acceptance": "Requires review of input method, hardware, display refresh rate and trace; no automatic pass.",
    }


if __name__ == "__main__":
    with pathlib.Path(sys.argv[1]).open() as stream:
        print(json.dumps(summarize(stream), ensure_ascii=False, indent=2))
