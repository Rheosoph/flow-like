"""Boot the released bot worker image against a synthetic parent, without a hub."""

import argparse
import base64
import json
import os
from pathlib import Path
import re
import selectors
import struct
import subprocess
import sys
import tempfile
import time
import uuid


DEVICE = "release-smoke-device"
BOT = "release-smoke-bot"
MAX_FRAME = 64 * 1024


def write_frame(stream, value):
    data = json.dumps(value, separators=(",", ":")).encode()
    stream.write(struct.pack(">I", len(data)) + data)
    stream.flush()


def read_exact(stream, size, deadline, keepalive=None):
    data = bytearray()
    with selectors.DefaultSelector() as selector:
        selector.register(stream, selectors.EVENT_READ)
        while len(data) < size:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("Worker did not send its startup poll before the deadline")
            wait = min(remaining, keepalive()) if keepalive is not None else remaining
            if not selector.select(wait):
                continue
            chunk = os.read(stream.fileno(), size - len(data))
            if not chunk:
                raise RuntimeError("Worker closed its parent pipe before completing startup")
            data.extend(chunk)
    return bytes(data)


def read_frame(stream, deadline, keepalive=None):
    size, = struct.unpack(">I", read_exact(stream, 4, deadline, keepalive))
    if not 0 < size <= MAX_FRAME:
        raise RuntimeError("Worker sent an invalid startup frame length")
    frame = json.loads(read_exact(stream, size, deadline, keepalive))
    if not isinstance(frame, dict):
        raise RuntimeError("Worker sent a non-object startup frame")
    return frame


def wait_for_exit(process, timeout, expected, context):
    try:
        result = process.wait(timeout=timeout)
    except subprocess.TimeoutExpired as error:
        raise TimeoutError(f"Worker did not stop {context}") from error
    if result != expected:
        raise RuntimeError(f"Worker exited with status {result}; expected {expected}")
    return result


def exercise_worker(process, startup_timeout=60, shutdown_timeout=35, keepalive_interval=5,
                    parent_loss="eof", init_timeout=40):
    if parent_loss == "unconfigured":
        return wait_for_exit(process, init_timeout, 1, "without its initial configuration")
    if parent_loss not in {"eof", "silent"}:
        raise ValueError("Unknown parent-loss scenario")
    write_frame(process.stdin, {
        "kind": "init", "base_url": "https://worker-smoke.invalid/api/v1",
        "device_id": DEVICE, "bot_id": BOT, "keys": None,
    })
    deadline = time.monotonic() + startup_timeout
    next_ping = time.monotonic() + keepalive_interval

    def keepalive():
        nonlocal next_ping
        now = time.monotonic()
        if now >= next_ping:
            write_frame(process.stdin, {"kind": "ping"})
            next_ping = now + keepalive_interval
        return next_ping - now

    previous_id = None
    for _ in range(2):
        frame = read_frame(process.stdout, deadline, keepalive)
        if (set(frame) != {"kind", "id", "method", "path", "body"}
                or frame["kind"] != "call" or frame["method"] != "POST"
                or frame["path"] != f"/devices/{DEVICE}/bots/poll"
                or frame["body"] != {"host_id": f"device:{DEVICE}"}
                or type(frame["id"]) is not int or frame["id"] <= 0
                or frame["id"] == previous_id):
            raise RuntimeError("Worker did not send the expected scoped poll")
        previous_id = frame["id"]
        body = json.dumps({"bots": [], "turns": [], "next_poll_ms": 500}).encode()
        write_frame(process.stdin, {
            "kind": "reply", "id": frame["id"], "status": 200,
            "body": base64.b64encode(body).decode(),
        })
    # Silence keeps the pipe open and requires the worker's parent watchdog to stop it.
    if parent_loss == "eof":
        process.stdin.close()
        context = "after its parent pipe closed"
    else:
        context = "after its parent stopped sending frames"
    return wait_for_exit(process, shutdown_timeout, 0, context)


def create_arguments(image, platform, profile, name, volume):
    return [
        "create", "--log-driver=none", "--interactive", "--init", "--pull=never",
        "--platform", platform, "--name", name, "--hostname", name,
        "--label=io.flow-like.ci-worker-smoke=true", "--network=none",
        "--user=65532:65532", "--cap-drop=ALL", "--security-opt=no-new-privileges",
        "--security-opt", f"seccomp={profile}", "--read-only", "--shm-size=1g",
        "--memory=4g", "--memory-swap=4g", "--cpus=2", "--pids-limit=512",
        "--tmpfs=/tmp:rw,nosuid,nodev,size=1g,mode=1777",
        "--tmpfs=/run:rw,nosuid,nodev,size=16m,mode=1777",
        "--mount", f"type=volume,source={volume},target=/var/lib/flow-like",
        "--env=FLOW_LIKE_PLAYGROUND=virtual", "--env=HOME=/var/lib/flow-like",
        image, "bot-container-worker",
    ]


def docker(*args, timeout=30, check=True):
    return subprocess.run(["docker", *args], check=check, capture_output=True,
                          text=True, timeout=timeout)


def smoke(image, platform, profile):
    if not re.fullmatch(r"[^\s@]+@sha256:[0-9a-f]{64}", image):
        raise ValueError("Smoke test requires the exact published image digest")
    profile = profile.resolve(strict=True)
    docker("pull", "--platform", platform, image, timeout=600)
    for parent_loss in ["eof", "silent", "unconfigured"]:
        smoke_worker(image, platform, profile, parent_loss)


def smoke_worker(image, platform, profile, parent_loss):
    name = "flow-like-worker-smoke-" + uuid.uuid4().hex
    volume = name + "-data"
    process = None
    created = False
    volume_created = False
    try:
        docker("volume", "create", "--label=io.flow-like.ci-worker-smoke=true", volume)
        volume_created = True
        # Keep the exited container until inspection confirms the worker itself stopped.
        docker(*create_arguments(image, platform, profile, name, volume))
        created = True
        with tempfile.TemporaryFile() as stderr:
            process = subprocess.Popen(
                ["docker", "start", "--attach", "--interactive", name],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr, bufsize=0,
            )
            try:
                result = exercise_worker(process, parent_loss=parent_loss)
                state = json.loads(docker("inspect", "--format", "{{json .State}}", name).stdout)
                if state.get("Status") != "exited" or state.get("ExitCode") != result:
                    raise RuntimeError(f"Container remained alive or failed during {parent_loss}")
            except Exception:
                stderr.seek(0, os.SEEK_END)
                stderr.seek(max(0, stderr.tell() - 8192))
                diagnostics = stderr.read().decode(errors="replace").strip()
                if diagnostics:
                    print(diagnostics, file=sys.stderr)
                raise
    finally:
        try:
            if process is not None:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=10)
                for stream in [process.stdin, process.stdout]:
                    if stream is not None:
                        stream.close()
        finally:
            try:
                docker("container", "rm", "--force", name, check=created)
            finally:
                docker("volume", "rm", volume, check=volume_created)
    print(f"Worker {parent_loss} shutdown passed for {platform}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True)
    parser.add_argument("--platform", required=True, choices=["linux/amd64", "linux/arm64"])
    parser.add_argument("--seccomp", type=Path, default=Path(__file__).resolve().parents[2]
                        / "apps/standalone/packaging/chrome-seccomp.json")
    arguments = parser.parse_args()
    smoke(arguments.image, arguments.platform, arguments.seccomp)
