"""Check the optional identity service's network and credential boundaries."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import threading
import unittest


ROOT = Path(__file__).resolve().parents[1]


def setup_module():
    spec = importlib.util.spec_from_file_location("oidc_test_setup", ROOT / "scripts/setup-env.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@unittest.skipUnless(shutil.which("docker"), "Docker Compose is required for configuration rendering")
class OidcComposeTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.template = setup_module().generate(
            (ROOT / ".env.example").read_text(), "trusted", "http://localhost:3001",
            "http://localhost:8080", "http://s3.localhost:9000",
        )

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="flow-like-oidc-compose-")
        self.addCleanup(directory.cleanup)
        self.directory = Path(directory.name)
        self.values = {
            line.partition("=")[0]: line.partition("=")[2]
            for line in self.template.splitlines() if line and not line.startswith("#")
        }
        self.values.update({
            "OIDC_MODE": "bundled", "OIDC_PUBLIC_URL": "https://auth.localhost:8443",
            "OIDC_HOSTNAME": "auth.localhost", "OIDC_PORT": "8443",
            "OIDC_DB_PASSWORD": "oidc_database_only", "OIDC_ADMIN_PASSWORD": "oidc_admin_only",
            "OIDC_BOOTSTRAP_PASSWORD": "oidc_bootstrap_only",
            "OIDC_ADMIN_USERNAME": "admin", "OIDC_BOOTSTRAP_USERNAME": "owner",
        })
        for setting in ("OIDC_REALM_FILE", "OIDC_TLS_CERT_FILE", "OIDC_TLS_KEY_FILE", "OIDC_CA_CERT_FILE"):
            path = self.directory / setting.lower()
            path.write_text("test fixture\n")
            self.values[setting] = str(path)

    def render(self, bundled=True, overlays=()):
        path = self.directory / ".env"
        path.write_text("\n".join(f"{key}={value}" for key, value in self.values.items()) + "\n")
        path.chmod(0o600)
        env = os.environ.copy()
        for key in (*self.values, "COMPOSE_FILE", "COMPOSE_PROFILES"):
            env.pop(key, None)
        files = ["docker-compose.yml"]
        if bundled:
            files.append("docker-compose.oidc.yml")
        files.extend(overlays)
        command = ["docker", "compose", "--env-file", str(path)]
        for file in files:
            command.extend(["-f", file])
        result = subprocess.run(command + ["config", "--format", "json"], cwd=ROOT,
                                env=env, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_external_identity_has_no_bundled_dependency(self):
        self.values["OIDC_MODE"] = "external"
        services = self.render(bundled=False)["services"]
        self.assertNotIn("oidc", services)
        self.assertNotIn("oidc-db", services)
        self.assertNotIn("oidc", services["api"]["depends_on"])
        self.assertNotIn("OIDC_CA_CERT_FILE", services["api"]["environment"])

    def test_database_and_credentials_are_confined_to_identity_services(self):
        config = self.render()
        services = config["services"]
        self.assertTrue(config["networks"]["oidc-database"]["internal"])
        consumers = {name for name, service in services.items() if "oidc-database" in service.get("networks", {})}
        self.assertEqual(consumers, {"oidc", "oidc-db"})
        self.assertNotIn("ports", services["oidc-db"])
        self.assertIn("oidc_database", config["volumes"])
        for name, service in services.items():
            environment = service.get("environment", {})
            for marker, allowed in [("oidc_database_only", {"oidc", "oidc-db"}),
                                    ("oidc_admin_only", {"oidc"}), ("oidc_bootstrap_only", {"oidc"})]:
                if marker in environment.values():
                    self.assertIn(name, allowed)
            for mount in service.get("volumes", []):
                if mount.get("source") == self.values["OIDC_TLS_KEY_FILE"]:
                    self.assertEqual(name, "oidc")
        self.assertEqual(services["api"]["depends_on"]["oidc"]["condition"], "service_healthy")
        self.assertEqual(services["oidc"]["depends_on"]["oidc-db"]["condition"], "service_healthy")
        self.assertEqual(services["api"]["environment"]["OIDC_CA_CERT_FILE"], "/etc/flow-like/oidc-ca.pem")
        for name in ("oidc", "api"):
            mounts = [mount for mount in services[name]["volumes"] if mount["type"] == "bind"]
            self.assertTrue(mounts)
            for mount in mounts:
                self.assertTrue(mount["read_only"])
                self.assertFalse(mount.get("bind", {}).get("create_host_path", False))

    def test_https_origin_has_same_port_and_hostname_inside_compose(self):
        self.values.update({"OIDC_PUBLIC_URL": "https://identity.example.test", "OIDC_HOSTNAME": "identity.example.test",
                            "OIDC_PORT": "443", "OIDC_BIND_ADDRESS": "0.0.0.0"})
        identity = self.render()["services"]["oidc"]
        self.assertEqual(identity["command"], ["start", "--import-realm"])
        self.assertEqual(identity["environment"]["KC_HOSTNAME"], self.values["OIDC_PUBLIC_URL"])
        self.assertEqual(identity["environment"]["KC_HTTP_ENABLED"], "false")
        self.assertEqual(identity["environment"]["KC_HTTPS_PORT"], "443")
        self.assertIn("identity.example.test", identity["networks"]["app"]["aliases"])
        self.assertEqual([(port["target"], port["published"]) for port in identity["ports"]], [(443, "443")])
        self.assertEqual(identity["environment"]["KC_HTTP_MANAGEMENT_HOST"], "127.0.0.1")
        self.assertEqual(identity["environment"]["KC_HTTP_MANAGEMENT_SCHEME"], "http")
        self.assertEqual(identity["environment"]["KC_HEALTH_ENABLED"], "true")
        self.assertEqual(identity["environment"]["KC_METRICS_ENABLED"], "true")

    def test_identity_survives_external_application_datastore_overlays(self):
        self.values["METRICS_REDIS_URL"] = "redis://metrics@redis.example.test:6379"
        services = self.render(overlays=("docker-compose.external-store.yml", "docker-compose.external-datastores.yml"))["services"]
        self.assertIn("oidc", services)
        self.assertIn("oidc-db", services)
        self.assertNotIn("postgres", services)
        self.assertNotIn("redis", services)
        self.assertEqual(services["oidc"]["environment"]["KC_DB_URL"], "jdbc:postgresql://oidc-db:5432/keycloak")

    def test_healthcheck_rejects_an_unready_identity_service(self):
        probe = self.render()["services"]["oidc"]["healthcheck"]["test"]
        self.assertEqual(probe[:3], ["CMD", "bash", "-ec"])
        for status, expected in [(200, 0), (503, 1)]:
            with self.subTest(status=status), socket.socket() as listener:
                try:
                    listener.bind(("127.0.0.1", 0))
                except PermissionError:
                    self.skipTest("Loopback listeners are unavailable in this sandbox")
                listener.listen(1)
                listener.settimeout(5)

                def respond():
                    connection, _ = listener.accept()
                    with connection:
                        connection.recv(4096)
                        connection.sendall(f"HTTP/1.0 {status} Test\r\n\r\n".encode())

                thread = threading.Thread(target=respond, daemon=True)
                thread.start()
                port = listener.getsockname()[1]
                # Compose config preserves escaped dollar signs for container execution.
                script = probe[3].replace("$$", "$").replace("/127.0.0.1/9000", f"/127.0.0.1/{port}")
                result = subprocess.run(["bash", "-ec", script], capture_output=True, text=True, timeout=5)
                thread.join(timeout=5)
                self.assertEqual(result.returncode, expected, result.stderr)


if __name__ == "__main__":
    unittest.main()
