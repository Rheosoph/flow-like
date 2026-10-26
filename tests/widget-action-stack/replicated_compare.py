#!/usr/bin/env python3
"""Compare replicated local trials only after validating every process and request."""

import argparse
from collections import Counter, defaultdict
import hashlib
import json
import math
from pathlib import Path
import sys

import cold_compare
import compare
import fixture_experiment
import topology


MODES = ("warm", "navigation", "action")
PROCESS_FIELDS = ("pool", "replica", "generation", "pid", "started", "binarySha256", "runtimeModel", "workerThreads")


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def is_sha256(value):
    return isinstance(value, str) and len(value) == 64 and all(char in "0123456789abcdef" for char in value)


def click_time(sample):
    origin, relative = sample.get("browserTimeOriginMs"), sample.get("clickAtNavigationMs")
    return origin + relative if compare.finite(origin) and compare.finite(relative) else None


def same_time(left, right):
    return compare.finite(left) and compare.finite(right) and math.isclose(left, right, rel_tol=0, abs_tol=.001)


def process_map(processes):
    if not isinstance(processes, list):
        raise ValueError("process inventory must be a list")
    result = {}
    for process in processes:
        identity = topology.instance_identity(process)
        if identity in result:
            raise ValueError("process snapshot contains duplicate instance identities")
        result[identity] = {field: process.get(field) for field in PROCESS_FIELDS}
    return result


def process_readiness(trial):
    result = {}
    snapshots = [("initial", trial.get("initialStartup"), "processes")]
    if trial.get("restart") is not None:
        snapshots.append(("restart", trial["restart"], "processes_after"))
    for phase, snapshot, field in snapshots:
        if not isinstance(snapshot, dict) or not isinstance(snapshot.get("measurements_ms"), dict):
            raise ValueError(f"{phase}: process readiness timings are missing")
        values, names = defaultdict(list), set()
        for process in snapshot.get(field, []):
            name, pool = process.get("name"), process.get("pool")
            elapsed = snapshot["measurements_ms"].get(name)
            if not isinstance(name, str) or not name or name in names or pool not in topology.POOLS or not compare.finite(elapsed):
                raise ValueError(f"{phase}: process readiness timing is incomplete or duplicated")
            names.add(name)
            values[pool].append(elapsed)
        if set(values) != set(topology.POOLS):
            raise ValueError(f"{phase}: process readiness does not cover every pool")
        result[phase] = {pool: compare.summary(values[pool]) for pool in topology.POOLS}
    return result


def run_ids(entries, issues):
    result = set()
    for entry in entries:
        executions = entry.get("execution", [])
        if not executions or entry.get("executionSuccessVerified") is not True:
            issues.append("browser operation lacks verified execution identifiers")
        for execution in executions:
            identifier = execution.get("runId")
            if execution.get("success") is not True or not isinstance(identifier, str) or not identifier:
                issues.append("browser operation contains unsuccessful or unidentified execution")
            elif identifier in result:
                issues.append("browser operations reuse an execution identifier")
            else:
                result.add(identifier)
    return result


def validate_accounting(trial, report, issues):
    navigation_ids = run_ids(report.get("navigations", []), issues)
    action_ids = run_ids([*report.get("samples", []), *report.get("resets", [])], issues)
    if navigation_ids & action_ids:
        issues.append("bootstrap and actions reuse execution identifiers")
    drains = [("accountingBeforeStop", navigation_ids | action_ids)]
    if trial["mode"] == "action":
        drains.append(("accountingBeforeRestart", navigation_ids))
    for field, expected in drains:
        drain = trial.get(field) or {}
        recorded = drain.get("runIds", [])
        if (not expected or drain.get("status") != "confirmed" or not isinstance(recorded, list)
                or len(recorded) != len(expected) or set(recorded) != expected
                or not compare.finite(drain.get("durationMs")) or not topology.positive_integer(drain.get("polls"))):
            issues.append(f"unverified {field}")
    return navigation_ids | action_ids


