import contextlib
import copy
import importlib.util
import io
import json
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("gcp_audit_deploy", Path(__file__).with_name("deploy.py"))
deploy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(deploy)

ARGS = [
    "--project", "audit-project", "--region", "europe-west1", "--bucket", "audit-evidence",
    "--image", "europe-west1-docker.pkg.dev/audit-project/backend/audit@sha256:" + "a" * 64,
    "--key-version", "projects/audit-project/locations/europe-west1/keyRings/audit/cryptoKeys/timeline/cryptoKeyVersions/1",
    "--api-service-account", "api@audit-project.iam.gserviceaccount.com",
    "--database-instance", "audit-postgres", "--database-host", "10.2.3.4", "--database-name", "flowlike",
    "--database-ca-secret", "database-ca", "--entry-key-secret", "entry-key",
    "--encryption-secret", "sink-key", "--config-secret", "worker-config",
    "--network", "private", "--subnet", "database",
]
WORKER = "flow-like-audit-worker@audit-project.iam.gserviceaccount.com"
USER = WORKER.removesuffix(".gserviceaccount.com")
INSTANCE = {
    "databaseVersion": "POSTGRES_17",
    "ipAddresses": [{"type": "PRIVATE", "ipAddress": "10.2.3.4"}],
    "settings": {
        "databaseFlags": [{"name": "cloudsql.iam_authentication", "value": "on"}],
        "ipConfiguration": {"sslMode": "ENCRYPTED_ONLY", "serverCaMode": "GOOGLE_MANAGED_INTERNAL_CA"},
    },
}


