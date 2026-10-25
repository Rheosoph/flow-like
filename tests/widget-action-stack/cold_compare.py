#!/usr/bin/env python3
"""Validate paired process-cold trials and report paired latency differences."""

import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path
import random
import sys

import compare


def paired_summary(pairs):
    differences = [candidate - baseline for baseline, candidate in pairs]
    if not differences:
        return {"n": 0, "baseline": compare.summary([]), "candidate": compare.summary([]),
                "pairedMedianDeltaMs": None, "pairedMedianDelta95CiMs": None}
    interval = None
    if len(differences) >= 4:
        rng = random.Random(74321)
        medians = [compare.percentile(rng.choices(differences, k=len(differences)), .5)
                   for _ in range(5000)]
        interval = [compare.percentile(medians, .025), compare.percentile(medians, .975)]
    return {"n": len(pairs), "baseline": compare.summary([pair[0] for pair in pairs]),
            "candidate": compare.summary([pair[1] for pair in pairs]),
            "pairedMedianDeltaMs": compare.percentile(differences, .5),
            "pairedMedianDelta95CiMs": interval}


def process_identities(snapshot, label, issues):
    identities = {}
    for process in snapshot or []:
        name = process.get("name")
        if name not in ("api", "runtime"):
            continue
        if name in identities:
            issues.append(f"{label}: duplicate {name} identity")
        pid, started = process.get("pid"), process.get("started")
        if not isinstance(pid, int) or isinstance(pid, bool) or pid <= 0 or not started:
            issues.append(f"{label}: missing {name} process identity")
        identities[name] = (name, pid, started)
    if set(identities) != {"api", "runtime"}:
        issues.append(f"{label}: API/runtime process identities required")
    return identities


def trial_metrics(trial, bundle):
    report = bundle["report"]
    navigation = report["navigations"][0]
    action = [sample for sample in report["samples"] if not sample.get("warmup")][0]
    startup = (trial.get("restart") if trial["mode"] == "action" else trial["initialStartup"]) or {}
    timings = startup.get("measurements_ms", {})
    metrics = {"API process readiness": timings.get("api"),
               "Executor process readiness": timings.get("runtime")}
    phase = "bootstrap" if trial["mode"] == "navigation" else "action"
    for category, label in (("bootstrap", "Bootstrap HTTP"), ("prerun", "Prerun HTTP"), ("invoke", "Invoke HTTP")):
        values = [record.get("totalMs") for record in report["network"]
                  if record.get("phase") == phase and record.get("category") == category]
        values = [value for value in values if compare.finite(value)]
        if values:
            metrics[label] = max(values)
    if trial["mode"] == "navigation":
        metrics.update({"Navigation to cards DOM": navigation.get("cardsDomMs"),
                        "Navigation to onLoad verified": navigation.get("bootstrapValidatedAtNavigationMs")})
    else:
        metrics.update({"Click to history DOM": action.get("clickToHistoryDomMs"),
                        "Click to frame opportunity": action.get("clickToHistoryFrameOpportunityMs"),
                        "Click to handler settled": action.get("clickToSettledMs")})
    return metrics


