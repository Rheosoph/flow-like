#!/usr/bin/env python3
"""Measure the local widget action server and save raw timings for comparison."""

from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
from http.client import HTTPException
import json
import math
import os
from pathlib import Path
import platform
import subprocess
import sys
import threading
import time
from typing import Any, BinaryIO, Callable
from urllib import error, parse, request


VARIANTS = ("baseline", "overlap", "bundled", "optimized")
SCENARIOS = ("warm", "cold", "burst")
EXPECTED_GETS = {"baseline": 1, "overlap": 1, "bundled": 0, "optimized": 0}
MAX_LINE_BYTES = 4 * 1024 * 1024
MAX_RESPONSE_BYTES = 32 * 1024 * 1024


class ProtocolError(ValueError):
    pass


def finite_number(value: Any) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def distribution(values: list[float]) -> dict[str, float | int] | None:
    """Use nearest-rank percentiles and keep the sample count visible."""
    if not values:
        return None
    ordered = sorted(values)
    return {
        "n": len(ordered),
        "min": ordered[0],
        "mean": sum(ordered) / len(ordered),
        "p50": ordered[math.ceil(len(ordered) * 0.50) - 1],
        "p95": ordered[math.ceil(len(ordered) * 0.95) - 1],
        "max": ordered[-1],
    }


def read_action(
    response: BinaryIO,
    person_id: int,
    scheduled_ns: int,
    timeout: float,
    clock: Callable[[], int] = time.perf_counter_ns,
) -> dict[str, Any]:
    """Time the semantic patch line, independently of the terminal line."""
    patch = None
    patch_ms = None
    read_bytes = 0
    while True:
        if (clock() - scheduled_ns) / 1e9 > timeout:
            raise TimeoutError("action exceeded its time limit")
        line = response.readline(MAX_LINE_BYTES + 1)
        received_ns = clock()
        if (received_ns - scheduled_ns) / 1e9 > timeout:
            raise TimeoutError("action exceeded its time limit")
        if not line:
            raise ProtocolError("response ended before a complete event")
        read_bytes += len(line)
        if len(line) > MAX_LINE_BYTES or read_bytes > MAX_RESPONSE_BYTES:
            raise ProtocolError("action response exceeds the benchmark byte limit")
        if not line.strip():
            continue
        try:
            event = json.loads(line)
        except (UnicodeError, json.JSONDecodeError) as exc:
            raise ProtocolError("action returned an invalid JSON line") from exc
        if not isinstance(event, dict):
            raise ProtocolError("action event must be an object")
        kind = event.get("type")
        if kind == "error":
            raise ProtocolError(str(event.get("message", event.get("error", "server error"))))
        if kind == "patch":
            if patch is not None:
                raise ProtocolError("multiple semantic patch events; timing would be ambiguous")
            if event.get("person_id") != person_id:
                raise ProtocolError("patch belongs to a different person")
            messages = event.get("messages")
            if not isinstance(messages, list) or not messages:
                raise ProtocolError("patch contains no UI messages")
            rows = event.get("expected_rows")
            if not isinstance(rows, int) or isinstance(rows, bool) or rows < 1:
                raise ProtocolError("patch has no positive expected row count")
            patch = event
            patch_ms = (received_ns - scheduled_ns) / 1e6
        elif kind == "complete":
            if patch is None:
                raise ProtocolError("action completed without a rental-history patch")
            sample = event.get("sample")
            if not isinstance(sample, dict):
                raise ProtocolError("complete event has no sample object")
            if sample.get("verified") is not True:
                raise ProtocolError("server did not verify the rental-history output")
            if isinstance(sample.get("rows"), bool) or sample.get("rows") != patch["expected_rows"]:
                raise ProtocolError("rendered row count differs from the expected result")
            gets = sample.get("widget_gets")
            if not isinstance(gets, int) or isinstance(gets, bool) or gets < 0:
                raise ProtocolError("complete event has no valid widget GET counter")
            if any(key.endswith("_ms") and (not finite_number(value) or value < 0)
                   for key, value in sample.items()):
                raise ProtocolError("complete event contains an invalid stage duration")
            return {
                "patch_ms": patch_ms,
                "complete_ms": (received_ns - scheduled_ns) / 1e6,
                "expected_rows": patch["expected_rows"],
                "message_count": len(patch["messages"]),
                "server": sample,
            }
        else:
            raise ProtocolError(f"unknown action event type: {kind!r}")


