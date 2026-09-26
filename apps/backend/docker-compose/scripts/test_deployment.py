"""Render actual Compose graphs and reject unsafe mode/configuration combinations."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / f"{name}.py")
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result

setup = module("setup-env")
preflight = module("preflight")
up = module("up")

def image_repository(workload):
    return "ghcr.io/rheosoph/flow-like-audit-worker" if workload == "audit-worker" else f"ghcr.io/rheosoph/flow-like-docker-compose-{workload}"

DOCKER_STUB = '''#!/usr/bin/env python3
import json, os, sys
with open(os.environ["DOCKER_LOG"], "a") as out:
    out.write(json.dumps(sys.argv[1:]) + "\\n")
if sys.argv[1] == "pull":
    if os.environ.get("DOCKER_PULL_DENIED") and os.environ["DOCKER_PULL_DENIED"] in sys.argv[-1]:
        print("Error response from daemon: denied", file=sys.stderr)
        sys.exit(1)
    print(sys.argv[-1] + ": Pulling from " + sys.argv[-1].split("/", 1)[-1].rsplit(":", 1)[0])
    if not os.environ.get("DOCKER_PULL_NO_DIGEST"):
        print("Digest: sha256:" + format(len(sys.argv[-1]), "064x"))
    print("Status: Downloaded newer image for " + sys.argv[-1])
elif sys.argv[1:3] == ["image", "inspect"]:
    reference = sys.argv[-1]
    if "{{json .RepoDigests}}" in sys.argv:
        repository = reference.rsplit(":", 1)[0]
        digest = "sha256:" + format(len(repository), "064x")
        print(json.dumps(["mirror.example.test/other@sha256:" + "f" * 64, repository + "@" + digest]))
    else:
        print(json.dumps("sha256:" + format(len(reference), "064x")))
'''

class DeploymentTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="flowlike-compose-test-")
        self.addCleanup(self.tmp.cleanup)
        self.path = Path(self.tmp.name) / ".env"
        self.text = setup.generate((ROOT / ".env.example").read_text(), "per-run", "http://localhost:3001", "http://localhost:8080", "http://s3.localhost:9000")
        self.text = self.text.replace("SANDBOX_IMAGE=", "SANDBOX_IMAGE=sha256:" + "a" * 64).replace("SANDBOX_GATEWAY_IMAGE=", "SANDBOX_GATEWAY_IMAGE=sha256:" + "b" * 64)

    def values(self, changes=None):
        values = {}
        for line in self.text.splitlines():
            if line and not line.startswith("#"):
                key, _, value = line.partition("=")
                values[key] = value
        values.update(changes or {})
        return values

    def write(self, values):
        self.path.write_text("\n".join(f"{key}={value}" for key, value in values.items()) + "\n")
        self.path.chmod(0o600)

    def stub_docker(self):
        directory = Path(self.tmp.name) / "bin"
        directory.mkdir(exist_ok=True)
        docker = directory / "docker"
        docker.write_text(DOCKER_STUB)
        docker.chmod(0o700)
        log = Path(self.tmp.name) / "docker.log"
        log.write_text("")
        env = {"PATH": str(directory) + os.pathsep + os.environ["PATH"], "DOCKER_LOG": str(log)}
        return env, lambda: [json.loads(line) for line in log.read_text().splitlines()]

    def render(self, changes=None, compose_file=None):
        values = self.values(changes)
        self.write(values)
        env = os.environ.copy()
        for key in values:
            env.pop(key, None)
        files = ["-f", compose_file] if compose_file else []
        process = subprocess.run(["docker", "compose", *files, "--env-file", str(self.path), "config", "--format", "json"], cwd=ROOT, env=env, capture_output=True, text=True, timeout=30)
        self.assertEqual(process.returncode, 0, process.stderr)
        config = json.loads(process.stdout)
        return values, config

    def test_hosted_frontend_api_settings_reach_compose_and_swarm(self):
        frontend = "https://app.example.test"
        self.text = setup.generate((ROOT / ".env.example").read_text(), "per-run", frontend, "https://api.example.test", "https://s3.example.test")
        for compose_file in [None, "docker-stack.yml"]:
            with self.subTest(compose_file=compose_file):
                _, config = self.render(compose_file=compose_file)
                api = config["services"]["api"]["environment"]
                self.assertEqual(api["FRONTEND_BASE_URL"], frontend)
                self.assertEqual(api["CORS_ALLOWED_ORIGINS"], frontend)
                web = config["services"]["web"]["environment"]
                self.assertEqual(web["FLOW_LIKE_WEB_REDIRECT_URL"], frontend + "/callback")

    def test_stripe_credentials_survive_setup_and_only_reach_api(self):
        supplied = {key: f"test_{index}" for index, key in enumerate(setup.STRIPE_SETTINGS)}
        self.text = setup.generate((ROOT / ".env.example").read_text(), "per-run", "http://localhost:3001", "http://localhost:8080", "http://s3.localhost:9000", stripe=supplied)
        for compose_file in ("docker-compose.yml", "docker-stack.yml"):
            _, config = self.render(compose_file=compose_file)
            for key, value in supplied.items():
                self.assertEqual(config["services"]["api"]["environment"][key], value)
                for name, service in config["services"].items():
                    if name != "api":
                        self.assertNotIn(key, service.get("environment", {}))
        with self.assertRaises(ValueError) as error:
            setup.generate("STRIPE_SECRET_KEY=", "per-run", "http://localhost:3001", "http://localhost:8080", "http://s3.localhost:9000", stripe={"STRIPE_SECRET_KEY": "secret_marker\nINJECTED=1"})
        self.assertNotIn("secret_marker", str(error.exception))

    def test_api_runtime_file_mount_and_explicit_source_switches(self):
        for compose_file in ("docker-compose.yml", "docker-stack.yml"):
            with self.subTest(compose_file=compose_file):
                _, config = self.render(compose_file=compose_file)
                api = config["services"]["api"]
                self.assertEqual(api["environment"]["FLOW_LIKE_CONFIG_FILE"], "/app/flow-like.config.json")
                self.assertIn({"source": "flowlike_runtime_config", "target": "/app/flow-like.config.json"}, api["configs"])
                self.assertEqual(config["configs"]["flowlike_runtime_config"]["file"], str(ROOT / "flow-like.config.example.json"))
                for source, value in (("FLOW_LIKE_CONFIG_JSON", "{\"name\":\"runtime\"}"), ("FLOW_LIKE_CONFIG_SECRET_REF", "hub-reference")):
                    _, config = self.render({"FLOW_LIKE_CONFIG_FILE": "", source: value}, compose_file=compose_file)
                    env = config["services"]["api"]["environment"]
                    self.assertEqual(env["FLOW_LIKE_CONFIG_FILE"], "")
                    self.assertEqual(env[source], value)

    def test_preflight_accepts_one_api_runtime_source_and_rejects_conflicts(self):
        values, config = self.render({"FLOW_LIKE_CONFIG_JSON": "{}"})
        self.assertTrue(any("Select one API runtime config source" in error for error in preflight.validate(values, config)))
        for changes in ({"FLOW_LIKE_CONFIG_FILE": "", "FLOW_LIKE_CONFIG_JSON": "{}"},
                        {"FLOW_LIKE_CONFIG_FILE": "", "FLOW_LIKE_CONFIG_SECRET_REF": "hub-reference"},
                        {"FLOW_LIKE_CONFIG_FILE": ""}):
            values, config = self.render(changes)
            self.assertEqual(preflight.validate(values, config), [], changes)

    def test_generator_switches_source_and_preserves_json_as_literal_data(self):
        template = (ROOT / ".env.example").read_text()
        value = "{\"name\":\"Runtime's $PUBLIC_API_URL\"}"
        self.text = setup.generate(template, "per-run", "http://localhost:3001", "http://localhost:8080", "http://s3.localhost:9000", {"FLOW_LIKE_CONFIG_JSON": value})
        _, config = self.render()
        env = config["services"]["api"]["environment"]
        self.assertEqual(env["FLOW_LIKE_CONFIG_FILE"], "")
        # `compose config` escapes literal dollars for reloading its output.
        self.assertEqual(env["FLOW_LIKE_CONFIG_JSON"], value.replace("$", "$$"))
        self.assertNotIn("FLOW_LIKE_CONFIG_JSON", config["services"]["audit-worker"]["environment"])
        with self.assertRaisesRegex(ValueError, "Select only one"):
            setup.generate(template, "per-run", "http://localhost:3001", "http://localhost:8080", "http://s3.localhost:9000", {"FLOW_LIKE_CONFIG_FILE": "/app/config", "FLOW_LIKE_CONFIG_JSON": "{}"})
        self.text = setup.generate(template, "per-run", "http://localhost:3001", "http://localhost:8080", "http://s3.localhost:9000", {"FLOW_LIKE_CONFIG_SECRET_REF": "hub-reference"})
        _, config = self.render()
        env = config["services"]["api"]["environment"]
        self.assertEqual(env["FLOW_LIKE_CONFIG_FILE"], "")
        self.assertEqual(env["FLOW_LIKE_CONFIG_SECRET_REF"], "hub-reference")

    def test_worker_embeds_the_api_build_config_and_receives_no_runtime_config(self):
        runtime = Path(self.tmp.name) / "runtime.json"
        runtime.write_text(json.dumps({"name": "Private API", "audit": {"level": "verbose"}}))
        runtime_sources = ({}, {"FLOW_LIKE_RUNTIME_CONFIG_FILE": str(runtime)},
                           {"FLOW_LIKE_CONFIG_FILE": "", "FLOW_LIKE_CONFIG_JSON": "{}"},
                           {"FLOW_LIKE_CONFIG_FILE": "", "FLOW_LIKE_CONFIG_SECRET_REF": "hub-reference"})
        for compose_file in ("docker-compose.yml", "docker-stack.yml"):
            for changes in runtime_sources:
                with self.subTest(compose_file=compose_file, changes=changes):
                    _, config = self.render(changes, compose_file=compose_file)
                    worker = config["services"]["audit-worker"]
                    self.assertNotIn("configs", worker)
                    for key in ("FLOW_LIKE_CONFIG_PATH", "FLOW_LIKE_CONFIG_JSON", "FLOW_LIKE_CONFIG_FILE",
                                "FLOW_LIKE_CONFIG_SECRET_REF", "AUDIT_WORKER_PAUSED", "AUDIT_API_DATABASE_ROLE", "BACKEND_KEY"):
                        self.assertNotIn(key, worker["environment"])
        for build_config in ("apps/backend/docker-compose/flow-like.config.example.json", "apps/backend/custom.config.json"):
            with self.subTest(build_config=build_config):
                _, config = self.render({"FLOW_LIKE_CONFIG": build_config})
                for name in ("api", "audit-worker"):
                    self.assertEqual(config["services"][name]["build"]["args"]["FLOW_LIKE_CONFIG"], build_config, name)
        values, config = self.render()
        self.assertEqual(preflight.validate(values, config), [])

    def test_generator_rejects_whitespace_and_duplicate_json_keys(self):
        template = (ROOT / ".env.example").read_text()
        for key in ("FLOW_LIKE_CONFIG_JSON", "FLOW_LIKE_CONFIG_FILE", "FLOW_LIKE_CONFIG_SECRET_REF", "FLOW_LIKE_RUNTIME_CONFIG_FILE"):
            inputs = [" ", "\t\n"]
            if key != "FLOW_LIKE_CONFIG_JSON":
                inputs += [" sensitive-marker", "sensitive-marker "]
            for value in inputs:
                with self.subTest(source=key), self.assertRaisesRegex(ValueError, "whitespace") as error:
                    setup.generate(template, "per-run", "http://localhost:3001", "http://localhost:8080", "http://s3.localhost:9000", {key: value})
                self.assertNotIn("sensitive-marker", str(error.exception))
        for text in ('{"authentication":{},"authentication":{}}', '{"authentication":{"variant":"first","variant":"second"}}'):
            with self.assertRaisesRegex(ValueError, "JSON object"):
                setup.generate(template, "per-run", "http://localhost:3001", "http://localhost:8080", "http://s3.localhost:9000", {"FLOW_LIKE_CONFIG_JSON": text})
        self.text = setup.generate(template, "per-run", "http://localhost:3001", "http://localhost:8080", "http://s3.localhost:9000", {"FLOW_LIKE_CONFIG_JSON": " \n{}\t"})
        _, config = self.render()
        self.assertEqual(config["services"]["api"]["environment"]["FLOW_LIKE_CONFIG_JSON"], "{}")
        self.assertEqual(config["services"]["api"]["environment"]["FLOW_LIKE_CONFIG_FILE"], "")

    def test_compose_preserves_invalid_whitespace_for_preflight_rejection(self):
        for key in ("FLOW_LIKE_CONFIG_JSON", "FLOW_LIKE_CONFIG_FILE", "FLOW_LIKE_CONFIG_SECRET_REF"):
            inputs = ["   "] if key == "FLOW_LIKE_CONFIG_JSON" else ["   ", " sensitive-marker", "sensitive-marker "]
            for value in inputs:
                with self.subTest(source=key):
                    values, config = self.render({"FLOW_LIKE_CONFIG_FILE": "", key: "'" + value + "'"})
                    self.assertEqual(config["services"]["api"]["environment"][key], value)
                    self.assertTrue(any("whitespace" in error for error in preflight.validate(values, config)))

    def test_whitespace_setup_creates_no_environment_file(self):
        env = {"PATH": os.environ["PATH"], "FLOW_LIKE_CONFIG_JSON": " \t "}
        result = subprocess.run(["python3", str(ROOT / "scripts/setup-env.py"), "--output", str(self.path)], env=env, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("whitespace", result.stderr)
        self.assertFalse(self.path.exists())

    def test_per_run_graph_and_secret_boundaries(self):
        values, config = self.render()
        self.assertEqual(preflight.validate(values, config), [])
        services = config["services"]
        self.assertNotIn("runtime", services)
        self.assertEqual(services["queue-bridge"]["environment"]["EXECUTION_ISOLATION_MODE"], "per_run")
        self.assertEqual(services["api"]["depends_on"]["object-store-init"]["condition"], "service_completed_successfully")
        for name in ["api", "queue-bridge", "execution-manager", "compiler", "signaling"]:
            env = services[name]["environment"]
            self.assertNotIn("RUSTFS_ROOT_PASSWORD", env)
            if name != "api":
                self.assertNotIn("BACKEND_KEY", env)
                self.assertNotIn("AWS_SECRET_ACCESS_KEY", env)
                self.assertNotIn("STS_ISSUER_SECRET_KEY", env)
        for name in ["postgres", "redis", "compiler", "execution-manager", "queue-bridge", "object-store"]:
            self.assertNotIn("ports", services[name])
        self.assertNotIn("database", services["execution-manager"]["networks"])
        self.assertNotIn("queue", services["execution-manager"]["networks"])
        self.assertTrue(config["networks"]["database"]["internal"])
        self.assertTrue(config["networks"]["queue"]["internal"])

    def test_scaling_timeouts_and_storage_forwarding(self):
        values, config = self.render({"API_REPLICAS": "3", "COMPILER_REPLICAS": "2", "WEB_REPLICAS": "2", "SIGNALING_REPLICAS": "2", "EXECUTION_TIMEOUT_SECONDS": "321", "COMPILER_ALLOWED_STORAGE_HOSTS": "http://s3.localhost:9000"})
        self.assertEqual(preflight.validate(values, config), [])
        services = config["services"]
        for name, replicas in [("api", 3), ("compiler", 2), ("web", 2), ("signaling", 2)]:
            self.assertEqual(services[name]["deploy"]["replicas"], replicas)
        self.assertEqual(services["queue-bridge"]["environment"]["EXECUTOR_TIMEOUT_SECS"], "321")
        self.assertEqual(services["execution-manager"]["environment"]["EXECUTION_TIMEOUT_SECONDS"], "321")
        self.assertEqual(services["compiler"]["environment"]["COMPILER_ALLOWED_STORAGE_HOSTS"], "http://s3.localhost:9000")
        self.assertEqual(services["api"]["environment"]["S3_STS_PROVIDER"], "rustfs")
        self.assertEqual(services["api"]["environment"]["STS_ENDPOINT_URL"], "http://object-store:9000")

    def test_explicit_trusted_profile(self):
        values, config = self.render({"EXECUTION_ISOLATION_MODE": "trusted_shared", "COMPOSE_PROFILES": "trusted", "EXECUTOR_URL": "http://runtime-gateway:9000"})
        self.assertEqual(preflight.validate(values, config), [])
        self.assertIn("runtime", config["services"])
        self.assertNotIn("execution-manager", config["services"])
        self.assertEqual(config["services"]["runtime"]["environment"]["EXECUTION_ISOLATION_MODE"], "trusted_shared")

    def test_native_manager_threads_healthcheck_and_warm_requirement(self):
        values, config = self.render({"EXECUTION_MANAGER_WORKER_THREADS": "3"})
        self.assertEqual(preflight.validate(values, config), [])
        manager = config["services"]["execution-manager"]
        self.assertEqual(manager["environment"]["EXECUTION_MANAGER_WORKER_THREADS"], "3")
        self.assertEqual(manager["healthcheck"]["test"], ["CMD", "/app/execution-manager", "healthcheck", "http://127.0.0.1:9000/ready"])
        for key, value in [("EXECUTION_MANAGER_WORKER_THREADS", "0"), ("EXECUTION_MANAGER_WORKER_THREADS", "65"), ("SANDBOX_WARM_POOL_SIZE", "0")]:
            values, config = self.render({key: value})
            self.assertTrue(any(key in error for error in preflight.validate(values, config)))

    def test_published_images_default_to_ghcr_tag_and_keep_local_build_blocks(self):
        for compose_file in ("docker-compose.yml", "docker-stack.yml"):
            with self.subTest(compose_file=compose_file):
                _, config = self.render(compose_file=compose_file)
                services = config["services"]
                for key, workload in preflight.IMAGE_WORKLOADS.items():
                    name = "object-store-init" if workload == "object-store-init" else workload
                    if name not in services:
                        continue
                    self.assertEqual(services[name]["image"], f"{image_repository(workload)}:dev", key)
                if compose_file == "docker-compose.yml":
                    self.assertEqual(services["queue-bridge"]["image"], "ghcr.io/rheosoph/flow-like-docker-compose-runtime:dev")
                    for name in ("api", "audit-worker", "queue-bridge", "execution-manager", "object-store-init", "db-init", "compiler", "signaling", "web", "sink-services"):
                        self.assertIn("build", services[name], name)
        _, config = self.render({"FLOW_LIKE_IMAGE_TAG": "1.2.3", "API_IMAGE": "registry.example.test/api@sha256:" + "c" * 64})
        self.assertEqual(config["services"]["api"]["image"], "registry.example.test/api@sha256:" + "c" * 64)
        self.assertEqual(config["services"]["web"]["image"], "ghcr.io/rheosoph/flow-like-docker-compose-web:1.2.3")

    def test_preflight_image_tag_and_digest_pin_alignment(self):
        for tag in ("dev", "1.2.3-beta", "sha-" + "a" * 40 + "-run-123-1", "_x", ""):
            values, config = self.render({"FLOW_LIKE_IMAGE_TAG": tag})
            self.assertEqual(preflight.validate(values, config), [], tag)
        for tag in ("-dev", "dev tag", "dev:latest", "a" * 129):
            values, config = self.render({"FLOW_LIKE_IMAGE_TAG": tag})
            self.assertTrue(any("FLOW_LIKE_IMAGE_TAG" in error for error in preflight.validate(values, config)), tag)
        runtime = "ghcr.io/rheosoph/flow-like-docker-compose-runtime@sha256:" + "1" * 64
        manager = "ghcr.io/rheosoph/flow-like-docker-compose-execution-manager@sha256:" + "2" * 64
        values, config = self.render({"SANDBOX_IMAGE": runtime, "SANDBOX_GATEWAY_IMAGE": manager, "RUNTIME_IMAGE": runtime, "EXECUTION_MANAGER_IMAGE": manager})
        self.assertEqual(preflight.validate(values, config), [])
        values, config = self.render({"SANDBOX_IMAGE": runtime, "SANDBOX_GATEWAY_IMAGE": manager, "RUNTIME_IMAGE": "", "EXECUTION_MANAGER_IMAGE": "flow-like-execution-manager:local"})
        errors = preflight.validate(values, config)
        self.assertTrue(any(error.startswith("RUNTIME_IMAGE must equal the SANDBOX_IMAGE") for error in errors))
        self.assertTrue(any(error.startswith("EXECUTION_MANAGER_IMAGE must equal the SANDBOX_GATEWAY_IMAGE") for error in errors))
        values, config = self.render({"RUNTIME_IMAGE": "flow-like-runtime:local", "EXECUTION_MANAGER_IMAGE": manager})
        self.assertEqual(preflight.validate(values, config), [])

    def test_pull_images_pins_digests_and_preserves_secrets(self):
        values = self.values({"FLOW_LIKE_IMAGE_TAG": "1.4.0", "API_IMAGE": "stale:local"})
        self.write(values)
        env, calls = self.stub_docker()
        result = subprocess.run(["python3", str(ROOT / "scripts/pull-images.py"), "--env-file", str(self.path)], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)
        pinned = preflight.read_env(self.path)
        for key, workload in preflight.IMAGE_WORKLOADS.items():
            repository = image_repository(workload)
            self.assertEqual(pinned[key], repository + "@sha256:" + format(len(repository + ":1.4.0"), "064x"))
        self.assertEqual(len([call for call in calls() if call[:2] == ["image", "inspect"]]), 0)
        self.assertEqual(pinned["SANDBOX_IMAGE"], pinned["RUNTIME_IMAGE"])
        self.assertEqual(pinned["SANDBOX_GATEWAY_IMAGE"], pinned["EXECUTION_MANAGER_IMAGE"])
        self.assertEqual(pinned["FLOW_LIKE_IMAGE_TAG"], "1.4.0")
        for key, value in values.items():
            if key not in pinned or (key.endswith("_IMAGE") and key in preflight.IMAGE_WORKLOADS) or key in preflight.SANDBOX_SOURCES:
                continue
            self.assertEqual(pinned[key], value.strip("'"), key)
        self.assertEqual(len(pinned), len(values))
        pulls = [call[-1] for call in calls() if call[0] == "pull"]
        self.assertEqual(pulls, [f"{image_repository(workload)}:1.4.0" for workload in preflight.IMAGE_WORKLOADS.values()])
        self.assertNotIn(values["BACKEND_KEY"], result.stdout + result.stderr)
        self.assertEqual(preflight.validate(pinned, self.render(pinned)[1]), [])
        immutable = "sha-" + "b" * 40 + "-run-42-1"
        result = subprocess.run(["python3", str(ROOT / "scripts/pull-images.py"), "--env-file", str(self.path), "--tag", immutable, "--registry", "ghcr.io/fork/"], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        pinned = preflight.read_env(self.path)
        self.assertEqual(pinned["FLOW_LIKE_IMAGE_TAG"], immutable)
        self.assertTrue(pinned["WEB_IMAGE"].startswith("ghcr.io/fork/flow-like-docker-compose-web@sha256:"))
        self.assertIn(f"ghcr.io/fork/flow-like-docker-compose-web:{immutable}", [call[-1] for call in calls() if call[0] == "pull"])

    def test_pull_images_falls_back_to_repository_digest_for_the_pulled_repository(self):
        self.write(self.values())
        env, calls = self.stub_docker()
        env["DOCKER_PULL_NO_DIGEST"] = "1"
        result = subprocess.run(["python3", str(ROOT / "scripts/pull-images.py"), "--env-file", str(self.path), "--tag", "1.4.0"], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        pinned = preflight.read_env(self.path)
        for key, workload in preflight.IMAGE_WORKLOADS.items():
            repository = image_repository(workload)
            self.assertEqual(pinned[key], repository + "@sha256:" + format(len(repository), "064x"))
        inspected = [call[-1] for call in calls() if call[:2] == ["image", "inspect"]]
        self.assertEqual(inspected, [f"{image_repository(workload)}:1.4.0" for workload in preflight.IMAGE_WORKLOADS.values()])

    def test_pull_images_failure_hints_login_and_changes_nothing(self):
        self.write(self.values())
        before = self.path.read_text()
        env, calls = self.stub_docker()
        env["DOCKER_PULL_DENIED"] = "flow-like-docker-compose-compiler"
        result = subprocess.run(["python3", str(ROOT / "scripts/pull-images.py"), "--env-file", str(self.path)], env=env, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("docker login ghcr.io", result.stderr)
        self.assertEqual(self.path.read_text(), before)
        pulls = [call[-1] for call in calls() if call[0] == "pull"]
        self.assertEqual(pulls[-1], "ghcr.io/rheosoph/flow-like-docker-compose-compiler:dev")
        self.assertEqual(len(pulls), list(preflight.IMAGE_WORKLOADS.values()).index("compiler") + 1)
        for arguments in (["--tag", "bad tag"], ["--registry", "ghcr.io"]):
            result = subprocess.run(["python3", str(ROOT / "scripts/pull-images.py"), "--env-file", str(self.path), *arguments], env=env, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0, arguments)
        self.path.chmod(0o644)
        result = subprocess.run(["python3", str(ROOT / "scripts/pull-images.py"), "--env-file", str(self.path)], env=env, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("chmod 600", result.stderr)
        self.assertEqual(len([call for call in calls() if call[0] == "pull"]), len(pulls))

    def test_prepare_images_resets_digest_pins_to_local_tags(self):
        digest = "ghcr.io/rheosoph/flow-like-docker-compose-runtime@sha256:" + "1" * 64
        self.write(self.values({"RUNTIME_IMAGE": digest, "EXECUTION_MANAGER_IMAGE": "", "COMPILER_IMAGE": "keep@sha256:" + "3" * 64}))
        env, calls = self.stub_docker()
        result = subprocess.run(["python3", str(ROOT / "scripts/prepare-images.py"), "--env-file", str(self.path)], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        pinned = preflight.read_env(self.path)
        self.assertEqual(pinned["RUNTIME_IMAGE"], "flow-like-runtime:local")
        self.assertEqual(pinned["EXECUTION_MANAGER_IMAGE"], "flow-like-execution-manager:local")
        self.assertEqual(pinned["COMPILER_IMAGE"], "keep@sha256:" + "3" * 64)
        self.assertEqual(pinned["SANDBOX_IMAGE"], "sha256:" + format(len("flow-like-runtime:local"), "064x"))
        self.assertEqual(pinned["SANDBOX_GATEWAY_IMAGE"], "sha256:" + format(len("flow-like-execution-manager:local"), "064x"))
        self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)
        build = next(call for call in calls() if call[0] == "compose")
        self.assertEqual(build[-3:], ["build", "runtime", "execution-manager"])
        self.write(self.values({"RUNTIME_IMAGE": "custom/runtime:review", "EXECUTION_MANAGER_IMAGE": "custom/manager:review"}))
        subprocess.run(["python3", str(ROOT / "scripts/prepare-images.py"), "--env-file", str(self.path)], env=env, check=True, capture_output=True)
        pinned = preflight.read_env(self.path)
        self.assertEqual(pinned["RUNTIME_IMAGE"], "custom/runtime:review")
        self.assertEqual(pinned["SANDBOX_IMAGE"], "sha256:" + format(len("custom/runtime:review"), "064x"))

    def test_up_defaults_to_no_build_and_assigns_local_tags_when_building(self):
        values = self.values()
        self.assertEqual(up.compose_command(values, self.path, False)[-2:], ["-d", "--no-build"])
        self.assertEqual(up.compose_command(values, self.path, True)[-2:], ["-d", "--build"])
        self.assertEqual(up.local_tags(values), {key: f"flow-like-{workload}:local" for key, workload in preflight.IMAGE_WORKLOADS.items()})
        local = self.values({key: f"flow-like-{workload}:local" for key, workload in preflight.IMAGE_WORKLOADS.items()})
        self.assertEqual(up.local_tags(local), {})
        local["SIGNALING_IMAGE"] = ""
        self.assertEqual(up.local_tags(local), {"SIGNALING_IMAGE": "flow-like-signaling:local"})
        values["WEB_IMAGE"] = "ghcr.io/rheosoph/flow-like-docker-compose-web@sha256:" + "4" * 64
        self.assertEqual(up.compose_command(values, self.path, False)[-1], "--no-build")
        with self.assertRaisesRegex(ValueError, r"digest-pinned images \(WEB_IMAGE\)"):
            up.compose_command(values, self.path, True)
        self.write(values)
        env, calls = self.stub_docker()
        result = subprocess.run(["python3", str(ROOT / "scripts/up.py"), "--env-file", str(self.path), "--build"], env=env, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("prepare-images.py", result.stderr)
        self.assertEqual(calls(), [])
        self.write(self.values({"RUNTIME_IMAGE": "custom/runtime:review"}))
        result = subprocess.run(["python3", str(ROOT / "scripts/up.py"), "--env-file", str(self.path), "--build"], env=env, capture_output=True, text=True)
        written = preflight.read_env(self.path)
        self.assertEqual(written["RUNTIME_IMAGE"], "custom/runtime:review")
        self.assertEqual(written["API_IMAGE"], "flow-like-api:local")
        self.assertEqual(written["OBJECT_STORE_INIT_IMAGE"], "flow-like-object-store-init:local")
        self.assertIn("Set API_IMAGE", result.stdout)
        self.assertNotIn("RUNTIME_IMAGE", result.stdout)

    def test_rejects_unsupported_compiler_and_unpinned_sandbox(self):
        values, config = self.render({"COMPILATION_BACKEND": "redis", "SANDBOX_IMAGE": "runtime:latest"})
        errors = preflight.validate(values, config)
        self.assertTrue(any("COMPILATION_BACKEND" in error for error in errors))
        self.assertTrue(any("SANDBOX_IMAGE" in error for error in errors))

    def test_rejects_mixed_execution_profiles(self):
        values, config = self.render({"COMPOSE_PROFILES": "per-run,trusted"})
        self.assertTrue(any("Shared runtime" in error for error in preflight.validate(values, config)))

    def test_rejects_shutdown_that_would_interrupt_execution(self):
        values, config = self.render({"EXECUTION_STOP_GRACE_PERIOD": "60s"})
        errors = preflight.validate(values, config)
        for name in ("execution-manager", "queue-bridge"):
            self.assertTrue(any(f"{name} stop_grace_period" in error for error in errors))
        values, config = self.render({"EXECUTION_TIMEOUT_SECONDS": "7200", "EXECUTION_STOP_GRACE_PERIOD": "2h5m"})
        self.assertEqual(preflight.validate(values, config), [])

    def test_external_store_disables_bootstrap(self):
        values, config = self.render({"OBJECT_STORE_MODE": "external", "COMPOSE_FILE": "docker-compose.yml:docker-compose.external-store.yml", "S3_INTERNAL_ENDPOINT": "https://s3.example.test", "AUDIT_BUCKET_ENDPOINT": "https://s3.example.test", "S3_PUBLIC_ENDPOINT": "https://s3.example.test", "STS_ENDPOINT_URL": "https://sts.example.test", "S3_STS_PROVIDER": "aws", "EXECUTION_OBJECT_STORE_TLS_GATEWAY": "true", "COMPILER_ALLOWED_STORAGE_HOSTS": "https://s3.example.test"})
        self.assertEqual(preflight.validate(values, config), [])
        for service in ["object-store", "object-store-init", "object-gateway"]:
            self.assertNotIn(service, config["services"])
        self.assertNotIn("object-store-init", config["services"]["api"].get("depends_on", {}))

    def test_external_store_and_datastores_compose_together(self):
        external = {"OBJECT_STORE_MODE": "external", "DATASTORE_MODE": "external", "COMPOSE_FILE": "docker-compose.yml:docker-compose.external-store.yml:docker-compose.external-datastores.yml", "S3_INTERNAL_ENDPOINT": "https://s3.example.test", "AUDIT_BUCKET_ENDPOINT": "https://s3.example.test", "S3_PUBLIC_ENDPOINT": "https://s3.example.test", "STS_ENDPOINT_URL": "https://sts.example.test", "S3_STS_PROVIDER": "aws", "EXECUTION_OBJECT_STORE_TLS_GATEWAY": "true", "COMPILER_ALLOWED_STORAGE_HOSTS": "https://s3.example.test", "METRICS_REDIS_URL": "redis://metrics@redis.example.test:6379", "DATABASE_URL": "postgresql://api:api-password@db.example.test/database", "MIGRATION_DATABASE_URL": "postgresql://owner:owner-password@db.example.test/database", "AUDIT_DATABASE_URL": "postgresql://audit:audit-password@db.example.test/database", "REDIS_URL": "rediss://api@redis.example.test:6379", "RUNTIME_REDIS_URL": "rediss://runtime@redis.example.test:6379", "SIGNALING_REDIS_URL": "rediss://signaling@redis.example.test:6379", "SINK_REDIS_URL": "rediss://sink@redis.example.test:6379"}
        values, config = self.render(external)
        self.assertEqual(preflight.validate(values, config), [])
        for service in ["object-store", "object-store-init", "object-gateway", "postgres", "redis"]:
            self.assertNotIn(service, config["services"])
        # db-init stays in the default profile: every `up` re-applies the schema and
        # the audit boundary before the API and the worker start.
        db_init = config["services"]["db-init"]
        self.assertNotIn("profiles", db_init)
        self.assertFalse(db_init.get("depends_on"))
        self.assertEqual(set(db_init["networks"]), {"app"})
        for name in ("api", "audit-worker"):
            self.assertEqual(set(config["services"][name]["depends_on"]), {"db-init"}, name)
            self.assertEqual(config["services"][name]["depends_on"]["db-init"]["condition"], "service_completed_successfully")

    def test_audit_credentials_are_confined_to_worker(self):
        for compose_file in ("docker-compose.yml", "docker-stack.yml"):
            values, config = self.render(compose_file=compose_file)
            api = config["services"]["api"]["environment"]
            worker = config["services"]["audit-worker"]["environment"]
            self.assertEqual(api["AUDIT_WORKER"], "off")
            self.assertEqual(worker["AUDIT_ENTRY_KEY"], api["AUDIT_ENTRY_KEY"])
            self.assertNotEqual(worker["DATABASE_URL"], api["DATABASE_URL"])
            self.assertNotIn("BACKEND_KEY", worker)
            for key in ["AUDIT_SIGNING_KEY", "AUDIT_BUCKET", "AUDIT_BUCKET_ACCESS_KEY_ID", "AUDIT_BUCKET_SECRET_ACCESS_KEY",
                        "AUDIT_KMS_KEY_ID", "AUDIT_VAULT_TOKEN", "AUDIT_VAULT_TOKEN_FILE"]:
                self.assertNotIn(key, api)
            public = json.loads(api["AUDIT_VERIFYING_KEYS"])
            self.assertIn(api["AUDIT_KID"], public)
            self.assertIn("PUBLIC KEY", public[api["AUDIT_KID"]])
            self.assertNotIn("PRIVATE KEY", json.dumps(public))
            self.assertFalse(config["services"]["audit-worker"].get("ports"))
        values, config = self.render()
        self.assertEqual(preflight.validate(values, config), [])
        worker = config["services"]["audit-worker"]["environment"]
        init = config["services"]["object-store-init"]["environment"]
        self.assertEqual(worker["AUDIT_BUCKET"], "flow-like-audit")
        self.assertEqual(worker["AUDIT_BUCKET_ENDPOINT"], "http://object-store:9000")
        self.assertEqual(init["AUDIT_BUCKET_SECRET_ACCESS_KEY"], worker["AUDIT_BUCKET_SECRET_ACCESS_KEY"])
        self.assertNotIn("AUDIT_SIGNING_KEY", init)
        self.assertEqual(init["AUDIT_BUCKET_LOCK_MODE"], "COMPLIANCE")
        self.assertEqual(init["AUDIT_BUCKET_RETENTION_YEARS"], "4")
        for change in ({"AUDIT_BUCKET_ENDPOINT": "http://s3.localhost:9000"}, {"AUDIT_BUCKET": "flow-like-logs"},
                       {"AUDIT_BUCKET_ACCESS_KEY_ID": values["AWS_ACCESS_KEY_ID"]}):
            values, config = self.render(change)
            self.assertTrue(any("AUDIT_BUCKET" in error for error in preflight.validate(values, config)), change)
        values, config = self.render({"AUDIT_BUCKET": ""})
        self.assertTrue(any("requires an immutable AUDIT_BUCKET" in error for error in preflight.validate(values, config)))

    def test_preflight_rejects_missing_worker_or_leaked_authority(self):
        values, config = self.render()
        del config["services"]["audit-worker"]
        self.assertTrue(any("dedicated audit-worker" in error for error in preflight.validate(values, config)))
        values, config = self.render()
        config["services"]["api"]["environment"]["AUDIT_SIGNING_KEY"] = "leaked"
        self.assertTrue(any("API must run without" in error for error in preflight.validate(values, config)))
        values, config = self.render({"AUDIT_DATABASE_URL": self.values()["DATABASE_URL"]})
        self.assertTrue(any("distinct database" in error for error in preflight.validate(values, config)))

    def test_swarm_stack_rejects_the_bundled_audit_endpoint(self):
        for mode in ("bundled", "external"):
            values, config = self.render({"OBJECT_STORE_MODE": mode}, compose_file="docker-stack.yml")
            self.assertNotIn("object-store", config["services"])
            self.assertTrue(any("AUDIT_BUCKET_ENDPOINT names the bundled object store" in error for error in preflight.validate(values, config)), mode)
        for change in ({"AUDIT_BUCKET": ""}, {"AUDIT_BUCKET_ENDPOINT": "https://s3.example.test"}):
            values, config = self.render({"OBJECT_STORE_MODE": "external", **change}, compose_file="docker-stack.yml")
            self.assertFalse(any("AUDIT_BUCKET_ENDPOINT" in error for error in preflight.validate(values, config)), change)

    def test_audit_kms_key_replaces_the_generated_signing_key(self):
        values = self.values()
        self.assertTrue(values["AUDIT_SIGNING_KEY"])
        self.assertEqual(values["AUDIT_KMS_KEY_ID"], "")
        template = (ROOT / ".env.example").read_text()
        kms = {"AUDIT_KMS_KEY_ID": "arn:aws:kms:eu-central-1:123456789012:key/0f1e2d3c", "AUDIT_KMS_AWS_ACCESS_KEY_ID": "AKIAKMS", "AUDIT_KMS_AWS_SECRET_ACCESS_KEY": "kms/secret+key"}
        self.text = setup.generate(template, "per-run", "http://localhost:3001", "http://localhost:8080", "http://s3.localhost:9000", None, kms)
        self.text = self.text.replace("SANDBOX_IMAGE=", "SANDBOX_IMAGE=sha256:" + "a" * 64).replace("SANDBOX_GATEWAY_IMAGE=", "SANDBOX_GATEWAY_IMAGE=sha256:" + "b" * 64)
        values, config = self.render()
        self.assertEqual(values["AUDIT_SIGNING_KEY"], "")
        self.assertEqual(preflight.validate(values, config), [])
        api = config["services"]["audit-worker"]["environment"]
        for key, value in kms.items():
            self.assertEqual(api[key], value)
        for change, problem in (({"AUDIT_SIGNING_KEY": self.values()["BACKEND_KEY"]}, "not both"),
                                ({"AUDIT_KMS_AWS_ACCESS_KEY_ID": "", "AUDIT_KMS_AWS_SECRET_ACCESS_KEY": ""}, "bundled storage"),
                                ({"AUDIT_KMS_AWS_SECRET_ACCESS_KEY": ""}, "or neither"),
                                ({"AUDIT_KMS_PROVIDER": "vault"}, "AUDIT_VAULT_ADDR"),
                                ({"AUDIT_KMS_PROVIDER": "hsm"}, "aws, gcp, azure or vault")):
            values, config = self.render(change)
            self.assertTrue(any(problem in error for error in preflight.validate(values, config)), change)
        values, config = self.render({"AUDIT_KMS_KEY_ID": "projects/p/locations/l/keyRings/r/cryptoKeys/k/cryptoKeyVersions/1", "AUDIT_KMS_AWS_ACCESS_KEY_ID": "", "AUDIT_KMS_AWS_SECRET_ACCESS_KEY": ""})
        self.assertEqual(preflight.validate(values, config), [])
        for invalid in ({"AUDIT_KMS_REGION": "eu-central-1"}, {"AUDIT_KMS_KEY_ID": "alias/audit key"},
                        {"AUDIT_KMS_KEY_ID": "alias/audit", "AUDIT_KMS_AWS_ACCESS_KEY_ID": "AKIAKMS"}):
            with self.assertRaises(ValueError):
                setup.generate(template, "per-run", "http://localhost:3001", "http://localhost:8080", "http://s3.localhost:9000", None, invalid)

    def test_audit_key_in_a_self_hosted_vault_needs_an_address_and_a_token(self):
        template = (ROOT / ".env.example").read_text()
        vault = {"AUDIT_KMS_KEY_ID": "transit/audit", "AUDIT_VAULT_ADDR": "https://vault.internal:8200",
                 "AUDIT_VAULT_TOKEN": "hvs.exampletoken"}
        self.text = setup.generate(template, "per-run", "http://localhost:3001", "http://localhost:8080", "http://s3.localhost:9000", None, vault)
        self.text = self.text.replace("SANDBOX_IMAGE=", "SANDBOX_IMAGE=sha256:" + "a" * 64).replace("SANDBOX_GATEWAY_IMAGE=", "SANDBOX_GATEWAY_IMAGE=sha256:" + "b" * 64)
        values, config = self.render()
        self.assertEqual(values["AUDIT_SIGNING_KEY"], "")
        # No AWS credentials: transit signs with a key the deployment holds itself.
        self.assertEqual(preflight.validate(values, config), [])
        api = config["services"]["audit-worker"]["environment"]
        for key, value in vault.items():
            self.assertEqual(api[key], value)
        for change, problem in (({"AUDIT_VAULT_TOKEN": ""}, "AUDIT_VAULT_TOKEN"),
                                ({"AUDIT_VAULT_ADDR": "vault.internal:8200"}, "must be a URL"),
                                ({"AUDIT_KMS_REGION": "eu-central-1"}, "belongs to AWS KMS"),
                                # A private CA is only consulted over TLS; http:// would
                                # put the bearer token on the network in the clear.
                                ({"AUDIT_VAULT_CA_FILE": "/etc/flow-like/vault-ca.pem",
                                  "AUDIT_VAULT_ADDR": "http://vault.internal:8200"}, "https://")):
            values, config = self.render(change)
            self.assertTrue(any(problem in error for error in preflight.validate(values, config)), change)
        # A token file replaces the token; a Vault Agent renews it in place.
        values, config = self.render({"AUDIT_VAULT_TOKEN": "", "AUDIT_VAULT_TOKEN_FILE": "/vault/secrets/token"})
        self.assertEqual(preflight.validate(values, config), [])
        for invalid in ({"AUDIT_KMS_KEY_ID": "transit/audit", "AUDIT_VAULT_ADDR": "vault.internal"},
                        {"AUDIT_KMS_KEY_ID": "transit/audit", "AUDIT_VAULT_ADDR": "https://vault.internal:8200"}):
            with self.assertRaises(ValueError):
                setup.generate(template, "per-run", "http://localhost:3001", "http://localhost:8080", "http://s3.localhost:9000", None, invalid)

    def test_rejects_unqualified_tls_storage_and_zero_limits(self):
        values, config = self.render({"S3_PUBLIC_ENDPOINT": "https://storage.example.test", "COMPILER_ALLOWED_STORAGE_HOSTS": "https://storage.example.test", "COMPILER_MAX_PARALLEL_TARGETS": "0"})
        errors = preflight.validate(values, config)
        self.assertTrue(any("TLS_GATEWAY" in error for error in errors))
        self.assertTrue(any("COMPILER_MAX_PARALLEL_TARGETS" in error for error in errors))

    def test_proxy_logs_exclude_queries_and_authorization(self):
        object_proxy = (ROOT / "proxy/object-store.conf.template").read_text()
        self.assertIn("access_log off;", object_proxy)
        self.assertIn("error_log /dev/null;", object_proxy)
        for path in [ROOT / "web/nginx.conf", *(ROOT / "proxy").glob("*.conf")]:
            source = path.read_text()
            self.assertIn("log_format flowlike_safe", source, path.name)
            self.assertIn("error_log /dev/null;", source, path.name)
            format_line = next(line for line in source.splitlines() if line.startswith("log_format"))
            self.assertNotIn("$request ", format_line)
            self.assertNotIn("$request_uri", format_line)
            self.assertNotIn("$args", format_line)
            self.assertNotIn("$http_authorization", format_line)

    def test_setup_never_overwrites_existing_file(self):
        self.path.write_text("keep this private value")
        process = subprocess.run(["python3", str(ROOT / "scripts/setup-env.py"), "--output", str(self.path)], capture_output=True, text=True)
        self.assertNotEqual(process.returncode, 0)
        self.assertEqual(self.path.read_text(), "keep this private value")

    def test_setup_creates_distinct_private_credentials(self):
        process = subprocess.run(["python3", str(ROOT / "scripts/setup-env.py"), "--output", str(self.path)], capture_output=True, text=True)
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)
        values = preflight.read_env(self.path)
        self.assertEqual(len({values[k] for k in ["AWS_ACCESS_KEY_ID", "STS_ISSUER_ACCESS_KEY", "RUSTFS_ROOT_USER"]}), 3)
        self.assertNotIn(values["BACKEND_KEY"], process.stdout)

if __name__ == "__main__":
    unittest.main()
