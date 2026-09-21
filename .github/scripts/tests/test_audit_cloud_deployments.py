"""Keep cloud audit identities, immutable storage and API workloads separate."""

import importlib.util
import base64
import contextlib
import io
import json
from pathlib import Path
from types import SimpleNamespace
import unittest
import tempfile
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[3]


def load(cloud):
    spec = importlib.util.spec_from_file_location(f"audit_deploy_{cloud}", ROOT / f"apps/backend/{cloud}/audit-worker/deploy.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


GCP, AZURE = load("gcp"), load("azure")
IMAGE = "ghcr.io/example/audit-worker@sha256:" + "a" * 64
GCP_ARGS = {
    "project": "audit-project", "region": "europe-west1", "bucket": "isolated-audit-bucket", "image": IMAGE,
    "key_version": "projects/audit-project/locations/europe-west1/keyRings/audit/cryptoKeys/timeline/cryptoKeyVersions/1",
    "api_service_account": "api@app-project.iam.gserviceaccount.com",
    "database_instance": "audit-db", "database_host": "10.0.0.2", "database_name": "flow_like", "database_user": None,
    "entry_key_secret": "audit-entry-key", "config_secret": "audit-config", "network": "private", "subnet": "database",
    "name": "flow-like-audit-worker", "encryption_secret": "audit-sink-key", "database_ca_secret": "cloud-sql-ca", "retention_days": 1461,
    "previous_entry_key_secret": None, "audit_kid": None, "verifying_keys_secret": None,
}
SCOPE = "/subscriptions/00000000-0000-0000-0000-000000000000/resourceGroups/audit"
AZURE_ARGS = {
    "subscription": "00000000-0000-0000-0000-000000000000", "resource_group": "audit", "location": "westeurope",
    "environment_id": f"{SCOPE}/providers/Microsoft.App/managedEnvironments/private",
    "api_identity_id": f"{SCOPE}/providers/Microsoft.ManagedIdentity/userAssignedIdentities/api",
    "storage_account": "isolatedauditstore", "image": IMAGE, "key_id": "https://audit.vault.azure.net/keys/timeline/version1",
    "key_vault_id": f"{SCOPE}/providers/Microsoft.KeyVault/vaults/audit",
    "secrets_vault_id": f"{SCOPE}/providers/Microsoft.KeyVault/vaults/secrets",
    "database_secret_uri": "https://secrets.vault.azure.net/secrets/database/version1",
    "entry_key_secret_uri": "https://secrets.vault.azure.net/secrets/entry-key/version1",
    "config_secret_uri": "https://secrets.vault.azure.net/secrets/config/version1",
    "name": "flow-like-audit-worker", "container": "audit", "retention_days": 1461,
    "encryption_secret_uri": "https://secrets.vault.azure.net/secrets/encryption/version1",
    "database_user": "audit-worker", "kid": "audit-v1", "verifying_keys_file": None,
}


class DeploymentTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        directory = tempfile.TemporaryDirectory(prefix="audit-public-keys-")
        cls.addClassCleanup(directory.cleanup)
        # Public P-256 generator point, encoded as SPKI. No signing secret.
        point = bytes.fromhex("6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296"
                              "4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5")
        prefix = bytes.fromhex("3059301306072a8648ce3d020106082a8648ce3d03010703420004")
        pem = "-----BEGIN PUBLIC KEY-----\n" + base64.b64encode(prefix + point).decode() + "\n-----END PUBLIC KEY-----\n"
        path = Path(directory.name) / "keys.json"
        path.write_text(json.dumps({"audit-v1": pem}))
        AZURE_ARGS["verifying_keys_file"] = str(path)

    def test_default_invocation_only_prints_the_plan(self):
        for module, values in ((GCP, GCP_ARGS), (AZURE, AZURE_ARGS)):
            args = [item for key, value in values.items() if value is not None
                    for item in ("--" + key.replace("_", "-"), str(value))]
            with self.subTest(cloud=module.__name__), patch.object(module, "run") as command, contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertEqual(module.main(args), 0)
                self.assertIsInstance(json.loads(output.getvalue()), list)
                command.assert_not_called()

    def test_api_binaries_cannot_start_an_audit_worker(self):
        for cloud in ("azure", "gcp"):
            source = (ROOT / f"apps/backend/{cloud}/api/src/main.rs").read_text()
            self.assertIn("ensure_api_only()?", source)
            self.assertNotIn("AuditWorkerContext", source)
            self.assertNotIn("audit_worker::spawn", source)
            manifest = (ROOT / f"apps/backend/{cloud}/api/Cargo.toml").read_text()
            self.assertNotIn(f'"audit-kms-{cloud}"', manifest)

    def test_rejects_mutable_images_and_short_retention(self):
        for module, values in ((GCP, GCP_ARGS), (AZURE, AZURE_ARGS)):
            for change in ({"image": "ghcr.io/example/image:latest"}, {"retention_days": 1095}):
                with self.subTest(cloud=module.__name__, change=change), self.assertRaises(ValueError):
                    module.plan(SimpleNamespace(**{**values, **change}))

    def test_rejects_reuse_of_api_identity(self):
        with self.assertRaisesRegex(ValueError, "separate"):
            GCP.plan(SimpleNamespace(**{**GCP_ARGS, "api_service_account": "flow-like-audit-worker@audit-project.iam.gserviceaccount.com"}))
        with self.assertRaisesRegex(ValueError, "separate"):
            AZURE.plan(SimpleNamespace(**{**AZURE_ARGS, "api_identity_id": f"{SCOPE}/providers/Microsoft.ManagedIdentity/userAssignedIdentities/flow-like-audit-worker"}))

    def test_gcp_trigger_has_no_signing_or_secret_permission(self):
        steps = GCP.plan(SimpleNamespace(**GCP_ARGS))
        trigger = "flow-like-audit-worker-trigger@audit-project.iam.gserviceaccount.com"
        bindings = [step["run"] for step in steps if "add-iam-policy-binding" in step.get("run", [])]
        trigger_bindings = [command for command in bindings if f"--member=serviceAccount:{trigger}" in command]
        self.assertEqual(len(trigger_bindings), 1)
        self.assertIn("--role=roles/run.invoker", trigger_bindings[0])
        serialized = json.dumps(steps)
        for forbidden in ("roles/storage.admin", "roles/storage.objectAdmin", "roles/cloudkms.admin"):
            self.assertNotIn(forbidden, serialized)
        api_bindings = [command for command in bindings if f"--member=serviceAccount:{GCP_ARGS['api_service_account']}" in command]
        self.assertEqual({command[3] for command in api_bindings}, {"audit-entry-key", "audit-sink-key"})
        self.assertTrue(all(command[1:3] == ["secrets", "add-iam-policy-binding"] and
                            "--role=roles/secretmanager.secretAccessor" in command for command in api_bindings))
        deployment = next(step["run"] for step in steps if step.get("run", [])[1:4] == ["run", "jobs", "deploy"])
        self.assertIn("--args=--once", deployment)
        self.assertFalse(any("DATABASE_URL=" in value for value in deployment))
        self.assertIn("GCP_POSTGRES_USER=flow-like-audit-worker@audit-project.iam", json.dumps(deployment))
        self.assertIn("GCP_POSTGRES_SERVER_CA=cloud-sql-ca:latest", json.dumps(deployment))
        ca_steps = GCP.plan(SimpleNamespace(**{**GCP_ARGS, "database_ca_secret": "cloud-sql-ca"}))
        self.assertIn("GCP_POSTGRES_SERVER_CA=cloud-sql-ca:latest", json.dumps(ca_steps))

    def test_azure_grants_only_blob_read_write_and_key_read_sign(self):
        steps = AZURE.plan(SimpleNamespace(**AZURE_ARGS))
        definitions = [json.loads(step["run"][step["run"].index("--body") + 1])["properties"]["permissions"][0]
                       for step in steps if any("/roleDefinitions/" in value for value in step.get("run", []))]
        self.assertEqual(len(definitions), 2)
        for definition in definitions:
            self.assertEqual(definition["actions"], [])
            self.assertTrue(all("*" not in permission and "delete" not in permission.lower()
                                for permission in definition["dataActions"]))
        self.assertEqual(definitions[0]["dataActions"], [
            "Microsoft.Storage/storageAccounts/blobServices/containers/blobs/read",
            "Microsoft.Storage/storageAccounts/blobServices/containers/blobs/write",
        ])
        command = steps[-1]["run"]
        job = json.loads(command[command.index("--body") + 1])
        self.assertNotIn("ingress", job["properties"]["configuration"])
        self.assertEqual(job["properties"]["template"]["containers"][0]["args"], ["--once"])
        self.assertTrue(all("keyVaultUrl" in secret and "value" not in secret
                            for secret in job["properties"]["configuration"]["secrets"]))

    def test_lock_validation_rejects_weak_or_unlocked_storage(self):
        gcp = {"retentionPolicy": {"retentionPeriod": 1461 * 86400, "isLocked": True},
               "iamConfiguration": {"uniformBucketLevelAccess": {"enabled": True}, "publicAccessPrevention": "enforced"}}
        azure = {"immutabilityPeriodSinceCreationInDays": 1461, "state": "Locked"}
        self.assertTrue(GCP.validate_lock(gcp, 1461))
        self.assertTrue(AZURE.validate_lock(azure, 1461))
        self.assertFalse(GCP.validate_lock({**gcp, "retentionPolicy": {**gcp["retentionPolicy"], "isLocked": False}}, 1461))
        self.assertFalse(AZURE.validate_lock({**azure, "state": "Unlocked"}, 1461))
        for metadata in ({**gcp, "retentionPolicy": {"retentionPeriod": 1095 * 86400}}, {**gcp, "iamConfiguration": {}}):
            with self.assertRaises(ValueError):
                GCP.validate_lock(metadata, 1461)
        with self.assertRaises(ValueError):
            AZURE.validate_lock({**azure, "allowProtectedAppendWritesAll": True}, 1461)

    def test_a_failed_lock_verification_prevents_job_deployment(self):
        cases = [(GCP, {"retentionPolicy": {"retentionPeriod": 1461 * 86400, "isLocked": False},
                        "iamConfiguration": {"uniformBucketLevelAccess": {"enabled": True}, "publicAccessPrevention": "enforced"}}, "lock_bucket"),
                 (AZURE, {"immutabilityPeriodSinceCreationInDays": 1461, "state": "Unlocked", "etag": "test"}, "lock_container")]
        for module, metadata, key in cases:
            lock = {key: True, "minimum_days": 1461, "inspect": ["inspect"], "run": ["lock", "<policy-etag>"]}
            with self.subTest(cloud=module.__name__), patch.object(module, "run", return_value=SimpleNamespace(returncode=0, stdout=json.dumps(metadata))) as call:
                with self.assertRaisesRegex(ValueError, "did not become locked"):
                    module.apply([lock, {"run": ["deploy-job"]}])
                self.assertFalse(any(item.args[0] == ["deploy-job"] for item in call.call_args_list))

    def test_unknown_or_granted_gcp_api_access_stops_deployment(self):
        step = {"deny_api_permission": "cloudkms.cryptoKeyVersions.useToSign", "inspect": ["inspect-iam"]}
        for result in ({}, {"overallAccessState": "CAN_ACCESS"}, {"overallAccessState": "UNKNOWN_INFO"}):
            with self.subTest(result=result), patch.object(GCP, "run", return_value=SimpleNamespace(stdout=json.dumps(result))) as call:
                with self.assertRaisesRegex(ValueError, "API isolation"):
                    GCP.apply([step, {"run": ["deploy-job"]}])
                self.assertEqual(call.call_count, 1)
        with patch.object(GCP, "run", return_value=SimpleNamespace(stdout='{"overallAccessState":"CANNOT_ACCESS"}')):
            GCP.apply([step])

    def test_azure_api_role_inspection_catches_inherited_wildcards(self):
        self.assertTrue(AZURE.unsafe_api_role({"permissions": [{"actions": ["*"], "notActions": ["Microsoft.Authorization/*/write"]}]}))
        self.assertTrue(AZURE.unsafe_api_role({"permissions": [{"dataActions": ["Microsoft.KeyVault/vaults/*"]}]}))
        self.assertFalse(AZURE.unsafe_api_role({"permissions": [{"actions": ["*/read"]}]}))
        self.assertFalse(AZURE.unsafe_api_role({"permissions": [{"dataActions": ["Microsoft.KeyVault/vaults/keys/sign/action"],
                                                               "notDataActions": ["Microsoft.KeyVault/vaults/keys/sign/action"]}]}))
        self.assertFalse(AZURE.unsafe_api_role({"permissions": [{"dataActions": [
            "Microsoft.KeyVault/vaults/keys/sign/action",
            "Microsoft.Storage/storageAccounts/blobServices/containers/blobs/write",
        ]}]}, worker=True))
        self.assertTrue(AZURE.unsafe_api_role({"permissions": [{"dataActions": [
            "Microsoft.Storage/storageAccounts/blobServices/containers/blobs/delete",
        ]}]}, worker=True))


if __name__ == "__main__":
    unittest.main()
