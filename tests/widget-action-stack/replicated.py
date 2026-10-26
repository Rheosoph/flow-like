#!/usr/bin/env python3
"""Compare paired application runs across independent local API and executor pools."""

import argparse
import contextlib
import copy
import json
from pathlib import Path
import shutil
import subprocess
import sys

import cold
import compare
import run
import stack
import topology


def trial_config(source, mode, replicas, samples=None, warmup=None):
    if mode not in ("warm", "navigation", "action"):
        raise ValueError("Unknown replicated benchmark mode")
    if not topology.positive_integer(replicas) or replicas > 64:
        raise ValueError("replicas must be between 1 and 64")
    config = json.loads(source.read_text())
    if config.get("cdpUrl"):
        raise ValueError("Replicated trials must own their browsers; remove cdpUrl")
    if config.get("storageState"):
        config["storageState"] = str((source.parent / config["storageState"]).resolve())
    if mode == "warm":
        samples = config.get("samples", 104) if samples is None else samples
        warmup = config.get("warmup", 3) if warmup is None else warmup
        if not topology.positive_integer(samples) or samples % replicas:
            raise ValueError("Warm samples must be a positive multiple of replicas")
        if not topology.positive_integer(warmup):
            raise ValueError("Warm trials require at least one warmup per worker")
    else:
        samples, warmup = replicas, 0
    config.update(samples=samples, warmup=warmup, navigations=1, concurrency=replicas,
                  headed=False, requireActionTrace=True, requireTerminalSuccess=True)
    config.pop("output", None)
    return config


def create_services(settings):
    from replica_services import ReplicaServices
    return ReplicaServices(settings)


def browser_evidence(directory, mode, replicas):
    bundle = compare.load(directory)
    report = bundle["report"]
    issues = list(compare.assessment(bundle)["issues"])
    navigations = report.get("navigations", [])
    measured = [sample for sample in report.get("samples", []) if not sample.get("warmup")]
    warmups = [sample for sample in report.get("samples", []) if sample.get("warmup")]
    if sorted(entry.get("worker", -1) for entry in navigations) != list(range(replicas)):
        issues.append("Every browser worker needs exactly one verified navigation")
    navigation_times = [entry.get("browserTimeOriginMs") for entry in navigations]
    clicks = []
    for sample in measured:
        origin, click = sample.get("browserTimeOriginMs"), sample.get("clickAtNavigationMs")
        if compare.finite(origin) and compare.finite(click):
            clicks.append(origin + click)
    if len(clicks) != len(measured) or not clicks:
        issues.append("Every measured action needs an absolute click timestamp")
    if not navigation_times or not all(compare.finite(value) for value in navigation_times):
        issues.append("Every navigation needs an absolute start timestamp")
    boundary = report.get("coldBoundary")
    if mode == "action":
        if not isinstance(boundary, dict):
            issues.append("Cold actions require a coordinated browser boundary")
        else:
            if (boundary.get("status") != "confirmed" or boundary.get("workerCount") != replicas
                    or boundary.get("bootstrapExecutionSuccessVerified") is not True
                    or boundary.get("controllerRestartConfirmed") is not True
                    or boundary.get("documentPreserved") is not True
                    or boundary.get("requestsCheckedThroughClick") is not True
                    or boundary.get("unexpectedApiRequests") != 0
                    or boundary.get("unexpectedExecutionRequests") != 0):
                issues.append("The coordinated cold-action boundary is unverified")
            proofs = boundary.get("workers", [])
            if sorted(proof.get("worker", -1) for proof in proofs) != list(range(replicas)):
                issues.append("Cold-action boundary is missing distinct worker proofs")
            for proof in proofs:
                if (proof.get("bootstrapExecutionSuccessVerified") is not True
                        or proof.get("documentPreserved") is not True
                        or proof.get("requestsCheckedThroughClick") is not True
                        or proof.get("unexpectedApiRequests") != 0):
                    issues.append("A browser worker failed its cold-action boundary")
            idle = boundary.get("pendingRequestsBeforeBoundary", {})
            if (idle.get("status") != "confirmed" or idle.get("workerCount") != replicas
                    or idle.get("pendingApiRequests") != 0):
                issues.append("Browser API requests were not drained before restart")
    elif boundary is not None:
        issues.append("Only cold-action trials may restart after bootstrap")
    return bundle, {
        "firstMeasuredClickUnixMs": min(clicks) if clicks else None,
        "firstNavigationUnixMs": min(navigation_times) if navigation_times and all(
            compare.finite(value) for value in navigation_times) else None,
        "warmupCount": len(warmups),
    }, issues


