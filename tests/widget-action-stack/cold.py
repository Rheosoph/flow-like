#!/usr/bin/env python3
"""Compare fresh local backend processes with paired runs of the actual application."""

import argparse
import contextlib
from datetime import datetime, timezone
import json
from pathlib import Path
import subprocess
import sys

import compare
import run
import stack


def timestamp():
    return datetime.now(timezone.utc).isoformat()


def write_json(path, value):
    stack.private_write(path, json.dumps(value, indent=2) + "\n")


def trial_config(source):
    config = json.loads(source.read_text())
    if config.get("cdpUrl"):
        raise ValueError("Cold trials must own their browser; remove cdpUrl from the configuration")
    config.update(samples=1, warmup=0, navigations=1, concurrency=1, headed=False,
                  requireActionTrace=True, requireTerminalSuccess=True)
    config.pop("output", None)
    return config


def trial_order(pair):
    return ("baseline", "candidate") if pair % 2 == 0 else ("candidate", "baseline")


def assess_trial(directory, mode):
    bundle = compare.load(directory)
    issues = list(compare.assessment(bundle)["issues"])
    boundary = bundle["report"].get("coldBoundary")
    if mode == "action":
        if not isinstance(boundary, dict) or any((
            boundary.get("status") != "confirmed",
            boundary.get("bootstrapExecutionSuccessVerified") is not True,
            boundary.get("controllerRestartConfirmed") is not True,
            boundary.get("documentPreserved") is not True,
            boundary.get("unexpectedExecutionRequests") != 0,
            boundary.get("unexpectedApiRequests") != 0,
            boundary.get("requestsCheckedThroughClick") is not True,
        )):
            issues.append("The browser did not verify the cold-action boundary")
    elif boundary is not None:
        issues.append("Navigation trial unexpectedly used a cold-action boundary")
    return issues


def run_trial(args, suite, mode, pair, position, variant, config):
    relative = Path(mode) / f"pair-{pair + 1:03d}-{variant}"
    directory = args.output / relative
    directory.mkdir(mode=0o700, parents=True)
    record = {
        "mode": mode, "pair": pair, "position": position, "variant": variant,
        "directory": str(relative), "status": "running", "startedAt": timestamp(),
        "wrapperExitCode": None, "issues": [], "initialStartup": None, "restart": None,
        "restartRequestedAfterBootstrap": False, "cleanup": {"status": "pending"},
    }
    suite["trials"].append(record)
    write_json(args.output / "cold-results.json", suite)
    write_json(directory / "cold-trial.json", record)
    settings = argparse.Namespace(**vars(args))
    settings.binary = Path(suite["binaries"][variant]["path"])
    settings.config = directory / "browser-config.json"
    settings.output = directory
    write_json(settings.config, config)
    services = None
    interrupted = None
    try:
        with (directory / "driver.log").open("w") as log, contextlib.redirect_stdout(log), contextlib.redirect_stderr(log):
            if run.file_hash(settings.binary) != suite["binaries"][variant]["sha256"]:
                raise ValueError(f"{variant} binary changed after the suite began")
            services = run.Services(settings)
            services.log_directory = directory / "logs"
            services.diagnostics["logs_directory"] = str(services.log_directory)
            services.start()
            record["initialStartup"] = {
                "measurements_ms": dict(services.timings),
                "processes": [dict(item) for item in services.records if item["name"] in ("api", "runtime")],
            }
            write_json(directory / "initial-startup-timings.json", record["initialStartup"])
            write_json(directory / "cold-trial.json", record)

            def restart():
                record["restartRequestedAfterBootstrap"] = True
                write_json(directory / "cold-trial.json", record)
                record["restart"] = services.restart_backend()
                write_json(directory / "cold-trial.json", record)

            log.flush()
            run.benchmark(settings, phase_handler=restart if mode == "action" else None, driver_output=log)
            record["wrapperExitCode"] = 0
            record["issues"].extend(assess_trial(directory, mode))
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
                record["cleanup"] = {
                    "status": "ok" if exited and inventory_removed else "failed",
                    "ownedServiceProcessesExited": exited,
                    "processInventoryRemoved": inventory_removed,
                }
                if not exited or not inventory_removed:
                    record["issues"].append("Owned service cleanup did not finish")
            except BaseException as error:
                record["cleanup"] = {"status": "failed", "error": f"{type(error).__name__}: {error}"}
                record["issues"].append("Owned service cleanup failed")
                if isinstance(error, (KeyboardInterrupt, SystemExit)):
                    interrupted = error
        else:
            record["cleanup"] = {"status": "ok", "ownedServiceProcessesExited": True,
                                 "processInventoryRemoved": not (args.state / "processes.json").exists()}
        record["status"] = "failed" if record["issues"] else "passed"
        record["finishedAt"] = timestamp()
        if record["issues"] and record["wrapperExitCode"] == 0:
            record["wrapperExitCode"] = 1
        write_json(directory / "cold-trial.json", record)
        write_json(args.output / "cold-results.json", suite)
    if interrupted is not None:
        raise interrupted
    return record


