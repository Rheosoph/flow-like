import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import Mock, patch
from urllib.parse import parse_qs, urlsplit

spec = importlib.util.spec_from_file_location("entrypoint", Path(__file__).with_name("entrypoint.py"))
entrypoint = importlib.util.module_from_spec(spec)
spec.loader.exec_module(entrypoint)


class EntrypointTests(unittest.TestCase):
    def test_private_ip_requires_ca_verification_and_quotes_credentials(self):
        environment = {"GCP_POSTGRES_HOST": "10.2.3.4", "GCP_POSTGRES_DATABASE": "flowlike",
                       "GCP_POSTGRES_USER": "worker@project.iam"}
        url = urlsplit(entrypoint.database_url(environment, "token:/?&", "/tmp/ca.pem"))
        self.assertEqual(url.hostname, "10.2.3.4")
        self.assertEqual(url.password, "token%3A%2F%3F%26")
        self.assertEqual(parse_qs(url.query), {"sslmode": ["verify-ca"], "sslrootcert": ["/tmp/ca.pem"]})

    def test_dns_checks_hostname(self):
        environment = {"GCP_POSTGRES_HOST": "instance.sql.goog", "GCP_POSTGRES_DATABASE": "db",
                       "GCP_POSTGRES_USER": "worker"}
        self.assertIn("sslmode=verify-full", entrypoint.database_url(environment, "token", "/tmp/ca"))
        environment["GCP_POSTGRES_HOST"] = "host/override?sslmode=disable"
        with self.assertRaises(ValueError):
            entrypoint.database_url(environment, "token", "/tmp/ca")

    def test_token_deadline_never_reaches_expiry(self):
        self.assertEqual(entrypoint.token_timeout({"access_token": "secret", "expires_in": 3600}), ("secret", 3480))
        self.assertEqual(entrypoint.token_timeout({"access_token": "secret", "expires_in": 300}), ("secret", 180))
        with self.assertRaises(ValueError):
            entrypoint.token_timeout({"access_token": "secret", "expires_in": 179})

    def test_rejects_credential_and_proxy_overrides(self):
        environment = {"GCP_POSTGRES_SERVER_CA": "-----BEGIN CERTIFICATE-----\nca\n-----END CERTIFICATE-----"}
        entrypoint.validate_environment(environment)
        for name in ("DATABASE_URL", "GCE_METADATA_HOST", "https_proxy", "GOOGLE_APPLICATION_CREDENTIALS_JSON"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                entrypoint.validate_environment({**environment, name: "override"})

    def test_launch_uses_sql_login_scope_ephemeral_ca_and_child_only_token(self):
        environment = {"GCP_POSTGRES_HOST": "10.2.3.4", "GCP_POSTGRES_DATABASE": "flowlike",
                       "GCP_POSTGRES_USER": "worker@project.iam",
                       "GCP_POSTGRES_SERVER_CA": "-----BEGIN CERTIFICATE-----\nca\n-----END CERTIFICATE-----"}
        child = Mock()
        child.wait.return_value = 0
        captured = {}

        def spawn(argv, env):
            captured["argv"], captured["env"] = argv, env
            ca = Path(parse_qs(urlsplit(env["DATABASE_URL"]).query)["sslrootcert"][0])
            self.assertEqual(ca.read_text(), environment["GCP_POSTGRES_SERVER_CA"])
            self.assertEqual(ca.stat().st_mode & 0o777, 0o600)
            captured["ca"] = ca
            return child

        responses = ["worker@project.iam.gserviceaccount.com", json.dumps({"access_token": "short-lived-token", "expires_in": 3600})]
        with patch.dict(entrypoint.os.environ, environment, clear=True), patch.object(entrypoint.sys, "argv", ["entrypoint.py", "--once"]), \
             patch.object(entrypoint, "metadata", side_effect=responses) as metadata, \
             patch.object(entrypoint.subprocess, "Popen", side_effect=spawn), patch.object(entrypoint.signal, "signal"):
            self.assertEqual(entrypoint.run(), 0)
            self.assertNotIn("DATABASE_URL", entrypoint.os.environ)
        self.assertEqual(captured["argv"], ["/app/flow-like-audit-worker", "--once"])
        self.assertIn("short-lived-token", captured["env"]["DATABASE_URL"])
        self.assertFalse(captured["ca"].exists())
        self.assertEqual(metadata.call_args_list[0].args, ("email",))
        self.assertEqual(parse_qs(metadata.call_args_list[1].args[0].split("?", 1)[1]), {"scopes": [entrypoint.SQL_SCOPE]})
        child.wait.assert_called_once_with(timeout=3480)

    def test_wrong_identity_cannot_request_database_token_or_spawn(self):
        environment = {"GCP_POSTGRES_USER": "api@project.iam",
                       "GCP_POSTGRES_SERVER_CA": "-----BEGIN CERTIFICATE-----\nca\n-----END CERTIFICATE-----"}
        with patch.dict(entrypoint.os.environ, environment, clear=True), patch.object(entrypoint.sys, "argv", ["entrypoint.py", "--once"]), \
             patch.object(entrypoint, "metadata", return_value="worker@project.iam.gserviceaccount.com") as metadata, \
             patch.object(entrypoint.subprocess, "Popen") as spawn:
            with self.assertRaisesRegex(ValueError, "does not match"):
                entrypoint.run()
            metadata.assert_called_once_with("email")
            spawn.assert_not_called()

    def test_wrapper_rejects_persistent_mode_before_metadata_access(self):
        with patch.object(entrypoint.sys, "argv", ["entrypoint.py"]), patch.object(entrypoint, "metadata") as metadata:
            with self.assertRaisesRegex(ValueError, "exactly --once"):
                entrypoint.run()
            metadata.assert_not_called()


if __name__ == "__main__":
    unittest.main()
