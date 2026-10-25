#!/usr/bin/env python3
"""Compare actual-app benchmark reports without treating failed trials as speedups."""

import argparse
import json
import math
from pathlib import Path
import sys


MISSING = object()


def nested(value, path):
    for key in path.split("."):
        if not isinstance(value, dict) or key not in value:
            return MISSING
        value = value[key]
    return value


def display(value):
    if value is MISSING:
        return "missing"
    rendered = json.dumps(value, sort_keys=True, separators=(",", ":"))
    return rendered if len(rendered) <= 140 else rendered[:137] + "..."


def load(directory):
    return {
        "report": json.loads((directory / "results.json").read_text()),
        "manifest": json.loads((directory / "run-manifest.json").read_text()),
    }


def effective_profile(bundle):
    manifest = bundle["manifest"]
    return manifest.get("binary_profile_hint") or manifest.get("requested_profile", MISSING)


def mismatches(first, second):
    fields = [
        *("report." + name for name in (
            "schemaVersion", "mode", "app", "configuration", "machine", "browser", "web.mode")),
        *("manifest." + name for name in (
            "web_mode", "worker_threads", "executor_slots", "thread_stack_mb", "compute_accounting_memory_mb",
            "enforced_memory_limit_mb", "backend_trace_sample_rate", "transport", "action_config_sha256",
            "telemetry_enabled", "telemetry_sink", "rust_log",
            "fixture.app_id", "fixture.graph_source_sha256", "fixture.graph_fixture_sha256",
            "fixture.database.records_sha256", "fixture.database.profile", "fixture.database.rows")),
        *("manifest.fixture.database." + name for name in ("data_storage_version", "pylance", "schema_sha256")),
    ]
    issues = []
    for field in fields:
        a, b = nested(first, field), nested(second, field)
        if a is MISSING or b is MISSING or a != b:
            issues.append(f"{field}: A={display(a)}, B={display(b)}")
    a, b = effective_profile(first), effective_profile(second)
    if a is MISSING or b is MISSING or a != b:
        issues.append(f"backend profile: A={display(a)}, B={display(b)}")
    # These are implementation variables. Missing provenance is still an error.
    for field in ("manifest.binary_sha256", "report.web.build.source.sha256"):
        a, b = nested(first, field), nested(second, field)
        if not isinstance(a, str) or not a or not isinstance(b, str) or not b:
            issues.append(f"{field}: A={display(a)}, B={display(b)}")
    for name, bundle in (("A", first), ("B", second)):
        if nested(bundle, "report.mode") != "actual-app":
            issues.append(f"{name}: report must come from the actual-app driver")
    return issues


def finite(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) and value >= 0


def percentile(values, fraction):
    if not values:
        return None
    ordered = sorted(values)
    position = (len(ordered) - 1) * fraction
    lower, upper = math.floor(position), math.ceil(position)
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def summary(values):
    values = [value for value in values if finite(value)]
    return {"n": len(values), "p50": percentile(values, 0.5), "p95": percentile(values, 0.95),
            "max": max(values) if values else None}


def assessment(bundle):
    report = bundle["report"]
    configuration = report.get("configuration", {})
    measured = [sample for sample in report.get("samples", []) if not sample.get("warmup")]
    passed = [sample for sample in measured
              if sample.get("status") == "ok" and sample.get("executionSuccessVerified") is True]
    warmups = [sample for sample in report.get("samples", []) if sample.get("warmup")]
    planned = configuration.get("samples", 0) * configuration.get("navigations", 0)
    planned_navigation = configuration.get("navigations", 0) * configuration.get("concurrency", 1)
    navigations = report.get("navigations", [])
    ready = [entry for entry in navigations if entry.get("status") == "ok"
             and entry.get("executionSuccessVerified") is True]
    resets = report.get("resets", [])
    reset_failed = sum(entry.get("status") != "ok" or entry.get("executionSuccessVerified") is not True
                       for entry in resets)
    bootstrap = [entry for entry in report.get("network", []) if entry.get("category") == "bootstrap"
                 and entry.get("phase") == "bootstrap" and 200 <= entry.get("status", 0) < 300
                 and not entry.get("networkError")]
    bootstrap_failed = sum(bool(entry.get("category") == "bootstrap" and (
        entry.get("networkError") or not 200 <= entry.get("status", 0) < 300
    )) for entry in report.get("network", []))
    metrics = {
        "Bootstrap HTTP": summary([entry.get("totalMs") for entry in bootstrap]),
        "Cards DOM": summary([entry.get("cardsDomMs") for entry in ready]),
        "History DOM (click)": summary([sample.get("clickToHistoryDomMs") for sample in passed]),
        "History frame opportunity": summary([sample.get("clickToHistoryFrameOpportunityMs") for sample in passed]),
        "Handler settled (click)": summary([sample.get("clickToSettledMs") for sample in passed]),
    }
    issues = []
    if report.get("status") != "passed":
        issues.append("driver did not finish with passed status")
    cleanup = report.get("cleanup")
    if cleanup is not None:
        if (not isinstance(cleanup, dict)
                or cleanup.get("status") not in ("ok", "passed", "success")
                or cleanup.get("wrapperExitCode", 0) != 0
                or cleanup.get("ownedChromeProcessesExited") is False):
            issues.append("driver cleanup did not complete successfully")
    if planned < 1 or len(measured) != planned:
        issues.append(f"measured {len(measured)} of {planned} planned actions")
    if len(passed) != len(measured):
        issues.append(f"{len(measured) - len(passed)} actions failed or lack verified terminal success")
    if len(navigations) != planned_navigation or len(ready) != planned_navigation:
        issues.append(f"{len(ready)} of {planned_navigation} navigations have verified terminal success")
    if report.get("browserErrorCount", 0):
        issues.append("application browser errors were recorded")
    if bootstrap_failed:
        issues.append(f"{bootstrap_failed} bootstrap HTTP requests failed or did not complete")
    if configuration.get("requireTerminalSuccess") is not True:
        issues.append("terminal success verification was disabled or absent")
    if configuration.get("requireActionTrace") is not True:
        issues.append("handler trace verification was disabled or absent")
    warmup_failed = sum(sample.get("status") != "ok" or sample.get("executionSuccessVerified") is not True
                        for sample in warmups)
    if warmup_failed:
        issues.append("a warmup action failed or lacks verified terminal success")
    planned_warmups = configuration.get("warmup", 0) * planned_navigation
    if len(warmups) != planned_warmups:
        issues.append(f"recorded {len(warmups)} of {planned_warmups} planned warmups")
    planned_resets = max(0, planned + planned_warmups - planned_navigation) if configuration.get("resetBetweenActions") else 0
    if len(resets) != planned_resets:
        issues.append(f"recorded {len(resets)} of {planned_resets} planned resets")
    if reset_failed:
        issues.append(f"{reset_failed} resets failed or lack verified terminal success")
    for label, metric in metrics.items():
        expected = len(ready) if label == "Cards DOM" else len(passed)
        if label != "Bootstrap HTTP" and metric["n"] != expected:
            issues.append(f"{label} timing missing from {expected - metric['n']} successful trials")
    identities = [(sample.get("navigation"), sample.get("index")) for sample in measured]
    if len(set(identities)) != len(identities):
        issues.append("measured action identities are duplicated")
    return {
        "metrics": metrics, "issues": issues, "planned": planned, "attempted": len(measured),
        "successful": len(passed), "failed": len(measured) - len(passed),
        "missing": max(0, planned - len(measured)), "navigations": len(ready),
        "planned_navigations": planned_navigation,
        "warmup_failed": warmup_failed, "reset_failed": reset_failed,
        "resets": len(resets), "planned_resets": planned_resets,
        "bootstrap_failed": bootstrap_failed,
    }


