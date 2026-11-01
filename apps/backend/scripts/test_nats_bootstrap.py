"""Verify generated trust against a real NATS server when NATS_SERVER_BIN is set."""

import base64
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import unittest

import nats_bootstrap


def decode_jwt(value):
    part = value.split(".")[1]
    return json.loads(base64.urlsafe_b64decode(part + "=" * (-len(part) % 4)))


class NatsBootstrapTest(unittest.TestCase):
    def test_broker_configuration_contains_only_public_trust(self):
        trust = nats_bootstrap.generate()
        config = nats_bootstrap.server_config(trust, ["https://app.example.test"])
        self.assertNotIn(trust["account_seed"], config)
        self.assertTrue(trust["account_seed"].startswith("SA"))
        account = decode_jwt(trust["account_jwt"])
        self.assertEqual(account["sub"], trust["account_public_key"])
        self.assertFalse(account["nats"]["disallow_bearer"])
        self.assertEqual(json.loads(config)["websocket"]["allowed_origins"], ["https://app.example.test"])
        with self.assertRaises(ValueError):
            nats_bootstrap.server_config(trust, [])
        with self.assertRaisesRegex(ValueError, "HTTP"):
            nats_bootstrap.server_config(trust, ["tauri://localhost"])

    def test_cli_preserves_existing_credentials_and_keeps_output_private(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "nats.env"
            command = [sys.executable, str(Path(nats_bootstrap.__file__)),
                       "--output", str(output), "--web-origin", "https://app.example.test",
                       "--websocket-url", "wss://api.example.test/nats"]
            result = subprocess.run(command, capture_output=True, text=True, check=True)
            original = output.read_bytes()
            self.assertEqual(output.stat().st_mode & 0o777, 0o600)
            self.assertNotIn("CHANNEL_NATS_ACCOUNT_SEED", result.stdout)
            result = subprocess.run(command, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(output.read_bytes(), original)


@unittest.skipUnless(os.environ.get("NATS_SERVER_BIN") or shutil.which("nats-server"), "Set NATS_SERVER_BIN to run broker integration tests")
class NatsTrustIntegrationTest(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="flow-like-nats-trust-test-")
        self.addCleanup(self.directory.cleanup)
        self.trust = nats_bootstrap.generate()
        config = json.loads(nats_bootstrap.server_config(self.trust, ["https://app.example.test"]))
        with socket.socket() as port:
            port.bind(("127.0.0.1", 0))
            self.port = port.getsockname()[1]
        config.update({"host": "127.0.0.1", "port": self.port, "http_port": -1})
        config.pop("websocket")
        path = Path(self.directory.name) / "nats.conf"
        path.write_text(json.dumps(config))
        self.server = subprocess.Popen([os.environ.get("NATS_SERVER_BIN") or shutil.which("nats-server"), "-c", str(path)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.addCleanup(self.stop)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                with socket.create_connection(("127.0.0.1", self.port), timeout=.1):
                    return
            except OSError:
                time.sleep(.02)
        self.fail("NATS did not start with generated trust")

    def stop(self):
        self.server.terminate()
        self.server.wait(timeout=5)

    def token(self, *, expired=False):
        seed = self.trust["account_seed"]
        decoded = base64.b32decode(seed + "=" * (-len(seed) % 8))
        private = bytes.fromhex("302e020100300506032b657004220420") + decoded[2:-2]
        _, public, _ = nats_bootstrap.keypair(160)
        now = int(time.time())
        return nats_bootstrap.signed_jwt(private, {
            "iss": self.trust["account_public_key"], "sub": public,
            "iat": now - 10, "exp": now - 1 if expired else now + 60,
            "nats": {"type": "user", "version": 2, "bearer_token": True,
                     "pub": {"allow": ["channel.run.inbox"]},
                     "sub": {"allow": ["channel.run.inbox.ack.>"]},
                     "subs": 2, "data": -1, "payload": 1048576},
        })

    def connect(self, token=None):
        connection = socket.create_connection(("127.0.0.1", self.port), timeout=2)
        self.addCleanup(connection.close)
        connection.recv(8192)
        connect = {"verbose": False, "pedantic": True, "lang": "python", "version": "test", "protocol": 1}
        if token:
            connect["jwt"] = token
        connection.sendall(b"CONNECT " + json.dumps(connect).encode() + b"\r\nPING\r\n")
        return connection, connection.recv(8192)

    def test_expiring_scoped_bearer_credentials_authenticate(self):
        connection, reply = self.connect(self.token())
        self.assertIn(b"PONG", reply)
        connection.sendall(b"PUB channel.another-run.inbox 2\r\n{}\r\nPING\r\n")
        self.assertIn(b"Permissions Violation", connection.recv(8192))

    def test_anonymous_and_expired_credentials_are_rejected(self):
        for token in (None, self.token(expired=True)):
            with self.subTest(expired=token is not None):
                _, reply = self.connect(token)
                self.assertIn(b"Authorization Violation", reply)


if __name__ == "__main__":
    unittest.main()