def run_action(
    origin: str,
    variant: str,
    cache: str,
    person_id: int,
    timeout: float,
    scheduled_ns: int | None = None,
    gate: threading.Event | None = None,
) -> dict[str, Any]:
    if scheduled_ns is None:
        scheduled_ns = time.perf_counter_ns()
    if gate is not None:
        gate.wait()
    sending_ns = time.perf_counter_ns()
    sample: dict[str, Any] = {
        "person_id": person_id,
        "ok": False,
        "client_queue_ms": (sending_ns - scheduled_ns) / 1e6,
    }
    body = json.dumps({"variant": variant, "cache": cache, "person_id": person_id}).encode()
    req = request.Request(
        origin + "/action", data=body,
        headers={"Content-Type": "application/json", "Accept": "application/x-ndjson"},
        method="POST",
    )
    try:
        # Explicitly bypass proxy environment settings for this local workload.
        opener = request.build_opener(request.ProxyHandler({}))
        remaining = timeout - (sending_ns - scheduled_ns) / 1e9
        if remaining <= 0:
            raise TimeoutError("action exhausted its time limit in the client queue")
        with opener.open(req, timeout=remaining) as response:
            sample.update(read_action(response, person_id, scheduled_ns, timeout))
        sample["ok"] = True
    except (OSError, error.URLError, HTTPException, ValueError, TimeoutError) as exc:
        sample["error"] = f"{type(exc).__name__}: {exc}"
        sample["failed_after_ms"] = (time.perf_counter_ns() - scheduled_ns) / 1e6
    return sample


def summarize(samples: list[dict[str, Any]], expected_gets: int, budget_ms: float | None) -> dict[str, Any]:
    successes = [sample for sample in samples if sample["ok"]]
    metrics = {
        name: distribution([sample[name] for sample in successes])
        for name in ("patch_ms", "complete_ms", "client_queue_ms")
    }
    stages: dict[str, list[float]] = {}
    for sample in successes:
        for key, value in sample["server"].items():
            if key.endswith("_ms") and finite_number(value) and value >= 0:
                stages.setdefault(key, []).append(float(value))
    gets = [sample["server"].get("widget_gets") for sample in successes]
    observed_gets = [value for value in gets if isinstance(value, int) and not isinstance(value, bool)]
    result = {
        "attempts": len(samples),
        "successes": len(successes),
        "failures": len(samples) - len(successes),
        "metrics": metrics,
        "server_stages_ms": {key: distribution(values) for key, values in sorted(stages.items())},
        "widget_gets": {
            "expected_per_action": expected_gets,
            "observed_total": sum(observed_gets),
            "samples_with_counter": len(observed_gets),
            "samples_outside_expected_path": sum(value != expected_gets for value in observed_gets),
        },
    }
    if budget_ms is not None:
        result["budget"] = {
            "patch_p95_ms": budget_ms,
            "actions_within_budget": sum(sample["patch_ms"] <= budget_ms for sample in successes),
            "total_actions": len(samples),
            "passed": bool(successes) and len(successes) == len(samples) and metrics["patch_ms"]["p95"] <= budget_ms,
        }
    return result


def csv_choice(value: str, choices: tuple[str, ...]) -> list[str]:
    selected = value.split(",")
    if not selected or len(set(selected)) != len(selected) or any(item not in choices for item in selected):
        raise argparse.ArgumentTypeError("choose distinct comma-separated values from " + ",".join(choices))
    return selected


def positive_int(value: str) -> int:
    parsed = int(value)
    if parsed < 1:
        raise argparse.ArgumentTypeError("must be at least 1")
    return parsed


