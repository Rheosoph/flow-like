"""Check target coverage and reject incomplete or mixed GHCR releases."""

import contextlib
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "container_images.py"
SPEC = importlib.util.spec_from_file_location("container_images", SCRIPT)
containers = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(containers)

OWNER = "rheosoph"
SOURCE_SHA = "a" * 40
DIGEST = "sha256:" + "b" * 64
RUN_ID = "12345"
RUN_ATTEMPT = "1"


def make_records(cloud="all"):
    return [containers.record(entry["id"], OWNER, SOURCE_SHA, DIGEST, RUN_ID, RUN_ATTEMPT)
            for entry in containers.matrix(cloud)["include"]]


def merge(records, cloud="all"):
    return containers.manifest(cloud, OWNER, SOURCE_SHA, RUN_ID, RUN_ATTEMPT, records)


class MatrixTests(unittest.TestCase):
    def test_matrix_covers_portable_targets_and_real_recipes(self):
        entries = containers.matrix("all")["include"]
        self.assertEqual(len(entries), 63)
        self.assertEqual(len({entry["id"] for entry in entries}), 63)
        self.assertEqual([len(containers.matrix(cloud)["include"]) for cloud in ("aws", "gcp", "azure")], [14, 8, 9])
        self.assertIn("gcp-api", {entry["id"] for entry in entries})
        self.assertIn("azure-api", {entry["id"] for entry in entries})
        self.assertEqual({entry["cloud"] for entry in entries if entry["workload"] == "web"}, {"docker-compose", "kubernetes"})
        root = SCRIPT.parents[2]
        for entry in entries:
            with self.subTest(target=entry["id"]):
                self.assertTrue((root / entry["dockerfile"]).is_file())
                self.assertTrue((root / entry["context"]).is_dir())
                self.assertEqual(entry["context"], ".")
                self.assertEqual(entry["runner"].endswith("-arm"), entry["platform"] == "linux/arm64")

    def test_layer_cache_tracks_whether_the_expensive_step_is_restorable(self):
        entries = {entry["id"]: entry for entry in containers.matrix("all")["include"]}
        # A whole-context COPY above `cargo build` makes every later layer a
        # guaranteed miss, so those targets must not export a layer cache.
        self.assertFalse(entries["gcp-api"]["layer_cache"])
        self.assertFalse(entries["docker-compose-runtime-amd64"]["layer_cache"])
        self.assertFalse(entries["docker-compose-web-amd64"]["layer_cache"])
        # These copy manifests first, or have no expensive step at all.
        self.assertTrue(entries["docker-compose-signaling-amd64"]["layer_cache"])
        self.assertTrue(entries["gcp-migration"]["layer_cache"])
        self.assertTrue(entries["azure-otel-collector"]["layer_cache"])
        self.assertEqual(sum(entry["layer_cache"] for entry in entries.values()), 16)

    def test_layer_cache_detection_reads_recipe_order(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "broken").mkdir()
            (root / "broken" / "Dockerfile").write_text("FROM rust\nCOPY . .\nRUN cargo build --release\n")
            (root / "good").mkdir()
            (root / "good" / "Dockerfile").write_text(
                "FROM rust\nCOPY Cargo.toml Cargo.lock ./\nRUN cargo build --release\nCOPY . .\n")
            (root / "plain").mkdir()
            (root / "plain" / "Dockerfile").write_text("FROM alpine\nCOPY . .\n")
            original = containers.REPOSITORY_ROOT
            containers.REPOSITORY_ROOT = root
            try:
                self.assertFalse(containers.layer_cache_enabled("broken/Dockerfile"))
                self.assertTrue(containers.layer_cache_enabled("good/Dockerfile"))
                self.assertTrue(containers.layer_cache_enabled("plain/Dockerfile"))
            finally:
                containers.REPOSITORY_ROOT = original

    def test_nonstandard_recipes_and_context_are_preserved(self):
        entries = {entry["id"]: entry for entry in containers.matrix("all")["include"]}
        self.assertEqual(entries["aws-executor-async"]["dockerfile"], "apps/backend/aws/executor-ecs/Dockerfile")
        self.assertEqual(entries["aws-executor-lambda-async"]["dockerfile"], "apps/backend/aws/executor-async/Dockerfile")
        self.assertEqual(entries["aws-executor-lambda-async"]["platform"], "linux/arm64")
        self.assertEqual(entries["aws-compiler"]["dockerfile"], "apps/backend/aws/compiler-ecs/Dockerfile")
        self.assertEqual(entries["aws-signaling"]["dockerfile"], "apps/backend/docker-compose/signaling/Dockerfile")
        self.assertEqual(entries["azure-otel-collector"]["context"], ".")
        self.assertEqual(entries["aws-executor"]["platform"], "linux/arm64")
        self.assertEqual(entries["aws-api"]["platform"], "linux/arm64")

    def test_ecs_api_publishes_one_multi_architecture_repository(self):
        entries = [entry for entry in containers.matrix("aws")["include"] if entry["workload"] == "api-ecs"]
        self.assertEqual({entry["id"]: entry["platform"] for entry in entries},
                         {"aws-api-ecs-amd64": "linux/amd64", "aws-api-ecs-arm64": "linux/arm64"})
        self.assertEqual({entry["image_suffix"] for entry in entries}, {"flow-like-aws-api-ecs"})
        self.assertEqual({entry["dockerfile"] for entry in entries}, {"apps/backend/aws/api-ecs/Dockerfile"})

    def test_dedicated_audit_workers_bind_their_cloud_features(self):
        entries = {entry["id"]: entry for entry in containers.matrix("all")["include"]}
        expected = {"docker-compose-audit-worker-amd64": ("flow-like-audit-worker", "aws,azure,gcp"),
                    "docker-compose-audit-worker-arm64": ("flow-like-audit-worker", "aws,azure,gcp"),
                    "azure-audit-worker": ("flow-like-azure-audit-worker", "azure"),
                    "gcp-audit-worker": ("flow-like-gcp-audit-worker", "gcp")}
        for target_id, (image, feature) in expected.items():
            with self.subTest(target=target_id):
                self.assertEqual(entries[target_id]["image_suffix"], image)
                self.assertEqual(entries[target_id]["audit_features"], feature)
                cloud = entries[target_id]["cloud"]
                recipe = f"apps/backend/{cloud}/audit-worker/Dockerfile" if cloud in ("azure", "gcp") else "apps/backend/audit-worker/Dockerfile"
                self.assertEqual(entries[target_id]["dockerfile"], recipe)
                self.assertEqual(entries[target_id]["context"], ".")
                inputs = containers.record(target_id, OWNER, SOURCE_SHA, DIGEST, RUN_ID, RUN_ATTEMPT)["build_inputs"]
                self.assertEqual(inputs["audit_features"], feature)
                self.assertEqual(inputs["runtime_config"], "compiled-audit-section-v1")
                config = "flow-like.config.json" if cloud in ("azure", "gcp") else "apps/backend/docker-compose/flow-like.config.example.json"
                self.assertEqual(inputs["flow_like_config_sha256"],
                                 hashlib.sha256((containers.REPOSITORY_ROOT / config).read_bytes()).hexdigest())
                recipe_text = (containers.REPOSITORY_ROOT / recipe).read_text()
                if cloud in ("azure", "gcp"):
                    self.assertIn("--mount=type=secret,id=flow_like_config,", recipe_text)
                else:
                    self.assertIn(f"ARG FLOW_LIKE_CONFIG={config}\n", recipe_text)
                if cloud in ("azure", "gcp"):
                    launcher = containers.REPOSITORY_ROOT / f"apps/backend/{cloud}/audit-worker/entrypoint.py"
                    self.assertEqual(inputs["entrypoint_sha256"], hashlib.sha256(launcher.read_bytes()).hexdigest())
                    self.assertEqual(inputs["database_auth"], "entra-managed-identity" if cloud == "azure" else "cloud-sql-iam")
                    self.assertFalse((containers.REPOSITORY_ROOT / (recipe + ".dockerignore")).exists())
                else:
                    self.assertNotIn("database_auth", inputs)

    def test_self_hosted_examples_share_the_generic_worker_audit_policy(self):
        policies = [json.loads((containers.REPOSITORY_ROOT / f"apps/backend/{cloud}/flow-like.config.example.json").read_text()).get("audit")
                    for cloud in ("docker-compose", "kubernetes")]
        self.assertEqual(policies[0], policies[1])

    def test_self_hosted_selectors_cover_both_native_architectures(self):
        self.assertEqual([len(containers.matrix(cloud)["include"]) for cloud in ("self-hosted", "docker-compose", "kubernetes")], [32, 20, 22])
        workloads = {}
        for entry in containers.matrix("self-hosted")["include"]:
            architecture = entry["platform"].split("/")[1]
            self.assertEqual(entry["id"], f"{entry['cloud']}-{entry['workload']}-{architecture}")
            expected = "flow-like-audit-worker" if entry["workload"] == "audit-worker" else f"flow-like-{entry['cloud']}-{entry['workload']}"
            self.assertEqual(entry["image_suffix"], expected)
            workloads.setdefault(entry["image_suffix"], set()).add(entry["platform"])
        self.assertEqual(len(workloads), 16)
        self.assertTrue(all(platforms == {"linux/amd64", "linux/arm64"} for platforms in workloads.values()))

    def test_kubernetes_reuses_exact_compose_dependency_targets(self):
        compose = {entry["id"]: entry for entry in containers.matrix("docker-compose")["include"]}
        kubernetes = containers.matrix("kubernetes")["include"]
        shared = {entry["id"]: entry for entry in kubernetes if entry["cloud"] == "docker-compose"}
        expected_ids = {f"docker-compose-{workload}-{architecture}"
                        for workload in ("runtime", "compiler", "signaling", "object-store-init", "audit-worker")
                        for architecture in ("amd64", "arm64")}
        self.assertEqual(set(shared), expected_ids)
        self.assertEqual(shared, {target_id: compose[target_id] for target_id in expected_ids})
        self.assertEqual(shared["docker-compose-object-store-init-amd64"]["dockerfile"], "apps/backend/docker-compose/object-store/Dockerfile")
        self.assertEqual(len({entry["id"] for entry in kubernetes}), len(kubernetes))


