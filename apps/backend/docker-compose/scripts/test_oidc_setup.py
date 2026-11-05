"""Exercise fresh OIDC setup, certificate trust and the rendered deployment."""
import copy
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import unittest

import oidc_preflight
import oidc_setup
import preflight

ROOT = Path(__file__).resolve().parents[1]


class OidcSetupTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory(prefix="flow-like-oidc-setup-")
        cls.addClassCleanup(cls.tmp.cleanup)
        cls.directory = Path(cls.tmp.name)
        cls.env_file = cls.directory / ".env.local"
        cls.process = cls.setup(cls.env_file, "--oidc", "bundled", "--mode", "trusted",
                                "--oidc-bootstrap-email", "owner@example.test",
                                "--oidc-redirect-uri", "https://desktop.example.test/callback")
        if cls.process.returncode:
            raise AssertionError(cls.process.stderr)
        cls.values = preflight.read_env(cls.env_file)
        cls.generated = Path(str(cls.env_file) + ".oidc")

    @staticmethod
    def setup(path, *arguments, overrides=None):
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(("FLOW_LIKE_CONFIG", "FLOW_LIKE_RUNTIME_CONFIG", "AUDIT_KMS_", "AUDIT_VAULT_", "STRIPE_"))}
        env.update(overrides or {})
        return subprocess.run(["python3", "-B", str(ROOT / "scripts/setup-env.py"), "--output", str(path), *arguments],
                              env=env, capture_output=True, text=True, timeout=30)

    def render(self):
        if not shutil.which("docker"):
            self.skipTest("Docker Compose is required for rendering")
        env = os.environ.copy()
        for key in self.values:
            env.pop(key, None)
        result = subprocess.run(["docker", "compose", "--env-file", str(self.env_file), "config", "--format", "json"],
                                cwd=ROOT, env=env, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, "Generated Compose configuration did not render")
        return json.loads(result.stdout)

    def test_fresh_setup_selects_complete_bundled_configuration(self):
        config = self.render()
        self.assertEqual(preflight.validate(self.values, config), [])
        self.assertIn("oidc", config["services"])
        self.assertIn("runtime", config["services"])
        self.assertEqual(self.values["COMPOSE_FILE"], "docker-compose.yml:docker-compose.oidc.yml")
        self.assertEqual(self.values["OIDC_MODE"], "bundled")
        self.assertEqual(config["services"]["oidc"]["environment"]["OIDC_BOOTSTRAP_EMAIL"], "owner@example.test")

    def test_secrets_are_private_distinct_and_never_printed(self):
        self.assertEqual(stat.S_IMODE(self.env_file.stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(self.generated.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE((self.generated / "ca.key").stat().st_mode), 0o600)
        keys = ("OIDC_ADMIN_PASSWORD", "OIDC_DB_PASSWORD", "OIDC_BOOTSTRAP_PASSWORD", "POSTGRES_PASSWORD")
        self.assertEqual(len({self.values[key] for key in keys}), len(keys))
        for key in keys:
            self.assertGreaterEqual(len(self.values[key]), 32)
            self.assertNotIn(self.values[key], self.process.stdout + self.process.stderr)
        realm = (self.generated / "flow-like-realm.json").read_text()
        self.assertIn("${OIDC_BOOTSTRAP_PASSWORD}", realm)
        self.assertNotIn(self.values["OIDC_BOOTSTRAP_PASSWORD"], realm)

    def test_realm_requires_pkce_and_has_the_api_access_token_audience(self):
        realm = json.loads((self.generated / "flow-like-realm.json").read_text())
        self.assertFalse(realm["registrationAllowed"])
        client = realm["clients"][0]
        self.assertEqual(client["clientId"], "flow-like")
        self.assertTrue(client["publicClient"])
        self.assertFalse(client["directAccessGrantsEnabled"])
        self.assertFalse(client["implicitFlowEnabled"])
        self.assertEqual(client["attributes"]["pkce.code.challenge.method"], "S256")
        self.assertEqual(set(client["redirectUris"]), {"http://localhost:3001/callback", "flow-like://auth", "https://desktop.example.test/callback"})
        self.assertIn("http://tauri.localhost", client["webOrigins"])
        self.assertNotIn("*", client["webOrigins"])
        mapper = client["protocolMappers"][0]
        self.assertEqual(mapper["protocolMapper"], "oidc-audience-mapper")
        self.assertEqual(mapper["config"]["included.client.audience"], "flow-like")
        self.assertEqual(mapper["config"]["access.token.claim"], "true")
        self.assertTrue(realm["users"][0]["credentials"][0]["temporary"])

    def test_hub_has_userinfo_and_exact_issuer(self):
        hub = json.loads((self.generated / "flow-like.config.json").read_text())
        auth = hub["authentication"]["openid"]
        self.assertEqual(auth["issuer"], "https://auth.localhost:8443/realms/flow-like")
        self.assertEqual(auth["authority"], auth["issuer"])
        self.assertEqual(auth["audience"], "flow-like")
        self.assertEqual(auth["user_info_url"], auth["issuer"] + "/protocol/openid-connect/userinfo")
        self.assertEqual(auth["redirect_uri"], "flow-like://auth")

    def test_certificate_is_trusted_only_for_the_configured_hostname(self):
        cert, key, ca = (self.generated / name for name in ("tls.crt", "tls.key", "ca.crt"))
        oidc_setup.verify_certificates(cert, key, ca, "auth.localhost")
        with self.assertRaises(ValueError):
            oidc_setup.verify_certificates(cert, key, ca, "another.localhost")

    def test_existing_secrets_and_identity_files_are_never_replaced(self):
        before = self.env_file.read_bytes()
        result = self.setup(self.env_file, "--oidc", "bundled")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(before, self.env_file.read_bytes())
        path = self.directory / "collision.env"
        generated = Path(str(path) + ".oidc")
        generated.mkdir()
        sentinel = generated / "keep"
        sentinel.write_text("existing identity state")
        result = self.setup(path, "--oidc", "bundled")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(path.exists())
        self.assertEqual(sentinel.read_text(), "existing identity state")

    def test_external_mode_has_no_identity_files_or_services(self):
        path = self.directory / "external.env"
        result = self.setup(path, "--oidc", "external", "--mode", "trusted")
        self.assertEqual(result.returncode, 0, result.stderr)
        values = preflight.read_env(path)
        self.assertEqual(values["OIDC_MODE"], "external")
        self.assertEqual(values["COMPOSE_FILE"], "docker-compose.yml")
        self.assertFalse(Path(str(path) + ".oidc").exists())

    def test_invalid_origin_redirects_and_incomplete_tls_leave_no_artifacts(self):
        invalid = [
            ("--oidc-url", "http://auth.localhost:8443"),
            ("--oidc-url", "https://localhost:8443"),
            ("--oidc-url", "https://127.0.0.1:8443"),
            ("--oidc-url", "https://auth.localhost:0"),
            ("--oidc-url", "https://auth.localhost:9000"),
            ("--oidc-url", "https://identity.example.com"),
            ("--oidc-redirect-uri", "https://app.example.test/*"),
            ("--oidc-redirect-uri", "http://untrusted.example.test/callback"),
            ("--oidc-tls-cert", str(self.generated / "tls.crt")),
        ]
        for index, arguments in enumerate(invalid):
            with self.subTest(arguments=arguments):
                path = self.directory / f"invalid-{index}.env"
                result = self.setup(path, "--oidc", "bundled", *arguments)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(path.exists())
                self.assertFalse(Path(str(path) + ".oidc").exists())

    def test_runtime_config_conflict_does_not_create_files(self):
        path = self.directory / "conflict.env"
        result = self.setup(path, "--oidc", "bundled", overrides={"FLOW_LIKE_CONFIG_JSON": "{}"})
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(path.exists())
        self.assertFalse(Path(str(path) + ".oidc").exists())

    def test_supplied_certificate_key_mismatch_is_rejected_and_cleaned(self):
        path = self.directory / "wrong-key.env"
        result = self.setup(path, "--oidc", "bundled", "--oidc-tls-cert", str(self.generated / "tls.crt"),
                            "--oidc-tls-key", str(self.generated / "ca.key"), "--oidc-ca-cert", str(self.generated / "ca.crt"))
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(path.exists())
        self.assertFalse(Path(str(path) + ".oidc").exists())

    def test_public_issuer_accepts_supplied_trusted_certificate_and_key(self):
        certificates = self.directory / "operator-certs"
        certificates.mkdir(mode=0o700)
        oidc_setup.certificates(certificates, "login.example.test", ())
        path = self.directory / "public.env"
        result = self.setup(path, "--oidc", "bundled", "--mode", "trusted",
                            "--oidc-url", "https://login.example.test", "--web-origin", "https://app.example.test",
                            "--api-url", "https://api.example.test", "--oidc-tls-cert", str(certificates / "tls.crt"),
                            "--oidc-tls-key", str(certificates / "tls.key"), "--oidc-ca-cert", str(certificates / "ca.crt"))
        self.assertEqual(result.returncode, 0, result.stderr)
        values = preflight.read_env(path)
        self.assertEqual(values["OIDC_PORT"], "443")
        self.assertEqual(values["OIDC_HOSTNAME"], "login.example.test")
        generated = Path(str(path) + ".oidc")
        self.assertFalse((generated / "ca.key").exists())
        self.assertEqual((generated / "tls.crt").read_bytes(), (certificates / "tls.crt").read_bytes())
        hub = json.loads((generated / "flow-like.config.json").read_text())
        self.assertEqual(hub["domain"], "api.example.test")
        self.assertTrue(hub["secure"])
        self.assertEqual(hub["authentication"]["openid"]["issuer"], "https://login.example.test/realms/flow-like")

    def test_preflight_rejects_mixed_modes_and_mismatched_issuer_routes(self):
        config = self.render()
        for change in ({"OIDC_MODE": "external"}, {"OIDC_HOSTNAME": "wrong.localhost"}, {"OIDC_PORT": "8444"}):
            with self.subTest(change=change):
                self.assertTrue(oidc_preflight.validate({**self.values, **change}, config))
        missing = copy.deepcopy(config)
        del missing["services"]["oidc"]
        self.assertTrue(oidc_preflight.validate(self.values, missing))
        exposed = copy.deepcopy(config)
        exposed["services"]["oidc"]["ports"][0]["host_ip"] = "0.0.0.0"
        self.assertTrue(oidc_preflight.validate(self.values, exposed))

    def test_preflight_reports_non_object_identity_json_without_a_traceback(self):
        config = self.render()
        path = self.directory / "invalid-identity.json"
        path.write_text("[]")
        self.assertTrue(oidc_preflight.validate({**self.values, "OIDC_REALM_FILE": str(path)}, config))
        hub = json.loads((self.generated / "flow-like.config.json").read_text())
        hub["authentication"]["openid"] = []
        path.write_text(json.dumps(hub))
        config["configs"]["flowlike_runtime_config"]["file"] = str(path)
        self.assertTrue(oidc_preflight.validate(self.values, config))


if __name__ == "__main__":
    unittest.main()
