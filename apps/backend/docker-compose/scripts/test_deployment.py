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

class DeploymentTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="flowlike-compose-test-")
        self.addCleanup(self.tmp.cleanup)
        self.path = Path(self.tmp.name) / ".env"
        self.text = setup.generate((ROOT / ".env.example").read_text(), "per-run", "http://localhost:3001", "http://localhost:8080", "http://s3.localhost:9000")
        self.text = self.text.replace("SANDBOX_IMAGE=", "SANDBOX_IMAGE=sha256:" + "a" * 64).replace("SANDBOX_GATEWAY_IMAGE=", "SANDBOX_GATEWAY_IMAGE=sha256:" + "b" * 64)

    def render(self, changes=None):
        values = {}
        for line in self.text.splitlines():
            if line and not line.startswith("#"):
                key, _, value = line.partition("=")
                values[key] = value
        values.update(changes or {})
        self.path.write_text("\n".join(f"{key}={value}" for key, value in values.items()) + "\n")
        self.path.chmod(0o600)
        env = os.environ.copy()
        for key in values:
            env.pop(key, None)
        process = subprocess.run(["docker", "compose", "--env-file", str(self.path), "config", "--format", "json"], cwd=ROOT, env=env, capture_output=True, text=True, timeout=30)
        self.assertEqual(process.returncode, 0, process.stderr)
        config = json.loads(process.stdout)
        return values, config

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
        values, config = self.render({"OBJECT_STORE_MODE": "external", "COMPOSE_FILE": "docker-compose.yml:docker-compose.external-store.yml", "S3_INTERNAL_ENDPOINT": "https://s3.example.test", "S3_PUBLIC_ENDPOINT": "https://s3.example.test", "STS_ENDPOINT_URL": "https://sts.example.test", "S3_STS_PROVIDER": "aws", "EXECUTION_OBJECT_STORE_TLS_GATEWAY": "true", "COMPILER_ALLOWED_STORAGE_HOSTS": "https://s3.example.test"})
        self.assertEqual(preflight.validate(values, config), [])
        for service in ["object-store", "object-store-init", "object-gateway"]:
            self.assertNotIn(service, config["services"])
        self.assertNotIn("object-store-init", config["services"]["api"].get("depends_on", {}))

    def test_external_store_and_datastores_compose_together(self):
        values, config = self.render({"OBJECT_STORE_MODE": "external", "DATASTORE_MODE": "external", "COMPOSE_FILE": "docker-compose.yml:docker-compose.external-store.yml:docker-compose.external-datastores.yml", "S3_INTERNAL_ENDPOINT": "https://s3.example.test", "S3_PUBLIC_ENDPOINT": "https://s3.example.test", "STS_ENDPOINT_URL": "https://sts.example.test", "S3_STS_PROVIDER": "aws", "EXECUTION_OBJECT_STORE_TLS_GATEWAY": "true", "COMPILER_ALLOWED_STORAGE_HOSTS": "https://s3.example.test", "METRICS_REDIS_URL": "redis://metrics@redis.example.test:6379", "DATABASE_URL": "postgresql://test@db.example.test/database", "REDIS_URL": "rediss://api@redis.example.test:6379", "RUNTIME_REDIS_URL": "rediss://runtime@redis.example.test:6379", "SIGNALING_REDIS_URL": "rediss://signaling@redis.example.test:6379", "SINK_REDIS_URL": "rediss://sink@redis.example.test:6379"})
        self.assertEqual(preflight.validate(values, config), [])
        for service in ["object-store", "object-store-init", "object-gateway", "postgres", "redis", "db-init"]:
            self.assertNotIn(service, config["services"])
        self.assertFalse(config["services"]["api"].get("depends_on"))

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
