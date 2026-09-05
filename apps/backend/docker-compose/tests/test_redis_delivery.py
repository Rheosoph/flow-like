"""Exercise the actual Rust Lua constants against an opt-in Redis test server.

Set REDIS_TEST_URL to redis:// or rediss://, then run this file with unittest.
Only UUID-prefixed test keys are created or deleted; the database is never flushed.
Without Redis, script discovery runs and protocol integration tests are skipped.
"""
import json
import os
from pathlib import Path
import re
import socket
import ssl
import time
import unittest
import urllib.parse
import uuid


ROOT = Path(__file__).resolve().parents[4]
QUEUE_SOURCE = ROOT / "packages/api/src/execution/queue.rs"
STATE_SOURCE = ROOT / "packages/api/src/execution/state/redis.rs"


def script(path, name):
    source = path.read_text(encoding="utf-8")
    match = re.search(r"\bconst\s+" + re.escape(name) + r'\s*:\s*&str\s*=\s*r#"(.*?)"#;',
                      source, re.S)
    if match is None:
        raise AssertionError(f"Could not locate production Lua constant {name}")
    return match.group(1)


class Redis:
    """Small RESP2 client so tests need no third-party Python dependencies."""
    def __init__(self, endpoint):
        parsed = urllib.parse.urlsplit(endpoint)
        if parsed.scheme not in ("redis", "rediss") or not parsed.hostname:
            raise ValueError("REDIS_TEST_URL must be a Redis URL")
        self.socket = socket.create_connection((parsed.hostname, parsed.port or 6379), timeout=5)
        if parsed.scheme == "rediss":
            self.socket = ssl.create_default_context().wrap_socket(self.socket, server_hostname=parsed.hostname)
        self.file = self.socket.makefile("rb")
        if parsed.password is not None:
            password = urllib.parse.unquote(parsed.password)
            if parsed.username:
                self.command("AUTH", urllib.parse.unquote(parsed.username), password)
            else:
                self.command("AUTH", password)
        if parsed.path.strip("/"):
            self.command("SELECT", int(parsed.path.strip("/")))

    def read(self):
        line = self.file.readline()
        if not line.endswith(b"\r\n"):
            raise EOFError("Incomplete Redis response")
        kind, value = line[:1], line[1:-2]
        if kind == b"+":
            return value.decode()
        if kind == b"-":
            raise RuntimeError(value.decode())
        if kind == b":":
            return int(value)
        if kind == b"$":
            length = int(value)
            if length == -1:
                return None
            payload = self.file.read(length)
            if len(payload) != length or self.file.read(2) != b"\r\n":
                raise EOFError("Incomplete Redis bulk response")
            return payload.decode()
        if kind == b"*":
            return None if int(value) == -1 else [self.read() for _ in range(int(value))]
        raise ValueError("Unknown Redis response type")

    def command(self, *args):
        pieces = [f"*{len(args)}\r\n".encode()]
        for arg in args:
            value = arg if isinstance(arg, bytes) else str(arg).encode()
            pieces.extend((f"${len(value)}\r\n".encode(), value, b"\r\n"))
        self.socket.sendall(b"".join(pieces))
        return self.read()

    def eval(self, source, keys, *args):
        return self.command("EVAL", source, len(keys), *keys, *args)

    def close(self):
        self.file.close()
        self.socket.close()


class ProductionScriptDiscovery(unittest.TestCase):
    def test_all_protocols_load_from_rust_source(self):
        for source, names in ((QUEUE_SOURCE, ("ENQUEUE_SCRIPT", "CLAIM", "COMPLETE", "REQUEUE")),
                              (STATE_SOURCE, ("RUN_LEASE_SCRIPT", "UPDATE_MUTABLE_RUN_SCRIPT", "CANCEL_RUN_SCRIPT"))):
            for name in names:
                self.assertIn("redis.call", script(source, name))