def validate_proxy_status(status, records, final_processes, issues):
    if not isinstance(status, dict) or status.get("closing") is not False or status.get("paused") is not False:
        issues.append("missing or unfinished final proxy status")
        return
    pools = status.get("pools")
    if not isinstance(pools, dict) or set(pools) != set(topology.POOLS):
        issues.append("proxy status lacks the exact configured pools")
        return
    for pool, state in pools.items():
        if not isinstance(state, dict):
            issues.append(f"{pool}: invalid proxy status")
            continue
        if state.get("failed") is not False or any(type(state.get(field)) is not int or state[field] != 0
                                                  for field in ("active", "queued", "quarantined", "failures", "rejected")):
            issues.append(f"{pool}: proxy has unfinished, failed or rejected requests")
        for event in ("admitted", "released"):
            count = sum(record.get("pool") == pool and record.get("event") == event for record in records)
            if type(state.get(event)) is not int or state[event] != count:
                issues.append(f"{pool}: {event} counter differs from request evidence")
        targets = state.get("targets", [])
        expected = {(process["replica"], process["generation"]) for process in final_processes if process["pool"] == pool}
        observed = []
        if not isinstance(targets, list):
            issues.append(f"{pool}: invalid target status")
            continue
        for target in targets:
            observed.append((target.get("id"), target.get("generation")))
            if target.get("state") != "idle" or target.get("requestId") is not None:
                issues.append(f"{pool}: target still holds an admission lease")
        if len(observed) != len(expected) or set(observed) != expected:
            issues.append(f"{pool}: proxy targets differ from final process inventory")


def validate_browser_boundary(trial, report, replicas, issues):
    boundary = report.get("coldBoundary")
    if trial["mode"] != "action":
        if boundary is not None:
            issues.append("non-action trial unexpectedly used a cold-action boundary")
        return
    if not isinstance(boundary, dict):
        issues.append("missing all-worker cold-action boundary")
        return
    flags = ("bootstrapExecutionSuccessVerified", "controllerRestartConfirmed", "documentPreserved", "requestsCheckedThroughClick")
    if (boundary.get("mode") != "cold-action" or boundary.get("status") != "confirmed"
            or any(boundary.get(flag) is not True for flag in flags)
            or boundary.get("unexpectedApiRequests") != 0 or boundary.get("unexpectedExecutionRequests") != 0
            or boundary.get("workerCount") != replicas or trial.get("restartRequestedAfterBootstrap") is not True):
        issues.append("unverified all-worker cold-action boundary")
    pending = boundary.get("pendingRequestsBeforeBoundary") or {}
    if (pending.get("status") != "confirmed" or pending.get("workerCount") != replicas
            or pending.get("pendingApiRequests") != 0 or not compare.finite(pending.get("durationMs"))
            or not topology.positive_integer(pending.get("polls"))):
        issues.append("browser did not drain all pending requests before restart")
    workers = boundary.get("workers", [])
    if not isinstance(workers, list) or [entry.get("worker") for entry in workers] != list(range(replicas)):
        issues.append("cold-action boundary does not cover every worker exactly once")
        return
    navigations = {entry["worker"]: entry for entry in report.get("navigations", [])}
    samples = {entry["worker"]: entry for entry in report.get("samples", []) if not entry.get("warmup")}
    started = boundary.get("startedAtUnixMs")
    if not compare.finite(started):
        issues.append("cold-action boundary has no start timestamp")
        return
    for proof in workers:
        worker = proof["worker"]
        sample, navigation = samples.get(worker, {}), navigations.get(worker, {})
        clicked = click_time(sample)
        if (proof.get("navigation") != 0 or proof.get("bootstrapExecutionSuccessVerified") is not True
                or proof.get("documentPreserved") is not True or proof.get("requestsCheckedThroughClick") is not True
                or proof.get("unexpectedApiRequests") != 0 or proof.get("unexpectedExecutionRequests") != 0
                or not same_time(proof.get("documentTimeOriginMs"), navigation.get("browserTimeOriginMs"))
                or not same_time(proof.get("documentTimeOriginMs"), sample.get("browserTimeOriginMs"))
                or not same_time(proof.get("checkedThroughUnixMs"), clicked)
                or not compare.finite(clicked) or clicked < started):
            issues.append(f"worker {worker}: cold-action document/click proof is invalid")
            continue
        if any(record.get("worker") == worker and compare.finite(record.get("startedAtUnixMs"))
               and started <= record["startedAtUnixMs"] < clicked for record in report.get("network", [])):
            issues.append(f"worker {worker}: API traffic occurred before its cold-action click")


