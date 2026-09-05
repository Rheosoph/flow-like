#!/usr/bin/env python3
"""Validate rendered deployment contracts without printing secret values."""
import argparse
import ipaddress
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]


def duration_seconds(value):
    """Read the duration format produced by Compose, including compound units."""
    text = str(value)
    parts = re.findall(r"(\d+(?:\.\d+)?)(ns|us|µs|ms|s|m|h)", text)
    if not parts or "".join(number + unit for number, unit in parts) != text:
        raise ValueError("Expected a Compose duration such as 65m or 1h5m")
    units = {"ns": 1e-9, "us": 1e-6, "µs": 1e-6, "ms": .001, "s": 1, "m": 60, "h": 3600}
    return sum(float(number) * units[unit] for number, unit in parts)


def read_env(path):
    result = {}
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if not separator or not re.fullmatch(r"[A-Z][A-Z0-9_]*", key):
            raise ValueError("Invalid environment assignment; use KEY=value without shell commands")
        if key in result:
            raise ValueError(f"Duplicate environment variable: {key}")
        result[key] = value.strip().strip("\"'")
    return result


def validate(values, config):
    errors = []
    services = config["services"]
    mode = values.get("EXECUTION_ISOLATION_MODE", "per_run")
    if mode not in {"per_run", "trusted_shared"}:
        errors.append("EXECUTION_ISOLATION_MODE must be per_run or trusted_shared")
    api = services.get("api", {}).get("environment", {})
    if api.get("COMPILATION_BACKEND") != "http":
        errors.append("This stack only implements COMPILATION_BACKEND=http")
    for key in ["MAX_CONCURRENT_EXECUTIONS", "QUEUE_WORKER_CONCURRENCY", "EXECUTION_TIMEOUT_SECONDS", "COMPILER_MAX_PARALLEL_TARGETS", "COMPILER_MAX_CONCURRENT_JOBS"]:
        value = values.get(key, "")
        if value and (not value.isdigit() or int(value) < 1):
            errors.append(f"{key} must be a positive integer")
    for name in ["api", "compiler", "web", "signaling", "execution-manager", "runtime", "queue-bridge"]:
        if name not in services:
            continue
        service = services[name]
        replicas = service.get("scale", service.get("deploy", {}).get("replicas", 1))
        if not isinstance(replicas, int) or replicas < 1:
            errors.append(f"{name} replicas must be positive")
    count = services.get("api", {}).get("deploy", {}).get("replicas", 1)
    pool = int(api.get("DATABASE_POOL_MAX_CONNECTIONS", "10"))
    limit = int(values.get("POSTGRES_MAX_CONNECTIONS", "100"))
    if values.get("DATASTORE_MODE", "bundled") == "bundled" and (count + 1) * pool + 10 > limit:
        errors.append("Database pool budget exceeds PostgreSQL max_connections including rollout/admin reserve")
    for key in ["BACKEND_KEY", "BACKEND_PUB", "SINK_SECRET", "SINK_TOKEN_ENCRYPTION_KEY", "EXECUTION_MANAGER_TOKEN"]:
        if not api.get(key):
            errors.append(f"{key} must be configured")
    for key in ["REDIS_API_PASSWORD", "REDIS_RUNTIME_PASSWORD", "REDIS_SIGNALING_PASSWORD", "REDIS_SINK_PASSWORD", "REDIS_METRICS_PASSWORD"]:
        if "redis" in services and not re.fullmatch(r"[a-fA-F0-9]{64,}", values.get(key, "")):
            errors.append(f"{key} requires at least 32 random bytes encoded as hex")
    if mode == "per_run":
        if "runtime" in services:
            errors.append("Shared runtime cannot run in per_run mode; remove the trusted profile")
        for name in ["execution-manager", "execution-gateway", "queue-bridge"]:
            if name not in services:
                errors.append(f"per_run requires {name}; enable COMPOSE_PROFILES=per-run")
        if api.get("EXECUTION_BACKEND") != "http" or api.get("ASYNC_EXECUTION_BACKEND") != "redis":
            errors.append("per_run requires HTTP interactive dispatch and Redis asynchronous dispatch")
        if api.get("EXECUTOR_URL") != "http://execution-gateway:9000":
            errors.append("per_run must route EXECUTOR_URL through execution-gateway")
        if api.get("EXECUTION_STATE_BACKEND") != "redis":
            errors.append("per_run queue ownership currently requires EXECUTION_STATE_BACKEND=redis")
        if values.get("SANDBOX_RUNTIME", "runsc") != "runsc":
            errors.append("per_run requires the runsc Docker runtime")
        for key in ["SANDBOX_IMAGE", "SANDBOX_GATEWAY_IMAGE"]:
            if not re.fullmatch(r"(?:[^\s]+@)?sha256:[a-f0-9]{64}", values.get(key, "")):
                errors.append(f"{key} must be an immutable image ID or repository@sha256 digest")
        manager_env = services.get("execution-manager", {}).get("environment", {})
        warm = manager_env.get("SANDBOX_WARM_POOL_SIZE", "2")
        if not str(warm).isdigit() or not 1 <= int(warm) <= 1024:
            errors.append("SANDBOX_WARM_POOL_SIZE must be between 1 and 1024; the Rust manager requires a warm reserve")
        for key, maximum in [("SANDBOX_CREATE_CONCURRENCY", 32), ("SANDBOX_IDLE_TIMEOUT_SECONDS", 3600),
                             ("SANDBOX_STARTUP_TIMEOUT_SECONDS", 600), ("EXECUTION_CLEANUP_TIMEOUT_SECONDS", 300),
                             ("EXECUTION_TERMINAL_GRACE_SECONDS", 300), ("MAX_CONCURRENT_EXECUTIONS", 1024),
                             ("EXECUTION_MANAGER_WORKER_THREADS", 64)]:
            value = str(manager_env.get(key, ""))
            if not value.isdigit() or not 1 <= int(value) <= maximum:
                errors.append(f"{key} must be between 1 and {maximum}")
        workers = str(manager_env.get("EXECUTION_MANAGER_WORKER_THREADS", "2"))
        pid_limit = services.get("execution-manager", {}).get("pids_limit", 256)
        if workers.isdigit() and int(pid_limit) < int(workers) + 32:
            errors.append("EXECUTION_MANAGER_PIDS must cover Tokio workers plus 32 threads for blocking I/O, SQLite and health checks")
        if services.get("queue-bridge", {}).get("environment", {}).get("EXECUTION_ISOLATION_MODE") != "per_run":
            errors.append("queue-bridge must forward to the manager in per_run mode")
    elif mode == "trusted_shared":
        if "runtime" not in services or "execution-manager" in services or "queue-bridge" in services:
            errors.append("trusted_shared requires only the trusted execution profile")
        if api.get("EXECUTOR_URL") != "http://runtime-gateway:9000":
            errors.append("trusted_shared must route EXECUTOR_URL to runtime-gateway")
    for name in ["execution-manager", "queue-bridge", "runtime"]:
        if name not in services:
            continue
        service = services[name]
        env = service.get("environment", {})
        try:
            budget = (int(env.get("EXECUTION_TIMEOUT_SECONDS", env.get("EXECUTOR_TIMEOUT_SECS", "3600"))) +
                      int(env.get("EXECUTION_STARTUP_GRACE_SECONDS", "120")) +
                      int(env.get("EXECUTION_TERMINAL_GRACE_SECONDS", "60")) +
                      int(env.get("EXECUTION_CLEANUP_TIMEOUT_SECONDS", "30")) + 60)
            if duration_seconds(service.get("stop_grace_period", "10s")) < budget:
                errors.append(f"{name} stop_grace_period must cover execution, startup, terminal and cleanup budgets plus 60 seconds")
        except (TypeError, ValueError):
            errors.append(f"{name} has an invalid execution or shutdown duration")
    for name, service in services.items():
        for port in service.get("ports", []):
            if name not in {"api-gateway", "object-gateway", "grafana"}:
                errors.append(f"{name} must not publish infrastructure ports in the production stack")
            if name == "grafana" and port.get("host_ip") != "127.0.0.1":
                errors.append("Grafana must bind to loopback")
    datastore_mode = values.get("DATASTORE_MODE", "bundled")
    if datastore_mode == "external":
        if "postgres" in services or "redis" in services or "db-init" in services:
            errors.append("External datastores require docker-compose.external-datastores.yml")
        for key in ["DATABASE_URL", "REDIS_URL", "RUNTIME_REDIS_URL", "SIGNALING_REDIS_URL", "SINK_REDIS_URL"]:
            endpoint = urlsplit(values.get(key, ""))
            if not endpoint.hostname or endpoint.hostname in {"postgres", "redis", "localhost", "127.0.0.1"}:
                errors.append(f"{key} must explicitly target the external datastore")
    elif datastore_mode == "bundled":
        if "postgres" not in services or "redis" not in services:
            errors.append("Set DATASTORE_MODE=external when removing bundled datastores")
    else:
        errors.append("DATASTORE_MODE must be bundled or external")
    endpoint = api.get("S3_PUBLIC_ENDPOINT", "")
    parsed = urlsplit(endpoint)
    if api.get("STORAGE_PROVIDER") == "aws" and endpoint:
        if parsed.scheme not in {"http", "https"} or not parsed.hostname or parsed.username or parsed.password or parsed.path not in {"", "/"} or parsed.query or parsed.fragment:
            errors.append("S3_PUBLIC_ENDPOINT must be an HTTP(S) origin without path or credentials")
        if parsed.hostname:
            try:
                ipaddress.ip_address(parsed.hostname)
                errors.append("Use a hostname for S3_PUBLIC_ENDPOINT; the compiler rejects IP literals")
            except ValueError:
                pass
        hosts = services.get("compiler", {}).get("environment", {}).get("COMPILER_ALLOWED_STORAGE_HOSTS", "").split(",")
        if endpoint.rstrip("/") not in hosts and parsed.hostname not in hosts:
            errors.append("COMPILER_ALLOWED_STORAGE_HOSTS must include the exact signed public S3 origin")
    if mode == "per_run" and parsed.scheme == "https" and values.get("EXECUTION_OBJECT_STORE_TLS_GATEWAY") != "true":
        errors.append("HTTPS storage requires EXECUTION_OBJECT_STORE_TLS_GATEWAY=true and a qualified bucket-only TLS gateway")
    store_mode = values.get("OBJECT_STORE_MODE", "bundled")
    if store_mode == "bundled":
        if "object-store" not in services or "object-store-init" not in services:
            errors.append("bundled storage requires object-store and object-store-init")
        if api.get("S3_STS_PROVIDER") != "rustfs":
            errors.append("Bundled RustFS requires S3_STS_PROVIDER=rustfs")
        if values.get("AWS_ACCESS_KEY_ID") in {values.get("RUSTFS_ROOT_USER"), values.get("STS_ISSUER_ACCESS_KEY")}:
            errors.append("Storage root, API and issuer must use separate identities")
        for key in ["META_BUCKET", "CONTENT_BUCKET", "LOG_BUCKET"]:
            if not re.fullmatch(r"[a-z0-9][a-z0-9-]{1,61}[a-z0-9]", values.get(key, "")):
                errors.append(f"{key} must be a DNS-compatible bucket without periods for gateway matching")
    elif store_mode == "external":
        if "object-store" in services or "object-store-init" in services or "object-gateway" in services:
            errors.append("External storage requires docker-compose.external-store.yml; bundled init must be disabled")
        if api.get("S3_INTERNAL_ENDPOINT") == "http://object-store:9000":
            errors.append("Configure the external S3_INTERNAL_ENDPOINT before changing storage mode")
    else:
        errors.append("OBJECT_STORE_MODE must be bundled or external")
    return errors


