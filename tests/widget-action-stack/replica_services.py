"""Launch independent local API and executor processes with one HTTP lease each."""

import json
import os
from pathlib import Path
import secrets
import time
import urllib.request

import run
import stack
from topology import validate_topology


POOL_PORTS = {"api_frontend": (8440, 18440), "api_callback": (8442, 18540), "runtime": (8441, 18640)}
CONTROL_PORT = 8443


def topology_for(replicas):
    if type(replicas) is not int or not 1 <= replicas <= 64:
        raise ValueError("replicas must be between 1 and 64")
    return validate_topology({
        "kind": "replicated-local", "apiFrontendReplicas": replicas,
        "apiCallbackReplicas": replicas, "executorReplicas": replicas,
        "requestsPerProcess": 1, "workerThreads": 1, "executorSlotsPerProcess": 1,
        "callbackRouting": "reserved-pool", "leaseBoundary": "upstream-eof",
        "runtimeModel": "current-thread", "cpuLimitEnforced": False, "memoryLimitEnforced": False,
    })


class ReplicaServices(run.Services):
    def __init__(self, args):
        self.topology = topology_for(args.replicas)
        super().__init__(args)
        self.directory = Path(args.output).resolve()
        self.directory.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.log_directory = self.directory / "logs"
        self.diagnostics["logs_directory"] = str(self.log_directory)
        self.admission_log = self.directory / "admissions.jsonl"
        self.inventory = []
        self.ready_at_unix_ms = None
        self.control_token = secrets.token_urlsafe(32)
        self.env.update(BENCH_WORKER_THREADS="1", BENCH_RUNTIME_MODEL="current-thread",
                        MAX_CONCURRENT_EXECUTIONS="1", API_URL="http://127.0.0.1:8442",
                        DATABASE_POOL_MAX_CONNECTIONS="4", DATABASE_POOL_MIN_CONNECTIONS="0",
                        API_BASE_URL="http://127.0.0.1:8442", CALLBACK_BASE_URL="http://127.0.0.1:8442",
                        EXECUTOR_URL="http://127.0.0.1:8441")
        self.binary = (args.binary or run.built_binary(self.state, args.profile)).resolve()
        self.binary_sha256 = None

    def persist_inventory(self):
        stack.private_write(self.state / "processes.json", json.dumps({
            "project": stack.load(self.state)["project"], "processes": self.records,
        }, indent=2) + "\n")
        stack.private_write(self.directory / "processes.json", json.dumps(self.inventory, indent=2) + "\n")

    def control(self, path, payload=None):
        body = None if payload is None else json.dumps(payload).encode()
        request = urllib.request.Request(f"http://127.0.0.1:{CONTROL_PORT}{path}", data=body,
                                         headers={"authorization": "Bearer " + self.control_token,
                                                  "content-type": "application/json"})
        with urllib.request.urlopen(request, timeout=min(10, self.args.timeout)) as response:
            return json.load(response)

    def wait_idle(self):
        deadline = time.monotonic() + self.args.timeout
        while time.monotonic() < deadline:
            for process in self.processes:
                if process.poll() is not None:
                    raise ValueError("Owned replica service exited before its accounting drain")
            status = self.control("/status")
            pools = status["pools"]
            if any(pool["failed"] or pool["failures"] or pool["rejected"] or pool["quarantined"] for pool in pools.values()):
                raise ValueError("Replica admission failed; inspect admissions.jsonl and process logs")
            if all(pool["active"] == 0 and pool["queued"] == 0 for pool in pools.values()):
                return status
            time.sleep(.05)
        raise ValueError("Replica streams did not reach upstream EOF before the deadline")

    def launch_backend(self):
        for pool, (_, base_port) in POOL_PORTS.items():
            for index in range(self.args.replicas):
                port = base_port + index
                role = "runtime" if pool == "runtime" else "api"
                name = f"{pool}-{index}"
                self.env.update(API_PORT=str(port), PORT=str(port), RUNTIME_PORT=str(port))
                health = "/health" if role == "runtime" else "/api/v1/health"
                origin = f"http://127.0.0.1:{port}"
                self.launch(name, [str(self.binary), role], origin + health)
                # Probe the actual running runtime; a legacy binary must not be mislabeled.
                with urllib.request.urlopen(origin + "/__benchmark/runtime", timeout=5) as response:
                    runtime = json.load(response)
                if runtime.get("runtimeModel") != "current-thread":
                    raise ValueError(f"{name} does not use the required current-thread runtime")
                record = self.records[-1]
                record.update(pool=pool, replica=name, generation=self.generation,
                              binarySha256=self.binary_sha256, workerThreads=1, runtimeModel="current-thread")
                self.inventory.append(dict(record))
                self.persist_inventory()
        self.ready_at_unix_ms = time.time() * 1000

    def start(self):
        if (self.state / "processes.json").exists():
            raise ValueError("A fixture process inventory already exists; stop its owned services first")
        if self.admission_log.exists():
            raise ValueError("Admission output already exists; use a fresh trial directory")
        if not self.binary.is_file():
            raise ValueError("Build the widget_action_stack example before running replicas")
        self.binary_sha256 = run.file_hash(self.binary)
        manifest = {
            "binary": str(self.binary), "binary_sha256": self.binary_sha256,
            "requested_profile": self.args.profile,
            "binary_profile_hint": next((part for part in reversed(self.binary.parts)
                                         if part in ("debug", "release-fast", "release")), None),
            "web_mode": self.args.web_mode, "worker_threads": 1, "executor_slots": 1,
            "thread_stack_mb": self.args.thread_stack_mb, "compute_accounting_memory_mb": 2048,
            "enforced_memory_limit_mb": None, "execution_topology": self.topology,
            "database_pool": {"maxConnections": 4, "minConnections": 0},
            "allowedOptionalResponses": [{"method": "GET", "pathCategory": "frontend_telemetry_config", "status": 404}],
            "startup_boundary": "Process launch to successful HTTP health response",
            "readiness_poll_ms": getattr(self.args, "readiness_poll_ms", 200),
            "host_load_average_start": list(os.getloadavg()) if hasattr(os, "getloadavg") else None,
            "transport": "Independent local HTTP processes with reserved callback capacity and upstream EOF admission",
            "limitations": ["Shared host CPU and memory without enforced quotas", "Local PostgreSQL, Redis and S3-compatible storage",
                            "HTTP dispatch rather than the Lambda service", "API Lambda compute-attempt wrapper is absent",
                            "Native default allocator rather than the AWS binaries' mimalloc",
                            "Local eager API initialization rather than AWS bootstrap"],
            **self.diagnostics,
        }
        imported = self.state / "import-result.json"
        if imported.is_file():
            manifest["fixture"] = json.loads(imported.read_text())
        stack.private_write(self.state / "run-manifest.json", json.dumps(manifest, indent=2) + "\n")
        self.launch("issuer", ["node", str(stack.HERE / "auth.mjs"), "serve", str(self.state)],
                    "https://127.0.0.1:9440/jwks", ("http://127.0.0.1:9441/health",))
        config = {"controlPort": CONTROL_PORT, "controlToken": self.control_token,
                  "logFile": str(self.admission_log), "pools": {
                      pool: {"port": gateway, "targets": [
                          {"id": f"{pool}-{index}", "url": f"http://127.0.0.1:{base + index}", "generation": 0}
                          for index in range(self.args.replicas)]}
                      for pool, (gateway, base) in POOL_PORTS.items()}}
        config_path = self.directory / "proxy-config.json"
        stack.private_write(config_path, json.dumps(config, indent=2) + "\n")
        for port, _ in POOL_PORTS.values():
            run.unused_port(f"http://127.0.0.1:{port}")
        self.launch("replica-proxy", ["node", str(stack.HERE / "replica-proxy.mjs"), "--config", str(config_path)],
                    f"http://127.0.0.1:{CONTROL_PORT}/health")
        self.launch_backend()
        mode = "dev" if self.args.web_mode == "development" else "serve"
        command = ["node", str(stack.HERE / "web.mjs"), mode,
                   "--state", str(getattr(self.args, "web_state", None) or self.state),
                   "--port", "3440", "--api-origin", "http://127.0.0.1:8440"]
        if getattr(self.args, "web_source_root", None):
            command.extend(["--source-root", str(self.args.web_source_root)])
        self.launch("web", command, "http://127.0.0.1:3440/__benchmark/health")

    def restart_backend(self):
        self.wait_idle()
        self.control("/pause", {})
        before = [dict(record) for record in self.records if "pool" in record]
        if len(before) != self.args.replicas * 3:
            raise ValueError("Cannot restart an incomplete replica inventory")
        started = time.perf_counter()
        for record in reversed(before):
            process = next(process for process in self.processes if process.pid == record["pid"])
            if process.poll() is not None or run.process_started(record["pid"]) != record["started"]:
                raise ValueError("Owned replica identity changed before cold restart")
            run.stop_record(record, process)
            process.wait(timeout=5)
            self.records.remove(record)
            self.processes.remove(process)
            self.persist_inventory()
        stop_ms = (time.perf_counter() - started) * 1000
        self.generation += 1
        if run.file_hash(self.binary) != self.binary_sha256:
            raise ValueError("Backend binary changed during the cold boundary")
        self.launch_backend()
        after = [dict(record) for record in self.records if "pool" in record]
        if {(item["pid"], item["started"]) for item in before} & {(item["pid"], item["started"]) for item in after}:
            raise ValueError("Cold restart reused a backend process")
        status = self.control("/status")
        if any(pool["rejected"] for pool in status["pools"].values()):
            raise ValueError("Unexpected traffic arrived during the cold restart")
        self.control("/generation", {"generation": self.generation})
        self.ready_at_unix_ms = time.time() * 1000
        return {"stop_ms": round(stop_ms, 3), "processes_before": before, "processes_after": after,
                "readyAtUnixMs": self.ready_at_unix_ms,
                "measurements_ms": {item["name"]: self.timings[item["name"]] for item in after}}
