"""Verify sequential helper publication retains each image's safety gates."""

import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import container_build_batch as batch
sys.path.pop(0)


IDENTITY = {"owner": "rheosoph", "sha": "a" * 40, "run_id": "123", "attempt": "1",
            "repository": "Rheosoph/flow-like", "actor": "ci"}
DIGEST = "sha256:" + "b" * 64
CONFIG = "sha256:" + "c" * 64


class FakeCommands:
    def __init__(self, fail_scan=None, bad_config=False, fail_logout=False, remove_error=None):
        self.calls = []
        self.fail_scan = fail_scan
        self.bad_config = bad_config
        self.fail_logout = fail_logout
        self.remove_error = remove_error

    def __call__(self, command, **kwargs):
        self.calls.append((command, kwargs))
        code, output = 0, ""
        if command[0] == "trivy" and command[1] == self.fail_scan:
            code = 1
        if command[:3] == ["docker", "buildx", "imagetools"]:
            output = json.dumps({"config": {"digest": "wrong" if self.bad_config else CONFIG}}
                                if "--raw" in command else {"digest": DIGEST})
        if command[:3] == ["docker", "image", "inspect"]:
            output = CONFIG
        if command[:2] == ["docker", "logout"] and self.fail_logout:
            raise batch.BuildError("logout failed")
        if command[:3] == ["docker", "image", "rm"] and self.remove_error:
            raise self.remove_error
        return subprocess.CompletedProcess(command, code, output, "")


