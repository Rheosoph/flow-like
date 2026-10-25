import importlib.util
from contextlib import redirect_stderr, redirect_stdout
import io
import json
from pathlib import Path
from types import SimpleNamespace
import unittest
import tempfile
from unittest.mock import patch


MODULE_PATH = Path(__file__).resolve().parents[1] / "widget-action-benchmark.py"
SPEC = importlib.util.spec_from_file_location("widget_action_benchmark", MODULE_PATH)
benchmark = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(benchmark)


def event_stream(*events):
    return io.BytesIO(b"\n".join(json.dumps(event).encode() for event in events) + b"\n")


def patch_event():
    return {"type": "patch", "messages": [{"surfaceUpdate": {}}], "person_id": 1, "expected_rows": 3}


def complete_event(**overrides):
    return {"type": "complete", "sample": {"verified": True, "rows": 3, "widget_gets": 0, **overrides}}


def sample(milliseconds=100, gets=0):
    return {
        "ok": True, "patch_ms": milliseconds, "complete_ms": milliseconds + 20,
        "client_queue_ms": 10, "server": {"queue_ms": 5, "widget_gets": gets},
    }


class MeasurementTests(unittest.TestCase):
    def test_patch_and_terminal_are_separate_boundaries(self):
        times = iter([0, 100_000_000, 100_000_000, 500_000_000])
        measured = benchmark.read_action(
            event_stream(patch_event(), complete_event()), 1, 0, 1, lambda: next(times)
        )
        self.assertEqual(measured["patch_ms"], 100)
        self.assertEqual(measured["complete_ms"], 500)
        self.assertEqual(measured["expected_rows"], 3)

    def test_invalid_or_unverified_output_is_never_a_successful_timing(self):
        invalid_sequences = [
            [complete_event()],
            [patch_event()],
            [patch_event(), patch_event(), complete_event()],
            [{**patch_event(), "person_id": 2}, complete_event()],
            [{**patch_event(), "messages": []}, complete_event()],
            [{**patch_event(), "expected_rows": 0}, complete_event()],
            [patch_event(), complete_event(verified=False)],
            [patch_event(), complete_event(rows=4)],
            [patch_event(), complete_event(widget_gets=-1)],
            [patch_event(), complete_event(graph_ms=float("nan"))],
            [patch_event(), {"type": "error", "message": "fixture failed"}],
            [{"type": "unrelated_update"}, complete_event()],
        ]
        for events in invalid_sequences:
            with self.subTest(events=events), self.assertRaises(benchmark.ProtocolError):
                benchmark.read_action(event_stream(*events), 1, 0, 1, lambda: 0)

    def test_timeout_and_malformed_json_fail_explicitly(self):
        with self.assertRaises(TimeoutError):
            benchmark.read_action(event_stream(patch_event()), 1, 0, 1, lambda: 2_000_000_000)
        with self.assertRaises(benchmark.ProtocolError):
            benchmark.read_action(io.BytesIO(b"{invalid}\n"), 1, 0, 1, lambda: 0)
        times = iter([0, 2_000_000_000])
        with self.assertRaises(TimeoutError):
            benchmark.read_action(event_stream(patch_event()), 1, 0, 1, lambda: next(times))

    def test_nearest_rank_percentiles_and_empty_results(self):
        result = benchmark.distribution(list(range(1, 21)))
        self.assertEqual(result["n"], 20)
        self.assertEqual(result["p50"], 10)
        self.assertEqual(result["p95"], 19)
        self.assertEqual(result["max"], 20)
        self.assertIsNone(benchmark.distribution([]))

    def test_failures_and_fallbacks_are_retained_in_the_summary(self):
        result = benchmark.summarize(
            [sample(100, 0), sample(700, 1), {"ok": False, "error": "timeout"}], 0, 500
        )
        self.assertEqual(result["attempts"], 3)
        self.assertEqual(result["failures"], 1)
        self.assertEqual(result["metrics"]["patch_ms"]["n"], 2)
        self.assertEqual(result["widget_gets"]["samples_outside_expected_path"], 1)
        self.assertEqual(result["budget"]["actions_within_budget"], 1)
        self.assertEqual(result["budget"]["total_actions"], 3)
        self.assertFalse(result["budget"]["passed"])
        self.assertFalse(benchmark.summarize([{"ok": False}], 0, 500)["budget"]["passed"])

    def test_burst_arrival_precedes_worker_availability(self):
        calls = []

        def run(origin, variant, cache, person_id, timeout, scheduled_ns=None, gate=None):
            self.assertIsNotNone(scheduled_ns)
            self.assertIsNotNone(gate)
            gate.wait(timeout=2)
            calls.append(scheduled_ns)
            return sample()

        args = SimpleNamespace(warmup=0, concurrency=3, samples=5, origin="http://local", timeout=5, budget_ms=None)
        with patch.object(benchmark, "run_action", run):
            result = benchmark.run_case(args, "optimized", "burst", 0)
        self.assertEqual(len(result["samples"]), 5)
        self.assertEqual(len(set(calls[:3])), 1)
        self.assertEqual(len(set(calls[3:])), 1)
        self.assertNotEqual(calls[0], calls[-1])

    def test_comparison_matches_cases_and_flags_changed_conditions(self):
        def report(latency, concurrency):
            return {
                "format_version": 1, "config": {"concurrency": concurrency}, "server_health": {},
                "cases": [{"scenario": "warm", "variant": "optimized", "summary": benchmark.summarize([sample(latency)], 0, None)}],
            }

        result = benchmark.compare_results(report(80, 8), report(100, 4))
        self.assertEqual(result["cases"][0]["patch_p95_delta_ms"], -20)
        self.assertEqual(result["cases"][0]["patch_p95_delta_percent"], -20)
        self.assertEqual(len(result["warnings"]), 1)


