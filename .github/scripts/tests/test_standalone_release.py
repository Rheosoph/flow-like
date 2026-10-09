import importlib.util
import base64
import contextlib
import hashlib
import io
import subprocess
from unittest.mock import patch
from urllib.error import HTTPError
import json
from pathlib import Path
import tempfile
import time
import unittest

spec = importlib.util.spec_from_file_location("standalone_release", Path(__file__).parents[1] / "standalone_release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)

DAY = 86400
IMAGE = "ghcr.io/example/agent@sha256:" + "a" * 64
BASE = "https://cdn.example/standalone"
STABLE_URL, STABLE_KEY = f"{BASE}/release.jws", "standalone/release.jws"
DATES = ("issued_at", "expires_at")
LINUX, MAC = "x86_64-unknown-linux-gnu", "aarch64-apple-darwin"
RUNTIME = release.RUNTIME_MANIFEST_JWS_TYPE


def runtime_url(target):
    return f"{BASE}/runtimes/{target}.jws"


def runtime_key(target):
    return f"standalone/runtimes/{target}.jws"


def runtime_list(target, sequence, issued_at=None, validity_days=365):
    pack = {"runtime": "llamacpp", "build": "b10809", "target": target, "backend": "cpu",
            "url": f"{BASE}/releases/{sequence}/runtimes/llamacpp-b10809-{target}-cpu.tar.gz", "size": 4096,
            "sha256": "a" * 64, "entrypoint": "llama-server",
            "files": [{"path": "llama-server", "size": 4096, "sha256": "b" * 64, "executable": True}]}
    return {"version": 1, "sequence": sequence, **release.validity(validity_days, issued_at), "packs": [pack]}


def accepted_by_agent_0_1_0(value, clock):
    # The check compiled into the agents of releases 1 and 2: a 30-day cap and no clock tolerance.
    return (0 < value["expires_at"] - value["issued_at"] <= 30 * DAY
            and value["issued_at"] <= clock < value["expires_at"])


def without_dates(value):
    return {name: field for name, field in value.items() if name not in DATES}


class StandaloneReleaseTests(unittest.TestCase):
    def fixture(self, directory):
        for target in release.TARGETS:
            (directory / f"flow-like-standalone-{target}").write_bytes(b"binary " + target.encode())
            (directory / f"{target}.json").write_text(json.dumps({"version": "1.2.3", "target": target, "state_schema_version": 4, "runtime": True}))

    def test_manifest_binds_every_target_size_digest_schema_and_pinned_image(self):
        with tempfile.TemporaryDirectory() as folder:
            directory = Path(folder)
            self.fixture(directory)
            value = release.manifest(directory, "https://releases.example/v1", 12, IMAGE, directory / "release.json", issued_at=100)
            self.assertEqual(value["sequence"], 12)
            self.assertEqual(value["state_schema_version"], 4)
            self.assertEqual(value["issued_at"], 100)
            self.assertEqual(value["expires_at"], 100 + 365 * DAY)
            self.assertEqual([artifact["target"] for artifact in value["artifacts"]], [
                "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "aarch64-apple-darwin",
            ])
            self.assertTrue(all(len(artifact["sha256"]) == 64 for artifact in value["artifacts"]))
            self.assertEqual((directory / "release.json").read_text(), json.dumps(value, separators=(",", ":")))

    def test_validity_is_chosen_at_signing_between_one_day_and_the_cap(self):
        self.assertEqual((release.DEFAULT_RELEASE_LIFETIME_DAYS, release.MAX_RELEASE_LIFETIME_DAYS), (365, 1825))
        with tempfile.TemporaryDirectory() as folder:
            directory = Path(folder)
            self.fixture(directory)
            output = directory / "release.json"
            args = (directory, "https://releases.example/v1", 12, IMAGE, output)
            for days in [1, 30, 365, 1825]:
                value = release.manifest(*args, issued_at=100, validity_days=days)
                self.assertEqual(value["expires_at"] - value["issued_at"], days * DAY)
            output.unlink()
            for days in [0, 1826, -30, True, 30.0, "30", None]:
                with self.assertRaisesRegex(ValueError, "between 1 and 1825 days"):
                    release.manifest(*args, validity_days=days)
            self.assertFalse(output.exists())

    def test_lists_are_dated_back_so_released_agents_with_a_slow_clock_accept_a_30_day_list(self):
        with tempfile.TemporaryDirectory() as folder:
            directory = Path(folder)
            self.fixture(directory)
            args = (directory, "https://releases.example/v1", 12, IMAGE, directory / "release.json")
            signer_clock = 1_800_000_000
            with patch.object(release.time, "time", return_value=signer_clock + 0.9):
                value = release.manifest(*args, validity_days=30)
                longer = release.manifest(*args, validity_days=31)
            self.assertEqual(release.ISSUED_BACKDATE_SECONDS, 300)
            self.assertEqual(value["issued_at"], signer_clock - 300)
            self.assertEqual(value["expires_at"] - value["issued_at"], 2_592_000)
            for behind in [0, 299, 300]:
                self.assertTrue(accepted_by_agent_0_1_0(value, signer_clock - behind))
            self.assertFalse(accepted_by_agent_0_1_0(value, signer_clock - 301))
            self.assertFalse(accepted_by_agent_0_1_0(longer, signer_clock))

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
        self.keys = json.dumps([self.key])
        self.requests = []
        StandaloneReleaseTests().fixture(self.directory)

    def tearDown(self):
        self.folder.cleanup()

    def jws(self, value, typ=release.RELEASE_JWS_TYPE):
        jwk = json.dumps({"crv":"Ed25519", "kty":"OKP", "x":self.key}, separators=(",", ":")).encode()
        def encode(value):
            return base64.urlsafe_b64encode(value).decode().rstrip("=")
        header = {"alg":"EdDSA", "typ":typ, "kid":encode(hashlib.sha256(jwk).digest())}
        message = (encode(json.dumps(header).encode()) + "." + encode(json.dumps(value).encode())).encode()
        (self.directory / "message").write_bytes(message)
        signature = subprocess.check_output(["openssl", "pkeyutl", "-sign", "-rawin", "-inkey", str(self.directory / "private.pem"), "-in", str(self.directory / "message")], stderr=subprocess.DEVNULL)
        return message + b"." + encode(signature).encode()

    def sign(self, value):
        compact = self.jws(value)
        (self.directory / "release.jws").write_bytes(compact)
        return compact

    def sign_runtime(self, target, value):
        """A re-dated runtime list as the renewal's sign job leaves it beside release.jws."""
        compact = self.jws(value, RUNTIME)
        (self.directory / "runtimes").mkdir(exist_ok=True)
        (self.directory / "runtimes" / f"{target}.jws").write_bytes(compact)
        return compact

    def runtime_renewal(self, store, target, **changes):
        published = release.verified_release(store.objects[runtime_key(target)], [self.key], False, RUNTIME)
        return self.sign_runtime(target, {**published, **release.validity(changes.pop("validity_days", 365)), **changes})

    def published_with_runtime_lists(self, sequence, *targets, validity_days=30):
        """A published release and its runtime lists, issued a day ago, as the release workflow leaves them."""
        dates = {"issued_at": int(time.time()) - DAY, "validity_days": validity_days}
        self.signed(sequence, **dates)
        store = MemoryStore()
        self.publish(store)
        for target in targets:
            store.objects[runtime_key(target)] = self.jws(runtime_list(target, sequence, **dates), RUNTIME)
        return store

    def signed(self, sequence, **validity):
        return self.sign(release.manifest(self.directory, f"https://cdn.example/standalone/releases/{sequence}", sequence, IMAGE, self.directory / "release.json", **validity))

    def publish(self, store, verify=lambda *_: None, verify_container=lambda *_: None):
        return release.publish_bundle(self.directory, BASE, "standalone", [self.key], store, verify, verify_container)

    def cdn(self, objects):
        def published(url, method="GET", limit=16384):
            self.requests.append((method, url))
            return objects.get(url)
        return patch.object(release, "published_object", side_effect=published)

    def renewal(self, store, **changes):
        published = release.verified_release(store.objects[STABLE_KEY], [self.key], current=False)
        return self.sign({**published, **release.validity(changes.pop("validity_days", 365)), **changes})

    def renew(self, store, verify=lambda *_: None):
        return release.renew_bundle(self.directory, BASE, "standalone", [self.key], store, verify)

    def test_preflight_refuses_published_or_partly_uploaded_sequences(self):
        base = BASE
        stable = self.signed(5)
        requests, cdn, keys = self.requests, self.cdn, self.keys
        with cdn({f"{base}/release.jws": stable}):
            with self.assertRaisesRegex(ValueError, "sequence 5 must be greater than the published sequence 5"):
                release.preflight(base, 5, keys)
            release.preflight(base, 6, keys)
        self.assertEqual(requests, [
            ("GET", STABLE_URL), ("GET", STABLE_URL),
            *[("GET", runtime_url(target)) for target in release.TARGETS],
            *[("HEAD", f"{base}/releases/6/flow-like-standalone-{target}") for target in release.TARGETS],
            ("HEAD", f"{base}/releases/6/release.jws"),
            *[("HEAD", f"{base}/releases/6/runtimes/{target}.jws") for target in release.TARGETS],
        ])
        partial = f"{base}/releases/1/flow-like-standalone-x86_64-unknown-linux-gnu"
        with cdn({partial: b""}), self.assertRaisesRegex(ValueError, "already uploaded flow-like-standalone-x86_64-unknown-linux-gnu for sequence 1"):
            release.preflight(base, 1, keys)

    def test_preflight_refuses_runtime_sequences_even_when_the_agent_release_did_not_publish(self):
        agent = self.signed(4)
        for target in release.TARGETS:
            # Expiry does not free a sequence for reuse.
            runtime = self.jws(runtime_list(target, 6, issued_at=100), RUNTIME)
            for published_agent in ({}, {STABLE_URL: agent}):
                with self.subTest(target=target, agent=bool(published_agent)), \
                        self.cdn({**published_agent, runtime_url(target): runtime}):
                    for sequence in (5, 6):
                        with self.assertRaisesRegex(ValueError, f"Release sequence {sequence} must be greater than the {target} runtime manifest sequence 6") as error:
                            release.preflight(BASE, sequence, self.keys, "1.2.4")
                        self.assertIn("Dispatch a new run with an unused higher sequence", str(error.exception))
                    release.preflight(BASE, 7, self.keys, "1.2.4")

    def test_preflight_refuses_immutable_runtime_manifests_before_any_stable_head_advances(self):
        agent = self.signed(5)
        for target in release.TARGETS:
            name = f"runtimes/{target}.jws"
            url = f"{BASE}/releases/6/{name}"
            with self.subTest(target=target), self.cdn({STABLE_URL: agent, url: b""}):
                with self.assertRaisesRegex(ValueError, f"already uploaded {name} for sequence 6; dispatch with a higher sequence"):
                    release.preflight(BASE, 6, self.keys, "1.2.4")
                self.assertIn(("HEAD", url), self.requests)
                release.preflight(BASE, 7, self.keys, "1.2.4")

    def test_preflight_authenticates_published_runtime_heads(self):
        target = "aarch64-apple-darwin"
        value = runtime_list(target, 6)
        valid = self.jws(value, RUNTIME)
        for compact in (self.jws(value), valid[:-8] + b"AAAAAAAA"):
            with self.subTest(compact=compact), self.cdn({runtime_url(target): compact}):
                with self.assertRaises(ValueError):
                    release.preflight(BASE, 7, self.keys)

    def test_preflight_refuses_a_release_whose_agent_version_was_not_bumped(self):
        with self.cdn({STABLE_URL: self.signed(3)}):
            for sequence in [4, 5]:
                with self.subTest(sequence=sequence):
                    with self.assertRaises(ValueError) as error:
                        release.preflight(BASE, sequence, self.keys, "1.2.3")
                    message = str(error.exception)
                    self.assertIn(f"Requested sequence {sequence} uses agent version 1.2.3", message)
                    self.assertIn("already published in sequence 3", message)
                    self.assertIn("bump the agent version in apps/standalone/Cargo.toml and update Cargo.lock", message)
                    self.assertIn("Changing the sequence alone does not change the agent version", message)
                    release.preflight(BASE, sequence, self.keys, "1.2.4")
            release.preflight(BASE, 4, self.keys)
        with self.cdn({}):
            release.preflight(BASE, 1, self.keys, "1.2.3")

    def test_verification_caps_the_lifetime_at_1825_days(self):
        release.verified_release(self.signed(10, validity_days=365), [self.key])
        longest = release.manifest(self.directory, f"{BASE}/releases/10", 10, IMAGE, self.directory / "release.json", validity_days=1825)
        self.assertEqual(release.verified_release(self.sign(longest), [self.key]), longest)
        beyond = self.sign({**longest, "expires_at": longest["expires_at"] + 1})
        for current in [True, False]:
            with self.assertRaisesRegex(ValueError, "lifetime"):
                release.verified_release(beyond, [self.key], current=current)
        store = MemoryStore()
        with self.assertRaisesRegex(ValueError, "lifetime"):
            self.publish(store)
        self.assertEqual(store.writes, [])

    def test_renewal_copies_the_published_list_and_changes_only_its_two_dates(self):
        stable = self.signed(5, validity_days=30)
        published = release.verified_release(stable, [self.key])
        output = self.directory / "renewal.json"
        clock = published["issued_at"] + 3 * DAY
        with self.cdn({STABLE_URL: stable}), patch.object(release.time, "time", return_value=clock + 0.9):
            value = release.renew_manifest(BASE, self.keys, output, 365, sequence=5)
        self.assertEqual(self.requests, [("GET", STABLE_URL)])
        self.assertEqual(value["issued_at"], clock - release.ISSUED_BACKDATE_SECONDS)
        self.assertEqual(value["expires_at"] - value["issued_at"], 365 * DAY)
        self.assertEqual(list(value), list(published))
        self.assertEqual(without_dates(value), without_dates(published))
        self.assertEqual(output.read_text(), json.dumps(value, separators=(",", ":")))
        with self.cdn({STABLE_URL: stable}), patch.object(release.time, "time", return_value=clock):
            bridge = release.renew_manifest(BASE, self.keys, output, 30)
        self.assertTrue(accepted_by_agent_0_1_0(bridge, clock - 300))

    def test_renewal_revives_a_list_that_ran_out(self):
        expired = self.signed(5, issued_at=100, validity_days=30)
        store = MemoryStore()
        with self.assertRaisesRegex(ValueError, "expired"):
            self.publish(store)
        self.assertEqual(store.writes, [])
        output = self.directory / "renewal.json"
        with self.cdn({STABLE_URL: expired}):
            value = release.renew_manifest(BASE, self.keys, output, only_if_days_left=60)
        self.assertEqual(value["expires_at"] - value["issued_at"], 365 * DAY)
        self.assertEqual(release.verified_release(self.sign(json.loads(output.read_text())), [self.key]), value)

    def test_renewal_keeps_a_release_within_the_cap_its_own_agents_were_built_with(self):
        published = release.verified_release(self.signed(2, validity_days=30), [self.key])
        first_agents = self.sign({**published, "release_version": "0.1.0"})
        output = self.directory / "renewal.json"
        with self.cdn({STABLE_URL: first_agents}):
            for days in [31, 365]:
                with self.assertRaisesRegex(ValueError, "Agents of release 0.1.0 accept at most 30 days"):
                    release.renew_manifest(BASE, self.keys, output, days, sequence=2)
            self.assertFalse(output.exists())
            value = release.renew_manifest(BASE, self.keys, output, 30, sequence=2)
        self.assertTrue(accepted_by_agent_0_1_0(value, int(time.time()) - 300))
        with self.cdn({STABLE_URL: self.sign({**published, "release_version": "0.1.1"})}):
            self.assertEqual(release.renew_manifest(BASE, self.keys, output, 1825)["release_version"], "0.1.1")

    def test_renewal_signs_nothing_for_an_unverified_missing_or_unexpected_list(self):
        stable = self.signed(5)
        output = self.directory / "renewal.json"
        signature = stable.rindex(b".") + 1
        mutated = stable[:signature] + (b"A" if stable[signature:signature + 1] != b"A" else b"B") + stable[signature + 1:]
        untrusted = json.dumps([base64.urlsafe_b64encode(bytes(32)).decode().rstrip("=")])
        for objects, keys, options, reason in [
            ({STABLE_URL: mutated}, self.keys, {}, "signature verification failed"),
            ({STABLE_URL: stable}, untrusted, {}, "not trusted"),
            ({}, self.keys, {}, "No published release to renew"),
            ({STABLE_URL: stable}, self.keys, {"sequence": 4}, "published release is number 5, not 4"),
            ({STABLE_URL: stable}, self.keys, {"validity_days": 0}, "between 1 and 1825 days"),
            ({STABLE_URL: stable}, self.keys, {"validity_days": 1826}, "between 1 and 1825 days"),
            ({STABLE_URL: stable}, self.keys, {"only_if_days_left": -1}, "zero or more"),
        ]:
            with self.subTest(reason), self.cdn(objects), self.assertRaisesRegex(ValueError, reason):
                release.renew_manifest(BASE, keys, output, **options)
        self.assertFalse(output.exists())

    def test_renewal_can_wait_until_few_days_are_left(self):
        stable = self.signed(5)
        output = self.directory / "renewal.json"
        notice = io.StringIO()
        with self.cdn({STABLE_URL: stable}), contextlib.redirect_stdout(notice):
            self.assertIsNone(release.renew_manifest(BASE, self.keys, output, only_if_days_left=60))
        self.assertFalse(output.exists())
        self.assertRegex(notice.getvalue(), r"^::notice::Release 5 is valid for 364 more days, more than 60; nothing was renewed\n$")
        with self.cdn({STABLE_URL: stable}):
            self.assertEqual(release.renew_manifest(BASE, self.keys, output, only_if_days_left=365)["sequence"], 5)
        self.assertTrue(output.exists())

    def test_renewed_list_replaces_only_the_stable_copy_and_is_read_back(self):
        bridge = {"issued_at": int(time.time()) - DAY, "validity_days": 30}
        first = self.signed(10, **bridge)
        store = MemoryStore()
        self.publish(store)
        writes = list(store.writes)
        renewed = self.renewal(store)
        read = []
        self.assertEqual(self.renew(store, lambda *call: read.append(call)), {"sequence": 10, "manifest_url": STABLE_URL})
        self.assertEqual(store.objects[STABLE_KEY], renewed)
        self.assertEqual(store.objects["standalone/releases/10/release.jws"], first)
        self.assertEqual(store.writes, writes + [(STABLE_KEY, False)])
        self.assertEqual(read, [(STABLE_URL, len(renewed), hashlib.sha256(renewed).hexdigest())])
        self.renew(store)
        self.assertEqual(store.writes, writes + [(STABLE_KEY, False)])
        shorter = self.renewal(store, validity_days=30)
        self.renew(store)
        self.assertEqual(store.objects[STABLE_KEY], shorter)
        self.assertEqual(self.signed(10, **bridge), first)
        with self.assertRaisesRegex(ValueError, "replace an existing signed release"):
            self.publish(store)
        self.assertEqual(store.objects[STABLE_KEY], shorter)

    def test_renewal_refuses_any_change_beyond_the_two_dates(self):
        self.signed(10, validity_days=30)
        store = MemoryStore()
        self.publish(store)
        stable, writes = store.objects[STABLE_KEY], list(store.writes)
        published = release.verified_release(stable, [self.key])
        artifact = {**published["artifacts"][0], "sha256": "0" * 64}
        for name, changed in {
            "sequence": 11,
            "release_version": "1.2.4",
            "state_schema_version": 5,
            "artifacts": [artifact] + published["artifacts"][1:],
            "container": {**published["container"], "image": "ghcr.io/example/agent@sha256:" + "b" * 64},
            "note": "not a release field",
        }.items():
            with self.subTest(name):
                self.renewal(store, **{name: changed})
                with self.assertRaisesRegex(ValueError, "only the dates"):
                    self.renew(store)
        self.renewal(store, issued_at=100, expires_at=200)
        with self.assertRaisesRegex(ValueError, "expired"):
            self.renew(store)
        self.renewal(store)
        def bad_cdn(*_): raise ValueError("CDN serves the earlier list")
        empty = MemoryStore()
        with self.assertRaisesRegex(ValueError, "No published release to renew"):
            self.renew(empty)
        self.assertEqual((store.objects[STABLE_KEY], store.writes, empty.writes), (stable, writes, []))
        with self.assertRaisesRegex(ValueError, "earlier list"):
            self.renew(store, bad_cdn)

    def test_renewal_loses_to_a_release_published_between_its_read_and_its_write(self):
        self.signed(10, validity_days=30)
        store = MemoryStore()
        self.publish(store)
        newer = self.signed(11)
        self.renewal(store)
        store.race = newer
        with self.assertRaisesRegex(ValueError, "concurrently"):
            self.renew(store)
        self.assertEqual(store.objects[STABLE_KEY], newer)
        store.race = None
        with self.assertRaisesRegex(ValueError, "only the dates"):
            self.renew(store)
        self.assertEqual(store.objects[STABLE_KEY], newer)

    def test_release_and_runtime_lists_never_stand_in_for_each_other(self):
        value = runtime_list(LINUX, 7)
        compact = self.jws(value, RUNTIME)
        self.assertEqual(release.verified_release(compact, [self.key], jws_type=RUNTIME), value)
        with self.assertRaisesRegex(ValueError, "^Unexpected release signature type$"):
            release.verified_release(compact, [self.key])
        with self.assertRaisesRegex(ValueError, "^Unexpected runtime manifest signature type$"):
            release.verified_release(self.signed(7), [self.key], jws_type=RUNTIME)
        flipped = compact[:-2] + (b"AA" if compact[-2:] != b"AA" else b"BA")
        untrusted = [base64.urlsafe_b64encode(bytes(32)).decode().rstrip("=")]
        expired = self.jws(runtime_list(LINUX, 7, issued_at=100), RUNTIME)
        for signed, keys, reason in [(flipped, [self.key], "Runtime manifest signature verification failed"),
                                     (compact, untrusted, "Runtime manifest signature is not trusted by STANDALONE_RELEASE_PUBLIC_KEYS"),
                                     (expired, [self.key], "Runtime manifest is expired or not yet valid")]:
            with self.subTest(reason), self.assertRaisesRegex(ValueError, f"^{reason}$"):
                release.verified_release(signed, keys, jws_type=RUNTIME)

    def test_renewal_copies_each_runtime_list_with_the_release_dates_under_its_own_number(self):
        stable = self.signed(7, validity_days=30)
        published = {LINUX: self.jws(runtime_list(LINUX, 7, validity_days=30), RUNTIME),
                     MAC: self.jws(runtime_list(MAC, 6, validity_days=30), RUNTIME)}
        output, runtimes = self.directory / "renewal.json", self.directory / "renewed-runtimes"
        warnings = io.StringIO()
        with self.cdn({STABLE_URL: stable, **{runtime_url(target): compact for target, compact in published.items()}}), \
                contextlib.redirect_stdout(warnings):
            value = release.renew_manifest(BASE, self.keys, output, 365, sequence=7, runtime_output=runtimes)
        self.assertEqual(self.requests, [("GET", url) for url in [STABLE_URL, *map(runtime_url, release.TARGETS)]])
        self.assertEqual(warnings.getvalue(), f"::warning::The {MAC} runtime manifest is number 6, older than release 7; "
                                              "it is renewed under its own number\n")
        self.assertEqual(sorted(path.name for path in runtimes.iterdir()), [f"{MAC}.json", f"{LINUX}.json"])
        for target, compact in published.items():
            before = release.verified_release(compact, [self.key], False, RUNTIME)
            after = json.loads((runtimes / f"{target}.json").read_text())
            self.assertEqual(list(after), list(before))
            self.assertEqual(without_dates(after), without_dates(before))
            self.assertEqual({name: after[name] for name in DATES}, {name: value[name] for name in DATES})
        self.assertEqual(value["expires_at"] - value["issued_at"], 365 * DAY)

    def test_renewal_refuses_a_runtime_list_newer_than_the_release_or_signed_as_another_document(self):
        stable = self.signed(7)
        output, runtimes = self.directory / "renewal.json", self.directory / "renewed-runtimes"
        for listed, reason in [(self.jws(runtime_list(LINUX, 8), RUNTIME), f"The {LINUX} runtime manifest is number 8, newer than release 7"),
                               (self.jws(runtime_list(LINUX, 7)), "Unexpected runtime manifest signature type")]:
            with self.subTest(reason), self.cdn({STABLE_URL: stable, runtime_url(LINUX): listed}), \
                    self.assertRaisesRegex(ValueError, reason):
                release.renew_manifest(BASE, self.keys, output, sequence=7, runtime_output=runtimes)
        self.assertFalse(output.exists() or runtimes.exists())

    def test_renewal_is_due_when_any_list_runs_short(self):
        stable = self.signed(7)
        output, runtimes = self.directory / "renewal.json", self.directory / "renewed-runtimes"
        arguments = ["renew-manifest", "--base-url", BASE, "--public-keys", self.keys, "--output", str(output),
                     "--only-if-days-left", "60", "--runtime-output", str(runtimes)]
        notice = io.StringIO()
        with self.cdn({STABLE_URL: stable, runtime_url(LINUX): self.jws(runtime_list(LINUX, 7), RUNTIME)}), \
                contextlib.redirect_stdout(notice):
            release.main(arguments)
        self.assertRegex(notice.getvalue(), r"^::notice::Release 7 and its runtime pack lists are valid for 364 more days, "
                                            r"more than 60; nothing was renewed\n$")
        self.assertFalse(output.exists() or runtimes.exists())
        with self.cdn({STABLE_URL: stable, runtime_url(LINUX): self.jws(runtime_list(LINUX, 7, validity_days=30), RUNTIME)}):
            release.main(arguments)
        renewed = json.loads(output.read_text())
        self.assertEqual((renewed["sequence"], renewed["expires_at"] - renewed["issued_at"]), (7, 365 * DAY))
        self.assertEqual(json.loads((runtimes / f"{LINUX}.json").read_text())["expires_at"], renewed["expires_at"])

    def test_renewal_replaces_each_stable_runtime_list_before_the_release_and_reads_each_back(self):
        store = self.published_with_runtime_lists(10, LINUX, MAC)
        writes = list(store.writes)
        renewed = {target: self.runtime_renewal(store, target) for target in (LINUX, MAC)}
        signed = self.renewal(store)
        read = []
        self.assertEqual(self.renew(store, lambda *call: read.append(call)), {"sequence": 10, "manifest_url": STABLE_URL})
        order = [target for target in release.TARGETS if target in renewed]
        self.assertEqual(store.writes, writes + [(runtime_key(target), False) for target in order] + [(STABLE_KEY, False)])
        self.assertEqual(read, [(runtime_url(target), len(renewed[target]), hashlib.sha256(renewed[target]).hexdigest())
                                for target in order] + [(STABLE_URL, len(signed), hashlib.sha256(signed).hexdigest())])
        self.assertEqual({target: store.objects[runtime_key(target)] for target in renewed}, renewed)
        self.renew(store)
        self.assertEqual(len(store.writes), len(writes) + 3, "a retry finds every list renewed")
        summary = io.StringIO()
        with contextlib.redirect_stdout(summary):
            release.summary(self.directory, self.keys, renewed=True)
        self.assertIn(f"| Runtime pack lists | {MAC} (number 10), {LINUX} (number 10) |\n", summary.getvalue())

    def test_runtime_list_renewal_writes_nothing_unless_every_published_list_is_only_redated(self):
        store = self.published_with_runtime_lists(10, LINUX)
        stable, writes = dict(store.objects), list(store.writes)
        self.renewal(store)
        published = release.verified_release(store.objects[runtime_key(LINUX)], [self.key], False, RUNTIME)
        pack = {**published["packs"][0], "sha256": "c" * 64}
        for changes, reason in [({"packs": [pack]}, f"may change only the dates of {runtime_key(LINUX)}"),
                                ({"sequence": 11}, f"may change only the dates of {runtime_key(LINUX)}"),
                                ({"issued_at": 100, "expires_at": 200}, "Runtime manifest is expired")]:
            with self.subTest(reason):
                self.runtime_renewal(store, LINUX, **changes)
                with self.assertRaisesRegex(ValueError, reason):
                    self.renew(store)
        (self.directory / "runtimes" / f"{LINUX}.jws").unlink()
        with self.assertRaisesRegex(ValueError, f"{runtime_key(LINUX)} is published, but this renewal carries no re-dated copy"):
            self.renew(store)
        self.runtime_renewal(store, LINUX)
        self.sign_runtime(MAC, runtime_list(MAC, 10))
        with self.assertRaisesRegex(ValueError, f"No published runtime manifest {runtime_key(MAC)} to renew"):
            self.renew(store)
        self.assertEqual((store.objects, store.writes), (stable, writes))

    def test_runtime_list_renewal_loses_to_a_concurrent_publisher_before_the_release_moves(self):
        store = self.published_with_runtime_lists(10, LINUX)
        release_list = store.objects[STABLE_KEY]
        self.runtime_renewal(store, LINUX)
        self.renewal(store)
        newer = self.jws(runtime_list(LINUX, 11), RUNTIME)
        store.race = newer
        with self.assertRaisesRegex(ValueError, f"^Stable runtime manifest {runtime_key(LINUX)} changed concurrently; refusing to overwrite it$"):
            self.renew(store)
        self.assertEqual((store.objects[runtime_key(LINUX)], store.objects[STABLE_KEY]), (newer, release_list))

    def test_only_a_renewal_summary_names_runtime_lists(self):
        self.signed(7)
        def summary(renewed):
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                release.summary(self.directory, self.keys, renewed=renewed)
            return output.getvalue()
        self.assertIn("| Runtime pack lists | none published |\n", summary(True))
        self.assertNotIn("Runtime pack lists", summary(False))

    def test_summary_states_number_version_end_date_and_the_next_step(self):
        self.signed(7, issued_at=1_800_000_000, validity_days=365)
        def summary(*options):
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                release.main(["summary", "--artifacts", str(self.directory), "--public-keys", self.keys, *options])
            return output.getvalue()
        published = summary()
        self.assertIn("### Standalone release 7 published\n", published)
        for line in ["| Release number | 7 |", "| Agent version | 1.2.3 |", "| Valid until | 2028-01-15 08:00:00 UTC (365 days) |",
                     "\nNext: set the hub's minimum release number to 7 (`standalone.release_trust.minimum_sequence` in the hub config)"]:
            self.assertIn(line, published)
        renewed = summary("--renewed")
        self.assertIn("### Standalone release 7 renewed\n", renewed)
        self.assertIn("| Valid until | 2028-01-15 08:00:00 UTC (365 days) |", renewed)
        self.assertIn("\nIf the hub's minimum release number is below 7, raise it to 7 ", renewed)
        self.assertNotIn("Next:", renewed)

    def test_commands_reach_their_functions_with_the_documented_options(self):
        artifacts = ["--artifacts", str(self.directory)]
        output = self.directory / "cli.json"
        bundle = artifacts + ["--base-url", f"{BASE}/releases/3", "--sequence", "3", "--image", IMAGE, "--output", str(output)]
        with patch.object(release.time, "time", return_value=1_800_000_000):
            release.main(["manifest", *bundle])
            default = json.loads(output.read_text())
            release.main(["manifest", *bundle, "--validity-days", "30"])
            bridge = json.loads(output.read_text())
        self.assertEqual((default["issued_at"], default["expires_at"] - default["issued_at"]), (1_800_000_000 - 300, 365 * DAY))
        self.assertEqual(bridge["expires_at"] - bridge["issued_at"], 30 * DAY)
        for days in ["0", "1826"]:
            with self.assertRaisesRegex(ValueError, "between 1 and 1825 days"):
                release.main(["manifest", *bundle, "--validity-days", days])
        stable = self.sign(bridge)
        published = ["--base-url", BASE, "--public-keys", self.keys]
        with self.cdn({STABLE_URL: stable}):
            with self.assertRaisesRegex(ValueError, "bump the agent version"):
                release.main(["preflight", *published, "--sequence", "4", "--release-version", "1.2.3"])
            release.main(["preflight", *published, "--sequence", "4", "--release-version", "1.2.4"])
            with self.assertRaisesRegex(ValueError, "not 2"):
                release.main(["renew-manifest", *published, "--output", str(output), "--sequence", "2"])
            with contextlib.redirect_stdout(io.StringIO()):
                release.main(["renew-manifest", *published, "--output", str(output), "--sequence", "3", "--only-if-days-left", "0"])
            self.assertEqual(json.loads(output.read_text()), bridge)
            release.main(["renew-manifest", *published, "--output", str(output), "--validity-days", "1825", "--sequence", "3"])
        renewal = json.loads(output.read_text())
        self.assertEqual(renewal["expires_at"] - renewal["issued_at"], 1825 * DAY)
        store = MemoryStore()
        store.objects[STABLE_KEY] = stable
        renewed = self.sign(renewal)
        printed = io.StringIO()
        with patch.object(release, "S3Store", return_value=store) as bucket, patch.object(release, "read_public") as readback, contextlib.redirect_stdout(printed):
            release.main(["renew-publish", *artifacts, *published, "--bucket", "releases", "--prefix", "standalone", "--endpoint", "https://s3.example"])
        bucket.assert_called_once_with("releases", "https://s3.example")
        self.assertEqual(readback.call_args.args[1:], (len(renewed), hashlib.sha256(renewed).hexdigest()))
        self.assertEqual(json.loads(printed.getvalue()), {"sequence": 3, "manifest_url": STABLE_URL})
        self.assertEqual(store.objects[STABLE_KEY], renewed)

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
        operations = []
        def docker(command, **options):
            self.assertEqual(command[-1], container["image"])
            config = Path(command[2])
            self.assertEqual(json.loads((config / "config.json").read_text()), {"auths": {}})
            self.assertEqual(options["env"], {"PATH": str(config), "DOCKER_CONFIG": str(config)})
            self.assertEqual(command[3:6], ["--host", "unix:///var/run/docker.sock", "image"])
            if command[6] == "rm":
                operations.append("rm")
                return subprocess.CompletedProcess(command, 1, stderr=b"No such image")
            operations.append(command[-2])
            return subprocess.CompletedProcess(command, 0, stderr=b"")
        with patch.object(release.shutil, "which", return_value="/usr/bin/docker"), patch.object(release.subprocess, "run", side_effect=docker):
            release.anonymous_container_readback(container)
        self.assertEqual(operations, ["rm", "linux/amd64", "rm", "linux/arm64"])

    def test_anonymous_pull_failures_report_the_docker_error(self):
        container = {"image": "ghcr.io/example/agent@sha256:" + "a" * 64, "platforms": ["linux/amd64"]}
        def failing(stderr):
            return patch.object(release.subprocess, "run", return_value=subprocess.CompletedProcess([], 1, stderr=stderr))
        with patch.object(release.shutil, "which", return_value="/usr/bin/docker"):
            with failing(b"Error response from daemon: denied\n"), self.assertRaisesRegex(ValueError, "linux/amd64: Error response from daemon: denied; make the digest-pinned GHCR package public"):
                release.anonymous_container_readback(container)
            with failing(b"Error response from daemon: cannot overwrite digest sha256:abc\n"), self.assertRaisesRegex(ValueError, r"cannot overwrite digest sha256:abc$"):
                release.anonymous_container_readback(container)
            with failing(None), self.assertRaisesRegex(ValueError, "anonymously pullable for linux/amd64: Docker reported no error message"):
                release.anonymous_container_readback(container)

    def test_large_binaries_have_a_native_bootstrap_package_for_every_target(self):
        for target in release.TARGETS:
            for size in [256 * 1024 * 1024, 657979008, release.MAX_RELEASE_BINARY_BYTES]:
                release.usable_package_modes([{"target": target, "size": size}], None)
        for size in [0, True, release.MAX_RELEASE_BINARY_BYTES + 1]:
            with self.assertRaisesRegex(ValueError, "2 GiB"):
                release.usable_package_modes([{"target": "aarch64-apple-darwin", "size": size}], None)

    def test_describe_rejects_oversized_binary_before_execution_or_copy(self):
        with tempfile.TemporaryDirectory() as folder:
            binary = Path(folder) / "binary"
            with binary.open("wb") as stream:
                stream.truncate(release.MAX_RELEASE_BINARY_BYTES + 1)
            with patch.object(release, "binary_info") as info, patch.object(release.shutil, "copyfile") as copy:
                with self.assertRaisesRegex(ValueError, "2 GiB"):
                    release.describe(binary, "aarch64-apple-darwin", Path(folder) / "output")
                info.assert_not_called()
                copy.assert_not_called()

    def test_signed_immutable_artifacts_read_back_before_manifest_and_idempotent_retry(self):
        signed = self.signed(10)
        store = MemoryStore()
        verified = []
        def readback(url, size, digest):
            if url.endswith("/standalone/release.jws"):
                self.assertEqual(len(verified), 4)
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

    def test_public_readback_identifies_itself_and_waits_out_cached_not_found(self):
        class Response:
            status = 200
            headers = {"Access-Control-Allow-Origin": "*"}
            body = b"artifact"
            def __enter__(self): return self
            def __exit__(self, *_): pass
            def read(self, _):
                value, self.body = self.body, b""
                return value
        requests = []
        def respond(*outcomes):
            pending = list(outcomes)
            def open_response(request, **_):
                requests.append(request)
                outcome = pending.pop(0)
                if isinstance(outcome, Exception):
                    raise outcome
                return outcome
            return open_response
        def status(code):
            return HTTPError("https://cdn.example/file", code, "", {}, None)
        digest = hashlib.sha256(b"artifact").hexdigest()
        with patch.object(release, "build_opener") as opener, patch.object(release.time, "sleep") as sleep:
            opener.return_value.open.side_effect = respond(status(404), status(404), Response())
            release.public_readback("https://cdn.example/file", 8, digest)
            self.assertEqual(sleep.call_count, 2)
            self.assertEqual({request.get_header("User-agent") for request in requests}, {"flow-like-standalone-release-verifier/1"})
            opener.return_value.open.side_effect = respond(*[status(404)] * release.CDN_NOT_FOUND_ATTEMPTS)
            with self.assertRaisesRegex(ValueError, "HTTP 404 for https://cdn.example/file"):
                release.public_readback("https://cdn.example/file", 8, digest)
            sleep.reset_mock()
            opener.return_value.open.side_effect = respond(status(403))
            with self.assertRaisesRegex(ValueError, "HTTP 403 for https://cdn.example/file"):
                release.public_readback("https://cdn.example/file", 8, digest)
            sleep.assert_not_called()


if __name__ == "__main__":
    unittest.main()