def positive_float(value: str) -> float:
    parsed = float(value)
    if not math.isfinite(parsed) or parsed <= 0:
        raise argparse.ArgumentTypeError("must be finite and greater than zero")
    return parsed


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--origin", default="http://127.0.0.1:4340")
    parser.add_argument("--variants", type=lambda value: csv_choice(value, VARIANTS), default=list(VARIANTS))
    parser.add_argument("--scenarios", type=lambda value: csv_choice(value, SCENARIOS), default=list(SCENARIOS))
    parser.add_argument("--samples", type=positive_int, default=20, help="measured actions per variant/scenario")
    parser.add_argument("--warmup", type=int, default=3, help="unmeasured warm actions per variant/scenario")
    parser.add_argument("--concurrency", type=positive_int, default=8, help="simultaneous clients in each finite burst")
    parser.add_argument("--timeout", type=positive_float, default=60, help="per-action time limit in seconds, including queues")
    parser.add_argument("--output", type=Path, default=Path("tmp/widget-action-benchmark/results.json"))
    parser.add_argument("--compare", type=Path, help="previous results JSON to compare")
    parser.add_argument("--budget-ms", type=positive_float, help="fail if patch p95 exceeds this explicit budget or an action fails")
    args = parser.parse_args()
    if args.warmup < 0:
        parser.error("--warmup must be nonnegative")
    url = parse.urlsplit(args.origin)
    if url.scheme not in ("http", "https") or not url.netloc or url.query or url.fragment:
        parser.error("--origin must be an HTTP(S) origin without query or fragment")
    args.origin = args.origin.rstrip("/")
    return args


def revision() -> dict[str, Any]:
    root = Path(__file__).resolve().parent.parent
    try:
        commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True, timeout=5).strip()
        dirty = bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=root, text=True, timeout=5))
        return {"commit": commit, "dirty": dirty}
    except (OSError, subprocess.SubprocessError):
        return {"commit": None, "dirty": None}


def compare_results(current: dict[str, Any], previous: dict[str, Any]) -> dict[str, Any]:
    if not isinstance(previous, dict) or previous.get("format_version") != 1:
        raise ValueError("comparison file uses an unsupported format")
    warnings = []
    if current["config"] != previous.get("config"):
        warnings.append("runner configuration differs; deltas are not a controlled comparison")
    if current["server_health"] != previous.get("server_health"):
        warnings.append("server metadata differs; inspect fixture and injected-delay settings before interpreting deltas")
    old_cases = {(case["scenario"], case["variant"]): case for case in previous.get("cases", [])}
    deltas = []
    for case in current["cases"]:
        old = old_cases.get((case["scenario"], case["variant"]))
        if old is None:
            continue
        before = old["summary"]["metrics"].get("patch_ms")
        after = case["summary"]["metrics"].get("patch_ms")
        if before and after:
            delta = after["p95"] - before["p95"]
            deltas.append({
                "scenario": case["scenario"], "variant": case["variant"],
                "patch_p95_delta_ms": delta,
                "patch_p95_delta_percent": delta / before["p95"] * 100 if before["p95"] else None,
                "previous_failures": old["summary"]["failures"],
                "current_failures": case["summary"]["failures"],
            })
    return {"warnings": warnings, "cases": deltas}


def run_case(args: argparse.Namespace, variant: str, scenario: str, expected_gets: int) -> dict[str, Any]:
    warmups = []
    if scenario != "cold":
        for index in range(args.warmup):
            warmups.append(run_action(args.origin, variant, "warm", 1 + index % 2, args.timeout))
        if any(not sample["ok"] for sample in warmups):
            raise RuntimeError("warmup failed: " + next(sample["error"] for sample in warmups if not sample["ok"]))
    samples = []
    if scenario == "burst":
        with ThreadPoolExecutor(max_workers=args.concurrency) as pool:
            for first in range(0, args.samples, args.concurrency):
                count = min(args.concurrency, args.samples - first)
                gate = threading.Event()
                # Arrival predates worker allocation, so client and server queues are included.
                scheduled_ns = time.perf_counter_ns()
                futures = [
                    pool.submit(run_action, args.origin, variant, "warm", 1 + (first + offset) % 2,
                                args.timeout, scheduled_ns, gate)
                    for offset in range(count)
                ]
                gate.set()
                samples.extend(future.result() for future in futures)
    else:
        for index in range(args.samples):
            samples.append(run_action(args.origin, variant, scenario, 1 + index % 2, args.timeout))
    return {
        "variant": variant, "scenario": scenario,
        "warmups": warmups, "samples": samples,
        "summary": summarize(samples, expected_gets, args.budget_ms),
    }


