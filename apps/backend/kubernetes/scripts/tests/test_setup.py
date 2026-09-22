"""Exercise setup and image-value generation without Docker or a cluster."""
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest
import importlib.util
from unittest.mock import patch

BASE = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("setup_config", BASE / "scripts/setup-config.py")
setup = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(setup)

class SetupTest(unittest.TestCase):
    def test_hosted_frontends_reuse_the_web_origin_for_separate_and_shared_hosts(self):
        web = "https://app.example.test"
        for api in ["https://api.example.test", web]:
            with self.subTest(api=api), patch.dict(os.environ, {"PUBLIC_WEB_URL": web, "PUBLIC_API_URL": api}, clear=True):
                _, values = setup.generate("flow-like", "flow-like")
            self.assertEqual(values["api"]["frontendBaseUrl"], web)
            self.assertIn(web, values["api"]["corsAllowedOrigins"])

    def test_stripe_credentials_are_private_api_secret_values(self):
        environment = {key: f"test_{index}" for index, key in enumerate(setup.STRIPE_SETTINGS)}
        with patch.dict(os.environ, environment, clear=True):
            objects, values = setup.generate("flow-like", "flow-like")
        api_name = values["api"]["existingSecret"]
        api_secret = next(item for item in objects["items"] if item["metadata"]["name"] == api_name)
        for key, value in environment.items():
            self.assertEqual(api_secret["stringData"][key], value)
            self.assertNotIn(value, json.dumps(values))
            self.assertFalse(any(key in item["stringData"] for item in objects["items"] if item != api_secret))
        with patch.dict(os.environ, {}, clear=True):
            objects, _ = setup.generate("flow-like", "flow-like")
        self.assertFalse(any(key in item["stringData"] for item in objects["items"] for key in setup.STRIPE_SETTINGS))

    def test_hub_file_and_json_are_private_objects_not_image_build_inputs(self):
        marker = {"name": "runtime-only", "domain": "configured.example.test"}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "hub.json"
            path.write_text(json.dumps(marker))
            for environment in ({"FLOW_LIKE_CONFIG_FILE": str(path)}, {"FLOW_LIKE_CONFIG_JSON": json.dumps(marker)}, {"FLOW_LIKE_RUNTIME_CONFIG_FILE": str(path)}, {"FLOW_LIKE_CONFIG": str(path)}):
                with self.subTest(source=next(iter(environment))), patch.dict(os.environ, environment, clear=True):
                    objects, values = setup.generate("flow-like", "flow-like")
                name = values["api"]["runtimeConfig"]["existingSecret"]
                secret = next(x for x in objects["items"] if x["metadata"]["name"] == name)
                self.assertEqual(json.loads(secret["stringData"]["flow-like.config.json"]), marker)
                self.assertNotIn("configured.example.test", json.dumps(values))

    def test_secret_reference_is_refused_because_the_worker_cannot_resolve_it(self):
        with patch.dict(os.environ, {"FLOW_LIKE_CONFIG_SECRET_REF": "hub-reference"}, clear=True):
            with self.assertRaisesRegex(ValueError, "FLOW_LIKE_CONFIG_SECRET_REF") as error:
                setup.generate("flow-like", "flow-like")
        self.assertNotIn("hub-reference", str(error.exception))

    def test_worker_shares_the_hub_config_and_learns_the_api_database_role(self):
        with patch.dict(os.environ, {}, clear=True):
            objects, values = setup.generate("flow-like", "flow-like")
        names = [x["metadata"]["name"] for x in objects["items"]]
        self.assertIn("flow-like-hub-config", names)
        self.assertFalse(any(name.endswith("-audit-config") for name in names))
        self.assertNotIn("runtimeConfig", values["audit"])
        self.assertEqual(values["audit"]["apiDatabaseRole"], "flowlike_api")
        urls = {"DATABASE_URL": "postgresql://owner:o@db.example.test:5432/flowlike?sslmode=verify-full",
                "API_DATABASE_URL": "postgresql://api%2Duser:a@db.example.test:5432/flowlike?sslmode=verify-full",
                "AUDIT_DATABASE_URL": "postgresql://audit:w@db.example.test:5432/flowlike?sslmode=verify-full"}
        with patch.dict(os.environ, urls, clear=True):
            _, values = setup.generate("flow-like", "flow-like")
        self.assertEqual(values["audit"]["apiDatabaseRole"], "api-user")

    def test_invalid_or_conflicting_runtime_sources_fail_without_content(self):
        for environment in ({"FLOW_LIKE_CONFIG_JSON": "sensitive-invalid-marker"}, {"FLOW_LIKE_CONFIG_FILE": "/not-present-sensitive-marker"}, {"FLOW_LIKE_CONFIG_JSON": "{}", "FLOW_LIKE_CONFIG_SECRET_REF": "sensitive-marker"}):
            with self.subTest(source=list(environment)), patch.dict(os.environ, environment, clear=True):
                with self.assertRaises(ValueError) as error:
                    setup.runtime_config()
                self.assertNotIn("sensitive", str(error.exception))

    def test_whitespace_runtime_sources_fail_instead_of_selecting_fallback(self):
        for key in ("FLOW_LIKE_CONFIG_JSON", "FLOW_LIKE_CONFIG_FILE", "FLOW_LIKE_CONFIG_SECRET_REF", "FLOW_LIKE_RUNTIME_CONFIG_FILE", "FLOW_LIKE_CONFIG"):
            inputs = [" ", "\t\n"]
            if key != "FLOW_LIKE_CONFIG_JSON":
                inputs += [" sensitive-marker", "sensitive-marker "]
            for value in inputs:
                with self.subTest(source=key), patch.dict(os.environ, {key: value}, clear=True):
                    with self.assertRaisesRegex(ValueError, "whitespace") as error:
                        setup.runtime_config()
                    self.assertNotIn("sensitive-marker", str(error.exception))
        with patch.dict(os.environ, {"FLOW_LIKE_CONFIG_JSON": " \n{}\t"}, clear=True):
            self.assertEqual(setup.runtime_config(), ("{}", None))

    def test_duplicate_json_keys_fail_without_normalizing_authentication(self):
        for text in ('{"authentication":{},"authentication":{}}', '{"authentication":{"variant":"first","variant":"second"}}'):
            with patch.dict(os.environ, {"FLOW_LIKE_CONFIG_JSON": text}, clear=True):
                with self.assertRaisesRegex(ValueError, "JSON object"):
                    setup.runtime_config()

    def test_whitespace_setup_creates_no_secret_or_values_files(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "config"
            env = {"PATH": os.environ["PATH"], "FLOW_LIKE_CONFIG_JSON": " \t "}
            result = subprocess.run(["python3", str(BASE / "scripts/setup-config.py"), "--output-dir", str(output)], env=env, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("whitespace", result.stderr)
            self.assertFalse(output.exists())

    def test_private_files_and_existing_secrets_are_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "config"
            command = ["python3", str(BASE / "scripts/setup-config.py"), "--output-dir", str(output)]
            env = {"PATH": os.environ["PATH"]}
            subprocess.run(command, env=env, check=True, capture_output=True)
            contents = (output / "secrets.yaml").read_bytes()
            self.assertEqual(stat.S_IMODE((output / "secrets.yaml").stat().st_mode), 0o600)
            self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o700)
            result = subprocess.run(command, env=env, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual((output / "secrets.yaml").read_bytes(), contents)

    def test_built_digests_and_partial_build_merge(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            docker = directory / "docker"
            docker.write_text("""#!/usr/bin/env python3
import json,os,sys
with open(os.environ['BUILD_LOG'],'a') as out: out.write(json.dumps(sys.argv[1:])+'\\n')
if sys.argv[1:3]==['image','inspect']:
    repository=sys.argv[3].rsplit(':',1)[0]
    print(json.dumps([repository+'@sha256:'+'a'*64]))
""")
            docker.chmod(0o700)
            output = directory / "values.json"
            output.write_text(json.dumps({"web": {"image": {"repository": "preserved/web", "tag": "existing"}}}))
            log = directory / "build.log"
            env = {"PATH": str(directory) + os.pathsep + os.environ["PATH"], "REGISTRY": "registry.example.com/team", "TAG": "review", "PUSH": "true", "COMPONENTS": "api executor execution-manager", "IMAGE_VALUES_FILE": str(output), "BUILD_LOG": str(log), "FLOW_LIKE_CONFIG": "runtime-only-config.json"}
            script = ["bash", str(BASE / "scripts/build-images.sh")]
            subprocess.run(script, env=env, check=True, capture_output=True)
            values = json.loads(output.read_text())
            self.assertEqual(values["web"]["image"]["tag"], "existing")
            self.assertEqual(values["executionManager"]["image"]["digest"], "sha256:" + "a" * 64)
            self.assertEqual(values["executionManager"]["sandbox"]["image"], "registry.example.com/team/flow-like-kubernetes-executor@sha256:" + "a" * 64)
            self.assertEqual(values["api"]["image"], {"repository": "registry.example.com/team/flow-like-kubernetes-api", "tag": "review", "pullPolicy": "IfNotPresent", "digest": "sha256:" + "a" * 64})
            self.assertEqual(values["executorPool"]["image"], values["executor"]["image"])
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            self.assertFalse(any("--build-arg" in call for call in calls))
            env.update({"COMPONENTS": "api", "FLOW_LIKE_BUILD_CONFIG": "flow-like.kubernetes.config.json"})
            subprocess.run(script, env=env, check=True, capture_output=True)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            self.assertTrue(any("FLOW_LIKE_CONFIG=flow-like.kubernetes.config.json" in call for call in calls))
            env.update({"PUSH": "false", "COMPONENTS": "executor"})
            subprocess.run(script, env=env, check=True, capture_output=True)
            values = json.loads(output.read_text())
            self.assertEqual(values["executionManager"]["sandbox"]["image"], "")
            self.assertNotIn("digest", values["executor"]["image"])
            self.assertIn("digest", values["executionManager"]["image"])

    def test_build_uses_published_repository_names_for_every_component(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            docker = directory / "docker"
            docker.write_text("#!/usr/bin/env python3\nimport json,os,sys\nwith open(os.environ['BUILD_LOG'],'a') as out: out.write(json.dumps(sys.argv[1:])+'\\n')\n")
            docker.chmod(0o700)
            output = directory / "values.json"
            log = directory / "build.log"
            env = {"PATH": str(directory) + os.pathsep + os.environ["PATH"], "IMAGE_VALUES_FILE": str(output), "BUILD_LOG": str(log)}
            subprocess.run(["bash", str(BASE / "scripts/build-images.sh")], env=env, check=True, capture_output=True)
            values = json.loads(output.read_text())
            expected = {
                ("api", "image"): "flow-like-kubernetes-api",
                ("audit", "image"): "flow-like-audit-worker",
                ("web", "image"): "flow-like-kubernetes-web",
                ("executor", "image"): "flow-like-kubernetes-executor",
                ("executorPool", "image"): "flow-like-kubernetes-executor",
                ("executionManager", "image"): "flow-like-kubernetes-execution-manager",
                ("database", "migration", "image"): "flow-like-kubernetes-migration",
                ("sinkServices", "image"): "flow-like-kubernetes-sink-trigger",
                ("executionManager", "queueBridge", "image"): "flow-like-docker-compose-runtime",
                ("compiler", "image"): "flow-like-docker-compose-compiler",
                ("signaling", "image"): "flow-like-docker-compose-signaling",
                ("rustfs", "bootstrap", "image"): "flow-like-docker-compose-object-store-init",
            }
            for path, name in expected.items():
                target = values
                for key in path:
                    target = target[key]
                self.assertEqual(target, {"repository": "ghcr.io/rheosoph/" + name, "tag": "local", "pullPolicy": "IfNotPresent"}, path)
            self.assertEqual(values["global"]["imageRegistry"], "")
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            dockerfiles = {call[call.index("-f") + 1] for call in calls if call[0] == "build"}
            self.assertIn(str(BASE / "sink-trigger/Dockerfile"), dockerfiles)
            self.assertFalse(any(call[0] == "push" for call in calls))

    def test_image_pull_secret_is_referenced_without_credentials(self):
        with patch.dict(os.environ, {}, clear=True):
            objects, values = setup.generate("flow-like", "flow-like", ["ghcr-pull", "mirror-pull"])
        self.assertEqual(values["global"], {"imagePullSecrets": [{"name": "ghcr-pull"}, {"name": "mirror-pull"}]})
        self.assertFalse(any(x["type"] == "kubernetes.io/dockerconfigjson" for x in objects["items"]))
        self.assertNotIn("docker-password", json.dumps(values))
        command = setup.pull_secret_command("ghcr-pull", "flow-like")
        self.assertTrue(command.startswith("kubectl create secret docker-registry ghcr-pull --namespace flow-like"))
        self.assertNotIn("ghp_", command)
        with patch.dict(os.environ, {}, clear=True), self.assertRaisesRegex(ValueError, "DNS subdomain"):
            setup.generate("flow-like", "flow-like", ["Not Valid"])

if __name__ == "__main__":
    unittest.main()
