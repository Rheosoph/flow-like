import importlib.util
import io
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import time
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "standalone_worker_smoke.py"
spec = importlib.util.spec_from_file_location("worker_smoke", SCRIPT)
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)
IMAGE = "ghcr.io/example/agent@sha256:" + "a" * 64
PROFILE = SCRIPT.parents[2] / "apps/standalone/packaging/chrome-seccomp.json"

WORKER = r'''
import base64, json, os, struct, sys, time
def read():
    header = sys.stdin.buffer.read(4)
    assert len(header) == 4
    size, = struct.unpack(">I", header)
    return json.loads(sys.stdin.buffer.read(size))
init = read()
assert init == {"kind": "init", "base_url": "https://worker-smoke.invalid/api/v1",
                "device_id": "release-smoke-device", "bot_id": "release-smoke-bot", "keys": None}
mode = sys.argv[1]
if mode == "early_exit":
    sys.exit(0)
for id in [1, 2]:
    if mode == "delayed_startup":
        for _ in range(3):
            assert read() == {"kind": "ping"}
    frame = {"kind": "call", "id": id, "method": "POST",
             "path": "/devices/release-smoke-device/bots/poll",
             "body": {"host_id": "device:release-smoke-device"}}
    if mode == "wrong_path":
        frame["path"] = "/devices/other-device/bots/poll"
    if mode == "wrong_body":
        frame["body"] = {"host_id": "device:other-device"}
    data = json.dumps(frame).encode()
    for byte in struct.pack(">I", len(data)) + data:
        os.write(1, bytes([byte]))
    reply = read()
    while reply == {"kind": "ping"}:
        reply = read()
    assert (reply["kind"], reply["id"], reply["status"]) == ("reply", id, 200)
    assert json.loads(base64.b64decode(reply["body"])) == {"bots": [], "turns": [], "next_poll_ms": 500}
assert sys.stdin.buffer.read() == b""
if mode == "hang_on_eof":
    time.sleep(60)
'''


