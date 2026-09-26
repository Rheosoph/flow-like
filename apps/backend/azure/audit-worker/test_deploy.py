"""Exercise the Azure audit planner without contacting Azure."""
import base64
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("azure_audit_deploy", Path(__file__).with_name("deploy.py"))
deploy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(deploy)

# Public P-256 generator point. No private signing material is stored here.
X = bytes.fromhex("6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296")
Y = bytes.fromhex("4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5")
PUBLIC_PEM = "-----BEGIN PUBLIC KEY-----\n" + base64.b64encode(deploy.P256_SPKI_PREFIX + X + Y).decode() + "\n-----END PUBLIC KEY-----\n"
SCOPE = "/subscriptions/00000000-0000-0000-0000-000000000000/resourceGroups/audit"
KEY = {
    "key": {"kty": "EC", "crv": "P-256", "keyOps": ["sign", "verify"],
            "x": base64.urlsafe_b64encode(X).decode().rstrip("="),
            "y": base64.urlsafe_b64encode(Y).decode().rstrip("=")},
    "attributes": {"enabled": True},
}


class DeploymentTest(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="azure-audit-plan-")
        self.addCleanup(self.directory.cleanup)
        self.keys = Path(self.directory.name) / "public-keys.json"
        self.keys.write_text(json.dumps({"audit-v1": PUBLIC_PEM}))
        self.values = {
            "subscription": "00000000-0000-0000-0000-000000000000",
            "resource_group": "audit", "location": "westeurope",
            "environment_id": f"{SCOPE}/providers/Microsoft.App/managedEnvironments/private",
            "api_identity_id": f"{SCOPE}/providers/Microsoft.ManagedIdentity/userAssignedIdentities/api",
            "storage_account": "isolatedauditstore", "image": "auditregistry.azurecr.io/audit-worker@sha256:" + "a" * 64,
            "key_id": "https://audit.vault.azure.net/keys/timeline/version1",
            "key_vault_id": f"{SCOPE}/providers/Microsoft.KeyVault/vaults/audit",
            "secrets_vault_id": f"{SCOPE}/providers/Microsoft.KeyVault/vaults/secrets",
            "database_secret_uri": "https://secrets.vault.azure.net/secrets/database/version1",
            "entry_key_secret_uri": "https://secrets.vault.azure.net/secrets/entry-key/version1",
            "encryption_secret_uri": "https://secrets.vault.azure.net/secrets/encryption/version1",
            "database_user": "audit-worker", "kid": "audit-v1", "verifying_keys_file": str(self.keys),
            "name": "flow-like-audit-worker", "container": "audit", "retention_days": 1461,
            "previous_entry_key_secret_uri": None, "registry_server": None,
        }

    def plan(self, **changes):
        return deploy.plan(SimpleNamespace(**{**self.values, **changes}))

    def job(self, steps):
        command = steps[-1]["run"]
        return json.loads(command[command.index("--body") + 1])["properties"]

    def test_preview_never_calls_cloud(self):
        args = [item for key, value in self.values.items() if value is not None
                for item in ("--" + key.replace("_", "-"), str(value))]
        with patch.object(deploy, "run") as run, contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(deploy.main(args), 0)
        self.assertIsInstance(json.loads(output.getvalue()), list)
        run.assert_not_called()

    def test_job_uses_managed_identity_and_cached_public_key(self):
        job = self.job(self.plan())
        container = job["template"]["containers"][0]
        values = {item["name"]: item.get("value", item.get("secretRef")) for item in container["env"]}
        self.assertEqual(values["AZURE_POSTGRES_USER"], "audit-worker")
        self.assertEqual(values["AUDIT_KID"], "audit-v1")
        self.assertEqual(json.loads(values["AUDIT_VERIFYING_KEYS"]), {"audit-v1": PUBLIC_PEM})
        self.assertEqual(values["SINK_TOKEN_ENCRYPTION_KEY"], "encryption")
        self.assertEqual(values["DATABASE_URL"], "database")
        self.assertEqual(container["args"], ["--once"])
        self.assertEqual(job["configuration"]["replicaRetryLimit"], 0)
        self.assertNotIn("AZURE_STORAGE_ACCOUNT_KEY", values)
        self.assertNotIn("FLOW_LIKE_CONFIG_JSON", values)
        self.assertTrue(all("keyVaultUrl" in secret and "value" not in secret for secret in job["configuration"]["secrets"]))

    def test_rejects_missing_export_key_or_unsafe_database_role(self):
        for changes in ({"encryption_secret_uri": None}, {"database_user": ""}, {"database_user": "postgres"},
                        {"database_user": "api"}, {"database_user": "role; DROP TABLE"}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                self.plan(**changes)

    def test_missing_new_required_flags_fail_before_any_cloud_call(self):
        args = [item for key, value in self.values.items() if value is not None and key != "encryption_secret_uri"
                for item in ("--" + key.replace("_", "-"), str(value))]
        with patch.object(deploy, "run") as run, contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
            deploy.main(args)
        self.assertEqual(error.exception.code, 2)
        run.assert_not_called()

    def test_rotation_and_existing_registry_configuration(self):
        job = self.job(self.plan(previous_entry_key_secret_uri="https://secrets.vault.azure.net/secrets/previous-entry/version1",
                                 registry_server="auditregistry.azurecr.io"))
        values = {item["name"]: item.get("secretRef") for item in job["template"]["containers"][0]["env"]}
        self.assertEqual(values["AUDIT_ENTRY_KEY_PREVIOUS"], "previous-entry-key")
        registry = job["configuration"]["registries"][0]
        self.assertEqual(registry["server"], "auditregistry.azurecr.io")
        self.assertTrue(registry["identity"].endswith("/userAssignedIdentities/flow-like-audit-worker"))
        for server in ("otherregistry.azurecr.io", "https://auditregistry.azurecr.io", "auditregistry.azurecr.io/path"):
            with self.subTest(server=server), self.assertRaises(ValueError):
                self.plan(registry_server=server)

    def test_wrong_vault_hostname_is_rejected(self):
        with self.assertRaises(ValueError):
            self.plan(key_id="https://audit.example.com/keys/timeline/version1")
        with self.assertRaises(ValueError):
            self.plan(entry_key_secret_uri="https://secrets.example.com/secrets/entry-key/version1")

    def test_public_key_map_requires_current_id_and_valid_p256_points(self):
        for value in ({"other": PUBLIC_PEM}, {"audit-v1": "-----BEGIN PRIVATE KEY-----\nx\n-----END PRIVATE KEY-----"},
                      {"audit-v1": PUBLIC_PEM.replace(base64.b64encode(deploy.P256_SPKI_PREFIX + X + Y).decode(),
                                                     base64.b64encode(deploy.P256_SPKI_PREFIX + bytes(64)).decode())}):
            self.keys.write_text(json.dumps(value))
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.plan()
        self.keys.write_text('{"audit-v1":"one", "audit-v1":"two"}')
        with self.assertRaisesRegex(ValueError, "duplicate"):
            self.plan()

    def test_key_verification_runs_before_retention_lock(self):
        steps = self.plan()
        verify = next(i for i, step in enumerate(steps) if step.get("verify_signing_key"))
        lock = next(i for i, step in enumerate(steps) if step.get("lock_container"))
        self.assertLess(verify, lock)
        deploy.validate_signing_key(KEY, PUBLIC_PEM)
        invalid_keys = [
            {**KEY, "attributes": {"enabled": False}},
            {**KEY, "key": {**KEY["key"], "crv": "P-384"}},
            {**KEY, "key": {**KEY["key"], "keyOps": ["verify"]}},
            {**KEY, "key": {**KEY["key"], "y": base64.urlsafe_b64encode(bytes(32)).decode()}},
        ]
        for metadata in invalid_keys:
            with self.subTest(metadata=metadata), self.assertRaises(ValueError):
                deploy.validate_signing_key(metadata, PUBLIC_PEM)

    def test_mismatched_public_key_stops_apply_before_job_creation(self):
        step = {"verify_signing_key": True, "public_pem": PUBLIC_PEM, "inspect": ["inspect-key"]}
        metadata = {**KEY, "key": {**KEY["key"], "x": base64.urlsafe_b64encode(bytes(32)).decode()}}
        with patch.object(deploy, "run", return_value=SimpleNamespace(stdout=json.dumps(metadata))) as run:
            with self.assertRaisesRegex(ValueError, "does not match"):
                deploy.apply([step, {"run": ["create-job"]}])
        run.assert_called_once_with(["inspect-key"])


if __name__ == "__main__":
    unittest.main()
