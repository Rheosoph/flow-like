import importlib.util
import base64
import hashlib
import subprocess
from unittest.mock import patch
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("standalone_release", Path(__file__).parents[1] / "standalone_release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class StandaloneReleaseTests(unittest.TestCase):
    def fixture(self, directory):
        for target in release.TARGETS:
            (directory / f"flow-like-standalone-{target}").write_bytes(b"binary " + target.encode())
            (directory / f"{target}.json").write_text(json.dumps({"version": "1.2.3", "target": target, "state_schema_version": 4, "runtime": True}))

    def test_manifest_binds_every_target_size_digest_schema_and_pinned_image(self):
        with tempfile.TemporaryDirectory() as folder:
            directory = Path(folder)
            self.fixture(directory)
            value = release.manifest(directory, "https://releases.example/v1", 12, "ghcr.io/example/agent@sha256:" + "a" * 64, directory / "release.json", issued_at=100)
            self.assertEqual(value["sequence"], 12)
            self.assertEqual(value["state_schema_version"], 4)
            self.assertEqual(value["expires_at"], 100 + 30 * 86400)
            self.assertEqual(len(value["artifacts"]), 4)
            self.assertTrue(all(len(artifact["sha256"]) == 64 for artifact in value["artifacts"]))
            self.assertEqual((directory / "release.json").read_text(), json.dumps(value, separators=(",", ":")))

    def test_refuses_unpinned_image_mixed_schema_missing_binary_and_credential_urls(self):
        with tempfile.TemporaryDirectory() as folder:
            directory = Path(folder)
            self.fixture(directory)
            args = (directory, "https://releases.example/v1", 12, "ghcr.io/example/agent@sha256:" + "a" * 64, directory / "release.json")
            for url in ["http://releases.example/v1", "https://user:password@releases.example/v1", "https://releases.example/v1?token=x"]:
                with self.assertRaises(ValueError):
                    release.manifest(args[0], url, *args[2:])
            with self.assertRaises(ValueError):
                release.manifest(args[0], args[1], args[2], "ghcr.io/example/agent:latest", args[4])
            target = next(iter(release.TARGETS))
            metadata = directory / f"{target}.json"
            info = json.loads(metadata.read_text())
            info["state_schema_version"] = 5
            metadata.write_text(json.dumps(info))
            with self.assertRaises(ValueError):
                release.manifest(*args)


class MemoryStore:
    def __init__(self):
        self.objects = {}
        self.writes = []
        self.race = None

    def read(self, key, maximum=16384, retain=True):
        value = self.objects.get(key)
        if value is None:
            return None
        if len(value) > maximum:
            raise ValueError("too large")
        digest = hashlib.sha256(value).hexdigest()
        return {"etag": digest, "sha256": digest, "size": len(value), "body": value if retain else None}

    def put(self, key, path, *, etag=None, immutable=True):
        if not immutable and self.race:
            self.objects[key] = self.race
        previous = self.objects.get(key)
        if ((etag is None and previous is not None)
                or (etag is not None and (previous is None or hashlib.sha256(previous).hexdigest() != etag))):
            raise FileExistsError()
        self.objects[key] = path.read_bytes()
        self.writes.append((key, immutable))


class StandalonePublisherTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.directory = Path(self.folder.name)
        subprocess.run(["openssl", "genpkey", "-algorithm", "ED25519", "-out", str(self.directory / "private.pem")], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        public = subprocess.check_output(["openssl", "pkey", "-in", str(self.directory / "private.pem"), "-pubout", "-outform", "DER"], stderr=subprocess.DEVNULL)[-32:]
        self.key = base64.urlsafe_b64encode(public).decode().rstrip("=")
        StandaloneReleaseTests().fixture(self.directory)

    def tearDown(self):
        self.folder.cleanup()

    def signed(self, sequence):
        value = release.manifest(self.directory, f"https://cdn.example/standalone/releases/{sequence}", sequence, "ghcr.io/example/agent@sha256:" + "a" * 64, self.directory / "release.json")
        jwk = json.dumps({"crv":"Ed25519", "kty":"OKP", "x":self.key}, separators=(",", ":")).encode()
        def encode(value):
            return base64.urlsafe_b64encode(value).decode().rstrip("=")
        header = {"alg":"EdDSA", "typ":"flow-like-standalone-release+jws", "kid":encode(hashlib.sha256(jwk).digest())}
        message = (encode(json.dumps(header).encode()) + "." + encode(json.dumps(value).encode())).encode()
        (self.directory / "message").write_bytes(message)
        signature = subprocess.check_output(["openssl", "pkeyutl", "-sign", "-rawin", "-inkey", str(self.directory / "private.pem"), "-in", str(self.directory / "message")], stderr=subprocess.DEVNULL)
        compact = message + b"." + encode(signature).encode()
        (self.directory / "release.jws").write_bytes(compact)
        return compact

    def publish(self, store, verify=lambda *_: None, verify_container=lambda *_: None):
        return release.publish_bundle(self.directory, "https://cdn.example/standalone", "standalone", [self.key], store, verify, verify_container)

    def test_anonymous_container_failure_never_uploads_or_promotes_release(self):
        signed = self.signed(10)
        store = MemoryStore()
        self.publish(store)
        self.signed(11)
        previous_writes = list(store.writes)
        def private_image(_): raise ValueError("image is not public")
        with self.assertRaisesRegex(ValueError, "not public"):
            self.publish(store, verify_container=private_image)
        self.assertEqual(store.objects["standalone/release.jws"], signed)
        self.assertEqual(store.writes, previous_writes)

    def test_anonymous_pulls_use_each_exact_platform_without_credentials_or_helpers(self):
        container = {"image": "ghcr.io/example/agent@sha256:" + "a" * 64,
                     "platforms": ["linux/amd64", "linux/arm64"]}
        platforms = []
        def pull(command, **options):
            self.assertEqual(command[-1], container["image"])
            platforms.append(command[-2])
            config = Path(command[2])
            self.assertEqual(json.loads((config / "config.json").read_text()), {"auths": {}})
            self.assertEqual(options["env"], {"PATH": str(config), "DOCKER_CONFIG": str(config)})
            self.assertEqual(command[3:5], ["--host", "unix:///var/run/docker.sock"])
            self.assertEqual(options["stderr"], subprocess.DEVNULL)
            return subprocess.CompletedProcess(command, 0)
        with patch.object(release.shutil, "which", return_value="/usr/bin/docker"), patch.object(release.subprocess, "run", side_effect=pull):
            release.anonymous_container_readback(container)
        self.assertEqual(platforms, container["platforms"])
        with patch.object(release.shutil, "which", return_value="/usr/bin/docker"), patch.object(release.subprocess, "run", return_value=subprocess.CompletedProcess([], 1)):
            with self.assertRaisesRegex(ValueError, "anonymously pullable"):
                release.anonymous_container_readback(container)

    def test_oversized_binary_requires_a_usable_signed_docker_platform(self):
        records = [{"target": "x86_64-unknown-linux-gnu", "size": release.MAX_BROWSER_BINARY_BYTES + 1}]
        release.usable_package_modes(records, {"platforms": ["linux/amd64"]})
        for container in [None, {"platforms": ["linux/arm64"]}]:
            with self.assertRaisesRegex(ValueError, "256 MiB"):
                release.usable_package_modes(records, container)
        records[0]["target"] = "aarch64-apple-darwin"
        with self.assertRaisesRegex(ValueError, "no signed Docker alternative"):
            release.usable_package_modes(records, {"platforms": ["linux/arm64"]})
        records[0]["size"] = release.MAX_BROWSER_BINARY_BYTES
        release.usable_package_modes(records, None)

    def test_signed_immutable_artifacts_read_back_before_manifest_and_idempotent_retry(self):
        signed = self.signed(10)
        store = MemoryStore()
        verified = []
        def readback(url, size, digest):
            if url.endswith("/standalone/release.jws"):
                self.assertEqual(len(verified), 5)
            else:
                self.assertNotIn("standalone/release.jws", store.objects)
            verified.append(url)
        self.assertEqual(self.publish(store, readback)["sequence"], 10)
        self.assertEqual(store.writes[-1], ("standalone/release.jws", False))
        self.assertEqual(store.objects["standalone/release.jws"], signed)
        writes = list(store.writes)
        self.publish(store)
        self.assertEqual(store.writes, writes)

    def test_modified_immutable_binary_or_bad_public_delivery_never_advances_manifest(self):
        self.signed(10)
        store = MemoryStore()
        name = f"standalone/releases/10/flow-like-standalone-{next(iter(release.TARGETS))}"
        store.objects[name] = b"malicious replacement"
        with self.assertRaises(ValueError):
            self.publish(store)
        self.assertNotIn("standalone/release.jws", store.objects)
        store = MemoryStore()
        def bad_cdn(*_): raise ValueError("CDN denied CORS")
        with self.assertRaises(ValueError):
            self.publish(store, bad_cdn)
        self.assertNotIn("standalone/release.jws", store.objects)

    def test_older_release_and_concurrent_publisher_cannot_replace_stable_head(self):
        old = self.signed(10)
        store = MemoryStore()
        self.publish(store)
        self.signed(9)
        with self.assertRaisesRegex(ValueError, "roll back"):
            self.publish(store)
        self.assertEqual(store.objects["standalone/release.jws"], old)
        newer = self.signed(12)
        store.race = newer
        self.signed(11)
        with self.assertRaisesRegex(ValueError, "concurrently"):
            self.publish(store)
        self.assertEqual(store.objects["standalone/release.jws"], newer)

    def test_expiry_during_upload_cannot_promote_stable_manifest(self):
        old = self.signed(10)
        store = MemoryStore()
        self.publish(store)
        signed = self.signed(11)
        expiry = release.verified_release(signed, [self.key])["expires_at"]
        with patch.object(release.time, "time", return_value=expiry - 1) as clock:
            def finish_upload(*_):
                clock.return_value = expiry
            with self.assertRaisesRegex(ValueError, "expired"):
                self.publish(store, finish_upload)
        self.assertEqual(store.objects["standalone/release.jws"], old)

    def test_wrong_signature_untrusted_key_and_mutated_local_binary_fail_before_upload(self):
        signed = self.signed(10)
        mutated = signed[:-1] + (b"A" if signed[-1:] != b"A" else b"B")
        with self.assertRaises(ValueError):
            release.verified_release(mutated, [self.key])
        with self.assertRaises(ValueError):
            release.verified_release(signed, [base64.urlsafe_b64encode(bytes(32)).decode().rstrip("=")])
        (self.directory / f"flow-like-standalone-{next(iter(release.TARGETS))}").write_bytes(b"changed")
        store = MemoryStore()
        with self.assertRaises(ValueError):
            self.publish(store)
        self.assertEqual(store.writes, [])

    def test_public_readback_requires_anonymous_cors_and_exact_bytes(self):
        class Response:
            status = 200
            headers = {"Access-Control-Allow-Origin":"*"}
            def __enter__(self): return self
            def __exit__(self, *_): pass
            def read(self, _):
                value, self.body = self.body, b""
                return value
        response = Response()
        def open_response(*args, **kwargs):
            response.body = b"artifact"
            return response
        with patch.object(release, "build_opener") as opener:
            opener.return_value.open.side_effect = open_response
            digest = hashlib.sha256(b"artifact").hexdigest()
            release.public_readback("https://cdn.example/file", 8, digest)
            response.headers = {}
            with self.assertRaises(ValueError):
                release.public_readback("https://cdn.example/file", 8, digest)
            response.headers = {"Access-Control-Allow-Origin":"*"}
            with self.assertRaises(ValueError):
                release.public_readback("https://cdn.example/file", 8, "0"*64)
        with self.assertRaises(ValueError):
            release.NoRedirect().redirect_request(None, None, 302, "", {}, "https://other.example")


if __name__ == "__main__":
    unittest.main()