class WorkerSmokeTests(unittest.TestCase):
    def worker(self, mode="ok"):
        process = subprocess.Popen([sys.executable, "-c", WORKER, mode],
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, bufsize=0)
        def cleanup():
            if process.poll() is None:
                process.kill()
                process.wait(timeout=5)
            for stream in [process.stdin, process.stdout, process.stderr]:
                stream.close()
        self.addCleanup(cleanup)
        return process

    def test_frame_uses_big_endian_length_and_json(self):
        output = io.BytesIO()
        smoke.write_frame(output, {"kind": "ping"})
        self.assertEqual(output.getvalue(), b'\x00\x00\x00\x0f{"kind":"ping"}')

    def test_fragmented_polls_receive_replies_then_parent_eof_stops_worker(self):
        process = self.worker()
        smoke.exercise_worker(process, startup_timeout=5, shutdown_timeout=5)
        self.assertEqual(process.returncode, 0, process.stderr.read())
        self.assertTrue(process.stdin.closed)

    def test_keepalives_continue_during_delayed_startup_and_stop_before_eof(self):
        process = self.worker("delayed_startup")
        with patch.object(smoke, "write_frame", wraps=smoke.write_frame) as write:
            smoke.exercise_worker(process, startup_timeout=5, shutdown_timeout=5,
                                  keepalive_interval=0.03)
        kinds = [call.args[1]["kind"] for call in write.call_args_list]
        self.assertGreaterEqual(kinds.count("ping"), 6)
        self.assertEqual(kinds[-1], "reply")
        self.assertEqual(process.returncode, 0, process.stderr.read())
        self.assertTrue(process.stdin.closed)

    def test_wrong_device_scope_is_rejected(self):
        for mode in ["wrong_path", "wrong_body"]:
            with self.subTest(mode=mode), self.assertRaisesRegex(RuntimeError, "scoped poll"):
                smoke.exercise_worker(self.worker(mode), startup_timeout=5)

    def test_exit_before_poll_is_not_a_successful_start(self):
        with self.assertRaisesRegex(RuntimeError, "before completing startup"):
            smoke.exercise_worker(self.worker("early_exit"), startup_timeout=5)

    def test_worker_must_exit_after_parent_eof(self):
        with self.assertRaisesRegex(TimeoutError, "did not stop"):
            smoke.exercise_worker(self.worker("hang_on_eof"), startup_timeout=5, shutdown_timeout=0.1)

    def test_invalid_lengths_are_rejected_without_reading_or_allocating_the_body(self):
        for length in [0, smoke.MAX_FRAME + 1, 2**32 - 1]:
            with self.subTest(length=length):
                read, write = os.pipe()
                with os.fdopen(read, "rb", buffering=0) as reader, os.fdopen(write, "wb") as writer:
                    writer.write(struct.pack(">I", length))
                    writer.flush()
                    with self.assertRaisesRegex(RuntimeError, "frame length"):
                        smoke.read_frame(reader, time.monotonic() + 1)

    def docker_run(self, mode="ok", state=None):
        calls = []
        process = self.worker(mode)
        def docker(*args, **kwargs):
            calls.append(args)
            value = state or {"Status": "exited", "ExitCode": 0}
            return subprocess.CompletedProcess(args, 0, json.dumps(value), "")
        return calls, process, patch.object(smoke, "docker", side_effect=docker)

    def test_complete_smoke_uses_restrictions_and_removes_only_its_resources(self):
        calls, process, docker = self.docker_run()
        with docker, patch.object(smoke.subprocess, "Popen", return_value=process):
            smoke.smoke(IMAGE, "linux/arm64", PROFILE)
        create = next(call for call in calls if call[0] == "create")
        for flag in ["--network=none", "--log-driver=none", "--read-only", "--cap-drop=ALL",
                     "--security-opt=no-new-privileges", "--user=65532:65532", "--shm-size=1g"]:
            self.assertIn(flag, create)
        self.assertEqual(create[-2:], (IMAGE, "bot-container-worker"))
        self.assertNotIn("--privileged", create)
        self.assertNotIn("--no-sandbox", create)
        name = create[create.index("--name") + 1]
        self.assertTrue(name.startswith("flow-like-worker-smoke-"))
        self.assertEqual(calls[-2:], [("container", "rm", "--force", name), ("volume", "rm", name + "-data")])
        self.assertIn(f"type=volume,source={name}-data,target=/var/lib/flow-like", create)
        self.assertTrue(any(call[0] == "inspect" for call in calls))

    def test_failed_startup_still_stops_child_and_removes_container_and_volume(self):
        calls, process, docker = self.docker_run("wrong_path")
        with docker, patch.object(smoke.subprocess, "Popen", return_value=process):
            with self.assertRaisesRegex(RuntimeError, "scoped poll"):
                smoke.smoke(IMAGE, "linux/amd64", PROFILE)
        self.assertIsNotNone(process.returncode)
        self.assertEqual([call[:2] for call in calls[-2:]], [("container", "rm"), ("volume", "rm")])

    def test_detached_attach_process_does_not_count_as_worker_shutdown(self):
        calls, process, docker = self.docker_run(state={"Status": "running", "ExitCode": 0})
        with docker, patch.object(smoke.subprocess, "Popen", return_value=process):
            with self.assertRaisesRegex(RuntimeError, "remained alive"):
                smoke.smoke(IMAGE, "linux/amd64", PROFILE)
        self.assertEqual([call[:2] for call in calls[-2:]], [("container", "rm"), ("volume", "rm")])

    def test_mutable_image_tags_are_refused_before_docker_is_called(self):
        with patch.object(smoke, "docker") as docker:
            with self.assertRaisesRegex(ValueError, "exact published image digest"):
                smoke.smoke("ghcr.io/example/agent:latest", "linux/amd64", PROFILE)
            docker.assert_not_called()


if __name__ == "__main__":
    unittest.main()
