#!/usr/bin/env python3
"""Prepare and run the real local widget-action pipeline with owned process groups."""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import signal
import socket
import ssl
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from urllib.parse import urlencode, urlsplit

import stack


def script(name, *args):
    return [sys.executable, str(stack.HERE / name), *map(str, args)]


def execute(command, state, timings, name, cwd=None, dependencies=False):
    started = time.perf_counter()
    env = stack.dependency_environment(state) if dependencies else stack.environment(state)
    stack.run(command, cwd=cwd or stack.REPO, env=env)
    timings[name] = round((time.perf_counter() - started) * 1000, 3)


def write_timings(state, kind, timings):
    destination = state / f"{kind}-timings.json"
    destination.write_text(json.dumps({"kind": kind, "measurements_ms": timings}, indent=2) + "\n")


def file_hash(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def service_diagnostics(state, env, trace=False):
    platform_path = Path(env.get("FLOW_LIKE_CONFIG_FILE", state / "platform.json"))
    platform = json.loads(platform_path.read_text())
    log_directory = state / "logs"
    if trace:
        traces = state / "traces"
        traces.mkdir(mode=0o700, exist_ok=True)
        directory = Path(tempfile.mkdtemp(prefix="run-", dir=traces))
        platform.setdefault("features", {})["telemetry"] = True
        platform_path = directory / "platform.json"
        stack.private_write(platform_path, json.dumps(platform, indent=2) + "\n")
        env["FLOW_LIKE_CONFIG_FILE"] = str(platform_path)
        env["FLOW_LIKE_TRACE_SAMPLE_RATE"] = "1"
        env["FLOW_LIKE_TELEMETRY_SINK"] = "log"
        env["FLOW_LIKE_BENCHMARK_TRACE"] = "1"
        log_directory = directory / "logs"
    try:
        sample_rate = float(env.get("FLOW_LIKE_TRACE_SAMPLE_RATE", "0.05"))
        sample_rate = min(1.0, max(0.0, sample_rate)) if math.isfinite(sample_rate) else 0.05
    except ValueError:
        sample_rate = 0.05
    return log_directory, {
        "trace_requested": trace,
        "telemetry_enabled": bool(platform.get("features", {}).get("telemetry", False)),
        "telemetry_sink": env.get("FLOW_LIKE_TELEMETRY_SINK", "db").lower(),
        "backend_trace_sample_rate": sample_rate,
        "rust_log": env.get("RUST_LOG", ""),
        "platform_config": str(platform_path),
        "logs_directory": str(log_directory),
        "trace_export_drain_ms": 5100 if trace else 0,
    }


def browser_config(state, fixture):
    index = json.loads((fixture / "fixture-index.json").read_text())
    browser = index["browser"]
    card = browser["person_widget_id"]
    row = browser["history_widget_id"]
    history = browser["history_element_id"]
    config = {
        "appUrl": "http://127.0.0.1:3440" + browser["route"] + "?" + urlencode(browser["query"]),
        "storageState": str(state / "storage-state.json"), "samples": 20, "warmup": 2,
        "navigations": 2, "timeoutMs": 30000, "requireActionTrace": True,
        "reset": {"selector": '[data-a2ui-element-ref$="/ps-detail-back-btn"]',
                  "text": "Zurück zur Personen-Übersicht (Swimlanes)"},
        "actions": [{
            "selector": f'[data-widget-id="{card}"] > [data-a2ui-element-ref$="/{person["id"]}"]',
            "text": person["name"],
            "historySelector": f'[data-a2ui-element-ref$="/{history}"]',
            "rowSelector": f'[data-widget-id="{row}"] > :first-child',
            "expectedText": person["expected_history_text"], "expectedRows": person["expected_rows"],
        } for person in browser["persons"]],
    }
    destination = fixture / "browser-config.json"
    stack.private_write(destination, json.dumps(config, indent=2) + "\n")
    stack.private_write(state / "browser-config-path.json", json.dumps({
        "path": str(destination.resolve()),
    }, indent=2) + "\n")
    return destination


def fixture_directory(args):
    fixture = (args.fixture or args.state / "fixture").resolve()
    if not args.metadata_export:
        index_path = fixture / "fixture-index.json"
        if not index_path.is_file():
            raise ValueError("Pass --metadata-export with a local app export or --fixture with a prepared fixture")
        database = json.loads(index_path.read_text())["database"]
        for name, expected in (("profile", args.fixture_profile), ("layout", args.fixture_layout)):
            if expected is not None and database.get(name) != expected:
                raise ValueError(f"Existing fixture {name} is {database.get(name)!r}, requested {expected!r}; "
                                 "provide --metadata-export to regenerate it or select a matching --fixture")
    return fixture


def prepare(args):
    state = args.state
    fixture = fixture_directory(args)
    timings = {}
    stack.run(script("stack.py", "--state", state, "prepare"))
    execute(script("stack.py", "--state", state, "up"), state, timings, "infrastructure_and_schema", dependencies=True)
    execute(["node", str(stack.HERE / "auth.mjs"), "prepare", str(state)], state, timings, "authentication")
    if args.metadata_export:
        command = ["uv", "run", "--script", str(stack.HERE / "fixture.py"), "--metadata-export",
                   str(args.metadata_export.resolve()), "--output", str(fixture), "--profile", args.fixture_profile or "production"]
        if args.fixture_layout is not None:
            command.extend(["--layout", args.fixture_layout])
        execute(command, state, timings, "sanitize_and_seed_data", dependencies=True)
    if not (fixture / "fixture-index.json").is_file():
        raise ValueError("Pass --metadata-export with a local app export or --fixture with a prepared fixture")
    execute(script("import_fixture.py", fixture, "--state", state), state, timings, "storage_and_database_import")
    browser_config(state, fixture)
    execute(["node", str(stack.HERE / "web.mjs"), "prepare", "--state", str(state)],
            state, timings, "web_configuration", dependencies=True)
    write_timings(state, "prepare", timings)
    print(f"Local fixture ready at {state}")


def target_binary(profile):
    target = Path(os.environ.get("CARGO_TARGET_DIR", stack.REPO / "target"))
    if not target.is_absolute():
        target = stack.REPO / target
    if os.environ.get("CARGO_BUILD_TARGET"):
        target /= os.environ["CARGO_BUILD_TARGET"]
    return target.resolve() / profile / "examples/widget_action_stack"


def built_binary(state, profile):
    record = state / "build-backend.json"
    if record.is_file():
        value = json.loads(record.read_text())
        if value.get("repository") == str(stack.REPO) and value.get("profile") == profile:
            return Path(value["binary"])
    return target_binary(profile)


def build(args):
    stack.load(args.state)
    timings = {}
    if args.part in ("all", "backend"):
        command = ["cargo", "build", "-p", "flow-like-api", "--example", "widget_action_stack",
                   "--no-default-features", "--features", "benchmark-stack", "--locked"]
        if args.profile != "debug":
            command.extend(["--profile", args.profile])
        # Build commands keep the developer's compiler/cache settings; launched services use isolated settings.
        started = time.perf_counter()
        stack.run(command, cwd=stack.REPO)
        timings["backend_build"] = round((time.perf_counter() - started) * 1000, 3)
        stack.private_write(args.state / "build-backend.json", json.dumps({
            "repository": str(stack.REPO), "profile": args.profile, "binary": str(target_binary(args.profile)),
        }, indent=2) + "\n")
    if args.part in ("all", "web"):
        execute(["node", str(stack.HERE / "web.mjs"), "build", "--state", str(args.state)],
                args.state, timings, "web_build", dependencies=True)
    write_timings(args.state, "build", timings)


def process_started(pid):
    result = subprocess.run(["ps", "-p", str(pid), "-o", "lstart="], capture_output=True, text=True)
    return result.stdout.strip() if result.returncode == 0 else None


def stop_record(record):
    pid = record["pid"]
    if not record.get("started") or process_started(pid) != record["started"]:
        return
    try:
        if os.getpgid(pid) != pid:
            return
        os.killpg(pid, signal.SIGINT)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if process_started(pid) != record["started"]:
                return
            time.sleep(0.05)
        if process_started(pid) == record["started"] and os.getpgid(pid) == pid:
            os.killpg(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def stop(state):
    path = state / "processes.json"
    if not path.exists():
        return
    records = json.loads(path.read_text())
    if records.get("project") != stack.load(state)["project"]:
        raise ValueError("Process inventory belongs to another fixture")
    for record in reversed(records["processes"]):
        stop_record(record)
    path.unlink(missing_ok=True)


def unused_port(url):
    parsed = urlsplit(url)
    with socket.socket() as connection:
        connection.settimeout(0.2)
        if connection.connect_ex((parsed.hostname, parsed.port)) == 0:
            raise ValueError(f"Port {parsed.port} is already in use; this runner will not adopt another service")


class Services:
    def __init__(self, args):
        self.args = args
        self.state = args.state
        self.env = stack.environment(self.state)
        if args.workers is not None:
            self.env["BENCH_WORKER_THREADS"] = str(args.workers)
        if args.executor_slots is not None:
            self.env["MAX_CONCURRENT_EXECUTIONS"] = str(args.executor_slots)
        self.env["RUST_MIN_STACK"] = str(args.thread_stack_mb * 1024 * 1024)
        self.log_directory, self.diagnostics = service_diagnostics(self.state, self.env, args.trace)
        self.records = []
        self.processes = []
        self.timings = {}
        self.handles = []

    def launch(self, name, command, url, extra_urls=()):
        urls = (url, *extra_urls)
        for endpoint in urls:
            unused_port(endpoint)
        logs = self.log_directory
        logs.mkdir(mode=0o700, exist_ok=True)
        handle = (logs / f"{name}.log").open("w")
        self.handles.append(handle)
        started = time.perf_counter()
        process = subprocess.Popen(command, cwd=stack.REPO, env=self.env, stdout=handle,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        self.processes.append(process)
        record = {"name": name, "pid": process.pid, "started": process_started(process.pid)}
        self.records.append(record)
        stack.private_write(self.state / "processes.json", json.dumps({
            "project": stack.load(self.state)["project"], "processes": self.records,
        }, indent=2) + "\n")
        context = ssl.create_default_context(cafile=str(self.state / "oidc-ca.pem"))
        deadline = time.monotonic() + self.args.timeout
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise ValueError(f"{name} exited ({process.returncode}); see {logs / (name + '.log')}")
            try:
                for endpoint in urls:
                    with urllib.request.urlopen(endpoint, context=context, timeout=1) as response:
                        if response.status != 200:
                            raise ValueError(f"{name} health returned {response.status}")
                self.timings[name] = round((time.perf_counter() - started) * 1000, 3)
                write_timings(self.state, "startup", self.timings)
                print(f"{name} ready in {self.timings[name]:.1f} ms", flush=True)
                return
            except (urllib.error.URLError, TimeoutError):
                pass
            time.sleep(0.2)
        raise ValueError(f"{name} readiness timed out; see {logs / (name + '.log')}")

    def start(self):
        if (self.state / "processes.json").exists():
            raise ValueError("A fixture process inventory already exists; use stop before restarting")
        binary = self.args.binary or built_binary(self.state, self.args.profile)
        if not binary.is_file():
            raise ValueError("Build the backend first with run.py build --part backend")
        manifest = {
            "binary": str(binary.resolve()), "requested_profile": self.args.profile,
            "binary_sha256": file_hash(binary),
            "binary_profile_hint": next((part for part in reversed(binary.parts)
                                         if part in ("debug", "release-fast", "release")), None),
            "web_mode": self.args.web_mode,
            "worker_threads": int(self.env.get("BENCH_WORKER_THREADS", "1")),
            "executor_slots": int(self.env["MAX_CONCURRENT_EXECUTIONS"]),
            "thread_stack_mb": self.args.thread_stack_mb,
            "compute_accounting_memory_mb": 2048, "enforced_memory_limit_mb": None,
            **self.diagnostics,
            "startup_boundary": "Process launch to successful HTTP health response",
            "transport": "Local HTTP API dispatch with production routers and Lambda compute-context accounting",
        }
        imported = self.state / "import-result.json"
        if imported.is_file():
            manifest["fixture"] = json.loads(imported.read_text())
        stack.private_write(self.state / "run-manifest.json", json.dumps(manifest, indent=2) + "\n")
        self.launch("issuer", ["node", str(stack.HERE / "auth.mjs"), "serve", str(self.state)],
                    "https://127.0.0.1:9440/jwks", ("http://127.0.0.1:9441/health",))
        self.launch("api", [str(binary.resolve()), "api"],
                    self.env["API_URL"] + "/api/v1/health")
        self.launch("runtime", [str(binary.resolve()), "runtime"],
                    self.env["EXECUTOR_URL"] + "/health")
        mode = "dev" if self.args.web_mode == "development" else "serve"
        self.launch("web", ["node", str(stack.HERE / "web.mjs"), mode, "--state", str(self.state),
                            "--port", "3440", "--api-origin", self.env["API_URL"]],
                    "http://127.0.0.1:3440/__benchmark/health")

    def close(self):
        if self.args.trace and any(record["name"] == "api" for record in self.records):
            # The production span exporter flushes every five seconds. Drain after
            # browser measurements finish so the last action's spans reach its log.
            time.sleep(5.1)
        for record in reversed(self.records):
            stop_record(record)
        for process in self.processes:
            try:
                process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                pass
        for handle in self.handles:
            handle.close()
        if self.records:
            (self.state / "processes.json").unlink(missing_ok=True)


def default_browser_config(state):
    selected = state / "browser-config-path.json"
    if selected.is_file():
        return Path(json.loads(selected.read_text())["path"])
    return state / "fixture/browser-config.json"


def benchmark(args):
    config = (args.config or default_browser_config(args.state)).resolve()
    if not config.is_file():
        raise ValueError("Provide --config with validated person/history selectors")
    output = (args.output or args.state / "results").resolve()
    command = ["node", str(stack.REPO / "tests/widget-action-browser/run-app.mjs"),
               "--config", str(config), "--output", str(output)]
    provenance = {}
    for filename in ("run-manifest.json", "startup-timings.json"):
        source = args.state / filename
        if source.is_file():
            provenance[filename] = source.read_text()
    if getattr(args, "trace", False) and "run-manifest.json" not in provenance:
        raise ValueError("Trace mode needs provenance from services started with serve --trace")
    if "run-manifest.json" in provenance:
        manifest = json.loads(provenance["run-manifest.json"])
        if getattr(args, "trace", False) and not (
                manifest.get("telemetry_enabled") is True and manifest.get("telemetry_sink") == "log"
                and manifest.get("backend_trace_sample_rate") == 1):
            raise ValueError("Start services with serve --trace before using benchmark --existing --trace")
        configuration = json.loads(config.read_text())
        for key in ("storageState", "output", "cdpUrl"):
            configuration.pop(key, None)
        manifest["action_config_sha256"] = hashlib.sha256(
            json.dumps(configuration, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        provenance["run-manifest.json"] = json.dumps(manifest, indent=2) + "\n"
    try:
        stack.run(command, cwd=stack.REPO, env=stack.environment(args.state))
    finally:
        output.mkdir(parents=True, exist_ok=True)
        for filename, contents in provenance.items():
            stack.private_write(output / filename, contents)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", type=Path, default=stack.DEFAULT_STATE)
    commands = parser.add_subparsers(dest="command", required=True)
    setup = commands.add_parser("prepare", help="Initialize infrastructure, authentication and sanitized app data")
    setup.add_argument("--metadata-export", type=Path)
    setup.add_argument("--fixture", type=Path)
    setup.add_argument("--fixture-profile", choices=("production", "quick"), help="Generated profile (default production); verify existing fixtures when explicit")
    setup.add_argument("--fixture-layout", choices=("fragmented", "compact"), help="Generated layout defaults to the selected profile; verify existing fixtures when explicit")
    compile_command = commands.add_parser("build", help="Explicitly build the backend and production web app")
    compile_command.add_argument("--part", choices=("all", "backend", "web"), default="all")
    compile_command.add_argument("--profile", choices=("debug", "release-fast", "release"), default="release-fast")
    commands.add_parser("stop", help="Stop only the process groups recorded for this fixture")
    for name in ("serve", "benchmark"):
        command = commands.add_parser(name)
        command.add_argument("--profile", choices=("debug", "release-fast", "release"), default="release-fast")
        command.add_argument("--binary", type=Path)
        command.add_argument("--web-mode", choices=("production", "development"), default="production")
        command.add_argument("--timeout", type=float, default=180)
        command.add_argument("--workers", type=int, help="Tokio workers per API/runtime process (default 1)")
        command.add_argument("--executor-slots", type=int, help="Actual runtime admission capacity")
        command.add_argument("--thread-stack-mb", type=int, default=32, help="Rust thread stack size in MiB (default 32)")
        command.add_argument("--trace", action="store_true", help="Enable complete API spans in private per-run logs for diagnosis")
        if name == "benchmark":
            command.add_argument("--config", type=Path)
            command.add_argument("--output", type=Path)
            command.add_argument("--existing", action="store_true", help="Measure an already running fixture")
    args = parser.parse_args()
    args.state = args.state.resolve()
    for name in ("workers", "executor_slots", "thread_stack_mb"):
        if getattr(args, name, None) is not None and getattr(args, name) < 1:
            raise ValueError(f"{name} must be positive")
    if args.command == "prepare":
        prepare(args)
    elif args.command == "build":
        build(args)
    elif args.command == "stop":
        stop(args.state)
    elif args.command == "benchmark" and args.existing:
        benchmark(args)
    else:
        services = Services(args)
        try:
            services.start()
            if args.command == "benchmark":
                benchmark(args)
            else:
                print("Fixture running. Press Ctrl+C to stop its processes.", flush=True)
                while True:
                    for process in services.processes:
                        if process.poll() is not None:
                            raise ValueError("A fixture service exited; inspect the private logs directory")
                    time.sleep(1)
        finally:
            services.close()


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        pass
    except (ValueError, subprocess.CalledProcessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
