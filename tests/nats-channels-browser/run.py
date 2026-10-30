"""Run Chromium and the Rust channel adapter against an authenticated local NATS broker."""

import importlib.util
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


ROOT = Path(__file__).resolve().parents[2]


def free_ports(count):
    sockets = []
    try:
        for _ in range(count):
            sock = socket.socket()
            sock.bind(("127.0.0.1", 0))
            sockets.append(sock)
        return [sock.getsockname()[1] for sock in sockets]
    finally:
        for sock in sockets:
            sock.close()


def main():
    server_bin = os.environ.get("NATS_SERVER_BIN") or shutil.which("nats-server")
    if not server_bin:
        raise SystemExit("Set NATS_SERVER_BIN to a nats-server executable.")
    worker = os.environ.get("NATS_WORKER_BIN") or ROOT / "target/debug/examples/nats-channel-worker"
    gateway_bin = os.environ.get("NATS_GATEWAY_BIN")
    if not Path(worker).is_file():
        raise SystemExit("Build cargo build -p flow-like-channels --features nats --example nats-channel-worker, or set NATS_WORKER_BIN.")
    spec = importlib.util.spec_from_file_location("nats_bootstrap", ROOT / "apps/backend/scripts/nats_bootstrap.py")
    bootstrap = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(bootstrap)
    ports = free_ports(4)
    client_port, ws_port, monitor_port, browser_port = ports
    trust = bootstrap.generate()
    config = json.loads(bootstrap.server_config(trust, [f"http://127.0.0.1:{browser_port}"]))
    config.update({"host": "127.0.0.1", "port": client_port, "http_port": monitor_port})
    config["websocket"].update({"host": "127.0.0.1", "port": ws_port})
    with tempfile.TemporaryDirectory(prefix="flow-like-nats-browser-") as directory:
        directory = Path(directory)
        server_config = directory / "nats.conf"
        server_config.write_text(json.dumps(config))
        fixture = directory / "fixture.json"
        fixture.write_text(json.dumps({
            "url": f"nats://127.0.0.1:{client_port}",
            "ws_url": f"ws://127.0.0.1:{ws_port}",
            "account_seed": trust["account_seed"], "channel_id": "browser-e2e",
        }))
        fixture.chmod(0o600)
        with (directory / "broker.log").open("w+") as logs:
            broker = subprocess.Popen([server_bin, "-c", str(server_config)], stdout=logs, stderr=logs)
            gateway = None
            try:
                ready = False
                for _ in range(100):
                    if broker.poll() is not None:
                        raise RuntimeError("The NATS test broker exited before becoming ready.")
                    try:
                        with urllib.request.urlopen(f"http://127.0.0.1:{monitor_port}/healthz", timeout=0.2) as response:
                            ready = response.status == 200
                        if ready:
                            break
                    except (OSError, urllib.error.URLError):
                        time.sleep(0.05)
                if not ready:
                    raise RuntimeError("The NATS test broker did not become ready.")
                env = {**os.environ, "NATS_WORKER_BIN": str(worker), "NATS_E2E_BROWSER_PORT": str(browser_port)}
                if gateway_bin:
                    # Refuse occupied production gateway ports before starting or configuring a process.
                    for port in (3128, 9001):
                        with socket.socket() as probe:
                            probe.bind(("127.0.0.1", port))
                    token = secrets.token_hex(32)
                    gateway = subprocess.Popen([gateway_bin, "--tcp"], stdout=logs, stderr=logs,
                                               env={**os.environ, "GATEWAY_TOKEN": token, "EXECUTION_TIMEOUT_SECONDS": "120"})
                    for _ in range(100):
                        if gateway.poll() is not None:
                            raise RuntimeError("The execution gateway exited before becoming ready.")
                        try:
                            with socket.create_connection(("127.0.0.1", 9001), timeout=0.2):
                                break
                        except OSError:
                            time.sleep(0.05)
                    policy = {
                        "callback_url": "http://callback:8080", "object_store_url": "http://objects:9000",
                        "app_id": "app", "run_id": "browser-e2e", "executor_jwt": "fixture",
                        "deadline": time.time() + 119, "allowed_https_hosts": [],
                        "channel_nats_url": f"nats://127.0.0.1:{client_port}",
                    }
                    request = urllib.request.Request("http://127.0.0.1:9001/configure",
                                                     data=json.dumps(policy).encode(), method="POST",
                                                     headers={"X-Gateway-Token": token, "Content-Type": "application/json"})
                    with urllib.request.urlopen(request, timeout=5) as response:
                        if response.status != 204:
                            raise RuntimeError("The execution gateway rejected the NATS test policy.")
                    env["CHANNEL_NATS_PROXY_URL"] = "http://127.0.0.1:3128"
                subprocess.run(["bun", "tests/nats-channels-browser/verify.ts", "--config", str(fixture)], cwd=ROOT, env=env, check=True, timeout=120)
            finally:
                if gateway:
                    gateway.terminate()
                    try:
                        gateway.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        gateway.kill()
                        gateway.wait()
                broker.terminate()
                try:
                    broker.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    broker.kill()
                    broker.wait()


if __name__ == "__main__":
    main()