def parser():
    command = argparse.ArgumentParser(description=__doc__)
    command.add_argument("--state", type=Path, default=stack.DEFAULT_STATE)
    command.add_argument("--baseline", type=Path, required=True)
    command.add_argument("--candidate", type=Path, required=True)
    command.add_argument("--config", type=Path)
    command.add_argument("--trials", type=int, default=12, help="Paired cycles per mode, alternating AB and BA")
    command.add_argument("--modes", choices=("navigation", "action"), nargs="+", default=["navigation", "action"])
    command.add_argument("--output", type=Path, required=True)
    command.add_argument("--profile", choices=("debug", "release-fast", "release"), default="release-fast")
    command.add_argument("--web-mode", choices=("production", "development"), default="production")
    command.add_argument("--workers", type=int, default=1)
    command.add_argument("--executor-slots", type=int, default=8)
    command.add_argument("--thread-stack-mb", type=int, default=32)
    command.add_argument("--timeout", type=float, default=180, help="Service readiness and cold-action handshake deadline in seconds")
    command.add_argument("--readiness-poll-ms", type=int, default=20)
    command.add_argument("--trace", action="store_true", help="Diagnostic run with API tracing; keep separate from timing suites")
    return command


def main(argv=None):
    args = parser().parse_args(argv)
    for name in ("trials", "workers", "executor_slots", "thread_stack_mb", "readiness_poll_ms", "timeout"):
        if getattr(args, name) <= 0:
            raise ValueError(f"{name} must be positive")
    if len(set(args.modes)) != len(args.modes):
        raise ValueError("Each mode may appear only once")
    args.state, args.output = args.state.resolve(), args.output.resolve()
    if (args.state / "processes.json").exists():
        raise ValueError("The fixture already owns processes; stop them before running cold trials")
    if args.output.exists() and any(args.output.iterdir()):
        raise ValueError("Use a new or empty output directory to preserve previous trials")
    args.config = (args.config or run.default_browser_config(args.state)).resolve()
    config = trial_config(args.config)
    binaries = {}
    for variant in ("baseline", "candidate"):
        binary = getattr(args, variant).resolve()
        if not binary.is_file():
            raise ValueError(f"{variant} binary does not exist: {binary}")
        binaries[variant] = {"path": str(binary), "sha256": run.file_hash(binary)}
    args.output.mkdir(mode=0o700, parents=True, exist_ok=True)
    suite = {
        "schemaVersion": 1, "mode": "paired-process-cold", "status": "running", "startedAt": timestamp(),
        "configuration": {"trials": args.trials, "modes": args.modes, "order": "alternating AB/BA pairs",
                          "samplesPerTrial": 1, "warmup": 0, "navigations": 1, "concurrency": 1,
                          "workerThreads": args.workers, "executorSlots": args.executor_slots,
                          "readinessPollMs": args.readiness_poll_ms, "trace": args.trace},
        "binaries": binaries,
        "boundaries": {
            "navigation": "Fresh owned API/runtime before browser navigation; normal bootstrap and onLoad run first",
            "action": "Bootstrap terminal-verified, then owned API/runtime restarted before the first click; browser document retained",
            "readiness": "Process launch to successful HTTP health response, measured separately and excluded from navigation/click clocks",
            "readinessPollingIntervalMs": args.readiness_poll_ms,
            "persistentCaches": "Storage, database, Redis, OS and filesystem caches retained; no disk or database flushing",
            "compute": "Local HTTP production routers with compute accounting; no Lambda process/container/network startup envelope",
        },
        "trials": [],
    }
    write_json(args.output / "cold-results.json", suite)
    try:
        for mode in args.modes:
            for pair in range(args.trials):
                records = []
                for position, variant in enumerate(trial_order(pair)):
                    print(f"{mode} pair {pair + 1}/{args.trials}: {variant}", flush=True)
                    record = run_trial(args, suite, mode, pair, position, variant, config)
                    records.append(record)
                    print(f"  {record['status']}: {record['directory']}", flush=True)
                    if record["status"] != "passed":
                        raise ValueError(f"Cold trial failed; inspect {args.output / record['directory'] / 'cold-trial.json'}")
                mismatch = compare.mismatches(*(compare.load(args.output / item["directory"]) for item in records))
                if mismatch:
                    raise ValueError("Paired trial configuration differs: " + "; ".join(mismatch))
        suite["status"] = "passed"
    except BaseException as error:
        suite["status"] = "failed"
        suite["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        suite["finishedAt"] = timestamp()
        write_json(args.output / "cold-results.json", suite)
    print(f"Results: {args.output / 'cold-results.json'}", flush=True)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