class DeployTests(unittest.TestCase):
    def test_preview_does_not_execute_cloud_commands(self):
        with patch.object(deploy, "run") as run, contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(deploy.main(ARGS), 0)
            self.assertIsInstance(json.loads(output.getvalue()), list)
            run.assert_not_called()

    def test_password_input_removed_and_ca_encryption_and_instance_required(self):
        for missing in ("--database-ca-secret", "--encryption-secret", "--database-instance"):
            args = ARGS.copy()
            index = args.index(missing)
            del args[index:index + 2]
            with self.subTest(missing=missing), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                deploy.parser().parse_args(args)
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            deploy.parser().parse_args(ARGS + ["--database-secret", "password-url"])

    def test_job_uses_iam_env_and_shared_keys_and_preserves_single_tick(self):
        steps = deploy.plan(deploy.parser().parse_args(ARGS + [
            "--audit-kid", "timeline-1", "--verifying-keys-secret", "public-keys",
            "--previous-entry-key-secret", "previous-entry-key",
        ]))
        commands = [step["run"] for step in steps if "run" in step]
        job = next(command for command in commands if command[1:4] == ["run", "jobs", "deploy"])
        env = dict(item.split("=", 1) for item in next(item for item in job if item.startswith("--set-env-vars=")).split("=", 1)[1].split(","))
        secrets = dict(item.split("=", 1) for item in next(item for item in job if item.startswith("--set-secrets=")).split("=", 1)[1].split(","))
        self.assertEqual(env["GCP_POSTGRES_USER"], USER)
        self.assertEqual(env["GCP_POSTGRES_HOST"], "10.2.3.4")
        self.assertEqual(env["GCP_POSTGRES_DATABASE"], "flowlike")
        self.assertEqual(env["AUDIT_KID"], "timeline-1")
        self.assertEqual(secrets["AUDIT_VERIFYING_KEYS"], "public-keys:latest")
        self.assertEqual(secrets["GCP_POSTGRES_SERVER_CA"], "database-ca:latest")
        self.assertEqual(secrets["AUDIT_ENTRY_KEY_PREVIOUS"], "previous-entry-key:latest")
        self.assertNotIn("DATABASE_URL", secrets)
        self.assertNotIn("DATABASE_URL", env)
        self.assertIn("--args=--once", job)
        self.assertIn("--parallelism=1", job)
        self.assertIn("--vpc-egress=private-ranges-only", job)
        self.assertIn("--memory=2Gi", job)
        api_bindings = [command for command in commands if "--member=serviceAccount:api@audit-project.iam.gserviceaccount.com" in command]
        self.assertEqual({command[3] for command in api_bindings}, {"entry-key", "sink-key", "public-keys", "previous-entry-key"})
        self.assertTrue(all(command[1:3] == ["secrets", "add-iam-policy-binding"] and "--role=roles/secretmanager.secretAccessor" in command for command in api_bindings))
        sql_binding = next(command for command in commands if "--role=roles/cloudsql.instanceUser" in command)
        self.assertIn(f"--member=serviceAccount:{WORKER}", sql_binding)
        self.assertNotIn("roles/cloudsql.admin", json.dumps(steps))
        scheduler_bindings = [command for command in commands if "--member=serviceAccount:flow-like-audit-worker-trigger@audit-project.iam.gserviceaccount.com" in command]
        self.assertEqual(len(scheduler_bindings), 1)
        self.assertIn("--role=roles/run.invoker", scheduler_bindings[0])

    def test_rejects_identity_mismatch_and_unpaired_public_keys(self):
        for extra in (["--database-user", "api@audit-project.iam"], ["--audit-kid", "timeline-1"],
                      ["--verifying-keys-secret", "public-keys"], ["--config-secret", "entry-key"],
                      ["--database-host", "db/?sslmode=disable"]):
            with self.subTest(extra=extra), self.assertRaises(ValueError):
                deploy.plan(deploy.parser().parse_args(ARGS + extra))
        deploy.plan(deploy.parser().parse_args(ARGS + ["--database-user", USER]))

    def test_database_preflight_requires_private_iam_postgres_without_client_cert(self):
        deploy.validate_database(INSTANCE, "10.2.3.4")
        variants = []
        for change in ("version", "flag", "tls", "public", "host"):
            invalid = copy.deepcopy(INSTANCE)
            if change == "version":
                invalid["databaseVersion"] = "MYSQL_8_0"
            elif change == "flag":
                invalid["settings"]["databaseFlags"] = []
            elif change == "tls":
                invalid["settings"]["ipConfiguration"]["sslMode"] = "TRUSTED_CLIENT_CERTIFICATE_REQUIRED"
            elif change == "public":
                invalid["ipAddresses"][0]["type"] = "PRIMARY"
            else:
                invalid["ipAddresses"][0]["ipAddress"] = "10.2.3.5"
            variants.append(invalid)
        for metadata in variants:
            with self.subTest(metadata=metadata), self.assertRaises(ValueError):
                deploy.validate_database(metadata, "10.2.3.4")

    def test_raw_ip_requires_a_per_instance_ca_including_legacy_default(self):
        for mode in (None, "CA_MODE_UNSPECIFIED", "GOOGLE_MANAGED_INTERNAL_CA"):
            metadata = copy.deepcopy(INSTANCE)
            if mode is None:
                del metadata["settings"]["ipConfiguration"]["serverCaMode"]
            else:
                metadata["settings"]["ipConfiguration"]["serverCaMode"] = mode
            deploy.validate_database(metadata, "10.2.3.4")
        for mode in ("GOOGLE_MANAGED_CAS_CA", "CUSTOMER_MANAGED_CAS_CA", "NEW_CA_MODE", None):
            metadata = copy.deepcopy(INSTANCE)
            metadata["settings"]["ipConfiguration"]["serverCaMode"] = mode
            with self.subTest(mode=mode), self.assertRaises(ValueError):
                deploy.validate_database(metadata, "10.2.3.4")

    def test_shared_ca_uses_instance_private_dns_mapping_and_normalizes_dot(self):
        for mode in ("GOOGLE_MANAGED_CAS_CA", "CUSTOMER_MANAGED_CAS_CA"):
            metadata = copy.deepcopy(INSTANCE)
            metadata["settings"]["ipConfiguration"]["serverCaMode"] = mode
            metadata["dnsNames"] = [{"name": "instance.region.sql-psa.goog.",
                                     "dnsScope": "INSTANCE", "connectionType": "PRIVATE_SERVICES_ACCESS"}]
            deploy.validate_database(metadata, "INSTANCE.region.sql-psa.goog.")
            for change in ({"connectionType": "PUBLIC"}, {"connectionType": "PRIVATE_SERVICE_CONNECT"},
                           {"dnsScope": "CLUSTER"}, {"name": "another.region.sql-psa.goog."}):
                invalid = copy.deepcopy(metadata)
                invalid["dnsNames"][0].update(change)
                invalid["dnsName"] = "instance.region.sql-psa.goog."
                with self.subTest(mode=mode, change=change), self.assertRaises(ValueError):
                    deploy.validate_database(invalid, "instance.region.sql-psa.goog")
            del metadata["dnsNames"]
            metadata["dnsName"] = "instance.region.sql-psa.goog."
            deploy.validate_database(metadata, "instance.region.sql-psa.goog")
        steps = deploy.plan(deploy.parser().parse_args(ARGS + ["--database-host", "INSTANCE.region.sql-psa.goog."]))
        self.assertIn('"host": "instance.region.sql-psa.goog"', json.dumps(steps))
        self.assertIn("GCP_POSTGRES_HOST=instance.region.sql-psa.goog,", json.dumps(steps))
        self.assertNotIn("INSTANCE.region.sql-psa.goog.", json.dumps(steps))

    def test_per_instance_ca_rejects_dns_without_private_services_access_san(self):
        metadata = copy.deepcopy(INSTANCE)
        metadata["dnsNames"] = [{"name": "instance.region.sql-psa.goog.",
                                 "dnsScope": "INSTANCE", "connectionType": "PRIVATE_SERVICES_ACCESS"}]
        with self.assertRaisesRegex(ValueError, "per-instance CA"):
            deploy.validate_database(metadata, "instance.region.sql-psa.goog")

    def test_shared_ca_raw_ip_stops_apply_before_job_creation(self):
        metadata = copy.deepcopy(INSTANCE)
        metadata["settings"]["ipConfiguration"]["serverCaMode"] = "GOOGLE_MANAGED_CAS_CA"
        preflight = {"validate_database": {"host": "10.2.3.4"}, "inspect": ["inspect-instance"]}
        with patch.object(deploy, "run", return_value=SimpleNamespace(stdout=json.dumps(metadata))) as run:
            with self.assertRaisesRegex(ValueError, "certificate DNS hostname"):
                deploy.apply([preflight, {"run": ["deploy-job"]}])
            run.assert_called_once_with(["inspect-instance"])

    def test_database_user_mapping_requires_matching_iam_service_account(self):
        for name in (USER, WORKER):
            deploy.validate_database_user([{"name": name, "type": "CLOUD_IAM_SERVICE_ACCOUNT"}], USER)
        for users in ([], [{"name": USER, "type": "BUILT_IN"}],
                      [{"name": "api@audit-project.iam", "type": "CLOUD_IAM_SERVICE_ACCOUNT"}]):
            with self.subTest(users=users), self.assertRaises(ValueError):
                deploy.validate_database_user(users, USER)

    def test_failed_database_preflight_stops_before_job_deployment(self):
        preflight = {"validate_database_user": USER, "inspect": ["inspect-users"]}
        with patch.object(deploy, "run", return_value=SimpleNamespace(stdout="[]")) as run:
            with self.assertRaises(ValueError):
                deploy.apply([preflight, {"run": ["deploy-job"]}])
            run.assert_called_once_with(["inspect-users"])


if __name__ == "__main__":
    unittest.main()