class HelperBatchTests(unittest.TestCase):
    def publish(self, fake, targets=("gcp-migration", "azure-migration")):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.object(batch, "run", fake), contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                failures = batch.publish_batch(list(targets), IDENTITY, "token", {}, root / "records", root)
            records = {path.stem: json.loads(path.read_text()) for path in (root / "records").glob("*.json")}
            return failures, records

    def test_each_image_is_scanned_before_login_and_records_its_own_digest(self):
        fake = FakeCommands()
        failures, records = self.publish(fake)
        self.assertEqual(failures, [])
        self.assertEqual(set(records), {"gcp-migration", "azure-migration"})
        self.assertTrue(all(record["digest"] == DIGEST for record in records.values()))
        commands = [command for command, _ in fake.calls]
        logins = [index for index, command in enumerate(commands) if command[:2] == ["docker", "login"]]
        logouts = [index for index, command in enumerate(commands) if command[:2] == ["docker", "logout"]]
        builds = [index for index, command in enumerate(commands) if command[:3] == ["docker", "buildx", "build"]]
        self.assertEqual(len(logins), 2)
        self.assertLess(logouts[0], builds[1])
        for build, login, logout in zip(builds, logins, logouts):
            scans = [command[1] for command in commands[build:login] if command[0] == "trivy"]
            self.assertEqual(scans, ["image", "fs"])
            self.assertIn(["docker", "push", commands[build][commands[build].index("--tag") + 1]], commands[login:logout])
        for command, kwargs in fake.calls:
            self.assertNotIn("GITHUB_TOKEN", kwargs["env"])
            self.assertNotIn("token", command)
            if command[:2] == ["docker", "login"]:
                self.assertEqual(kwargs["input"], "token\n")

    def test_failed_scans_never_authenticate_or_record_and_remaining_images_run(self):
        for scan in ("image", "fs"):
            with self.subTest(scan=scan):
                fake = FakeCommands(fail_scan=scan)
                failures, records = self.publish(fake)
                self.assertEqual(failures, ["gcp-migration", "azure-migration"])
                self.assertEqual(records, {})
                commands = [command for command, _ in fake.calls]
                self.assertEqual(sum(command[:3] == ["docker", "buildx", "build"] for command in commands), 2)
                self.assertFalse(any(command[:2] == ["docker", "login"] for command in commands))
                self.assertFalse(any(command[:2] == ["docker", "push"] for command in commands))
                self.assertEqual(sum("report-secrets" in command for command in commands), 2)

    def test_mismatched_pushed_config_is_not_recorded(self):
        fake = FakeCommands(bad_config=True)
        failures, records = self.publish(fake)
        self.assertEqual(len(failures), 2)
        self.assertEqual(records, {})
        self.assertEqual(sum(command[:2] == ["docker", "logout"] for command, _ in fake.calls), 2)

    def test_logout_failure_stops_before_another_build(self):
        fake = FakeCommands(fail_logout=True)
        with self.assertRaises(batch.CredentialCleanupError):
            self.publish(fake)
        self.assertEqual(sum(command[:3] == ["docker", "buildx", "build"] for command, _ in fake.calls), 1)

    def test_image_cleanup_cannot_mask_logout_failure(self):
        for error in (subprocess.TimeoutExpired(["docker", "image", "rm"], 120), OSError("cleanup failed")):
            with self.subTest(error=type(error).__name__):
                fake = FakeCommands(fail_logout=True, remove_error=error)
                with self.assertRaises(batch.CredentialCleanupError):
                    self.publish(fake)
                self.assertEqual(sum(command[:3] == ["docker", "buildx", "build"] for command, _ in fake.calls), 1)

    def test_image_cleanup_does_not_fail_a_verified_publication(self):
        fake = FakeCommands(remove_error=subprocess.TimeoutExpired(["docker", "image", "rm"], 120))
        failures, records = self.publish(fake)
        self.assertEqual(failures, [])
        self.assertEqual(set(records), {"gcp-migration", "azure-migration"})

    def test_signaling_smoke_and_layer_cache_are_preserved(self):
        fake = FakeCommands()
        failures, _ = self.publish(fake, ("gcp-signaling",))
        self.assertEqual(failures, [])
        commands = [command for command, _ in fake.calls]
        build = next(command for command in commands if command[:3] == ["docker", "buildx", "build"])
        self.assertIn("type=gha,version=2,scope=container-v1-gcp-signaling", build)
        self.assertIn("--provenance=false", build)
        self.assertIn("--load", build)
        self.assertNotIn("--push", build)
        smoke = next(index for index, command in enumerate(commands) if command[:2] == ["bash", "apps/backend/signaling/tests/image-smoke.sh"])
        scan = next(index for index, command in enumerate(commands) if command[0] == "trivy")
        self.assertLess(smoke, scan)

    def test_batches_reject_mixed_architectures_heavy_or_duplicate_targets(self):
        for targets in (("gcp-migration", "aws-migration"), ("gcp-api",), ("gcp-migration", "gcp-migration"), ()):
            with self.subTest(targets=targets), self.assertRaises(ValueError):
                self.publish(FakeCommands(), targets)

    def test_main_removes_publication_credentials_from_child_environment(self):
        env = {"GITHUB_TOKEN": "publish-token", "GH_TOKEN": "other-token", "OWNER": IDENTITY["owner"],
               "GITHUB_SHA": IDENTITY["sha"], "GITHUB_RUN_ID": IDENTITY["run_id"], "GITHUB_RUN_ATTEMPT": IDENTITY["attempt"],
               "GITHUB_REPOSITORY": IDENTITY["repository"], "GITHUB_ACTOR": IDENTITY["actor"],
               "BATCH_TARGETS": '["gcp-migration"]', "RUNNER_TEMP": "/tmp"}
        with patch.dict(os.environ, env, clear=True), patch.object(batch, "publish_batch", return_value=[]) as publish:
            self.assertEqual(batch.main(["--records-dir", "/tmp/unused-records"]), 0)
        self.assertEqual(publish.call_args.args[2], "publish-token")
        self.assertNotIn("GITHUB_TOKEN", publish.call_args.args[3])
        self.assertNotIn("GH_TOKEN", publish.call_args.args[3])


if __name__ == "__main__":
    unittest.main()