def run(args):
    if not stat.S_ISREG(args.env_file.lstat().st_mode) or stat.S_IMODE(args.env_file.stat().st_mode) & 0o077:
        raise ValueError("Deployment env file must be a regular private file (chmod 600)")
    values = read_env(args.env_file)
    process_env = os.environ.copy()
    # Explicit --env-file is authoritative; do not let an inherited shell silently
    # replace secrets or change the deployment mode after validation.
    for key in values:
        process_env.pop(key, None)
    command = ["docker", "compose", "--env-file", str(args.env_file), "config", "--format", "json"]
    result = subprocess.run(command, cwd=ROOT, env=process_env, capture_output=True, text=True, timeout=30)
    if result.returncode:
        raise ValueError("Compose configuration failed; check required variables and overlay paths (output hidden because it can contain secrets)")
    config = json.loads(result.stdout)
    errors = validate(values, config)
    if errors:
        raise ValueError("\n".join(errors))
    if not args.config_only:
        info = subprocess.run(["docker", "info", "--format", "{{json .}}"], capture_output=True, text=True, timeout=30)
        if info.returncode:
            raise ValueError("Docker daemon is unavailable")
        host = json.loads(info.stdout)
        if host.get("OSType") != "linux":
            raise ValueError("This deployment requires a Linux Docker daemon")
        if values.get("EXECUTION_ISOLATION_MODE", "per_run") == "per_run":
            version = subprocess.run(["docker", "version", "--format", "{{json .Server}}"], capture_output=True, text=True, timeout=30)
            if version.returncode:
                raise ValueError("Could not determine Docker Engine API compatibility")
            server = json.loads(version.stdout)

            def api_version(value):
                return tuple(int(part) for part in value.split("."))

            if not api_version(server.get("MinAPIVersion", "1.0")) <= (1, 47) <= api_version(server.get("ApiVersion", "0.0")):
                raise ValueError("The execution manager requires Docker Engine API 1.47 support")
            if "runsc" not in host.get("Runtimes", {}):
                raise ValueError("Install and configure gVisor runsc before enabling per-run execution")
            for key in ["SANDBOX_IMAGE", "SANDBOX_GATEWAY_IMAGE"]:
                check = subprocess.run(["docker", "image", "inspect", values[key]], capture_output=True, timeout=30)
                if check.returncode:
                    raise ValueError(f"{key} is not available on the execution Docker daemon")
    print("Configuration checks passed." if args.config_only else "Configuration and host prerequisites passed. Run the isolation qualification before admitting untrusted tenants.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--env-file", type=Path, default=ROOT / ".env")
    parser.add_argument("--config-only", action="store_true", help="Render checks only; does not qualify the execution host")
    args = parser.parse_args()
    args.env_file = args.env_file.absolute()
    try:
        run(args)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(f"Preflight failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