def render(first, second, mismatch):
    a, b = assessment(first), assessment(second)
    lines = []
    for name, bundle, result in (("A", first, a), ("B", second, b)):
        db = bundle["manifest"].get("fixture", {}).get("database", {})
        lines.append(f"{name}: layout={db.get('layout', 'unknown')}, profile={effective_profile(bundle)}, "
                     f"actions {result['successful']}/{result['planned']} successful, {result['failed']} failed, "
                     f"{result['missing']} unattempted; navigation {result['navigations']}/{result['planned_navigations']}; "
                     f"warmup failures {result['warmup_failed']}; resets {result['resets']}/{result['planned_resets']} "
                     f"({result['reset_failed']} failed); bootstrap HTTP failures {result['bootstrap_failed']}")
        manifest = bundle["manifest"]
        lines.append(f"{name}: telemetry={manifest.get('telemetry_enabled', 'unknown')}, "
                     f"sink={manifest.get('telemetry_sink', 'unknown')}, "
                     f"sampling={manifest.get('backend_trace_sample_rate', 'unknown')}, "
                     f"RUST_LOG={manifest.get('rust_log', 'unknown')}")
    for label, field in (("Backend binary", "manifest.binary_sha256"),
                         ("Frontend source", "report.web.build.source.sha256")):
        left, right = nested(first, field), nested(second, field)
        lines.append(f"{label}: {'same' if left == right else 'changed'}")
    if mismatch:
        lines.append("Configuration mismatches:")
        lines.extend("  " + issue for issue in mismatch)
    for name, result in (("A", a), ("B", b)):
        lines.extend(f"{name}: {issue}" for issue in result["issues"])
    comparable = not mismatch and not a["issues"] and not b["issues"]
    lines.extend(["", "Times in ms. Bootstrap measures the HTTP request; cards start at navigation; actions start at click."])
    header = ["Metric", "A n", "A p50", "A p95", "A max", "B n", "B p50", "B p95", "B max", "p95 change"]
    rows = [header]
    for label in a["metrics"]:
        left, right = a["metrics"][label], b["metrics"][label]
        def values(metric):
            return [str(metric["n"]), *(f"{metric[key]:.1f}" if metric[key] is not None else "n/a"
                                        for key in ("p50", "p95", "max"))]
        change = "n/a"
        if comparable and left["p95"] is not None and left["p95"] > 0 and right["p95"] is not None:
            change = f"{(right['p95'] / left['p95'] - 1) * 100:+.1f}%"
        rows.append([label, *values(left), *values(right), change])
    widths = [max(len(row[column]) for row in rows) for column in range(len(header))]
    lines.extend("  ".join(cell.ljust(width) for cell, width in zip(row, widths)) for row in rows)
    if not comparable:
        lines.append("Percent changes withheld: unmatched, failed, incomplete or unverified trials cannot establish a speedup.")
    lines.append("Percentiles use successful verified actions only; frame opportunity does not mean physical paint.")
    return "\n".join(lines), a["issues"] or b["issues"]


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--allow-mismatch", action="store_true", help="Print unmatched reports without asserting a speedup")
    args = parser.parse_args(argv)
    first, second = load(args.baseline), load(args.candidate)
    mismatch = mismatches(first, second)
    output, invalid = render(first, second, mismatch)
    print(output)
    if mismatch and not args.allow_mismatch:
        print("Comparison rejected. Use --allow-mismatch only to inspect the unmatched reports.", file=sys.stderr)
        return 2
    return 1 if invalid else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, TypeError, KeyError) as error:
        print(f"Cannot compare reports: {error}", file=sys.stderr)
        sys.exit(2)