class ReportTests(unittest.TestCase):
    class Opener:
        def __init__(self, fail=False):
            self.fail = fail

        def open(self, req, timeout):
            if isinstance(req, str):
                return io.BytesIO(b'{"profile":"local"}')
            if self.fail:
                return event_stream({"type": "error", "message": "fixture failed"})
            action = json.loads(req.data)
            gets = benchmark.EXPECTED_GETS[action["variant"]]
            return event_stream(
                {**patch_event(), "person_id": action["person_id"]}, complete_event(widget_gets=gets)
            )

    def args(self, output):
        return SimpleNamespace(
            origin="http://local", variants=["optimized"], scenarios=["warm"],
            samples=2, warmup=1, concurrency=2, timeout=5,
            output=output, compare=None, budget_ms=None,
        )

    def run_main(self, args, fail=False):
        with patch.object(benchmark, "arguments", return_value=args), \
             patch.object(benchmark.request, "build_opener", return_value=self.Opener(fail)), \
             patch.object(benchmark, "revision", return_value={"commit": "test", "dirty": False}), \
             redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            return benchmark.main()

    def test_comparison_can_read_the_same_path_as_output(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "results.json"
            args = self.args(output)
            self.assertEqual(self.run_main(args), 0)
            previous = json.loads(output.read_text())
            previous["cases"][0]["summary"]["metrics"]["patch_ms"]["p95"] = 500
            output.write_text(json.dumps(previous))
            args.compare = output
            self.assertEqual(self.run_main(args), 0)
            current = json.loads(output.read_text())
            self.assertLess(current["comparison"]["cases"][0]["patch_p95_delta_ms"], -400)

    def test_a_failed_warmup_replaces_stale_results_with_a_failed_report(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "results.json"
            output.write_text('{"status":"old-result"}')
            self.assertEqual(self.run_main(self.args(output), fail=True), 1)
            failed = json.loads(output.read_text())
            self.assertEqual(failed["status"], "failed")
            self.assertIn("fixture failed", failed["error"])


if __name__ == "__main__":
    unittest.main()