class ManifestTests(unittest.TestCase):
    def test_complete_release_is_sorted_and_references_digests(self):
        records = make_records()
        manifest = merge(list(reversed(records)))
        self.assertEqual(manifest["images"], records)
        self.assertEqual(manifest["source_sha"], SOURCE_SHA)
        self.assertTrue(all(entry["image"] == f"{entry['repository']}@{DIGEST}" for entry in manifest["images"]))

    def test_selected_cloud_is_complete(self):
        for cloud in ("aws", "gcp", "azure", "docker-compose"):
            with self.subTest(cloud=cloud):
                result = merge(make_records(cloud), cloud)
                self.assertEqual({entry["cloud"] for entry in result["images"]}, {cloud})

    def test_kubernetes_manifest_requires_shared_dependencies(self):
        records = make_records("kubernetes")
        self.assertEqual(len(merge(records, "kubernetes")["images"]), 22)
        own_records = [entry for entry in records if entry["cloud"] == "kubernetes"]
        with self.assertRaisesRegex(ValueError, "missing image records"):
            merge(own_records, "kubernetes")

    def test_architectures_share_a_repository_but_not_records_or_tags(self):
        records = make_records("self-hosted")
        self.assertEqual(len(merge(records, "self-hosted")["images"]), 32)
        entries = {entry["id"]: entry for entry in records}
        amd64 = entries["docker-compose-api-amd64"]
        arm64 = entries["docker-compose-api-arm64"]
        self.assertEqual(amd64["repository"], arm64["repository"])
        self.assertNotEqual(amd64["tag"], arm64["tag"])
        self.assertEqual(amd64["tag"], f"sha-{SOURCE_SHA}-amd64-run-{RUN_ID}-{RUN_ATTEMPT}")
        self.assertEqual(arm64["tag"], f"sha-{SOURCE_SHA}-arm64-run-{RUN_ID}-{RUN_ATTEMPT}")
        arm64["platform"] = amd64["platform"]
        with self.assertRaisesRegex(ValueError, "does not match the release: platform"):
            merge(records, "self-hosted")

    def test_missing_duplicate_and_unexpected_targets_fail(self):
        records = make_records()
        for variant, message in (
            (records[:-1], "missing image records"),
            ([], "missing image records"),
            (records + [records[0]], "duplicate image record"),
            ([dict(records[0], id="unknown-api")] + records[1:], "unexpected target"),
        ):
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                merge(variant)
        with self.assertRaisesRegex(ValueError, "unexpected target"):
            merge(records, "aws")

    def test_metadata_mismatches_and_unknown_fields_fail(self):
        for field, replacement in (
            ("source_sha", "c" * 40),
            ("build", {"run_id": "98765", "run_attempt": RUN_ATTEMPT}),
            ("build", {"run_id": RUN_ID, "run_attempt": "2"}),
            ("build_inputs", {"dockerfile_sha256": "0" * 64}),
            ("platform", "linux/s390x"),
            ("repository", "ghcr.io/another-owner/flow-like-aws-api"),
            ("image", "ghcr.io/rheosoph/flow-like-aws-api:latest"),
            ("dockerfile", "unexpected/Dockerfile"),
            ("context", ".."),
            ("schema_version", 2),
            ("schema_version", True),
            ("schema_version", 1.0),
            ("tag", "latest"),
            ("extra", "unexpected content"),
        ):
            records = copy.deepcopy(make_records())
            records[0][field] = replacement
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "does not match the release"):
                merge(records)

    def test_invalid_digest_or_record_shape_fails(self):
        for digest in ("latest", "sha256:abc", "sha256:" + "B" * 64, None):
            records = make_records()
            records[0]["digest"] = digest
            with self.subTest(digest=digest), self.assertRaisesRegex(ValueError, "SHA-256"):
                merge(records)
        for entry in ([], None, "image", {"id": []}):
            with self.subTest(entry=entry), self.assertRaisesRegex(ValueError, "object with a target ID"):
                merge([entry])

    def test_record_identity_is_normalized_and_validated(self):
        result = containers.record("aws-api", "Rheosoph", SOURCE_SHA, DIGEST, RUN_ID, RUN_ATTEMPT)
        self.assertEqual(result["repository"], "ghcr.io/rheosoph/flow-like-aws-api")
        self.assertEqual(result["tag"], f"sha-{SOURCE_SHA}-arm64-run-{RUN_ID}-{RUN_ATTEMPT}")
        for owner, sha, run_id, attempt in (
            ("rheosoph/other", SOURCE_SHA, RUN_ID, RUN_ATTEMPT),
            (OWNER, "dev", RUN_ID, RUN_ATTEMPT),
            (OWNER, "0" * 40, RUN_ID, RUN_ATTEMPT),
            (OWNER, SOURCE_SHA, "0", RUN_ATTEMPT),
            (OWNER, SOURCE_SHA, RUN_ID, "-1"),
        ):
            with self.subTest(owner=owner, sha=sha, run_id=run_id, attempt=attempt), self.assertRaises(ValueError):
                containers.record("aws-api", owner, sha, DIGEST, run_id, attempt)

    def test_build_inputs_bind_recipe_and_default_aws_configuration(self):
        entries = {entry["id"]: entry for entry in make_records()}
        for entry in entries.values():
            recipe = containers.REPOSITORY_ROOT / entry["dockerfile"]
            self.assertEqual(entry["build_inputs"]["dockerfile_sha256"], hashlib.sha256(recipe.read_bytes()).hexdigest())
        config = containers.REPOSITORY_ROOT / "flow-like.config.json"
        for target_id in ("aws-api", "aws-api-ecs-amd64", "aws-api-ecs-arm64", "gcp-api", "azure-api"):
            inputs = entries[target_id]["build_inputs"]
            self.assertEqual(inputs["flow_like_config_sha256"], hashlib.sha256(config.read_bytes()).hexdigest())
            self.assertEqual(inputs["runtime_config"], "full-document-v1")
            self.assertEqual(inputs["variant"], "runtime-config-public-default")
        for target_id in ("aws-api", "aws-api-ecs-amd64", "aws-api-ecs-arm64", "aws-file-tracker"):
            self.assertEqual(entries[target_id]["build_inputs"]["database_tls"], "dsql-no-custom-ca")
        self.assertNotIn("database_tls", entries["aws-executor"]["build_inputs"])
        self.assertNotIn("database_tls", entries["gcp-executor"]["build_inputs"])
        self.assertNotIn("flow_like_config_sha256", entries["aws-file-tracker"]["build_inputs"])

    def test_self_hosted_api_records_bind_each_example_configuration(self):
        entries = {entry["id"]: entry for entry in make_records("self-hosted")}
        for cloud in ("docker-compose", "kubernetes"):
            config = containers.REPOSITORY_ROOT / f"apps/backend/{cloud}/flow-like.config.example.json"
            for architecture in ("amd64", "arm64"):
                build_inputs = entries[f"{cloud}-api-{architecture}"]["build_inputs"]
                self.assertEqual(build_inputs["flow_like_config_sha256"], hashlib.sha256(config.read_bytes()).hexdigest())
                self.assertEqual(build_inputs["variant"], "runtime-config-self-hosted-default")
                self.assertEqual(build_inputs["runtime_config"], "full-document-v1")
                self.assertNotIn("database_tls", build_inputs)
        self.assertNotIn("variant", entries["docker-compose-runtime-amd64"]["build_inputs"])

    def test_self_hosted_web_records_identify_runtime_configuration(self):
        entries = {entry["id"]: entry for entry in make_records("self-hosted")}
        for cloud in ("docker-compose", "kubernetes"):
            for architecture in ("amd64", "arm64"):
                build_inputs = entries[f"{cloud}-web-{architecture}"]["build_inputs"]
                self.assertEqual(build_inputs["variant"], "runtime-public-config")
                self.assertNotIn("flow_like_config_sha256", build_inputs)


