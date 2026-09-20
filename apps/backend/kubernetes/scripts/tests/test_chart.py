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

try:
    import yaml
except ImportError:
    yaml = None

BASE = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("setup_config", BASE / "scripts/setup-config.py")
setup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(setup)

def mapping(loader, node, deep=False):
    result = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if key in result:
            raise AssertionError(f"Duplicate YAML key: {key}")
        result[key] = loader.construct_object(value_node, deep=deep)
    return result

if yaml:
    class UniqueLoader(yaml.SafeLoader):
        pass

    UniqueLoader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, mapping)

@unittest.skipUnless(yaml, "PyYAML required (pip install PyYAML)")
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

    def test_stripe_api_secret_is_not_inherited_by_other_workloads(self):
        api_secret = {"secretRef": {"name": self.values["api"]["existingSecret"]}}
        api = self.resource("Deployment", "api")["spec"]["template"]["spec"]
        self.assertIn(api_secret, api["containers"][0]["envFrom"])
        for resource in self.docs:
            if resource["kind"] not in {"Deployment", "StatefulSet", "Job"} or resource["metadata"]["name"] == "flow-like-api":
                continue
            for container in resource["spec"]["template"]["spec"]["containers"]:
                self.assertNotIn(api_secret, container.get("envFrom", []))

    def test_namespace_reaches_api_without_sink_services(self):
        env = self.env("api", self.render("--set", "sinkServices.enabled=false"))
        self.assertEqual(env["K8S_NAMESPACE"]["valueFrom"]["fieldRef"]["fieldPath"], "metadata.namespace")
        self.assertEqual(env["KUBERNETES_NAMESPACE"]["valueFrom"], env["K8S_NAMESPACE"]["valueFrom"])
        self.assertNotIn("SINK_SCHEDULER_PROVIDER", env)

    def test_generated_hub_config_is_secret_mounted_read_only_at_runtime(self):
        env = self.env("api")
        self.assertEqual(env["FLOW_LIKE_CONFIG_FILE"]["value"], "/etc/flow-like/flow-like.config.json")
        self.assertNotIn("FLOW_LIKE_CONFIG_JSON", env)
        self.assertNotIn("FLOW_LIKE_CONFIG_SECRET_REF", env)
        pod = self.resource("Deployment", "api")["spec"]["template"]["spec"]
        mount = next(x for x in pod["containers"][0]["volumeMounts"] if x["name"] == "api-runtime-config")
        self.assertEqual(mount["mountPath"], "/etc/flow-like")
        self.assertTrue(mount["readOnly"])
        volume = next(x for x in pod["volumes"] if x["name"] == "api-runtime-config")
        self.assertEqual(volume["secret"]["secretName"], "flow-like-hub-config")
        self.assertEqual(volume["secret"]["defaultMode"], 0o440)
        self.assertEqual(volume["secret"]["items"], [{"key": "flow-like.config.json", "path": "flow-like.config.json"}])
        secret = next(x for x in self.secrets["items"] if x["metadata"]["name"] == "flow-like-hub-config")
        self.assertIsInstance(json.loads(secret["stringData"]["flow-like.config.json"]), dict)
        self.assertNotIn("stringData", json.dumps(self.values))

    def test_api_configmap_and_secret_key_env_sources(self):
        docs = self.render("--set-string", "api.runtimeConfig.existingSecret=,api.runtimeConfig.existingConfigMap=public-hub,api.runtimeConfig.key=hub.json")
        pod = self.resource("Deployment", "api", docs)["spec"]["template"]["spec"]
        volume = next(x for x in pod["volumes"] if x["name"] == "api-runtime-config")
        self.assertEqual(volume["configMap"]["name"], "public-hub")
        self.assertEqual(volume["configMap"]["items"][0]["key"], "hub.json")
        docs = self.render("--set-string", "api.runtimeConfig.existingSecret=,api.runtimeConfig.secretKeyRef.name=private-hub,api.runtimeConfig.secretKeyRef.key=json")
        env = self.env("api", docs)
        self.assertEqual(env["FLOW_LIKE_CONFIG_JSON"]["valueFrom"]["secretKeyRef"], {"name": "private-hub", "key": "json"})
        self.assertNotIn("FLOW_LIKE_CONFIG_FILE", env)
        pod = self.resource("Deployment", "api", docs)["spec"]["template"]["spec"]
        self.assertNotIn("api-runtime-config", [x["name"] for x in pod["volumes"]])

    def test_api_secret_store_reference_and_embedded_fallback(self):
        docs = self.render("--set-string", "api.runtimeConfig.existingSecret=,api.runtimeConfig.secretRef=hub_config")
        env = self.env("api", docs)
        self.assertEqual(env["FLOW_LIKE_CONFIG_SECRET_REF"]["value"], "hub_config")
        self.assertNotIn("FLOW_LIKE_CONFIG_FILE", env)
        docs = self.render("--set-string", "api.runtimeConfig.existingSecret=")
        self.assertFalse(any(key.startswith("FLOW_LIKE_CONFIG_") for key in self.env("api", docs)))

    def test_api_conflicting_sources_and_duplicate_env_fail_render(self):
        for settings in ("api.runtimeConfig.existingConfigMap=hub", "api.runtimeConfig.secretKeyRef.name=hub", "api.runtimeConfig.secretRef=hub"):
            error = self.render("--set-string", settings, valid=False)
            self.assertIn("api.runtimeConfig must select at most one", error)
        error = self.render("--set-string", "api.env[0].name=FLOW_LIKE_CONFIG_JSON,api.env[0].value=example", valid=False)
        self.assertIn("Use api.runtimeConfig", error)

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

    def test_web_runtime_config_is_public_and_keeps_read_only_filesystem(self):
        env = self.env("web")
        self.assertEqual(set(env), {"FLOW_LIKE_WEB_API_URL", "FLOW_LIKE_WEB_REDIRECT_URL", "FLOW_LIKE_WEB_LOGOUT_URL"})
        self.assertEqual(env["FLOW_LIKE_WEB_API_URL"]["value"], self.values["api"]["publicUrl"])
        self.assertEqual(env["FLOW_LIKE_WEB_REDIRECT_URL"]["value"], "")
        self.assertEqual(env["FLOW_LIKE_WEB_LOGOUT_URL"]["value"], "")
        pod = self.resource("Deployment", "web")["spec"]["template"]["spec"]
        container = pod["containers"][0]
        self.assertNotIn("envFrom", container)
        self.assertTrue(container["securityContext"]["readOnlyRootFilesystem"])
        self.assertEqual(container["securityContext"]["runAsUser"], 1000)
        self.assertIn("/tmp", [mount["mountPath"] for mount in container["volumeMounts"]])

    def test_web_runtime_config_supports_explicit_domain_overrides(self):
        docs = self.render("--set-string", "web.runtimeConfig.apiUrl=https://other-api.example.test,web.runtimeConfig.redirectUrl=https://web.example.test/callback,web.runtimeConfig.logoutUrl=https://web.example.test/")
        env = self.env("web", docs)
        self.assertEqual(env["FLOW_LIKE_WEB_API_URL"]["value"], "https://other-api.example.test")
        self.assertEqual(env["FLOW_LIKE_WEB_REDIRECT_URL"]["value"], "https://web.example.test/callback")
        self.assertEqual(env["FLOW_LIKE_WEB_LOGOUT_URL"]["value"], "https://web.example.test/")

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

    def test_published_tags_are_pulled_always_and_digests_pin_every_first_party_image(self):
        for name in ["api", "web", "queue-bridge"]:
            container = self.resource("Deployment", name)["spec"]["template"]["spec"]["containers"][0]
            self.assertTrue(container["image"].startswith("ghcr.io/rheosoph/flow-like-"), container["image"])
            self.assertTrue(container["image"].endswith(":dev"))
            self.assertEqual(container["imagePullPolicy"], "Always")
        api = self.resource("Deployment", "api")["spec"]["template"]["spec"]
        self.assertEqual(api["containers"][0]["image"], "ghcr.io/rheosoph/flow-like-kubernetes-api:dev")
        self.assertEqual({c["image"] for c in api["initContainers"]}, {"ghcr.io/rheosoph/flow-like-kubernetes-migration:dev"})
        env = self.env("api")
        self.assertEqual(env["K8S_EXECUTOR_IMAGE"]["value"], "ghcr.io/rheosoph/flow-like-kubernetes-executor:dev")
        self.assertEqual(env["K8S_IMAGE_PULL_SECRETS"]["value"], "")
        digest = "sha256:" + "d" * 64
        pins = ",".join(f"{key}.image.digest={digest}" for key in ["api", "web", "executor", "database.migration", "executionManager", "executionManager.queueBridge", "rustfs.bootstrap", "compiler", "signaling", "executorPool", "sinkServices"])
        docs = self.render("--set-string", pins + ",global.imageRegistry=mirror.example.com/", "--set", "compiler.enabled=true,signaling.enabled=true")
        images = {c["image"] for d in docs if d["kind"] in ("Deployment", "Job") for c in d["spec"]["template"]["spec"].get("containers", []) + d["spec"]["template"]["spec"].get("initContainers", [])}
        first_party = {i for i in images if "flow-like-" in i}
        self.assertEqual(len(first_party), 8, first_party)
        for image in first_party:
            self.assertTrue(image.startswith("mirror.example.com/ghcr.io/rheosoph/flow-like-"), image)
            self.assertTrue(image.endswith("@" + digest), image)
        self.assertEqual(self.env("api", docs)["K8S_EXECUTOR_IMAGE"]["value"], "mirror.example.com/ghcr.io/rheosoph/flow-like-kubernetes-executor@" + digest)
        for bad in ("sha256:short", "md5:" + "d" * 64, "D" * 71):
            error = self.render("--set-string", f"api.image.digest={bad}", valid=False)
            self.assertIn("sha256:<64 hex characters>", error)

    def test_pull_secrets_reach_every_pod_and_the_pod_creating_controllers(self):
        docs = self.render("--set", "global.imagePullSecrets[0].name=ghcr-pull,global.imagePullSecrets[1].name=mirror-pull,sinkServices.enabled=true")
        for doc in docs:
            if doc["kind"] in ("Deployment", "Job", "StatefulSet"):
                pod = doc["spec"]["template"]["spec"]
                images = [c["image"] for c in pod.get("containers", []) + pod.get("initContainers", [])]
                if any("flow-like-" in image for image in images):
                    self.assertEqual(pod.get("imagePullSecrets"), [{"name": "ghcr-pull"}, {"name": "mirror-pull"}], doc["metadata"]["name"])
        env = self.env("api", docs)
        self.assertEqual(env["K8S_IMAGE_PULL_SECRETS"]["value"], "ghcr-pull,mirror-pull")
        self.assertEqual(env["SINK_IMAGE_PULL_SECRETS"]["value"], "ghcr-pull,mirror-pull")
        self.assertEqual(env["SINK_TRIGGER_IMAGE"]["value"], "ghcr.io/rheosoph/flow-like-kubernetes-sink-trigger:dev")
        self.assertEqual(env["SINK_SCHEDULER_PROVIDER"]["value"], "kubernetes")
        self.assertEqual(env["K8S_CONFIGMAP_NAME"]["value"], "flow-like-sink-config")
        self.assertEqual(env["K8S_SECRET_NAME"]["value"], "flow-like-sink-secrets")
        self.assertEqual(self.env("execution-manager", docs)["SANDBOX_IMAGE_PULL_SECRETS"]["value"], json.dumps([{"name": "ghcr-pull"}, {"name": "mirror-pull"}], separators=(",", ":")))
        self.assertNotIn("SINK_TRIGGER_IMAGE", self.env("api"))

    def test_production_example_mirrors_published_names_by_digest(self):
        docs = self.render("-f", str(BASE / "helm/values-production.yaml"))
        images = {c["image"] for d in docs if d["kind"] in ("Deployment", "Job") for c in d["spec"]["template"]["spec"].get("containers", []) + d["spec"]["template"]["spec"].get("initContainers", [])}
        first_party = sorted(i for i in images if "flow-like-" in i)
        self.assertTrue(first_party)
        for image in first_party:
            self.assertRegex(image, r"^registry\.example\.com/flow-like-(kubernetes|docker-compose)-[a-z-]+@sha256:[0-9a-f]{64}$")
        pod = self.resource("Deployment", "api", docs)["spec"]["template"]["spec"]
        self.assertEqual(pod["imagePullSecrets"], [{"name": "registry-credentials"}])
        self.assertEqual(self.env("execution-manager", docs)["SANDBOX_IMAGE"]["value"], self.env("api", docs)["K8S_EXECUTOR_IMAGE"]["value"])

    def test_resolved_image_values_render_pinned_per_run_deployment(self):
        spec = importlib.util.spec_from_file_location("resolve_images", BASE / "scripts/resolve-images.py")
        resolver = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(resolver)
        resolved = {component: (f"ghcr.io/rheosoph/{name}", "sha256:" + format(index, "x").rjust(64, "0")) for index, (component, name) in enumerate(resolver.REPOSITORIES.items(), start=1)}
        with tempfile.NamedTemporaryFile("w", suffix=".yaml", dir=self.tmp.name, delete=False) as handle:
            json.dump(resolver.image_values(resolved, "1.2.3", pull_secrets=["ghcr-pull"], arch="arm64"), handle)
        docs = self.render("-f", handle.name)
        manager = self.resource("Deployment", "execution-manager", docs)["spec"]["template"]["spec"]
        self.assertEqual(manager["containers"][0]["image"], "ghcr.io/rheosoph/flow-like-kubernetes-execution-manager@" + resolved["execution-manager"][1])
        self.assertEqual(manager["containers"][0]["imagePullPolicy"], "IfNotPresent")
        self.assertEqual(manager["nodeSelector"], {"kubernetes.io/arch": "arm64"})
        env = self.env("execution-manager", docs)
        self.assertEqual(env["SANDBOX_IMAGE"]["value"], "ghcr.io/rheosoph/flow-like-kubernetes-executor@" + resolved["executor"][1])
        self.assertEqual(json.loads(env["SANDBOX_NODE_SELECTOR"]["value"]), {"kubernetes.io/arch": "arm64"})
        self.assertEqual(self.env("api", docs)["K8S_EXECUTOR_IMAGE"]["value"], env["SANDBOX_IMAGE"]["value"])
        self.assertEqual(self.env("api", docs)["K8S_IMAGE_PULL_SECRETS"]["value"], "ghcr-pull")
        self.assertEqual(self.resource("Deployment", "queue-bridge", docs)["spec"]["template"]["spec"]["containers"][0]["image"], "ghcr.io/rheosoph/flow-like-docker-compose-runtime@" + resolved["runtime"][1])

    def test_signaling_scales_with_redis_and_hpa_owns_replicas(self):
        docs = self.render("--set", "compiler.enabled=true,compiler.autoscaling.enabled=true,signaling.enabled=true,signaling.replicaCount=2,signaling.fanoutMode=redis")
        self.assertNotIn("replicas", self.resource("Deployment", "compiler", docs)["spec"])
        self.assertIn("REDIS_URL", self.env("signaling", docs))

if __name__ == "__main__":
    unittest.main()