def admission_evidence(directory, record, processes):
    path = directory / "admissions.jsonl"
    rows = [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
    bundle = compare.load(directory)
    allowed, invocation_issues = topology.verified_frontend_invocations(
        bundle["report"], record.get("accountingBeforeStop"))
    optional, optional_issues = topology.verified_optional_responses(
        bundle["report"], bundle["manifest"].get("allowedOptionalResponses", []),
        record["firstMeasuredClickUnixMs"])
    validation = topology.validate_admissions(
        rows, processes, record["topology"], allowed_frontend_disconnects=allowed,
        allowed_optional_responses=optional)
    issues = [*invocation_issues, *optional_issues, *validation["issues"]]
    status = record.get("proxyStatus", {})
    if status.get("paused") is not False or status.get("closing") is not False:
        issues.append("The final proxy status must be open and unpaused")
    for pool in topology.POOLS:
        counters = status.get("pools", {}).get(pool, {})
        if counters.get("failed") is not False or any(
                type(counters.get(field)) is not int or counters[field] != 0
                for field in ("active", "queued", "quarantined", "failures", "rejected")):
            issues.append(f"Proxy pool {pool} has missing, pending, or failed requests")
    record["admissionValidation"] = {
        "status": "invalid" if issues else validation["status"], "issues": list(issues),
        "instances": validation["instances"], "requestCount": len(validation["requests"]),
        "clock": validation.get("clock"),
    }
    if record["mode"] == "warm":
        boundary = record["firstMeasuredClickUnixMs"]
        ids = [request["requestId"] for request in validation["requests"]
               if request["successful"] and request["pathCategory"] != "health"
               and (request["pool"] != "runtime" or request["pathCategory"] == "executor_sse")
               and compare.finite(request["releasedAtUnixMs"])
               and request["releasedAtUnixMs"] <= boundary]
        coverage = topology.validate_warm_coverage(validation, processes, ids, boundary)
        record["warmupCoverage"] = {"status": "failed" if coverage else "confirmed",
                                    "requestIds": ids, "issues": coverage}
        issues.extend(coverage)
    else:
        expected = (record["restart"]["processes_after"] if record["mode"] == "action"
                    else record["initialStartup"]["processes"])
        boundary = record["firstMeasuredClickUnixMs"] if record["mode"] == "action" else record["firstNavigationUnixMs"]
        issues.extend(topology.validate_cold_window(validation, expected, record["readyAtUnixMs"], boundary))
    return issues


def run_trial(args, suite, mode, pair, position, variant, config):
    relative = Path(mode) / f"pair-{pair + 1:03d}-{variant}"
    directory = args.output / relative
    directory.mkdir(mode=0o700, parents=True)
    record = {
        "mode": mode, "pair": pair, "position": position, "variant": variant,
        "directory": str(relative), "status": "running", "startedAt": cold.timestamp(),
        "wrapperExitCode": None, "issues": [], "initialStartup": None, "restart": None,
        "restartRequestedAfterBootstrap": False, "cleanup": {"status": "pending"},
    }
    suite["trials"].append(record)

    def persist():
        cold.write_json(directory / "replicated-trial.json", record)
        cold.write_json(args.output / "replicated-results.json", suite)

    persist()
    settings = argparse.Namespace(**vars(args))
    settings.binary = Path(suite["binaries"][variant]["path"])
    settings.web_state = getattr(args, variant + "_web_state", None)
    settings.web_source_root = getattr(args, variant + "_web_source_root", None)
    settings.config, settings.output = directory / "browser-config.json", directory
    cold.write_json(settings.config, config)
    services, interrupted = None, None
    try:
        with (directory / "driver.log").open("w") as log, contextlib.redirect_stdout(log), contextlib.redirect_stderr(log):
            if run.file_hash(settings.binary) != suite["binaries"][variant]["sha256"]:
                raise ValueError(f"{variant} binary changed after suite initialization")
            if suite.get("frontends") and cold.frontend_identity(settings.web_state or args.state) != suite["frontends"][variant]:
                raise ValueError(f"{variant} frontend changed after suite initialization")
            services = create_services(settings)
            services.start()
            record["topology"] = copy.deepcopy(services.topology)
            record["readyAtUnixMs"] = services.ready_at_unix_ms
            record["initialStartup"] = {
                "measurements_ms": dict(services.timings), "processes": copy.deepcopy(services.inventory),
                "readyAtUnixMs": services.ready_at_unix_ms,
            }
            persist()

            def restart():
                record["restartRequestedAfterBootstrap"] = True
                persist()
                record["accountingBeforeRestart"] = cold.wait_for_accounting(args.state, directory, args.timeout)
                record["idleBeforeRestart"] = services.wait_idle()
                record["restart"] = services.restart_backend()
                record["readyAtUnixMs"] = services.ready_at_unix_ms
                persist()

            run.benchmark(settings, phase_handler=restart if mode == "action" else None, driver_output=log)
            record["wrapperExitCode"] = 0
            bundle, evidence, issues = browser_evidence(directory, mode, args.replicas)
            record.update(evidence)
            record["issues"].extend(issues)
            if suite.get("frontends"):
                build = bundle["report"]["web"]["build"]
                if {"sourceSha256": build["source"]["sha256"], "buildId": build["buildId"]} != suite["frontends"][variant]:
                    record["issues"].append("Served frontend differs from the selected build")
            if bundle["manifest"].get("binary_sha256") != suite["binaries"][variant]["sha256"]:
                record["issues"].append("Served binary differs from the selected build")
            if not record["issues"]:
                record["accountingBeforeStop"] = cold.wait_for_accounting(args.state, directory, args.timeout)
                record["proxyStatus"] = services.wait_idle()
    except BaseException as error:
        record["wrapperExitCode"] = error.returncode if isinstance(error, subprocess.CalledProcessError) else 1
        record["issues"].append(f"{type(error).__name__}: {error}")
        if isinstance(error, (KeyboardInterrupt, SystemExit)):
            interrupted = error
    finally:
        if services is not None:
            try:
                services.close()
                exited = all(process.poll() is not None for process in services.processes)
                inventory_removed = not (args.state / "processes.json").exists()
                record["cleanup"] = {"status": "ok" if exited and inventory_removed else "failed",
                                     "ownedServiceProcessesExited": exited, "processInventoryRemoved": inventory_removed}
                if not exited or not inventory_removed:
                    record["issues"].append("Owned service or proxy cleanup did not finish")
            except BaseException as error:
                record["cleanup"] = {"status": "failed", "error": f"{type(error).__name__}: {error}"}
                record["issues"].append("Owned service cleanup failed")
                if isinstance(error, (KeyboardInterrupt, SystemExit)):
                    interrupted = error
            try:
                processes = copy.deepcopy(services.inventory)
                cold.write_json(directory / "processes.json", processes)
                source = Path(services.admission_log)
                destination = directory / "admissions.jsonl"
                if source.resolve() != destination.resolve() and source.is_file():
                    shutil.copyfile(source, destination)
                    destination.chmod(0o600)
                record["evidence"] = {"processesSha256": run.file_hash(directory / "processes.json"),
                                      "admissionsSha256": run.file_hash(destination)}
                if not record["issues"]:
                    record["issues"].extend(admission_evidence(directory, record, processes))
            except BaseException as error:
                record["issues"].append(f"Admission evidence: {type(error).__name__}: {error}")
                if isinstance(error, (KeyboardInterrupt, SystemExit)):
                    interrupted = error
        else:
            record["cleanup"] = {"status": "ok", "ownedServiceProcessesExited": True,
                                 "processInventoryRemoved": not (args.state / "processes.json").exists()}
        record["status"] = "failed" if record["issues"] else "passed"
        record["finishedAt"] = cold.timestamp()
        if record["issues"] and record["wrapperExitCode"] == 0:
            record["wrapperExitCode"] = 1
        persist()
    if interrupted is not None:
        raise interrupted
    return record


def parser():
    command = argparse.ArgumentParser(description=__doc__)
    command.add_argument("--state", type=Path, default=stack.DEFAULT_STATE)
    command.add_argument("--baseline", type=Path, required=True)
    command.add_argument("--candidate", type=Path, required=True)
    for variant in ("baseline", "candidate"):
        command.add_argument(f"--{variant}-web-state", type=Path)
        command.add_argument(f"--{variant}-web-source-root", type=Path)
    command.add_argument("--config", type=Path)
    command.add_argument("--trials", type=int, default=4, help="Paired trials per mode in alternating AB/BA order")
    command.add_argument("--modes", choices=("warm", "navigation", "action"), nargs="+", default=["warm", "navigation", "action"])
    command.add_argument("--replicas", type=int, default=8, help="Processes per API frontend, callback and executor pool")
    command.add_argument("--samples", type=int, help="Warm measured actions; config value or 104, divisible by replicas")
    command.add_argument("--warmup", type=int, help="Warmup rounds per browser; config value or 3")
    command.add_argument("--output", type=Path, required=True)
    command.add_argument("--profile", choices=("debug", "release-fast", "release"), default="release-fast")
    command.add_argument("--web-mode", choices=("production", "development"), default="production")
    command.add_argument("--workers", type=int, default=1, help="Must be one per replica")
    command.add_argument("--executor-slots", type=int, default=1, help="Must be one per replica")
    command.add_argument("--thread-stack-mb", type=int, default=32)
    command.add_argument("--timeout", type=float, default=180)
    command.add_argument("--readiness-poll-ms", type=int, default=20)
    command.add_argument("--trace", action="store_true", help="Diagnostic tracing; keep separate from timing suites")
    return command


def main(argv=None):
    args = parser().parse_args(argv)
    for name in ("trials", "replicas", "thread_stack_mb", "readiness_poll_ms", "timeout"):
        if getattr(args, name) <= 0:
            raise ValueError(f"{name} must be positive")
    if args.replicas > 64 or args.workers != 1 or args.executor_slots != 1:
        raise ValueError("Use at most 64 replicas with one worker and one execution slot per process")
    if len(set(args.modes)) != len(args.modes):
        raise ValueError("Each mode may appear only once")
    args.state, args.output = args.state.resolve(), args.output.resolve()
    for variant in ("baseline", "candidate"):
        for option in ("web_state", "web_source_root"):
            name = f"{variant}_{option}"
            if getattr(args, name) is not None:
                setattr(args, name, getattr(args, name).resolve())
    if (args.state / "processes.json").exists():
        raise ValueError("The fixture owns processes; stop them before running a replicated suite")
    if args.output.exists() and any(args.output.iterdir()):
        raise ValueError("Use a new or empty output directory to preserve previous trials")
    args.config = (args.config or run.default_browser_config(args.state)).resolve()
    configs = {mode: trial_config(args.config, mode, args.replicas, args.samples, args.warmup)
               for mode in set(args.modes) | {"warm"}}
    binaries = {}
    for variant in ("baseline", "candidate"):
        binary = getattr(args, variant).resolve()
        if not binary.is_file():
            raise ValueError(f"{variant} binary does not exist: {binary}")
        binaries[variant] = {"path": str(binary), "sha256": run.file_hash(binary)}
    frontends = {variant: cold.frontend_identity(getattr(args, variant + "_web_state") or args.state)
                 for variant in ("baseline", "candidate")} if args.web_mode == "production" else None
    args.output.mkdir(mode=0o700, parents=True, exist_ok=True)
    suite = {
        "schemaVersion": 1, "mode": "paired-replicated-process", "status": "running", "startedAt": cold.timestamp(),
        "configuration": {"trials": args.trials, "modes": args.modes, "order": "alternating-ab-ba",
                          "replicas": args.replicas, "concurrency": args.replicas, "navigations": 1,
                          "warmSamples": configs["warm"]["samples"], "warmup": configs["warm"]["warmup"],
                          "workerThreads": args.workers, "executorSlots": args.executor_slots,
                          "threadStackMb": args.thread_stack_mb, "readinessPollMs": args.readiness_poll_ms, "trace": args.trace},
        "binaries": binaries, "frontends": frontends,
        "boundaries": {
            "warm": "Independent pools warmed by verified bootstrap and warmup; every replica must finish work before measured clicks",
            "navigation": "Fresh backend pools before concurrent browser navigation",
            "action": "All browser bootstraps verified and accounted, then every backend pool restarted while documents remain open",
            "admission": "One active upstream request per process through response EOF; callbacks use a reserved API pool",
            "persistentCaches": "Database, Redis, object storage and OS caches retained; no Lambda platform or network envelope",
            "accounting": "All verified operations must commit quota and compute completion before restart and shutdown; excluded from UI clocks",
        }, "trials": [],
    }
    cold.write_json(args.output / "replicated-results.json", suite)
    try:
        for mode in args.modes:
            for pair in range(args.trials):
                records = []
                for position, variant in enumerate(cold.trial_order(pair)):
                    print(f"{mode} pair {pair + 1}/{args.trials}: {variant}", flush=True)
                    record = run_trial(args, suite, mode, pair, position, variant, configs[mode])
                    records.append(record)
                    print(f"  {record['status']}: {record['directory']}", flush=True)
                    if record["status"] != "passed":
                        raise ValueError(f"Replicated trial failed; inspect {args.output / record['directory'] / 'replicated-trial.json'}")
                bundles = [compare.load(args.output / item["directory"]) for item in records]
                mismatch = compare.mismatches(*bundles)
                if bundles[0]["manifest"]["fixture"]["database"] != bundles[1]["manifest"]["fixture"]["database"]:
                    mismatch.append("Paired trials changed physical storage layout")
                if mismatch:
                    raise ValueError("Paired trial configuration differs: " + "; ".join(mismatch))
        suite["status"] = "passed"
    except BaseException as error:
        suite["status"] = "failed"
        suite["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        suite["finishedAt"] = cold.timestamp()
        cold.write_json(args.output / "replicated-results.json", suite)
    print(f"Results: {args.output / 'replicated-results.json'}", flush=True)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
