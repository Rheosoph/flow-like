"""Test the actual Redis startup ACL and production Lua on an isolated server.

Set REDIS_SERVER_BIN to a Redis 7 binary. This suite starts a loopback-only
server in a temporary directory and always terminates it; it uses no existing
Redis data. The production startup script is copied with only its ACL file path
redirected into that directory. No system installation is required.
"""
import importlib.util
import json
import os
from pathlib import Path
import secrets
import shlex
import socket
import subprocess
import tempfile
import time
import unittest
import uuid

COMPOSE = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("delivery", COMPOSE / "tests/test_redis_delivery.py")
delivery = importlib.util.module_from_spec(spec)
spec.loader.exec_module(delivery)


@unittest.skipUnless(os.environ.get("REDIS_SERVER_BIN"), "REDIS_SERVER_BIN is not configured")
class RedisAclTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix="flowlike-redis-acl-")
        cls.addClassCleanup(cls.temp.cleanup)
        folder = Path(cls.temp.name)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            cls.port = listener.getsockname()[1]
        cls.passwords = {name: secrets.token_hex(32) for name in ("api", "runtime", "signaling", "sink", "metrics")}
        env = os.environ.copy()
        for name, password in cls.passwords.items():
            env[f"REDIS_{name.upper()}_PASSWORD"] = password
        env["REDIS_MAXMEMORY"] = "32mb"
        script = (COMPOSE / "scripts/redis-start.sh").read_text()
        script = script.replace("/tmp/users.acl", str(folder / "users.acl"))
        startup = folder / "start.sh"
        startup.write_text(script)
        # Add only local test binding/data/log settings after the production args.
        wrapper = folder / "redis-server"
        extra = ["--bind", "127.0.0.1", "--port", str(cls.port), "--dir", str(folder), "--logfile", str(folder / "redis.log")]
        wrapper.write_text("#!/bin/sh\nexec " + shlex.quote(os.environ["REDIS_SERVER_BIN"]) + " \"$@\" " + shlex.join(extra) + "\n")
        wrapper.chmod(0o700)
        env["PATH"] = str(folder) + os.pathsep + env["PATH"]
        cls.process = subprocess.Popen(["sh", str(startup)], cwd=folder, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        cls.addClassCleanup(cls.stop)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if cls.process.poll() is not None:
                text = cls.process.stdout.read().decode(errors="replace")
                if (folder / "redis.log").exists():
                    text += (folder / "redis.log").read_text()
                for password in cls.passwords.values():
                    text = text.replace(password, "<redacted>")
                raise AssertionError("Production Redis startup failed: " + text)
            try:
                client = cls.client("metrics")
                assert client.command("PING") == "PONG"
                client.close()
                return
            except (OSError, RuntimeError):
                time.sleep(0.05)
        raise AssertionError("Test Redis did not become ready")

    @classmethod
    def stop(cls):
        cls.process.terminate()
        try:
            cls.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            cls.process.kill()
            cls.process.wait(timeout=5)
        cls.process.stdout.close()

    @classmethod
    def client(cls, name):
        return delivery.Redis(f"redis://{name}:{cls.passwords[name]}@127.0.0.1:{cls.port}")

    def test_runtime_can_claim_ack_and_lease_but_cannot_read_other_keys(self):
        api, runtime = self.client("api"), self.client("runtime")
        self.addCleanup(api.close)
        self.addCleanup(runtime.close)
        prefix = "exec:acl-test:" + uuid.uuid4().hex
        ready, pending, deadlines, dead, run, lease = [prefix + suffix for suffix in (":ready", ":pending", ":deadlines", ":dead", ":run", ":lease")]
        notify = ready + ":notify"
        published = ready + ":published"
        enqueue = delivery.script(delivery.QUEUE_SOURCE, "ENQUEUE_SCRIPT")
        claim = delivery.script(delivery.QUEUE_SOURCE, "CLAIM")
        complete = delivery.script(delivery.QUEUE_SOURCE, "COMPLETE")
        self.assertEqual(api.eval(enqueue, [ready, pending, dead, notify, published], "payload", 10), 1)
        self.assertEqual(runtime.eval(claim, [ready, pending, deadlines, dead, notify, published], "delivery", 30000, 300000), "payload")
        self.assertEqual(runtime.command("BLPOP", notify, 1), [notify, "ready"])
        retry = delivery.script(delivery.QUEUE_SOURCE, "REQUEUE")
        self.assertEqual(runtime.eval(retry, [pending, deadlines, ready, notify], "delivery"), 1)
        self.assertEqual(runtime.eval(claim, [ready, pending, deadlines, dead, notify, published], "delivery-two", 30000, 300000), "payload")
        self.assertEqual(runtime.eval(complete, [pending, deadlines, dead, published], "delivery-two", ""), 1)
        now = int(time.time() * 1000)
        api.command("SET", run, json.dumps({"app_id": "app", "status": "PENDING", "updated_at": now, "started_at": None, "expires_at": now + 60000}), "PX", 60000)
        leased = runtime.eval(delivery.script(delivery.STATE_SOURCE, "RUN_LEASE_SCRIPT"), [run, lease], "claim", "app", "job", "owner", 30000)
        self.assertEqual(leased[0], 1)
        api.command("SET", "other:private", "private-value")
        for args in [("GET", "other:private"), ("SET", "other:private", "overwrite"), ("FLUSHDB",), ("CONFIG", "SET", "maxmemory", "0")]:
            with self.assertRaisesRegex(RuntimeError, "NOPERM"):
                runtime.command(*args)
        with self.assertRaisesRegex(RuntimeError, "NOPERM"):
            api.command("FLUSHDB")

    def test_signaling_metrics_and_sink_credentials_are_distinct(self):
        signaling, metrics, sink = [self.client(name) for name in ("signaling", "metrics", "sink")]
        for client in (signaling, metrics, sink):
            self.addCleanup(client.close)
        self.assertEqual(signaling.command("PUBLISH", "signal:test", "message"), 0)
        with self.assertRaisesRegex(RuntimeError, "NOPERM"):
            signaling.command("PUBLISH", "exec:test", "message")
        self.assertEqual(metrics.command("PING"), "PONG")
        self.assertIn("redis_version:", metrics.command("INFO", "server"))
        with self.assertRaisesRegex(RuntimeError, "NOPERM"):
            metrics.command("GET", "exec:test")
        self.assertEqual(sink.command("SET", "sink:test", "state"), "OK")
        self.assertEqual(sink.command("KEYS", "sink:*"), ["sink:test"])
        with self.assertRaisesRegex(RuntimeError, "NOPERM"):
            sink.command("GET", "exec:test")
        anonymous = delivery.Redis(f"redis://127.0.0.1:{self.port}")
        self.addCleanup(anonymous.close)
        with self.assertRaisesRegex(RuntimeError, "NOAUTH"):
            anonymous.command("PING")

    def test_full_production_delivery_protocol_suite_with_api_acl(self):
        env = os.environ.copy()
        env["REDIS_TEST_URL"] = f"redis://api:{self.passwords['api']}@127.0.0.1:{self.port}"
        result = subprocess.run(["python3", str(COMPOSE / "tests/test_redis_delivery.py"), "-v"], env=env, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertRegex(result.stderr, r"Ran [1-9][0-9]* tests")
        self.assertNotIn("skipped", result.stderr)


if __name__ == "__main__":
    unittest.main()
