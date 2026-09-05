"""Render and inspect security-sensitive chart combinations (requires helm and PyYAML)."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import urllib.parse
from unittest.mock import patch
import yaml

BASE = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("setup_config", BASE / "scripts/setup-config.py")
setup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(setup)

class UniqueLoader(yaml.SafeLoader):
    pass

def mapping(loader, node, deep=False):
    result = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if key in result:
            raise AssertionError(f"Duplicate YAML key: {key}")
        result[key] = loader.construct_object(value_node, deep=deep)
    return result

UniqueLoader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, mapping)

class ChartTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory(prefix="flow-like-chart-test-")
        with patch.dict(os.environ, {}, clear=True):
            cls.secrets, cls.values = setup.generate("flow-like", "flow-like")
        cls.values["executionManager"] = {"image": {"digest": "sha256:" + "a" * 64}, "sandbox": {"image": "example.invalid/executor@sha256:" + "b" * 64}}
        cls.values_path = Path(cls.tmp.name) / "values.json"
        cls.values_path.write_text(json.dumps(cls.values))
        cls.docs = cls.render()

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    @classmethod
    def render(cls, *args, valid=True):
        result = subprocess.run(["helm", "template", "flow-like", str(BASE / "helm"), "--namespace", "flow-like", "-f", str(cls.values_path), *args], capture_output=True, text=True)
        if not valid:
            if result.returncode == 0:
                raise AssertionError("unsafe configuration rendered successfully")
            return result.stderr
        if result.returncode:
            raise AssertionError(result.stderr)
        return [doc for doc in yaml.load_all(result.stdout, Loader=UniqueLoader) if doc]

    def resource(self, kind, suffix, docs=None):
        return next(x for x in docs or self.docs if x["kind"] == kind and x["metadata"]["name"] == "flow-like-" + suffix)

    def env(self, component, docs=None):
        pod = self.resource("Deployment", component, docs)["spec"]["template"]["spec"]
        result = {}
        for item in pod["containers"][0].get("env", []):
            self.assertNotIn(item["name"], result)
            result[item["name"]] = item
        return result

    def test_default_isolation_and_queue_consumer(self):
        api = self.env("api")
        queue = self.env("queue-bridge")
        self.assertEqual(api["EXECUTION_ISOLATION_MODE"]["value"], "per_run")
        self.assertEqual(api["EXECUTION_STATE_BACKEND"]["value"], "redis")
        self.assertEqual(api["REDIS_EXECUTION_QUEUE"]["value"], "exec:jobs:v3")
        self.assertEqual(queue["REDIS_EXECUTION_QUEUE"]["value"], api["REDIS_EXECUTION_QUEUE"]["value"])
        self.assertFalse(any(x["metadata"]["name"] == "flow-like-executor-pool" for x in self.docs))

    def test_hour_long_deadline_allowances_match_between_components(self):
        api = self.env("api")
        bridge = self.env("queue-bridge")
        manager = self.env("execution-manager")
        keys = ["EXECUTION_TIMEOUT_SECONDS", "EXECUTION_STARTUP_GRACE_SECONDS", "EXECUTION_TERMINAL_GRACE_SECONDS", "EXECUTION_CLEANUP_TIMEOUT_SECONDS"]
        for key in keys:
            self.assertEqual(api[key]["value"], bridge[key]["value"])
            self.assertEqual(api[key]["value"], manager[key]["value"])
        lifetime = sum(int(api[key]["value"]) for key in keys) + int(api["EXECUTION_QUEUE_MAX_WAIT_SECONDS"]["value"]) + int(api["EXECUTION_CREDENTIAL_MARGIN_SECONDS"]["value"])
        self.assertEqual(lifetime, 4140)
        self.assertGreater(int(api["STS_SESSION_TTL_SECONDS"]["value"]), lifetime)

    def test_manager_redis_claims_share_configured_endpoint(self):
        api = self.env("api")
        manager = self.env("execution-manager")
        self.assertEqual(api["REDIS_URL"], manager["REDIS_URL"])
        policy = self.resource("NetworkPolicy", "redis-access")
        components = policy["spec"]["ingress"][0]["from"][0]["podSelector"]["matchExpressions"][0]["values"]
        self.assertIn("execution-manager", components)

    def test_native_manager_worker_configuration(self):
        docs = self.render("--set", "executionManager.workerThreads=3")
        self.assertEqual(self.env("execution-manager", docs)["EXECUTION_MANAGER_WORKER_THREADS"]["value"], "3")
        for threads in (0, 65):
            error = self.render("--set", f"executionManager.workerThreads={threads}", valid=False)
            self.assertIn("executionManager.workerThreads must be between 1 and 64", error)

    def test_store_api_never_receives_root_credentials(self):
        api = self.resource("Deployment", "api")["spec"]["template"]["spec"]["containers"][0]
        names = [x["secretRef"]["name"] for x in api["envFrom"]]
        self.assertNotIn("flow-like-rustfs-root", names)
        env = self.env("api")
        self.assertEqual(env["RUNTIME_CREDENTIALS_PROVIDER"]["value"], "aws")
        self.assertEqual(env["S3_STS_PROVIDER"]["value"], "rustfs")
        self.assertNotEqual(env["STS_ENDPOINT_URL"]["value"], env["S3_PUBLIC_ENDPOINT"]["value"])
        self.assertEqual(env["STS_SESSION_TTL_SECONDS"]["value"], "7200")

    def test_only_migration_init_mounts_api_token(self):
        pod = self.resource("Deployment", "api")["spec"]["template"]["spec"]
        self.assertFalse(pod["automountServiceAccountToken"])
        self.assertNotIn("migration-api-access", [v["name"] for v in pod["containers"][0]["volumeMounts"]])
        self.assertEqual(len(pod["initContainers"]), 2)
        rules = self.resource("Role", "api")["rules"]
        self.assertEqual(rules[0]["verbs"], ["get"])
        self.assertEqual(set(rules[0]["resourceNames"]), {"flow-like-db-migrate-1", "flow-like-object-init-1"})

    def test_infrastructure_security_and_pvc_rollout(self):
        redis = self.resource("Deployment", "redis")
        self.assertEqual(redis["spec"]["strategy"]["type"], "Recreate")
        container = redis["spec"]["template"]["spec"]["containers"][0]
        self.assertTrue(container["securityContext"]["readOnlyRootFilesystem"])
        self.assertIn("maxmemory-policy noeviction", container["args"][0])
        self.assertNotIn("--requirepass", container["args"][0])
        for name in ["web", "queue-bridge", "object-gateway", "redis", "api"]:
            pod = self.resource("Deployment", name)["spec"]["template"]["spec"]
            self.assertFalse(pod["automountServiceAccountToken"])
        rustfs = self.resource("StatefulSet", "rustfs")
        self.assertEqual(rustfs["spec"]["replicas"], 1)

    def test_sandbox_never_receives_static_network_grants(self):
        def selects(selector, component):
            labels = {"app.kubernetes.io/name": "flow-like", "app.kubernetes.io/instance": "flow-like", "app.kubernetes.io/component": component}
            if any(labels.get(k) != v for k, v in selector.get("matchLabels", {}).items()):
                return False
            for expr in selector.get("matchExpressions", []):
                value = labels.get(expr["key"])
                if expr["operator"] == "In" and value not in expr["values"]:
                    return False
                if expr["operator"] == "NotIn" and value in expr["values"]:
                    return False
            return True
        policies = [d for d in self.docs if d["kind"] == "NetworkPolicy" and selects(d["spec"]["podSelector"], "execution-sandbox")]
        self.assertTrue(policies)
        for policy in policies:
            self.assertFalse(policy["spec"].get("egress"))
            self.assertFalse(policy["spec"].get("ingress"))

    def test_cilium_host_denial_is_mandatory_for_isolated_mode(self):
        policy = self.resource("CiliumNetworkPolicy", "sandbox-host-deny")
        self.assertEqual(policy["spec"]["endpointSelector"]["matchLabels"]["app.kubernetes.io/component"], "execution-sandbox")
        self.assertEqual(set(policy["spec"]["egressDeny"][0]["toEntities"]), {"host", "remote-node", "kube-apiserver"})
        self.assertIn("169.254.0.0/16", policy["spec"]["egressDeny"][1]["toCIDR"])
        probes = self.resource("CiliumNetworkPolicy", "node-health-probes")["spec"]["ingress"][0]
        ports = {port["port"] for port in probes["toPorts"][0]["ports"]}
        self.assertTrue({"8080", "9001"} <= ports)
        docs = self.render("--set", "execution.isolationMode=trusted_shared,execution.asyncBackend=http")
        self.assertFalse(any(x["kind"] == "CiliumNetworkPolicy" for x in docs))

    def test_unsafe_configurations_fail(self):
        for overrides in ["networkPolicy.enabled=false", "execution.asyncBackend=kubernetes_job", "redis.auth.enabled=false", "signaling.enabled=true,signaling.replicaCount=2", "compiler.enabled=true,compiler.maxConcurrentJobs=0", "storage.provider=azure", "execution.isolationMode=trusted_shared"]:
            with self.subTest(overrides=overrides):
                self.render("--set", overrides, valid=False)

    def test_external_tls_store_and_redis(self):
        docs = self.render("--set", "rustfs.enabled=false,redis.enabled=false,redis.externalExistingSecret=external-redis", "--set", "storage.s3.publicEndpoint=https://s3.example.com,storage.s3.internalEndpoint=https://s3.internal.example.com,storage.s3.stsEndpoint=https://sts.internal.example.com")
        env = self.env("api", docs)
        self.assertEqual(env["REDIS_URL"]["valueFrom"]["secretKeyRef"]["name"], "external-redis")
        self.assertEqual(env["S3_PUBLIC_ENDPOINT"]["value"], "https://s3.example.com")
        self.assertFalse(any(x["kind"] == "StatefulSet" and x["metadata"]["name"].endswith("rustfs") for x in docs))

    def test_trusted_mode_uses_direct_http_for_async(self):
        docs = self.render("--set", "execution.isolationMode=trusted_shared,execution.asyncBackend=http")
        self.assertTrue(self.resource("Deployment", "executor-pool", docs))
        self.assertEqual(self.env("api", docs)["ASYNC_EXECUTION_BACKEND"]["value"], "http")
        self.assertFalse(any(x["metadata"]["name"] == "flow-like-queue-bridge" for x in docs))

    def test_compiler_origin_preserves_http_and_port(self):
        docs = self.render("--set", "compiler.enabled=true")
        value = self.env("compiler", docs)["COMPILER_ALLOWED_STORAGE_HOSTS"]["value"]
        self.assertIn("http://flow-like-object-gateway.flow-like.svc.cluster.local:9000", value)

    def test_setup_encodes_redis_and_keeps_secrets_out_of_values(self):
        password = 'x:@/%?# $ complex'
        with patch.dict(os.environ, {"REDIS_PASSWORD": password}, clear=True):
            manifest, values = setup.generate("flow-like", "flow-like")
        redis = next(x["stringData"] for x in manifest["items"] if x["metadata"]["name"] == "flow-like-redis")
        self.assertEqual(urllib.parse.unquote(urllib.parse.urlsplit(redis["REDIS_URL"]).password), password)
        self.assertNotIn(password, json.dumps(values))
        self.assertNotIn("BACKEND_KEY", json.dumps(values))

    def test_enabled_service_volume_mounts_resolve_at_pod_level(self):
        docs = self.render("--set", "compiler.enabled=true,signaling.enabled=true")
        for name in ["api", "compiler", "signaling", "web", "queue-bridge", "object-gateway"]:
            pod = self.resource("Deployment", name, docs)["spec"]["template"]["spec"]
            volumes = {volume["name"] for volume in pod["volumes"]}
            for container in pod["containers"]:
                self.assertIn("image", container)
                self.assertNotIn("volumes", container)
                self.assertIn("resources", container)
                for mount in container.get("volumeMounts", []):
                    self.assertIn(mount["name"], volumes)

    def test_signaling_scales_with_redis_and_hpa_owns_replicas(self):
        docs = self.render("--set", "compiler.enabled=true,compiler.autoscaling.enabled=true,signaling.enabled=true,signaling.replicaCount=2,signaling.fanoutMode=redis")
        self.assertNotIn("replicas", self.resource("Deployment", "compiler", docs)["spec"])
        self.assertIn("REDIS_URL", self.env("signaling", docs))

if __name__ == "__main__":
    unittest.main()