@unittest.skipUnless(os.environ.get("REDIS_TEST_URL"), "REDIS_TEST_URL is not configured")
class RedisDeliveryTests(unittest.TestCase):
    def setUp(self):
        self.redis = Redis(os.environ["REDIS_TEST_URL"])
        self.prefix = "flow-like-test:" + uuid.uuid4().hex
        self.ready, self.pending, self.deadlines, self.dead = [self.prefix + suffix for suffix in (":ready", ":pending", ":deadlines", ":dead")]
        self.notify = self.ready + ":notify"
        self.published = self.ready + ":published"
        self.run, self.lease = self.prefix + ":run", self.prefix + ":lease"
        self.keys = [self.ready, self.pending, self.deadlines, self.dead, self.run, self.lease, self.notify, self.published]
        self.enqueue_script = script(QUEUE_SOURCE, "ENQUEUE_SCRIPT")
        self.claim_script = script(QUEUE_SOURCE, "CLAIM")
        self.complete_script = script(QUEUE_SOURCE, "COMPLETE")
        self.lease_script = script(STATE_SOURCE, "RUN_LEASE_SCRIPT")

    def tearDown(self):
        try:
            self.redis.command("DEL", *self.keys)
        finally:
            self.redis.close()

    def enqueue(self, payload, maximum=2):
        return self.redis.eval(self.enqueue_script, [self.ready, self.pending, self.dead, self.notify, self.published], payload, maximum)

    def claim(self, delivery, lifetime=30000):
        return self.redis.eval(self.claim_script, [self.ready, self.pending, self.deadlines, self.dead, self.notify, self.published], delivery, lifetime, 300000)

    def complete(self, delivery, reason=""):
        return self.redis.eval(self.complete_script, [self.pending, self.deadlines, self.dead, self.published], delivery, reason)

    def run_operation(self, operation, token="owner-one", argument=30000, app="app-one", job="job-one"):
        return self.redis.eval(self.lease_script, [self.run, self.lease], operation, app, job, token, argument)

    def create_run(self, expired=False):
        now = int(time.time() * 1000)
        value = {"app_id": "app-one", "status": "PENDING", "updated_at": now,
                 "started_at": None, "expires_at": now - 1 if expired else now + 60000}
        self.redis.command("SET", self.run, json.dumps(value), "PX", 60000)

    def test_fifo_admission_counts_inflight_and_ack_is_idempotent(self):
        first = '{"job_id":"first","synthetic_secret":"keep-exact-bytes"}'
        self.assertEqual(self.enqueue(first), 1)
        self.assertEqual(self.enqueue("second"), 1)
        self.assertEqual(self.claim("delivery-one"), first)
        self.assertEqual(self.redis.command("HGET", self.pending, "delivery-one"), first)
        self.assertEqual(self.enqueue("overflow"), 0)
        self.assertEqual(self.complete("wrong-delivery"), 0)
        self.assertEqual(self.complete("delivery-one"), 1)
        self.assertEqual(self.complete("delivery-one"), 0)
        self.assertEqual(self.enqueue("third"), 1)
        self.assertEqual(self.claim("delivery-two"), "second")

    def test_failure_retains_payload_and_consumes_admission(self):
        self.assertEqual(self.enqueue("accepted-payload", maximum=1), 1)
        self.assertEqual(self.claim("failed-delivery"), "accepted-payload")
        self.assertEqual(self.complete("failed-delivery", "execution_failed_requires_reconciliation"), 1)
        dead = json.loads(self.redis.command("LINDEX", self.dead, 0))
        self.assertEqual(dead["payload"], "accepted-payload")
        self.assertEqual(dead["reason"], "execution_failed_requires_reconciliation")
        self.assertEqual(self.enqueue("overflow", maximum=1), 0)
        self.assertIsNone(self.claim("next"))

    def test_expired_delivery_is_quarantined_without_reexecution(self):
        self.enqueue("uncertain-side-effects")
        self.claim("expired-delivery")
        # Force the server deadline into the past instead of sleeping in a test.
        self.redis.command("ZADD", self.deadlines, 0, "expired-delivery")
        self.assertIsNone(self.claim("new-delivery"))
        self.assertEqual(self.complete("expired-delivery"), 0)
        dead = json.loads(self.redis.command("LINDEX", self.dead, 0))
        self.assertEqual(dead["payload"], "uncertain-side-effects")
        self.assertEqual(dead["reason"], "delivery_expired_requires_reconciliation")
        self.assertEqual(self.redis.command("HLEN", self.pending), 0)

    def test_non_admission_requeues_exact_bytes_once_and_preserves_backlog(self):
        original = '{"job_id":"accepted","credential":"opaque-test-value"}'
        self.enqueue(original)
        self.enqueue("second")
        self.assertEqual(self.claim("not-admitted"), original)
        retry = script(QUEUE_SOURCE, "REQUEUE")
        keys = [self.pending, self.deadlines, self.ready, self.notify]
        self.assertEqual(self.redis.eval(retry, keys, "wrong-owner"), 0)
        self.assertEqual(self.redis.eval(retry, keys, "not-admitted"), 1)
        self.assertEqual(self.redis.eval(retry, keys, "not-admitted"), 0)
        self.assertEqual(self.redis.command("HLEN", self.pending), 0)
        self.assertEqual(self.redis.command("ZCARD", self.deadlines), 0)
        self.assertEqual(self.redis.command("LLEN", self.dead), 0)
        self.assertEqual(self.enqueue("overflow"), 0)
        self.assertEqual(self.claim("next"), "second")
        self.assertEqual(self.claim("retry"), original)

    def test_non_admission_cannot_requeue_expired_or_already_quarantined_claim(self):
        self.enqueue("retained")
        self.claim("expired")
        self.redis.command("ZADD", self.deadlines, 0, "expired")
        retry = script(QUEUE_SOURCE, "REQUEUE")
        keys = [self.pending, self.deadlines, self.ready, self.notify]
        self.assertEqual(self.redis.eval(retry, keys, "expired"), 0)
        self.assertEqual(self.redis.command("HGET", self.pending, "expired"), "retained")
        self.assertIsNone(self.claim("reconcile"))
        self.assertEqual(self.redis.eval(retry, keys, "expired"), 0)
        self.assertEqual(self.redis.command("LLEN", self.dead), 1)

    def test_notifications_are_bounded_and_wake_an_idle_consumer(self):
        import threading
        waiting = Redis(os.environ["REDIS_TEST_URL"])
        self.addCleanup(waiting.close)
        result = []
        started = threading.Event()
        def wait():
            started.set()
            result.append(waiting.command("BLPOP", self.notify, 2))
        worker = threading.Thread(target=wait)
        worker.start()
        started.wait(timeout=1)
        self.enqueue("first", maximum=100)
        worker.join(timeout=3)
        self.assertFalse(worker.is_alive())
        self.assertEqual(result, [[self.notify, "ready"]])
        for number in range(20):
            self.enqueue(str(number), maximum=100)
        self.assertEqual(self.redis.command("LLEN", self.notify), 1)
        self.redis.command("DEL", self.notify)
        self.assertEqual(self.claim("first"), "first")
        self.assertEqual(self.redis.command("RPOP", self.notify), "ready")

    def test_expired_queue_wait_is_retained_before_any_execution_claim(self):
        import hashlib
        payload = "waited-too-long"
        self.enqueue(payload)
        identity = hashlib.sha1(payload.encode()).hexdigest()
        self.redis.command("HSET", self.published, identity, 1)
        self.assertIsNone(self.claim("never-started"))
        self.assertEqual(self.redis.command("HLEN", self.pending), 0)
        self.assertEqual(self.redis.command("HLEN", self.published), 0)
        dead = json.loads(self.redis.command("LINDEX", self.dead, 0))
        self.assertEqual(dead["payload"], payload)
        self.assertEqual(dead["reason"], "queue_wait_expired_before_execution")

    def test_retry_keeps_original_publication_time_and_settlement_cleans_metadata(self):
        import hashlib
        payload = "retained-publication"
        self.enqueue(payload)
        identity = hashlib.sha1(payload.encode()).hexdigest()
        published = self.redis.command("HGET", self.published, identity)
        self.claim("first")
        self.redis.eval(script(QUEUE_SOURCE, "REQUEUE"), [self.pending, self.deadlines, self.ready, self.notify], "first")
        self.assertEqual(self.redis.command("HGET", self.published, identity), published)
        self.claim("retry")
        self.complete("retry")
        self.assertEqual(self.redis.command("HLEN", self.published), 0)

    def test_run_lease_fences_app_job_and_owner_and_finishes_once(self):
        self.create_run()
        self.assertEqual(self.run_operation("claim", app="other-app")[0], -1)
        claim = self.run_operation("claim")
        self.assertEqual(claim[0], 1)
        self.assertEqual(json.loads(claim[1])["status"], "RUNNING")
        self.assertEqual(self.run_operation("claim", token="competing-owner")[0], 2)
        self.assertEqual(self.run_operation("claim", job="different-job")[0], -1)
        self.assertEqual(self.run_operation("validate", token="wrong-owner", argument="")[0], -1)
        renewed = self.run_operation("claim", argument=45000)
        self.assertGreater(renewed[2], claim[2])
        update = json.dumps({"status": "COMPLETED", "progress": 100})
        self.assertEqual(self.run_operation("finish", argument=update)[0], 1)
        self.assertEqual(self.run_operation("claim")[0], 3)
        self.assertEqual(self.run_operation("validate", argument="")[0], -1)
        self.assertEqual(json.loads(self.redis.command("GET", self.run))["status"], "COMPLETED")

    def test_expired_owner_cannot_finish_after_takeover(self):
        self.create_run()
        self.assertEqual(self.run_operation("claim")[0], 1)
        lease = json.loads(self.redis.command("GET", self.lease))
        lease["expires_at"] = 1
        self.redis.command("SET", self.lease, json.dumps(lease), "PX", 60000)
        self.assertEqual(self.run_operation("validate", argument="")[0], -1)
        self.assertEqual(self.run_operation("claim", token="new-owner")[0], 1)
        update = json.dumps({"status": "FAILED"})
        self.assertEqual(self.run_operation("finish", argument=update)[0], -1)
        self.assertEqual(json.loads(self.redis.command("GET", self.run))["status"], "RUNNING")

    def test_unleased_updates_and_expired_runs_fail_closed(self):
        self.create_run()
        self.run_operation("claim")
        overwrite = json.dumps({"app_id": "app-one", "status": "COMPLETED"})
        result = self.redis.eval(script(STATE_SOURCE, "UPDATE_MUTABLE_RUN_SCRIPT"),
                                 [self.run, self.lease], overwrite, 60)
        self.assertEqual(result[0], 3)
        self.redis.command("DEL", self.run, self.lease)
        self.create_run(expired=True)
        self.assertEqual(self.run_operation("claim")[0], 0)

    def test_confirmed_cancellation_revokes_owner_and_prevents_reexecution(self):
        self.create_run()
        self.run_operation("claim")
        cancellation = script(STATE_SOURCE, "CANCEL_RUN_SCRIPT")
        self.assertEqual(self.redis.eval(cancellation, [self.run, self.lease], "other-app")[0], -1)
        self.assertEqual(self.redis.command("EXISTS", self.lease), 1)
        result = self.redis.eval(cancellation, [self.run, self.lease], "app-one")
        self.assertEqual(result[0], 1)
        self.assertEqual(json.loads(result[1])["status"], "CANCELLED")
        self.assertEqual(self.redis.command("EXISTS", self.lease), 0)
        self.assertEqual(self.run_operation("claim", token="next-delivery")[0], 3)
        self.assertEqual(self.run_operation("finish", argument=json.dumps({"status": "COMPLETED"}))[0], -1)
        self.assertEqual(self.redis.eval(cancellation, [self.run, self.lease], "app-one")[0], 2)
        self.redis.command("DEL", self.run, self.lease)
        self.create_run()
        self.run_operation("claim")
        self.run_operation("finish", argument=json.dumps({"status": "COMPLETED"}))
        terminal = self.redis.eval(cancellation, [self.run, self.lease], "app-one")
        self.assertEqual(terminal[0], 2)
        self.assertEqual(json.loads(terminal[1])["status"], "COMPLETED")
        self.assertEqual(self.redis.command("EXISTS", self.lease), 0)


if __name__ == "__main__":
    unittest.main()