def preflight_timing_anomaly(record, preflights):
    elapsed = record.get("totalMs")
    return (isinstance(elapsed, (int, float)) and not isinstance(elapsed, bool) and math.isfinite(elapsed) and elapsed < 0
            and record.get("method") == "OPTIONS" and str(record.get("category")).endswith("_preflight")
            and type(record.get("status")) is int and 200 <= record["status"] < 300
            and not record.get("networkError") and compare.finite(record.get("responseHeadersMs"))
            and record.get("benchmarkRequestId") in preflights
            and preflights[record["benchmarkRequestId"]] == record["status"])


def validate_workload(trial, bundle, config, issues, allowed_disconnects, allowed_optional, preflights):
    report, manifest = bundle["report"], bundle["manifest"]
    replicas, mode = config["replicas"], trial["mode"]
    expected_samples = config["warmSamples"] if mode == "warm" else replicas
    expected_warmup = config["warmup"] if mode == "warm" else 0
    configuration = report.get("configuration", {})
    for field, expected in (("samples", expected_samples), ("warmup", expected_warmup), ("navigations", 1), ("concurrency", replicas)):
        if type(configuration.get(field)) is not int or configuration[field] != expected:
            issues.append(f"browser configuration {field} must equal {expected}")
    if trial.get("warmupCount") != expected_warmup * replicas:
        issues.append("recorded warmup count differs from workload")
    for field, setting in (("worker_threads", "workerThreads"), ("executor_slots", "executorSlots"),
                           ("readiness_poll_ms", "readinessPollMs"), ("trace_requested", "trace")):
        if setting not in config or manifest.get(field) != config[setting]:
            issues.append(f"manifest {field} differs from suite configuration")
    navigations = report.get("navigations", [])
    if (len(navigations) != replicas or {entry.get("worker") for entry in navigations} != set(range(replicas))
            or any(entry.get("index") != 0 for entry in navigations)):
        issues.append("navigation evidence does not cover every browser worker")
    measured = [entry for entry in report.get("samples", []) if not entry.get("warmup")]
    warmups = [entry for entry in report.get("samples", []) if entry.get("warmup")]
    expected_warm_workers = Counter({worker: expected_warmup for worker in range(replicas) if expected_warmup})
    if (Counter(entry.get("worker") for entry in warmups) != expected_warm_workers
            or any(entry.get("navigation") != 0 for entry in warmups)):
        issues.append("warmup evidence does not cover every worker equally")
    expected_resets = max(0, expected_samples // replicas + expected_warmup - 1) if configuration.get("resetBetweenActions") else 0
    expected_reset_workers = Counter({worker: expected_resets for worker in range(replicas) if expected_resets})
    if Counter(entry.get("worker") for entry in report.get("resets", [])) != expected_reset_workers:
        issues.append("reset evidence does not cover every worker equally")
    bursts = defaultdict(list)
    for sample in measured:
        if not isinstance(sample.get("burst"), int) or isinstance(sample["burst"], bool) or sample["burst"] < 0:
            issues.append("measured action has no valid burst identity")
            continue
        bursts[sample["burst"]].append(sample)
        if sample.get("navigation") != 0 or not compare.finite(click_time(sample)):
            issues.append("measured action lacks navigation/click evidence")
    expected_bursts = expected_samples // replicas
    if set(bursts) != set(range(expected_bursts)):
        issues.append("measured burst identities are incomplete")
    for burst, samples in bursts.items():
        if len(samples) != replicas or {sample.get("worker") for sample in samples} != set(range(replicas)):
            issues.append(f"burst {burst}: action evidence does not cover every worker exactly once")
        scheduled = {sample.get("scheduledAtUnixMs") for sample in samples}
        if len(scheduled) != 1 or not all(compare.finite(value) for value in scheduled):
            issues.append(f"burst {burst}: missing common scheduling boundary")
        for sample in samples:
            if (not compare.finite(sample.get("scheduledToClickMs"))
                    or not same_time(sample.get("scheduledAtUnixMs", 0) + sample.get("scheduledToClickMs", 0), click_time(sample))):
                issues.append(f"burst {burst}: scheduled-to-click evidence is inconsistent")
    for worker in range(replicas):
        for phase, category in (("bootstrap", "bootstrap"), ("bootstrap", "prerun"), ("bootstrap", "invoke"),
                                ("action", "prerun"), ("action", "invoke")):
            matching = [record for record in report.get("network", [])
                        if record.get("worker") == worker and record.get("navigation") == 0
                        and record.get("phase") == phase and record.get("category") == category]
            if not matching or any(not compare.finite(record.get("totalMs")) for record in matching):
                issues.append(f"worker {worker}: missing {phase}/{category} HTTP evidence")
    for record in report.get("network", []):
        request_id = record.get("benchmarkRequestId")
        terminal_cancel = (request_id in allowed_disconnects and record.get("cancelled") is True
                           and record.get("networkError") == "net::ERR_ABORTED")
        status_ok = (type(record.get("status")) is int and 200 <= record["status"] < 300) or request_id in allowed_optional
        timing_valid = compare.finite(record.get("totalMs")) or preflight_timing_anomaly(record, preflights)
        if (record.get("networkError") and not terminal_cancel) or not status_ok or not timing_valid:
            issues.append("browser HTTP request failed or lacks complete timing")
    origins = [entry.get("browserTimeOriginMs") for entry in navigations]
    clicks = [click_time(entry) for entry in measured]
    if not origins or not all(compare.finite(value) for value in origins) or not same_time(trial.get("firstNavigationUnixMs"), min(origins)):
        issues.append("first navigation boundary differs from browser evidence")
    if not clicks or not all(compare.finite(value) for value in clicks) or not same_time(trial.get("firstMeasuredClickUnixMs"), min(clicks)):
        issues.append("first measured click differs from browser evidence")


def validate_trial(directory, trial, suite, seen_processes):
    issues = []
    bundle = compare.load(directory)
    report, manifest = bundle["report"], bundle["manifest"]
    issues.extend(compare.assessment(bundle)["issues"])
    issues.extend(fixture_experiment.trial_issues(
        suite, trial["variant"], manifest.get("fixture"), trial.get("fixtureState")))
    if json.loads((directory / "replicated-trial.json").read_text()) != trial:
        issues.append("saved trial evidence differs from suite record")
    arrangement = topology.normalize_topology(manifest)
    replicas = suite["configuration"]["replicas"]
    if (arrangement["kind"] != "replicated-local" or any(arrangement[field] != replicas for field in topology.POOLS.values())
            or trial.get("topology") != arrangement):
        issues.append("trial does not use the configured replicated topology")
    evidence = trial.get("evidence") or {}
    for file, field in (("admissions.jsonl", "admissionsSha256"), ("processes.json", "processesSha256")):
        if not is_sha256(evidence.get(field)) or sha256(directory / file) != evidence[field]:
            issues.append(f"{file} evidence hash mismatch")
    records = [json.loads(line) for line in (directory / "admissions.jsonl").read_text().splitlines() if line.strip()]
    processes = json.loads((directory / "processes.json").read_text())
    allowed_disconnects, cancellation_issues = topology.verified_frontend_invocations(report, trial.get("accountingBeforeStop"))
    issues.extend(cancellation_issues)
    allowed_optional, optional_issues = topology.verified_optional_responses(
        report, manifest.get("allowedOptionalResponses", []), trial.get("firstMeasuredClickUnixMs"))
    issues.extend(optional_issues)
    validation = topology.validate_admissions(records, processes, arrangement, allowed_frontend_disconnects=allowed_disconnects,
                                               allowed_optional_responses=allowed_optional)
    issues.extend(validation["issues"])
    inventory = process_map(processes)
    validation["processReadiness"] = process_readiness(trial)
    initial = process_map((trial.get("initialStartup") or {}).get("processes"))
    if not initial or {identity[2] for identity in initial} != {0}:
        issues.append("initial process snapshot must contain generation zero")
    final, expected_inventory = initial, dict(initial)
    restart = trial.get("restart")
    if trial["mode"] == "action":
        if not isinstance(restart, dict):
            issues.append("cold action has no backend restart")
        else:
            before, final = process_map(restart.get("processes_before")), process_map(restart.get("processes_after"))
            if before != initial or not final or {identity[2] for identity in final} != {1}:
                issues.append("restart did not replace the complete initial generation")
            expected_inventory.update(final)
    elif restart is not None:
        issues.append("non-action trial unexpectedly restarted its backend")
    if inventory != expected_inventory:
        issues.append("process inventory differs from startup/restart snapshots")
    binary_sha = suite.get("binaries", {}).get(trial["variant"], {}).get("sha256")
    if (not is_sha256(binary_sha) or manifest.get("binary_sha256") != binary_sha
            or any(process.get("binarySha256") != binary_sha for process in processes)):
        issues.append("backend binary provenance mismatch")
    for process in processes:
        identity = process.get("pid"), process.get("started")
        if identity in seen_processes:
            issues.append("backend process identity was reused across a restart or trial")
        seen_processes.add(identity)
    build = report.get("web", {}).get("build", {})
    frontend = {"sourceSha256": build.get("source", {}).get("sha256"), "buildId": build.get("buildId")}
    if (not is_sha256(frontend["sourceSha256"]) or not isinstance(frontend["buildId"], str) or not frontend["buildId"]
            or frontend != suite.get("frontends", {}).get(trial["variant"])):
        issues.append("selected frontend provenance mismatch")
    preflights = {request["requestId"]: request["status"] for request in validation["requests"]
                  if request["pool"] in ("api_frontend", "api_callback") and request["method"] == "OPTIONS" and request["successful"]}
    validation["browserTimingAnomalies"] = [
        {field: record.get(field) for field in ("worker", "navigation", "phase", "category", "benchmarkRequestId", "totalMs", "responseHeadersMs")}
        for record in report.get("network", []) if preflight_timing_anomaly(record, preflights)]
    validate_workload(trial, bundle, suite["configuration"], issues, allowed_disconnects, allowed_optional, preflights)
    all_run_ids = validate_accounting(trial, report, issues)
    runtime_requests = [request for request in validation["requests"] if request["pool"] == "runtime" and request["pathCategory"] != "health"]
    if len(runtime_requests) != len(all_run_ids) or any(request["pathCategory"] != "executor_sse" for request in runtime_requests):
        issues.append("executor request count differs from verified browser executions")
    validate_browser_boundary(trial, report, replicas, issues)
    validate_proxy_status(trial.get("proxyStatus"), records, list(final.values()), issues)
    if trial["mode"] == "action":
        validate_proxy_status(trial.get("idleBeforeRestart"), [record for record in records if record.get("generation") == 0],
                              list(initial.values()), issues)
    ready, first_click, first_navigation = (trial.get(field) for field in ("readyAtUnixMs", "firstMeasuredClickUnixMs", "firstNavigationUnixMs"))
    readiness = restart if trial["mode"] == "action" else trial.get("initialStartup")
    if not isinstance(readiness, dict) or not same_time(ready, readiness.get("readyAtUnixMs")):
        issues.append("pool readiness differs from startup/restart evidence")
    if trial["mode"] == "warm":
        selected = [request["requestId"] for request in validation["requests"]
                    if request["successful"] and request["pathCategory"] != "health"
                    and (request["pool"] != "runtime" or request["pathCategory"] == "executor_sse")
                    and compare.finite(request["releasedAtUnixMs"]) and compare.finite(first_click)
                    and request["releasedAtUnixMs"] <= first_click]
        issues.extend(topology.validate_warm_coverage(validation, list(final.values()), selected, first_click))
        if compare.finite(ready) and compare.finite(first_navigation) and ready > first_navigation:
            issues.append("pool readiness occurred after navigation")
    else:
        boundary = first_click if trial["mode"] == "action" else first_navigation
        issues.extend(topology.validate_cold_window(validation, list(final.values()), ready, boundary))
        runtime_counts = Counter()
        for request in validation["requests"]:
            if (request["pool"] == "runtime" and request["pathCategory"] == "executor_sse"
                    and topology.instance_identity(request) in final and compare.finite(request["queuedAtUnixMs"])
                    and compare.finite(boundary) and request["queuedAtUnixMs"] >= boundary
                    and (trial["mode"] == "action" or compare.finite(first_click) and request["queuedAtUnixMs"] < first_click)):
                runtime_counts[topology.instance_identity(request)] += 1
        expected = {identity for identity in final if identity[0] == "runtime"}
        if set(runtime_counts) != expected or any(count != 1 for count in runtime_counts.values()):
            issues.append("cold burst did not execute exactly once on every fresh executor")
    cleanup = trial.get("cleanup") or {}
    if (cleanup.get("status") != "ok" or cleanup.get("ownedServiceProcessesExited") is not True
            or cleanup.get("processInventoryRemoved") is not True):
        issues.append("owned service cleanup is incomplete")
    return bundle, issues, validation


def burst_metrics(report):
    bursts = defaultdict(list)
    for sample in report["samples"]:
        if not sample.get("warmup"):
            bursts[sample["burst"]].append(sample)
    result = []
    for burst, samples in sorted(bursts.items()):
        clicks = [click_time(sample) for sample in samples]
        schedule = samples[0]["scheduledAtUnixMs"]
        result.append({"burst": burst, "actions": len(samples), "clickSpreadMs": max(clicks) - min(clicks),
                       "scheduleToLastHistoryDomMs": max(click_time(sample) + sample["clickToHistoryDomMs"] for sample in samples) - schedule,
                       "scheduleToLastHandlerSettledMs": max(click_time(sample) + sample["clickToSettledMs"] for sample in samples) - schedule})
    return result


def admission_summaries(trial, validation, report):
    start = trial["firstNavigationUnixMs"] if trial["mode"] == "navigation" else trial["firstMeasuredClickUnixMs"]
    end = trial["firstMeasuredClickUnixMs"] if trial["mode"] == "navigation" else math.inf
    pools = {}
    for pool in topology.POOLS:
        requests = [request for request in validation["requests"] if request["pool"] == pool]
        measured = [request for request in requests if compare.finite(request["queuedAtUnixMs"])
                    and start <= request["queuedAtUnixMs"] < end]
        def summarize(selected):
            return {"requests": len(selected), "admissionWaitMs": compare.summary([item.get("admissionWaitMs") for item in selected]),
                    "leaseMs": compare.summary([item.get("leaseMs") for item in selected]),
                    "statusCounts": dict(Counter(str(item.get("status")) for item in selected)),
                    "verifiedTerminalCancellations": sum(item.get("verifiedTerminalCancellation") is True for item in selected),
                    "optionalResponses": sum(item.get("optionalResponse") is True for item in selected)}
        pools[pool] = {"allRequests": summarize(requests), "measurementWindow": summarize(measured)}
    history_ids = {record.get("benchmarkRequestId") for record in report.get("network", [])
                   if record.get("phase") == "action" and record.get("category") == "invoke"}
    history = [request for request in validation["requests"] if request["requestId"] in history_ids and request["pool"] == "api_frontend"]
    return {"status": validation["status"], "instances": validation["instances"], "clock": validation.get("clock"),
            "pools": pools, "browserTimingAnomalies": validation.get("browserTimingAnomalies", []),
            "measuredHistoryFrontendInvocations": summarize(history),
            "measurementWindow": "navigation until first action" if trial["mode"] == "navigation"
            else "first measured click through shutdown; includes resets between bursts"}


def trial_metrics(trial, bundle):
    report = bundle["report"]
    measured = [entry for entry in report["samples"] if not entry.get("warmup")]
    metrics = {}
    selected = report["navigations"] if trial["mode"] == "navigation" else measured
    fields = (("cardsDomMs", "Navigation to cards DOM"), ("bootstrapValidatedAtNavigationMs", "Navigation to onLoad verified")) if trial["mode"] == "navigation" else (
        ("clickToHistoryDomMs", "Click to history DOM"), ("clickToHistoryFrameOpportunityMs", "Click to frame opportunity"),
        ("clickToSettledMs", "Click to handler settled"))
    for field, label in fields:
        values = [entry.get(field) for entry in selected]
        if not values or not all(compare.finite(value) for value in values):
            raise ValueError(f"missing {label} timing")
        metrics[f"{label}: trial median"] = compare.percentile(values, .5)
        metrics[f"{label}: trial maximum"] = max(values)
    phase = "bootstrap" if trial["mode"] == "navigation" else "action"
    for category in (("bootstrap", "prerun", "invoke") if phase == "bootstrap" else ("prerun", "invoke")):
        values = [record["totalMs"] for record in report["network"] if record.get("phase") == phase and record.get("category") == category]
        if not values or not all(compare.finite(value) for value in values):
            raise ValueError(f"missing {phase}/{category} HTTP timing")
        metrics[f"{category} HTTP: trial median"] = compare.percentile(values, .5)
    bursts = burst_metrics(report)
    for field, label in (("clickSpreadMs", "Measured click spread"), ("scheduleToLastHistoryDomMs", "Schedule to last history DOM"),
                         ("scheduleToLastHandlerSettledMs", "Schedule to last handler settled")):
        values = [burst[field] for burst in bursts]
        if values:
            metrics[f"{label}: trial median"] = compare.percentile(values, .5)
            metrics[f"{label}: trial maximum"] = max(values)
    return metrics


def analyze(directory):
    directory = Path(directory).resolve()
    suite = json.loads((directory / "replicated-results.json").read_text())
    config = suite.get("configuration") or {}
    count, replicas, modes = config.get("trials"), config.get("replicas"), config.get("modes")
    if (not topology.positive_integer(count) or not topology.positive_integer(replicas)
            or not isinstance(modes, list) or not modes or len(set(modes)) != len(modes) or any(mode not in MODES for mode in modes)
            or not topology.positive_integer(config.get("warmSamples")) or config["warmSamples"] % replicas
            or type(config.get("warmup")) is not int or config["warmup"] < 0):
        raise ValueError("invalid trial, replica, mode or warm workload configuration")
    issues = fixture_experiment.identity_issues(suite)
    if suite.get("schemaVersion") != 1 or suite.get("mode") != "paired-replicated-process":
        issues.append("unknown replicated suite format")
    if suite.get("status") != "passed":
        issues.append("replicated suite did not finish successfully")
    if (config.get("concurrency") != replicas or config.get("workerThreads") != 1 or config.get("executorSlots") != 1
            or config.get("order") != "alternating-ab-ba"):
        issues.append("suite topology or pairing configuration is invalid")
    trials = suite.get("trials", [])
    expected_order = [(mode, pair, variant) for mode in modes for pair in range(count)
                      for variant in (("baseline", "candidate") if pair % 2 == 0 else ("candidate", "baseline"))]
    if [(trial.get("mode"), trial.get("pair"), trial.get("variant")) for trial in trials] != expected_order:
        issues.append("suite trial sequence is incomplete, duplicated or not alternating AB/BA")
    bundles, entries, seen_processes, references = {}, {}, set(), {}
    requests, descriptive, admissions, bursts, readiness = defaultdict(Counter), defaultdict(list), {}, {}, {}
    repeated = []
    for trial in trials:
        key = trial.get("mode"), trial.get("pair"), trial.get("variant")
        label = "/".join(map(str, key))
        mode, pair, variant = key
        if (mode not in modes or type(pair) is not int or not 0 <= pair < count
                or variant not in ("baseline", "candidate") or key in entries):
            issues.append(f"{label}: invalid or duplicate trial identity")
            continue
        entries[key] = trial
        first = "baseline" if pair % 2 == 0 else "candidate"
        if trial.get("position") != (0 if variant == first else 1):
            issues.append(f"{label}: AB/BA position mismatch")
        if trial.get("status") != "passed" or trial.get("wrapperExitCode") != 0 or trial.get("issues"):
            issues.append(f"{label}: wrapper failed or recorded issues")
        path = (directory / trial.get("directory", "")).resolve()
        if directory not in path.parents:
            issues.append(f"{label}: report directory is outside suite")
            continue
        try:
            bundle, trial_issues, admission = validate_trial(path, trial, suite, seen_processes)
        except (OSError, ValueError, TypeError, KeyError, IndexError) as error:
            issues.append(f"{label}: cannot validate trial: {error}")
            continue
        issues.extend(f"{label}: {issue}" for issue in trial_issues)
        reference = references.setdefault(mode, bundle)
        issues.extend(f"{label}: {issue}" for issue in fixture_experiment.pair_mismatches(
            reference, bundle, layout_experiment=suite.get("fixtureLayoutExperiment") is not None))
        bundles[key] = bundle
        try:
            admissions[label] = admission_summaries(trial, admission, bundle["report"])
            bursts[label] = burst_metrics(bundle["report"])
            readiness[label] = admission["processReadiness"]
        except (KeyError, TypeError, ValueError) as error:
            issues.append(f"{label}: incomplete admission/burst metrics: {error}")
        report = bundle["report"]
        descriptive[(mode, variant)].extend(entry.get("clickToHistoryDomMs") for entry in report["samples"] if not entry.get("warmup") and entry.get("status") == "ok")
        routes = Counter()
        for record in report.get("network", []):
            phase, category = record.get("phase"), record.get("category")
            requests[(mode, variant)][f"{phase}/{category}"] += 1
            if record.get("routeKey") and not str(category).endswith("_preflight"):
                routes[(record.get("worker"), record.get("navigation"), phase, category, record["routeKey"])] += 1
        for (worker, navigation, phase, category, route), repeats in routes.items():
            if repeats > 1:
                repeated.append({"trial": label, "worker": worker, "navigation": navigation, "phase": phase,
                                 "category": category, "routeKey": route, "count": repeats})
    metrics = {}
    for mode in modes:
        pairs = defaultdict(list)
        for pair in range(count):
            keys = [(mode, pair, variant) for variant in ("baseline", "candidate")]
            if any(key not in bundles for key in keys):
                issues.append(f"{mode}/{pair}: missing validated trial pair")
                continue
            try:
                first, second = [trial_metrics(entries[key], bundles[key]) for key in keys]
                for name in first.keys() | second.keys():
                    a, b = first.get(name), second.get(name)
                    if not compare.finite(a) or not compare.finite(b):
                        issues.append(f"{mode}/{pair}: missing {name} metric")
                    else:
                        pairs[name].append((a, b))
            except (ValueError, TypeError, KeyError, IndexError) as error:
                issues.append(f"{mode}/{pair}: invalid metric evidence: {error}")
        metrics[mode] = {name: cold_compare.paired_summary(values) for name, values in sorted(pairs.items())}
    if issues:
        for measurements in metrics.values():
            for metric in measurements.values():
                metric["pairedMedianDeltaMs"] = None
                metric["pairedMedianDelta95CiMs"] = None
    anomalies = sum(len(trial.get("browserTimingAnomalies", [])) for trial in admissions.values())
    warnings = ([f"{anomalies} synthetic CDP preflight timing anomalies retained and excluded from latency metrics; matching proxy responses completed successfully."]
                if anomalies else [])
    return {"schemaVersion": 1, "status": "invalid" if issues else "passed", "issues": issues, "warnings": warnings,
            "configuration": config, "fixtureLayoutExperiment": suite.get("fixtureLayoutExperiment"),
            "metrics": metrics, "admissions": admissions, "bursts": bursts, "processReadiness": readiness,
            "actionLatencyDescriptive": {"/".join(key): compare.summary(values) for key, values in descriptive.items()},
            "requestCounts": {"/".join(key): dict(value) for key, value in requests.items()},
            "repeatedRoutesWithinWorkerPhase": repeated,
            "notes": ["Each paired observation is one complete trial; simultaneous browser actions are correlated.",
                      "Within-trial medians and maxima are paired; action p50/p95 summaries are descriptive only.",
                      "Differences are candidate minus baseline. Intervals resample trial pairs with a fixed seed.",
                      "Click spread is reported separately; maximum individual latency is not burst makespan.",
                      "Process readiness reports each process launch-to-health time by pool; it is separate from browser clocks and is not summed into Lambda startup latency.",
                      "Local processes share a host, database, Redis and object storage; CPU and memory quotas are not enforced.",
                      "Fresh processes retain infrastructure and OS caches. This does not simulate Lambda provisioning latency.",
                      "Any evidence failure withholds all paired differences and intervals."]}


def render(result):
    lines = [f"Replicated local comparison: {result['status']}"]
    if result.get("fixtureLayoutExperiment"):
        lines.append("Experiment: physical fixture layout; backend and frontend held fixed")
    lines.extend("  " + issue for issue in result["issues"])
    lines.extend("  Warning: " + warning for warning in result.get("warnings", []))
    for mode, measurements in result["metrics"].items():
        lines.extend(["", mode])
        for name, metric in measurements.items():
            delta, interval = metric["pairedMedianDeltaMs"], metric["pairedMedianDelta95CiMs"]
            effect = f"{delta:+.2f} ms" if delta is not None else "withheld"
            confidence = f" [{interval[0]:+.2f}, {interval[1]:+.2f}]" if interval is not None else ""
            lines.append(f"  {name}: {metric['n']} trial pairs, delta {effect}{confidence}")
    lines.extend(["", *result["notes"]])
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
    return 1 if result["issues"] else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"Cannot compare replicated trials: {error}", file=sys.stderr)
        sys.exit(2)