def display_case(case: dict[str, Any]) -> None:
    summary = case["summary"]
    patch = summary["metrics"]["patch_ms"]
    complete = summary["metrics"]["complete_ms"]
    values = [f"{patch[name]:8.1f}" if patch else "       -" for name in ("p50", "p95", "max")]
    complete_p95 = f"{complete['p95']:8.1f}" if complete else "       -"
    print(f"{case['scenario']:6} {case['variant']:9} {' '.join(values)} {complete_p95} "
          f"{summary['successes']:3}/{summary['attempts']:<3} "
          f"{summary['widget_gets']['observed_total']:4}", flush=True)


def main() -> int:
    args = arguments()
    report = None
    try:
        # Read comparison input before writing output; both paths may name the same file.
        previous = json.loads(args.compare.read_text()) if args.compare else None
        opener = request.build_opener(request.ProxyHandler({}))
        with opener.open(args.origin + "/health", timeout=min(args.timeout, 10)) as response:
            body = response.read(MAX_RESPONSE_BYTES + 1)
            if len(body) > MAX_RESPONSE_BYTES:
                raise ValueError("health response exceeds the benchmark byte limit")
            health = json.loads(body)
        if not isinstance(health, dict):
            raise ValueError("health response must be an object")
        server_gets = health.get("expected_widget_gets", {})
        if not isinstance(server_gets, dict):
            raise ValueError("health response contains invalid expected widget GET counts")
        expected_gets = {**EXPECTED_GETS, **server_gets}
        if any(not isinstance(value, int) or isinstance(value, bool) or value < 0 for value in expected_gets.values()):
            raise ValueError("health response contains invalid expected widget GET counts")
        report: dict[str, Any] = {
            "format_version": 1,
            "status": "running",
            "created_at": datetime.now(timezone.utc).isoformat(),
            "origin": args.origin,
            "source": revision(),
            "host": {"platform": platform.platform(), "machine": platform.machine(), "cpu_count": os.cpu_count(),
                     "python": platform.python_version()},
            "config": {key: getattr(args, key) for key in ("samples", "warmup", "concurrency", "timeout", "variants", "scenarios")},
            "server_health": health,
            "measurement": {
                "patch": "client receipt of verified rental-history patch line; browser paint is excluded",
                "complete": "client receipt of terminal complete line",
                "burst": "finite simultaneous arrivals measured before worker allocation, including client and server queues",
                "cold": "server cache coldness; process startup and OS page-cache coldness are not implied",
                "percentiles": "nearest rank over successful actions; failure counts remain separate",
            },
            "cases": [],
        }
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n")
        print("Milliseconds to semantic patch receipt; browser paint is excluded.")
        print("scenario variant   patch-p50 patch-p95 patch-max done-p95    ok/total GETs")
        for scenario in args.scenarios:
            for variant in args.variants:
                case = run_case(args, variant, scenario, expected_gets[variant])
                report["cases"].append(case)
                display_case(case)
                args.output.write_text(json.dumps(report, indent=2) + "\n")
        if previous is not None:
            report["comparison"] = compare_results(report, previous)
            for warning in report["comparison"]["warnings"]:
                print("Comparison: " + warning, file=sys.stderr)
            for case in report["comparison"]["cases"]:
                print(f"Delta {case['scenario']}/{case['variant']}: patch p95 {case['patch_p95_delta_ms']:+.1f} ms")
        report["status"] = "complete"
        args.output.write_text(json.dumps(report, indent=2) + "\n")
        print(f"Raw samples and server configuration: {args.output}")
        if args.samples < 100:
            print("Use at least 100 samples per case before relying on tail-latency comparisons.")
        failed = any(case["summary"]["failures"] for case in report["cases"])
        exceeded = args.budget_ms is not None and any(not case["summary"]["budget"]["passed"] for case in report["cases"])
        return 1 if failed or exceeded else 0
    except (OSError, error.URLError, HTTPException, ValueError, RuntimeError, KeyboardInterrupt) as exc:
        if report is not None:
            report["status"] = "failed"
            report["error"] = f"{type(exc).__name__}: {exc}"
            try:
                args.output.write_text(json.dumps(report, indent=2) + "\n")
            except OSError:
                pass
        print(f"Benchmark failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