def analyze(directory):
    directory = Path(directory).resolve()
    suite = json.loads((directory / "cold-results.json").read_text())
    issues = []
    config = suite.get("configuration", {})
    count, modes = config.get("trials"), config.get("modes", [])
    if not isinstance(count, int) or isinstance(count, bool) or count < 1:
        raise ValueError("positive trial count required")
    if not modes or len(set(modes)) != len(modes) or any(mode not in ("navigation", "action") for mode in modes):
        raise ValueError("unique navigation/action modes required")
    if suite.get("schemaVersion") != 1 or suite.get("mode") != "paired-process-cold":
        issues.append("unknown suite format")
    if suite.get("status") != "passed":
        issues.append("suite did not finish successfully")
    trials = suite.get("trials", [])
    if len(trials) != count * len(modes) * 2:
        issues.append("suite is incomplete")
    entries, bundles, seen_processes, references = {}, {}, set(), {}
    requests = defaultdict(Counter)
    duplicate_requests = []
    for trial in trials:
        mode, pair, variant = trial.get("mode"), trial.get("pair"), trial.get("variant")
        label = f"{mode}/{pair}/{variant}"
        key = (mode, pair, variant)
        if mode not in modes or not isinstance(pair, int) or not 0 <= pair < count or variant not in ("baseline", "candidate"):
            issues.append(f"{label}: invalid trial identity")
            continue
        if key in entries:
            issues.append(f"{label}: duplicate trial identity")
        entries[key] = trial
        expected_first = "baseline" if pair % 2 == 0 else "candidate"
        if trial.get("position") != (0 if variant == expected_first else 1):
            issues.append(f"{label}: AB/BA order mismatch")
        if trial.get("status") != "passed" or trial.get("wrapperExitCode") != 0 or trial.get("issues"):
            issues.append(f"{label}: wrapper failed or recorded issues")
        cleanup = trial.get("cleanup", {})
        if cleanup.get("status") != "ok" or cleanup.get("ownedServiceProcessesExited") is not True:
            issues.append(f"{label}: incomplete cleanup")
        initial = process_identities(trial.get("initialStartup", {}).get("processes"), label, issues)
        for identity in initial.values():
            if identity in seen_processes:
                issues.append(f"{label}: reused process identity")
            seen_processes.add(identity)
        restart = trial.get("restart")
        if mode == "action":
            if not restart:
                issues.append(f"{label}: missing backend restart")
            else:
                before = process_identities(restart.get("processes_before"), label, issues)
                after = process_identities(restart.get("processes_after"), label, issues)
                if before != initial:
                    issues.append(f"{label}: restart did not replace the initial backend")
                for identity in after.values():
                    if identity in seen_processes:
                        issues.append(f"{label}: restarted process was reused")
                    seen_processes.add(identity)
        elif restart is not None:
            issues.append(f"{label}: navigation trial unexpectedly restarted after bootstrap")
        path = (directory / trial.get("directory", "")).resolve()
        if directory not in path.parents:
            issues.append(f"{label}: report directory is outside suite")
            continue
        try:
            bundle = compare.load(path)
            assessment = compare.assessment(bundle)
        except (OSError, ValueError, KeyError, TypeError) as error:
            issues.append(f"{label}: cannot assess report: {error}")
            continue
        issues.extend(f"{label}: {issue}" for issue in assessment["issues"])
        configuration = bundle["report"].get("configuration", {})
        for name, expected in (("samples", 1), ("warmup", 0), ("navigations", 1), ("concurrency", 1)):
            if configuration.get(name) != expected:
                issues.append(f"{label}: {name} must equal {expected}")
        expected_hash = suite.get("binaries", {}).get(variant, {}).get("sha256")
        if not expected_hash or bundle["manifest"].get("binary_sha256") != expected_hash:
            issues.append(f"{label}: binary provenance mismatch")
        if mode == "action":
            boundary = bundle["report"].get("coldBoundary", {})
            if (boundary.get("status") != "confirmed" or boundary.get("mode") != "cold-action"
                    or boundary.get("bootstrapExecutionSuccessVerified") is not True
                    or boundary.get("controllerRestartConfirmed") is not True
                    or boundary.get("documentPreserved") is not True
                    or boundary.get("unexpectedExecutionRequests") != 0
                    or trial.get("restartRequestedAfterBootstrap") is not True):
                issues.append(f"{label}: unverified cold-action boundary")
        if mode in references:
            issues.extend(f"{label}: {issue}" for issue in compare.mismatches(references[mode], bundle))
        else:
            references[mode] = bundle
            issues.extend(f"{label}: {issue}" for issue in compare.mismatches(bundle, bundle))
        bundles[key] = bundle
        routes = Counter()
        for record in bundle["report"].get("network", []):
            phase, category = record.get("phase"), record.get("category")
            requests[(mode, variant)][f"{phase}/{category}"] += 1
            if record.get("routeKey") and not str(category).endswith("_preflight"):
                routes[(phase, category, record["routeKey"])] += 1
        for (phase, category, route), repeats in routes.items():
            if repeats > 1:
                duplicate_requests.append({"trial": label, "phase": phase, "category": category,
                                           "routeKey": route, "count": repeats})
    metrics = {}
    for mode in modes:
        pairs = defaultdict(list)
        for pair in range(count):
            keys = [(mode, pair, variant) for variant in ("baseline", "candidate")]
            if any(key not in bundles for key in keys):
                issues.append(f"{mode}/{pair}: missing valid pair")
                continue
            try:
                first, second = [trial_metrics(entries[key], bundles[key]) for key in keys]
                for name in first.keys() | second.keys():
                    a, b = first.get(name), second.get(name)
                    if not compare.finite(a) or not compare.finite(b):
                        issues.append(f"{mode}/{pair}: missing {name} timing")
                    else:
                        pairs[name].append((a, b))
            except (KeyError, IndexError, TypeError) as error:
                issues.append(f"{mode}/{pair}: invalid metric data: {error}")
        metrics[mode] = {name: paired_summary(values) for name, values in pairs.items()}
    return {"schemaVersion": 1, "status": "passed" if not issues else "invalid", "issues": issues,
            "configuration": config, "boundaries": suite.get("boundaries", {}), "metrics": metrics,
            "requestCounts": {"/".join(key): dict(value) for key, value in requests.items()},
            "repeatedRoutesWithinPhase": duplicate_requests,
            "notes": ["Differences are candidate minus baseline within each pair; negative is faster.",
                      "95% intervals bootstrap paired median differences with a fixed seed, 5000 resamples.",
                      "Few cold trials give exploratory intervals and unstable p95 estimates, not a production SLA.",
                      "Readiness polling and process launch are separate from navigation/click clocks.",
                      "Database, storage, Redis and OS caches are retained; no Lambda cold-start envelope is simulated.",
                      "Repeated route keys identify requests to inspect, not proof they are redundant."]}


def render(result):
    lines = [f"Paired cold comparison: {result['status']}"]
    lines.extend("  " + issue for issue in result["issues"])
    for mode, metrics in result["metrics"].items():
        lines.extend(["", mode])
        rows = [["Metric", "Pairs", "Baseline p50", "Candidate p50", "Paired delta", "95% interval"]]
        for name, metric in metrics.items():
            interval = metric["pairedMedianDelta95CiMs"]
            valid = not result["issues"]
            rows.append([name, str(metric["n"]), f"{metric['baseline']['p50']:.1f}",
                         f"{metric['candidate']['p50']:.1f}",
                         f"{metric['pairedMedianDeltaMs']:+.1f}" if valid else "withheld",
                         f"[{interval[0]:+.1f}, {interval[1]:+.1f}]" if valid and interval else "n/a"])
        widths = [max(len(row[i]) for row in rows) for i in range(len(rows[0]))]
        lines.extend("  ".join(cell.ljust(width) for cell, width in zip(row, widths)) for row in rows)
    lines.extend(["", "All times are milliseconds.", *result["notes"]])
    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("suite", type=Path)
    parser.add_argument("--json-output", type=Path)
    args = parser.parse_args(argv)
    result = analyze(args.suite)
    if args.json_output:
        args.json_output.write_text(json.dumps(result, indent=2) + "\n")
    print(render(result))
    return 0 if not result["issues"] else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"Cannot compare cold trials: {error}", file=sys.stderr)
        sys.exit(2)