class CLITests(unittest.TestCase):
    def test_record_and_manifest_files_round_trip(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            common = ["--owner", OWNER, "--source-sha", SOURCE_SHA, "--run-id", RUN_ID, "--run-attempt", RUN_ATTEMPT]
            for entry in containers.matrix("gcp")["include"]:
                output = root / "records" / entry["id"] / "image.json"
                self.assertEqual(containers.main(["record", "--target", entry["id"], "--digest", DIGEST, "--output", str(output), *common]), 0)
            output = root / "release.json"
            self.assertEqual(containers.main(["manifest", "--cloud", "gcp", "--records-dir", str(root / "records"), "--output", str(output), *common]), 0)
            self.assertEqual(json.loads(output.read_text()), merge(make_records("gcp"), "gcp"))

    def test_invalid_json_does_not_echo_artifact_contents(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "image.json"
            path.write_text("sensitive-invalid-content")
            with self.assertRaisesRegex(ValueError, "cannot read image record image.json") as error:
                containers.read_records(path.parent)
            self.assertNotIn("sensitive-invalid-content", str(error.exception))

    def test_matrix_cli_emits_one_json_line(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.assertEqual(containers.main(["matrix", "--cloud", "aws"]), 0)
        self.assertEqual(len(output.getvalue().splitlines()), 1)
        self.assertEqual(json.loads(output.getvalue()), containers.matrix("aws"))




class FakeRegistryRun:
    """Answer docker buildx imagetools and git calls from an in-memory registry."""

    def __init__(self, existing=None, ancestors=(), unknown=(), versions=None, fail_create=False, docker_media_types=False):
        self.existing = dict(existing or {})
        self.ancestors = set(ancestors)
        self.unknown = set(unknown)
        self.versions = dict(versions or {})
        self.fail_create = fail_create
        self.docker_media_types = docker_media_types
        self.commands = []
        self.references = {}

    def __call__(self, command):
        self.commands.append(list(command))
        return subprocess.CompletedProcess(command, *self.answer(command))

    def answer(self, command):
        if command[:2] == ["git", "merge-base"]:
            revision = command[3]
            if revision in self.unknown:
                return 128, "", "fatal: Not a valid commit name"
            return (0 if revision in self.ancestors else 1), "", ""
        if command[:2] == ["git", "tag"]:
            return 0, "\n".join(self.versions.get(command[3], [])) + "\n", ""
        assert command[:3] == ["docker", "buildx", "imagetools"], command
        if command[3] == "create":
            return self.create(command[4:])
        return self.inspect(command[4:])

    def create(self, arguments):
        if self.fail_create:
            return 1, "", "ERROR: push denied\n"
        tags, sources, annotations, prefer_index = [], [], {}, True
        iterator = iter(arguments)
        for argument in iterator:
            if argument == "--tag":
                tags.append(next(iterator))
            elif argument == "--annotation":
                key, _, value = next(iterator).partition("=")
                annotations[key] = value
            elif argument == "--prefer-index=false":
                prefer_index = False
            else:
                sources.append(argument)
        resolved = [self.resolve(source) for source in sources]
        if len(resolved) == 1 and not annotations and (not prefer_index or "manifests" in resolved[0]["manifest"]):
            image = resolved[0]
        else:
            manifests = [entry for source in resolved for entry in (source["manifest"].get("manifests") or [source["manifest"]])]
            digest = "sha256:" + hashlib.sha256(json.dumps([entry["digest"] for entry in manifests] + sorted(annotations.items())).encode()).hexdigest()
            image = {"manifest": {"schemaVersion": 2, "digest": digest, "manifests": manifests, "annotations": {k.partition(":")[2]: v for k, v in annotations.items()}},
                     "image": {entry["platform"]: source["image"] for source, entry in zip(resolved, manifests)}}
            if self.docker_media_types:
                # buildx builds a Docker manifest list from schema2 sources and drops every index annotation.
                image["manifest"].pop("annotations")
        for tag in tags:
            self.references[tag] = image
            repository = tag.rpartition(":")[0]
            self.references[f"{repository}@{image['manifest']['digest']}"] = image
        return 0, "", ""

    def resolve(self, reference):
        if reference in self.references:
            return self.references[reference]
        repository, _, digest = reference.partition("@")
        architecture = "amd64" if digest.startswith("sha256:a") else "arm64"
        manifest = {"schemaVersion": 2, "digest": digest, "platform": f"linux/{architecture}"}
        return {"manifest": manifest, "image": {"architecture": architecture, "config": {"Labels": {containers.REVISION_ANNOTATION: SOURCE_SHA}}}}

    def inspect(self, arguments):
        reference = arguments[0]
        image = self.references.get(reference) or self.existing.get(reference)
        if image is None:
            return 1, "", f"ERROR: {reference}: not found\n"
        if arguments[1:] == ["--raw"]:
            raw = dict(image["manifest"])
            raw.pop("digest")
            return 0, json.dumps(raw), ""
        if arguments[1:] == ["--format", "{{json .Manifest}}"]:
            return 0, json.dumps(image["manifest"]), ""
        return 0, json.dumps({"name": reference, **image}), ""


def existing_index(revision=SOURCE_SHA, version=None, annotate=True):
    annotations = {containers.REVISION_ANNOTATION: revision}
    if version:
        annotations[containers.VERSION_ANNOTATION] = version
    manifest = {"schemaVersion": 2, "digest": "sha256:" + "e" * 64, "manifests": [{"digest": "sha256:" + "f" * 64, "platform": "linux/amd64"}]}
    labels = {containers.REVISION_ANNOTATION: revision}
    if annotate:
        manifest["annotations"] = annotations
    return {"manifest": manifest, "image": {"linux/amd64": {"config": {"Labels": labels}}}}


def existing_single_platform_index(revision=SOURCE_SHA):
    """buildx inspects a one-platform index with a single image config instead of a platform map."""
    index = existing_index(revision, annotate=False)
    return {"manifest": index["manifest"], "image": {"architecture": "amd64", "config": {"Labels": {containers.REVISION_ANNOTATION: revision}}}}


def existing_manifest(revision=SOURCE_SHA):
    return {"manifest": {"schemaVersion": 2, "digest": "sha256:" + "e" * 64},
            "image": {"architecture": "amd64", "config": {"Labels": {containers.REVISION_ANNOTATION: revision}}}}


def release_manifest(cloud="self-hosted", digests=None):
    records = []
    for entry in containers.matrix(cloud)["include"]:
        architecture = entry["platform"].split("/")[1]
        digest = (digests or {}).get(entry["id"]) or ("sha256:" + ("a" if architecture == "amd64" else "b") * 32 + hashlib.sha256(entry["id"].encode()).hexdigest()[:32])
        records.append(containers.record(entry["id"], OWNER, SOURCE_SHA, digest, RUN_ID, RUN_ATTEMPT))
    return containers.manifest(cloud, OWNER, SOURCE_SHA, RUN_ID, RUN_ATTEMPT, records)


class ChannelTagTests(unittest.TestCase):
    def test_refs_map_to_channel_tags_and_versions(self):
        for ref, tags, version in (
            ("refs/heads/dev", ["dev"], "dev"),
            ("refs/heads/main", ["main"], "main"),
            ("refs/heads/alpha", ["alpha"], "alpha"),
            ("refs/tags/v1.2.3", ["1.2.3", "1.2", "1", "latest"], "1.2.3"),
            ("refs/tags/v0.4.0", ["0.4.0", "0.4", "latest"], "0.4.0"),
            ("refs/tags/v1.2.3-rc.1", ["1.2.3-rc.1"], "1.2.3-rc.1"),
            ("refs/tags/beta-v0.4.0", ["0.4.0-beta", "beta"], "0.4.0"),
            ("refs/tags/beta-v0.4.0-rc.1", ["0.4.0-rc.1-beta", "beta"], "0.4.0-rc.1"),
            ("refs/tags/rc-v2.0.0", ["2.0.0-rc", "rc"], "2.0.0"),
            ("refs/heads/feature/x", [], None),
            ("refs/heads/beta", [], None),
            ("refs/tags/v1.2", [], None),
            ("refs/tags/v01.2.3", [], None),
            ("refs/tags/1.2.3", [], None),
            ("refs/tags/Beta-v1.2.3", [], None),
            ("refs/tags/v1.2.3+build", [], None),
            ("refs/pull/1/merge", [], None),
            ("", [], None),
        ):
            with self.subTest(ref=ref):
                self.assertEqual(containers.channel_tags(ref), tags)
                self.assertEqual(containers.release_version(ref), version)
                self.assertTrue(all(containers.TAG.fullmatch(tag) for tag in tags))

    def test_guard_classification_and_semver_ordering(self):
        self.assertEqual([containers.tag_guard(tag) for tag in ("latest", "1", "1.2", "dev", "beta", "1.2.3", "1.2.3-beta", "1.2.3-rc.1")],
                         ["version", "version", "version", "ancestor", "ancestor", None, None, None])
        ordered = ["0.9.0", "1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-beta.2", "1.0.0-beta.11", "1.0.0-rc.1", "1.0.0", "1.0.1", "1.10.0", "2.0.0"]
        self.assertEqual(sorted(reversed(ordered), key=containers.semver_key), ordered)
        self.assertIsNone(containers.semver_key("dev"))
        self.assertIsNone(containers.semver_key(None))

    def test_tags_cli_prints_channel_tags(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.assertEqual(containers.main(["tags", "--ref", "refs/tags/v1.2.3"]), 0)
        self.assertEqual(json.loads(output.getvalue()), {"ref": "refs/tags/v1.2.3", "version": "1.2.3", "tags": ["1.2.3", "1.2", "1", "latest"]})


class History:
    def __init__(self, ancestors=(), unknown=(), versions=None):
        self.ancestors, self.unknown, self.versions = set(ancestors), set(unknown), versions or {}

    def is_ancestor(self, revision, source_sha):
        return None if revision in self.unknown else revision in self.ancestors

    def tagged_version(self, revision):
        return self.versions.get(revision)


class TagGuardTests(unittest.TestCase):
    OLD = "c" * 40

    def decide(self, tag, existing, version="1.2.3", **history):
        return containers.tag_decision(tag, existing, SOURCE_SHA, version, History(**history))

    def test_existing_metadata_is_read_from_index_annotations_or_image_labels(self):
        self.assertEqual(containers.existing_release(existing_index(self.OLD, "1.0.0")), {"revision": self.OLD, "version": "1.0.0"})
        self.assertEqual(containers.existing_release(existing_index(self.OLD, annotate=False)), {"revision": self.OLD, "version": None})
        self.assertEqual(containers.existing_release(existing_manifest(self.OLD)), {"revision": self.OLD, "version": None})
        self.assertEqual(containers.existing_release(existing_single_platform_index(self.OLD)), {"revision": self.OLD, "version": None})
        self.assertEqual(containers.existing_release({}), {"revision": None, "version": None})
        self.assertEqual(containers.existing_release("garbage"), {})

    def test_branch_tags_follow_ancestry(self):
        self.assertEqual(self.decide("dev", None)[0], True)
        self.assertEqual(self.decide("dev", {"revision": SOURCE_SHA})[0], True)
        self.assertEqual(self.decide("dev", {"revision": self.OLD}, ancestors=[self.OLD])[0], True)
        self.assertEqual(self.decide("dev", {"revision": self.OLD}, unknown=[self.OLD])[0], True)
        self.assertEqual(self.decide("dev", {"revision": None})[0], True)
        self.assertEqual(self.decide("dev", {"revision": "HEAD"})[0], True)
        move, reason = self.decide("beta", {"revision": self.OLD})
        self.assertFalse(move)
        self.assertIn(self.OLD, reason)
        self.assertIn(SOURCE_SHA, reason)

    def test_version_tags_never_regress(self):
        for tag in ("latest", "1", "1.2"):
            with self.subTest(tag=tag):
                self.assertTrue(self.decide(tag, None)[0])
                self.assertTrue(self.decide(tag, {"version": "1.2.3"})[0])
                self.assertTrue(self.decide(tag, {"version": "1.2.2"})[0])
                self.assertTrue(self.decide(tag, {"version": "1.2.3-rc.1"})[0])
                self.assertTrue(self.decide(tag, {"version": "dev"})[0])
                self.assertTrue(self.decide(tag, {})[0])
                move, reason = self.decide(tag, {"version": "1.3.0"})
                self.assertFalse(move)
                self.assertIn("1.3.0", reason)
                self.assertFalse(self.decide(tag, {"version": "2.0.0"}, version="1.2.3")[0])
                self.assertFalse(self.decide(tag, {"version": "1.2.3"}, version="1.2.3-rc.1")[0])

    def test_version_falls_back_to_the_git_tag_on_the_recorded_commit(self):
        self.assertFalse(self.decide("latest", {"revision": self.OLD}, versions={self.OLD: "1.3.0"})[0])
        self.assertTrue(self.decide("latest", {"revision": self.OLD}, versions={self.OLD: "1.2.0"})[0])
        self.assertTrue(self.decide("latest", {"revision": self.OLD})[0])
        self.assertTrue(self.decide("latest", {"revision": "bad", "version": "x"}, versions={"bad": "9.0.0"})[0])
        self.assertFalse(self.decide("latest", {"revision": self.OLD, "version": "1.4.0"}, versions={self.OLD: "1.0.0"})[0])

    def test_exact_versions_and_prerelease_tags_are_not_guarded(self):
        for tag in ("1.2.3", "1.2.3-beta", "1.2.3-rc.1"):
            self.assertEqual(self.decide(tag, {"revision": self.OLD, "version": "9.9.9"}), (True, "unguarded tag"))


class PublishTests(unittest.TestCase):
    def repository(self, suffix):
        return f"ghcr.io/{OWNER}/flow-like-{suffix}"

    def publish(self, release, ref, **kwargs):
        run = FakeRegistryRun(**kwargs)
        with contextlib.redirect_stderr(io.StringIO()) as errors:
            document = containers.publish(release, ref, "Rheosoph/flow-like", containers.Registry(run))
        return document, run, errors.getvalue()

    def test_self_hosted_repositories_become_annotated_indexes_with_every_tag(self):
        release = release_manifest("self-hosted")
        document, run, _ = self.publish(release, "refs/tags/v1.2.3")
        self.assertEqual(containers.validate_indexes(document, release), document)
        self.assertEqual(document["ref"], "refs/tags/v1.2.3")
        self.assertEqual(len(document["images"]), 16)
        immutable = f"sha-{SOURCE_SHA}-run-{RUN_ID}-{RUN_ATTEMPT}"
        for image in document["images"]:
            self.assertEqual(image["kind"], "index")
            self.assertEqual(image["platforms"], ["linux/amd64", "linux/arm64"])
            self.assertEqual(image["tags"], [immutable, "1.2.3", "1.2", "1", "latest"])
            self.assertEqual(image["skipped_tags"], [])
            created = run.references[f"{image['repository']}:{immutable}"]
            self.assertEqual(created["manifest"]["digest"], image["digest"])
            self.assertEqual(created["manifest"]["annotations"], {
                containers.REVISION_ANNOTATION: SOURCE_SHA,
                containers.SOURCE_ANNOTATION: "https://github.com/Rheosoph/flow-like",
                containers.VERSION_ANNOTATION: "1.2.3",
            })
            for tag in image["tags"]:
                self.assertIs(run.references[f"{image['repository']}:{tag}"], created)
        creates = [command for command in run.commands if command[3] == "create"]
        self.assertEqual(len(creates), 32)
        self.assertTrue(all("--prefer-index=false" not in command for command in creates[::2]))
        self.assertTrue(all("--prefer-index=false" in command and "--annotation" not in command for command in creates[1::2]))
        sources = {source for command in creates[::2] for source in command[-2:]}
        self.assertEqual(sources, {entry["image"] for entry in release["images"]})

    def test_cloud_repositories_are_carbon_copied_without_annotations(self):
        release = release_manifest("aws")
        document, run, _ = self.publish(release, "refs/heads/main")
        self.assertEqual(len(document["images"]), 13)
        ecs_api = self.repository("aws-api-ecs")
        records = {entry["repository"]: entry for entry in release["images"]}
        for image in document["images"]:
            self.assertEqual(image["tags"][1:], ["main"])
            if image["repository"] == ecs_api:
                continue
            self.assertEqual(image["kind"], "manifest")
            self.assertEqual(image["digest"], records[image["repository"]]["digest"])
            self.assertEqual(image["platforms"], [records[image["repository"]]["platform"]])
            self.assertNotIn("manifests", run.references[f"{image['repository']}:main"]["manifest"])
        ecs = next(image for image in document["images"] if image["repository"] == ecs_api)
        self.assertEqual((ecs["kind"], ecs["platforms"]), ("index", ["linux/amd64", "linux/arm64"]))
        for command in run.commands:
            if command[3] == "create" and not any(argument.startswith(f"{ecs_api}@") for argument in command):
                self.assertIn("--prefer-index=false", command)
                self.assertNotIn("--annotation", command)

    def test_single_platform_copy_that_becomes_an_index_fails(self):
        release = release_manifest("gcp")
        run = FakeRegistryRun()
        original = run.create

        def wrap(arguments):
            return original([argument for argument in arguments if argument != "--prefer-index=false"])
        run.create = wrap
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaisesRegex(ValueError, "single-platform manifest was not preserved"):
            containers.publish(release, "refs/heads/dev", "Rheosoph/flow-like", containers.Registry(run))

    def test_registry_failures_stop_publication_without_index_output(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaisesRegex(ValueError, "imagetools create failed"):
            self.publish(release_manifest("gcp"), "refs/heads/dev", fail_create=True)

    def test_guarded_tags_are_skipped_and_recorded_without_failing(self):
        old = "c" * 40
        api, web = self.repository("kubernetes-api"), self.repository("kubernetes-web")
        existing = {
            f"{api}:latest": existing_index(old, "2.0.0"),
            f"{api}:1": existing_index(old, "1.9.0"),
            f"{web}:latest": existing_index(old, annotate=False),
        }
        release = release_manifest("kubernetes")
        document, run, errors = self.publish(release, "refs/tags/v1.2.3", existing=existing, versions={old: ["v1.3.0", "v1.3.1"]})
        images = {image["repository"]: image for image in document["images"]}
        self.assertEqual(images[api]["tags"][1:], ["1.2.3", "1.2"])
        self.assertEqual([entry["tag"] for entry in images[api]["skipped_tags"]], ["1", "latest"])
        self.assertIn("2.0.0", images[api]["skipped_tags"][1]["reason"])
        self.assertEqual(images[web]["tags"][1:], ["1.2.3", "1.2", "1"])
        self.assertEqual(images[web]["skipped_tags"], [{"tag": "latest", "reason": "existing version 1.3.1 is newer than 1.2.3"}])
        self.assertEqual(images[self.repository("docker-compose-runtime")]["skipped_tags"], [])
        self.assertIn("::warning::", errors)
        self.assertNotIn(f"{api}:latest", run.references)
        self.assertIn(f"{api}:1.2", run.references)
        self.assertEqual(containers.validate_indexes(document, release), document)

    def test_branch_tags_only_move_forward(self):
        old = "c" * 40
        api = self.repository("docker-compose-api")
        release = release_manifest("docker-compose")
        existing = {f"{api}:dev": existing_index(old), f"{self.repository('docker-compose-web')}:dev": existing_manifest(old)}
        document, run, _ = self.publish(release, "refs/heads/dev", existing=existing)
        images = {image["repository"]: image for image in document["images"]}
        self.assertEqual(images[api]["skipped_tags"][0]["tag"], "dev")
        self.assertEqual(images[self.repository("docker-compose-web")]["tags"], [images[api]["tags"][0]])
        self.assertEqual([command for command in run.commands if command[:2] == ["git", "merge-base"]][0], ["git", "merge-base", "--is-ancestor", old, SOURCE_SHA])
        document, _, _ = self.publish(release, "refs/heads/dev", existing=existing, ancestors=[old])
        self.assertTrue(all(image["tags"][1:] == ["dev"] for image in document["images"]))
        document, _, _ = self.publish(release, "refs/heads/dev", existing=existing, unknown=[old])
        self.assertTrue(all(image["tags"][1:] == ["dev"] for image in document["images"]))

    def test_docker_manifest_lists_warn_that_index_annotations_were_dropped(self):
        release = release_manifest("kubernetes")
        document, run, errors = self.publish(release, "refs/tags/v1.2.3", docker_media_types=True)
        immutable = document["images"][0]["tags"][0]
        for image in document["images"]:
            self.assertNotIn("annotations", run.references[f"{image['repository']}:{immutable}"]["manifest"])
            self.assertIn(f"::warning::{image['repository']}:{immutable} is a Docker manifest list", errors)
        self.assertEqual(containers.validate_indexes(document, release), document)
        _, _, errors = self.publish(release, "refs/tags/v1.2.3")
        self.assertNotIn("Docker manifest list", errors)
        _, _, errors = self.publish(release_manifest("gcp"), "refs/tags/v1.2.3", docker_media_types=True)
        self.assertNotIn("Docker manifest list", errors)

    def test_unreleased_refs_only_get_the_immutable_tag(self):
        release = release_manifest("azure")
        document, run, _ = self.publish(release, "refs/heads/feature")
        self.assertTrue(all(len(image["tags"]) == 1 and not image["skipped_tags"] for image in document["images"]))
        self.assertEqual(len([command for command in run.commands if command[3] == "create"]), 9)
        self.assertNotIn(containers.VERSION_ANNOTATION, json.dumps(run.commands))


class IndexesTests(unittest.TestCase):
    def setUp(self):
        self.release = release_manifest("kubernetes")
        with contextlib.redirect_stderr(io.StringIO()):
            self.document = containers.publish(self.release, "refs/tags/beta-v0.4.0", "Rheosoph/flow-like", containers.Registry(FakeRegistryRun()))

    def test_index_manifest_shape(self):
        self.assertEqual(set(self.document), {"schema_version", "registry", "owner", "source_sha", "build", "ref", "images"})
        self.assertEqual(self.document["build"], {"run_id": RUN_ID, "run_attempt": RUN_ATTEMPT})
        self.assertEqual([image["repository"] for image in self.document["images"]], sorted({entry["repository"] for entry in self.release["images"]}))
        for image in self.document["images"]:
            self.assertEqual(set(image), {"repository", "cloud", "workload", "kind", "digest", "platforms", "tags", "skipped_tags"})
            self.assertEqual(image["tags"], [f"sha-{SOURCE_SHA}-run-{RUN_ID}-{RUN_ATTEMPT}", "0.4.0-beta", "beta"])

    def test_release_binding_and_record_mutations_fail(self):
        with self.assertRaisesRegex(ValueError, "does not belong to this build"):
            containers.validate_manifest(self.release, SOURCE_SHA, "999", RUN_ATTEMPT)
        with self.assertRaisesRegex(ValueError, "does not match"):
            containers.validate_manifest(dict(self.release, owner="other"))
        with self.assertRaisesRegex(ValueError, "every channel tag"):
            containers.validate_indexes(dict(self.document, ref="refs/tags/v0.4.0"), self.release)
        with self.assertRaisesRegex(ValueError, "does not match the release"):
            containers.validate_indexes(dict(self.document, owner="other"), self.release)
        with self.assertRaisesRegex(ValueError, "released repository"):
            containers.validate_indexes(self.document, release_manifest("docker-compose"))
        for mutate, message in (
            (lambda image: image.update(digest="latest"), "SHA-256"),
            (lambda image: image.update(tags=["beta"]), "immutable tag first"),
            (lambda image: image.update(tags=image["tags"][:1]), "every channel tag"),
            (lambda image: image.update(tags=image["tags"] + ["latest"]), "every channel tag"),
            (lambda image: image.update(skipped_tags=[{"tag": "beta", "reason": "kept"}]), "every channel tag"),
            (lambda image: image.update(skipped_tags=[{"tag": "x", "reason": ""}]), "malformed skipped tag"),
            (lambda image: image.update(skipped_tags=[{"tag": "x", "reason": "r", "extra": 1}]), "malformed skipped tag"),
            (lambda image: image.update(kind="manifest"), "does not match the release: kind"),
            (lambda image: image.update(platforms=["linux/amd64"]), "does not match the release: platforms"),
            (lambda image: image.update(workload="api"), "does not match the release: workload"),
            (lambda image: image.update(extra=True), "does not match the release: extra"),
            (lambda image: image.update(repository="ghcr.io/rheosoph/flow-like-aws-api"), "naming a released repository"),
        ):
            document = copy.deepcopy(self.document)
            mutate(document["images"][0])
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                containers.validate_indexes(document, self.release)
        for images, message in (
            (self.document["images"][1:], "missing index records"),
            (self.document["images"] + self.document["images"][:1], "duplicate index record"),
            ("images", "must be a list"),
        ):
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                containers.indexes(self.release, self.document["ref"], images)
        with self.assertRaisesRegex(ValueError, "fully qualified"):
            containers.indexes(self.release, "dev", self.document["images"])

    def test_single_platform_digest_must_match_the_record(self):
        release = release_manifest("aws")
        with contextlib.redirect_stderr(io.StringIO()):
            document = containers.publish(release, "refs/heads/dev", "Rheosoph/flow-like", containers.Registry(FakeRegistryRun()))
        document["images"][0]["digest"] = "sha256:" + "9" * 64
        with self.assertRaisesRegex(ValueError, "single-platform digest"):
            containers.validate_indexes(document, release)

    def test_publish_and_indexes_cli_round_trip_with_stubbed_tools(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            stub = root / "bin"
            stub.mkdir()
            state = root / "registry.json"
            state.write_text("{}")
            script = f"""#!/usr/bin/env python3
import json, sys
from pathlib import Path
state = Path({str(state)!r})
references = json.loads(state.read_text())
args = sys.argv[1:]
if Path(sys.argv[0]).name == "git":
    sys.exit(1 if args[1] == "merge-base" else 0)
assert args[:2] == ["buildx", "imagetools"], args
if args[2] == "create":
    tags = [args[i + 1] for i, a in enumerate(args) if a == "--tag"]
    sources = [a for a in args[3:] if not a.startswith("--") and a not in tags]
    if sources[0] in references:
        image = references[sources[0]]
    elif len(sources) == 1 and "--prefer-index=false" in args:
        image = {{"digest": sources[0].partition("@")[2]}}
    else:
        image = {{"digest": "sha256:" + "d" * 64, "manifests": [{{"digest": s.partition("@")[2]}} for s in sources]}}
    for tag in tags:
        references[tag] = image
        references[tag.rpartition(":")[0] + "@" + image["digest"]] = image
    state.write_text(json.dumps(references))
    sys.exit(0)
image = references.get(args[3])
if image is None:
    sys.exit(1)
if args[4:] == ["--raw"]:
    print(json.dumps({{k: v for k, v in image.items() if k != "digest"}}))
elif args[4:] == ["--format", "{{{{json .Manifest}}}}"]:
    print(json.dumps(image))
else:
    print(json.dumps({{"manifest": image, "image": {{}}}}))
"""
            for name in ("docker", "git"):
                (stub / name).write_text(script)
                (stub / name).chmod(0o700)
            release = release_manifest("gcp")
            manifest_path = root / "container-images.json"
            manifest_path.write_text(json.dumps(release))
            output = root / "container-indexes.json"
            environment = dict(os.environ, PATH=f"{stub}{os.pathsep}{os.environ['PATH']}")
            common = ["--manifest", str(manifest_path), "--source-sha", SOURCE_SHA, "--run-id", RUN_ID, "--run-attempt", RUN_ATTEMPT]
            process = subprocess.run(
                [sys.executable, str(SCRIPT), "publish", "--ref", "refs/tags/v2.0.0", "--github-repository", "Rheosoph/flow-like", "--output", str(output), *common],
                env=environment, capture_output=True, text=True,
            )
            self.assertEqual(process.returncode, 0, process.stderr)
            document = json.loads(output.read_text())
            self.assertEqual(containers.validate_indexes(document, release), document)
            self.assertTrue(all(image["tags"] == [f"sha-{SOURCE_SHA}-run-{RUN_ID}-{RUN_ATTEMPT}", "2.0.0", "2.0", "2", "latest"] for image in document["images"]))
            process = subprocess.run([sys.executable, str(SCRIPT), "indexes", "--manifest", str(manifest_path), "--path", str(output)], capture_output=True, text=True)
            self.assertEqual(process.returncode, 0, process.stderr)
            self.assertEqual(json.loads(process.stdout), document)
            output.write_text(json.dumps(dict(document, owner="other")))
            process = subprocess.run([sys.executable, str(SCRIPT), "indexes", "--manifest", str(manifest_path), "--path", str(output)], capture_output=True, text=True)
            self.assertEqual(process.returncode, 1)
            self.assertIn("does not match the release", process.stderr)
            process = subprocess.run(
                [sys.executable, str(SCRIPT), "publish", "--ref", "refs/heads/dev", "--github-repository", "Rheosoph/flow-like", "--manifest", str(manifest_path),
                 "--source-sha", "d" * 40, "--run-id", RUN_ID, "--run-attempt", RUN_ATTEMPT],
                env=environment, capture_output=True, text=True,
            )
            self.assertEqual(process.returncode, 1)
            self.assertIn("does not belong to this build", process.stderr)


if __name__ == "__main__":
    unittest.main()
