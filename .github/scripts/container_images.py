#!/usr/bin/env python3
"""Select portable container targets, validate complete GHCR build manifests and publish release references."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys


SCHEMA_VERSION = 1
CLOUDS = ("all", "aws", "gcp", "azure", "docker-compose", "kubernetes", "self-hosted")
REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SELF_HOSTED_CLOUDS = ("docker-compose", "kubernetes")
KUBERNETES_SHARED_WORKLOADS = {"runtime", "compiler", "signaling", "object-store-init", "audit-worker"}
API_WORKLOADS = {"api", "api-ecs"}
CHANNEL_BRANCHES = ("dev", "main", "alpha")
SEMVER = re.compile(
    r"(?P<major>0|[1-9][0-9]*)\.(?P<minor>0|[1-9][0-9]*)\.(?P<patch>0|[1-9][0-9]*)"
    r"(?:-(?P<prerelease>[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
)
TAG = re.compile(r"[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}")
DIGEST = re.compile(r"sha256:[0-9a-f]{64}")
COMMIT = re.compile(r"[0-9a-f]{40}")
REVISION_ANNOTATION = "org.opencontainers.image.revision"
VERSION_ANNOTATION = "org.opencontainers.image.version"
SOURCE_ANNOTATION = "org.opencontainers.image.source"


WHOLE_CONTEXT_COPY = re.compile(r"^COPY\s+(?:--\S+\s+)*\.\s+\.?/?$")
EXPENSIVE_STEP = re.compile(r"\b(?:cargo build|cargo chef|cargo install|bun install|npm ci|go build)\b")


def layer_cache_enabled(dockerfile):
    """Report whether a recipe can ever restore its expensive step from a layer cache.

    A whole-context `COPY . .` above that step changes digest on every commit, so
    everything below it is a guaranteed miss and exporting it is a pure write.
    """
    lines = [line.strip() for line in (REPOSITORY_ROOT / dockerfile).read_text().splitlines()]
    expensive = next((index for index, line in enumerate(lines) if EXPENSIVE_STEP.search(line)), None)
    if expensive is None:
        return True
    whole_copy = next((index for index, line in enumerate(lines) if WHOLE_CONTEXT_COPY.match(line)), None)
    return whole_copy is None or whole_copy > expensive


def target(cloud, workload, platform="linux/amd64", recipe=None, context=".", architecture_id=False,
           image_suffix=None, audit_features=None):
    target_id = f"{cloud}-{workload}"
    if architecture_id:
        target_id += f"-{platform.split('/')[1]}"
    dockerfile = recipe or f"apps/backend/{cloud}/{workload}/Dockerfile"
    return {
        "id": target_id,
        "cloud": cloud,
        "workload": workload,
        "dockerfile": dockerfile,
        "context": context,
        "platform": platform,
        "runner": "ubuntu-24.04-arm" if platform == "linux/arm64" else "ubuntu-24.04",
        "image_suffix": image_suffix or f"flow-like-{cloud}-{workload}",
        "audit_features": audit_features or "",
        "layer_cache": layer_cache_enabled(dockerfile),
    }


# APIs embed reviewed defaults and accept a full configuration at startup.
# Publication records identify the fallback separately from the runtime contract.
TARGETS = tuple(sorted([
    target("aws", "api", "linux/arm64"),
    # The optional ECS API runs on whichever capacity a self-hosting account has.
    *[target("aws", "api-ecs", f"linux/{architecture}", architecture_id=True) for architecture in ("amd64", "arm64")],
    target("aws", "executor", "linux/arm64"),
    target("aws", "executor-async", recipe="apps/backend/aws/executor-ecs/Dockerfile"),
    target("aws", "executor-lambda-async", "linux/arm64", recipe="apps/backend/aws/executor-async/Dockerfile"),
    target("aws", "compiler", "linux/arm64", recipe="apps/backend/aws/compiler-ecs/Dockerfile"),
    target("aws", "file-tracker", "linux/arm64"),
    target("aws", "media-transformer", "linux/arm64"),
    target("aws", "event-bridge", "linux/arm64"),
    target("aws", "maintenance", "linux/arm64"),
    target("aws", "audit-worker", "linux/arm64"),
    *[target(cloud, "audit-worker", audit_features=cloud)
      for cloud in ("azure", "gcp")],
    *[target("docker-compose", "audit-worker", f"linux/{architecture}", architecture_id=True,
             recipe="apps/backend/audit-worker/Dockerfile", image_suffix="flow-like-audit-worker", audit_features="aws,azure,gcp")
      for architecture in ("amd64", "arm64")],
    target("aws", "signaling", "linux/arm64", recipe="apps/backend/docker-compose/signaling/Dockerfile"),
    target("aws", "migration", "linux/arm64"),
    *[target("gcp", workload) for workload in (
        "api", "queue-worker", "executor", "signaling", "migration", "scheduler", "maintenance",
    )],
    *[target("azure", workload) for workload in (
        "api", "queue-worker", "executor", "maintenance", "scheduler", "migration", "signaling",
    )],
    target("azure", "otel-collector"),
    *[target("docker-compose", workload, f"linux/{architecture}", architecture_id=True,
             recipe="apps/backend/docker-compose/object-store/Dockerfile" if workload == "object-store-init" else None)
      for workload in ("api", "runtime", "compiler", "execution-manager", "sink-services", "signaling", "db-init", "object-store-init", "web")
      for architecture in ("amd64", "arm64")],
    *[target("kubernetes", workload, f"linux/{architecture}", architecture_id=True)
      for workload in ("api", "executor", "execution-manager", "sink-trigger", "migration", "web")
      for architecture in ("amd64", "arm64")],
], key=lambda entry: entry["id"]))
TARGET_BY_ID = {entry["id"]: entry for entry in TARGETS}


def matrix(cloud):
    if cloud not in CLOUDS:
        raise ValueError("unsupported cloud")
    def selected(entry):
        if cloud == "all":
            return True
        if cloud == "self-hosted":
            return entry["cloud"] in SELF_HOSTED_CLOUDS
        # The Helm chart uses the same Compose packages for these services.
        if cloud == "kubernetes" and entry["cloud"] == "docker-compose":
            return entry["workload"] in KUBERNETES_SHARED_WORKLOADS
        return cloud == entry["cloud"]
    return {"include": [dict(entry) for entry in TARGETS if selected(entry)]}


def build_identity(owner, source_sha, run_id, run_attempt):
    owner = owner.lower()
    if not re.fullmatch(r"[a-z0-9](?:[a-z0-9-]{0,37}[a-z0-9])?", owner):
        raise ValueError("owner must be a GitHub user or organization name")
    if not COMMIT.fullmatch(source_sha) or source_sha == "0" * 40:
        raise ValueError("source SHA must be a nonzero 40-character lowercase Git commit SHA")
    for name, value in (("run ID", run_id), ("run attempt", run_attempt)):
        if not re.fullmatch(r"[1-9][0-9]*", value):
            raise ValueError(f"{name} must be a positive decimal integer")
    return owner, {"run_id": run_id, "run_attempt": run_attempt}


def record(target_id, owner, source_sha, digest, run_id, run_attempt):
    owner, build = build_identity(owner, source_sha, run_id, run_attempt)
    if target_id not in TARGET_BY_ID:
        raise ValueError("unknown container target")
    if not isinstance(digest, str) or not DIGEST.fullmatch(digest):
        raise ValueError("digest must be a lowercase SHA-256 image digest")
    entry = TARGET_BY_ID[target_id]
    repository = f"ghcr.io/{owner}/{entry['image_suffix']}"
    architecture = entry["platform"].split("/")[1]
    build_inputs = {
        "dockerfile_sha256": hashlib.sha256((REPOSITORY_ROOT / entry["dockerfile"]).read_bytes()).hexdigest(),
    }
    api = entry["workload"] in API_WORKLOADS or (entry["cloud"] == "aws" and entry["workload"] == "audit-worker")
    if entry["audit_features"]:
        build_inputs["audit_features"] = entry["audit_features"]
        # Dedicated workers compile the audit section of their API's build config.
        config = ("apps/backend/docker-compose/flow-like.config.example.json"
                  if entry["cloud"] in SELF_HOSTED_CLOUDS else "flow-like.config.json")
        build_inputs["runtime_config"] = "compiled-audit-section-v1"
        build_inputs["flow_like_config_sha256"] = hashlib.sha256((REPOSITORY_ROOT / config).read_bytes()).hexdigest()
    if entry["workload"] == "audit-worker" and entry["cloud"] in ("azure", "gcp"):
        launcher = REPOSITORY_ROOT / f"apps/backend/{entry['cloud']}/audit-worker/entrypoint.py"
        build_inputs["entrypoint_sha256"] = hashlib.sha256(launcher.read_bytes()).hexdigest()
        build_inputs["database_auth"] = "entra-managed-identity" if entry["cloud"] == "azure" else "cloud-sql-iam"
    if api:
        build_inputs["runtime_config"] = "full-document-v1"
    if api and entry["cloud"] not in SELF_HOSTED_CLOUDS:
        build_inputs["flow_like_config_sha256"] = hashlib.sha256((REPOSITORY_ROOT / "flow-like.config.json").read_bytes()).hexdigest()
        build_inputs["variant"] = "runtime-config-public-default"
    if entry["cloud"] in SELF_HOSTED_CLOUDS and entry["workload"] == "api":
        config = REPOSITORY_ROOT / f"apps/backend/{entry['cloud']}/flow-like.config.example.json"
        build_inputs["flow_like_config_sha256"] = hashlib.sha256(config.read_bytes()).hexdigest()
        build_inputs["variant"] = "runtime-config-self-hosted-default"
    if entry["cloud"] in SELF_HOSTED_CLOUDS and entry["workload"] == "web":
        build_inputs["variant"] = "runtime-public-config"
    if entry["cloud"] == "aws" and (api or entry["workload"] == "file-tracker"):
        build_inputs["database_tls"] = "dsql-no-custom-ca"
    return {
        "schema_version": SCHEMA_VERSION,
        "id": entry["id"],
        "cloud": entry["cloud"],
        "workload": entry["workload"],
        "dockerfile": entry["dockerfile"],
        "context": entry["context"],
        "platform": entry["platform"],
        "source_sha": source_sha,
        "build": build,
        "build_inputs": build_inputs,
        "repository": repository,
        "tag": f"sha-{source_sha}-{architecture}-run-{run_id}-{run_attempt}",
        "digest": digest,
        "image": f"{repository}@{digest}",
    }


def manifest(cloud, owner, source_sha, run_id, run_attempt, records):
    owner, build = build_identity(owner, source_sha, run_id, run_attempt)
    expected_ids = {entry["id"] for entry in matrix(cloud)["include"]}
    images = {}
    for entry in records:
        if not isinstance(entry, dict) or not isinstance(entry.get("id"), str):
            raise ValueError("image record must be an object with a target ID")
        target_id = entry["id"]
        if target_id not in expected_ids:
            raise ValueError("image record has an unexpected target ID")
        if target_id in images:
            raise ValueError(f"duplicate image record for {target_id}")
        expected = record(target_id, owner, source_sha, entry.get("digest"), run_id, run_attempt)
        # Reject extra fields too: release output contains only this reviewed
        # schema, and cannot silently carry arbitrary artifact contents.
        if json.dumps(entry, sort_keys=True) != json.dumps(expected, sort_keys=True):
            fields = sorted(set(entry) ^ set(expected) | {
                key for key in expected if key in entry
                and json.dumps(entry[key], sort_keys=True) != json.dumps(expected[key], sort_keys=True)
            })
            raise ValueError(f"image record for {target_id} does not match the release: {', '.join(fields)}")
        images[target_id] = expected
    missing = sorted(expected_ids - images.keys())
    if missing:
        raise ValueError(f"missing image records: {', '.join(missing)}")
    return {
        "schema_version": SCHEMA_VERSION,
        "cloud": cloud,
        "registry": "ghcr.io",
        "owner": owner,
        "source_sha": source_sha,
        "build": build,
        "images": [images[target_id] for target_id in sorted(images)],
    }


def read_records(directory):
    if not directory.is_dir():
        raise ValueError("records directory does not exist")
    records = []
    for path in sorted(directory.rglob("*.json")):
        try:
            records.append(json.loads(path.read_text(encoding="utf-8")))
        except (OSError, UnicodeError, json.JSONDecodeError) as error:
            raise ValueError(f"cannot read image record {path.name}") from error
    return records


def read_json(path, description):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read {description} {path.name}") from error


def validate_manifest(document, source_sha=None, run_id=None, run_attempt=None):
    if not isinstance(document, dict) or not isinstance(document.get("build"), dict):
        raise ValueError("release manifest must be an object with build identity")
    build = document["build"]
    if not isinstance(document.get("images"), list):
        raise ValueError("release manifest must list its image records")
    expected = manifest(
        document.get("cloud"), str(document.get("owner", "")), str(document.get("source_sha", "")),
        str(build.get("run_id", "")), str(build.get("run_attempt", "")), document["images"],
    )
    if json.dumps(document, sort_keys=True) != json.dumps(expected, sort_keys=True):
        raise ValueError("release manifest does not match its records")
    for name, actual, wanted in (
        ("source SHA", expected["source_sha"], source_sha),
        ("run ID", build["run_id"], run_id),
        ("run attempt", build["run_attempt"], run_attempt),
    ):
        if wanted is not None and actual != wanted:
            raise ValueError(f"release manifest {name} {actual} does not belong to this build")
    return expected


def parse_ref(ref):
    branch = re.fullmatch(r"refs/heads/([a-z]+)", ref or "")
    if branch and branch.group(1) in CHANNEL_BRANCHES:
        return {"channel": branch.group(1), "version": branch.group(1), "semver": None}
    tag = re.fullmatch(r"refs/tags/(?:(?P<channel>[a-z]+)-)?v(?P<version>.+)", ref or "")
    if tag and SEMVER.fullmatch(tag.group("version")):
        return {"channel": tag.group("channel"), "version": tag.group("version"), "semver": tag.group("version")}
    return None


def release_version(ref):
    release = parse_ref(ref)
    return release["version"] if release else None


def channel_tags(ref):
    release = parse_ref(ref)
    if release is None:
        return []
    if release["semver"] is None:
        return [release["channel"]]
    version = release["semver"]
    if release["channel"]:
        return [f"{version}-{release['channel']}", release["channel"]]
    parts = SEMVER.fullmatch(version)
    tags = [version]
    if not parts.group("prerelease"):
        tags.append(f"{parts.group('major')}.{parts.group('minor')}")
        if parts.group("major") != "0":
            tags.append(parts.group("major"))
        tags.append("latest")
    return tags


def tag_guard(tag):
    if tag == "latest" or re.fullmatch(r"[0-9]+(?:\.[0-9]+)?", tag):
        return "version"
    if re.fullmatch(r"[a-z]+", tag):
        return "ancestor"
    return None


def semver_key(version):
    parts = SEMVER.fullmatch(version or "")
    if not parts:
        return None
    numbers = tuple(int(parts.group(name)) for name in ("major", "minor", "patch"))
    prerelease = parts.group("prerelease")
    if not prerelease:
        return (*numbers, 1, ())
    identifiers = tuple((0, int(part)) if part.isdigit() else (1, part) for part in prerelease.split("."))
    return (*numbers, 0, identifiers)


def existing_release(inspection):
    """Read the revision and version an existing tag was published from."""
    if not isinstance(inspection, dict):
        return {}
    descriptor = inspection.get("manifest") if isinstance(inspection.get("manifest"), dict) else {}
    image = inspection.get("image") if isinstance(inspection.get("image"), dict) else {}
    configs = list(image.values()) if "manifests" in descriptor and "config" not in image else [image]
    labels = {}
    for config in configs:
        if isinstance(config, dict) and isinstance(config.get("config"), dict):
            labels.update(config["config"].get("Labels") or {})
    annotations = descriptor.get("annotations") or {}
    return {
        "revision": annotations.get(REVISION_ANNOTATION) or labels.get(REVISION_ANNOTATION),
        "version": annotations.get(VERSION_ANNOTATION) or labels.get(VERSION_ANNOTATION),
    }


def tag_decision(tag, existing, source_sha, version, history):
    """Return (move, note); a guarded tag stays when the registry holds a newer release."""
    guard = tag_guard(tag)
    if guard is None:
        return True, "unguarded tag"
    if existing is None:
        return True, "tag is absent"
    revision = existing.get("revision")
    if not isinstance(revision, str) or not COMMIT.fullmatch(revision):
        revision = None
    if guard == "ancestor":
        if revision is None:
            return True, "existing revision is unknown"
        if revision == source_sha:
            return True, "existing revision is this commit"
        ancestor = history.is_ancestor(revision, source_sha)
        if ancestor is None:
            return True, f"existing revision {revision} is not in this repository"
        if ancestor:
            return True, f"existing revision {revision} is an ancestor"
        return False, f"existing revision {revision} is not an ancestor of {source_sha}"
    current = existing.get("version")
    if semver_key(current) is None:
        # Docker manifest lists drop index annotations; fall back to the v tag on the recorded commit.
        current = history.tagged_version(revision) if revision else None
    if semver_key(current) is None or semver_key(version) is None:
        return True, "existing version is unknown"
    if semver_key(version) >= semver_key(current):
        return True, f"existing version {current} is not newer"
    return False, f"existing version {current} is newer than {version}"


def run_command(command, timeout=900):
    return subprocess.run(command, capture_output=True, text=True, timeout=timeout)


class Registry:
    """Shell out to docker and git; the process inherits registry credentials from docker login."""

    def __init__(self, run=run_command):
        self.run = run

    def docker(self, *arguments):
        try:
            process = self.run(["docker", "buildx", "imagetools", *arguments])
        except (OSError, subprocess.SubprocessError) as error:
            raise ValueError(f"docker buildx imagetools {arguments[0]} could not run") from error
        if process.returncode != 0:
            print(process.stderr, file=sys.stderr, end="")
            raise ValueError(f"docker buildx imagetools {arguments[0]} failed")
        return process.stdout

    def inspect(self, reference):
        try:
            process = self.run(["docker", "buildx", "imagetools", "inspect", reference, "--format", "{{json .}}"])
        except (OSError, subprocess.SubprocessError):
            return None
        if process.returncode != 0:
            return None
        try:
            return json.loads(process.stdout)
        except json.JSONDecodeError:
            return {}

    def is_ancestor(self, revision, source_sha):
        try:
            process = self.run(["git", "merge-base", "--is-ancestor", revision, source_sha])
        except (OSError, subprocess.SubprocessError):
            return None
        if process.returncode == 0:
            return True
        if process.returncode == 1:
            return False
        return None

    def tagged_version(self, revision):
        try:
            process = self.run(["git", "tag", "--points-at", revision, "v*"])
        except (OSError, subprocess.SubprocessError):
            return None
        if process.returncode != 0:
            return None
        versions = [tag[1:] for tag in process.stdout.split() if semver_key(tag[1:]) is not None]
        return max(versions, key=semver_key) if versions else None

    def descriptor(self, reference):
        try:
            descriptor = json.loads(self.docker("inspect", reference, "--format", "{{json .Manifest}}"))
        except json.JSONDecodeError as error:
            raise ValueError(f"cannot read the registry descriptor of {reference}") from error
        if not isinstance(descriptor, dict) or not isinstance(descriptor.get("digest"), str) or not DIGEST.fullmatch(descriptor["digest"]):
            raise ValueError(f"registry descriptor of {reference} has no digest")
        return descriptor

    def raw_manifest(self, reference):
        try:
            return json.loads(self.docker("inspect", reference, "--raw"))
        except json.JSONDecodeError as error:
            raise ValueError(f"cannot read the raw manifest of {reference}") from error


def log(message, warning=False):
    print(f"::warning::{message}" if warning else f"container images: {message}", file=sys.stderr)


def publish(document, ref, github_repository, registry):
    source_sha = document["source_sha"]
    build = document["build"]
    immutable_tag = f"sha-{source_sha}-run-{build['run_id']}-{build['run_attempt']}"
    version = release_version(ref)
    tags = channel_tags(ref)
    groups = {}
    for image in document["images"]:
        groups.setdefault(image["repository"], []).append(image)
    images = []
    for repository, records in sorted(groups.items()):
        immutable = f"{repository}:{immutable_tag}"
        sources = [entry["image"] for entry in records]
        if len(records) == 1:
            registry.docker("create", "--prefer-index=false", "--tag", immutable, *sources)
            if "manifests" in registry.raw_manifest(immutable):
                raise ValueError(f"{immutable} became an image index; the single-platform manifest was not preserved")
            kind = "manifest"
        else:
            annotations = {
                REVISION_ANNOTATION: source_sha,
                SOURCE_ANNOTATION: f"https://github.com/{github_repository}",
            }
            if version:
                annotations[VERSION_ANNOTATION] = version
            registry.docker(
                "create", "--tag", immutable,
                *[argument for key, value in sorted(annotations.items()) for argument in ("--annotation", f"index:{key}={value}")],
                *sources,
            )
            kind = "index"
        descriptor = registry.descriptor(immutable)
        if kind == "index" and (descriptor.get("annotations") or {}).get(REVISION_ANNOTATION) != source_sha:
            log(f"{immutable} is a Docker manifest list; index annotations were not stored, the guard will use image labels and Git tags", warning=True)
        digest = descriptor["digest"]
        published = {entry.get("digest") for entry in descriptor.get("manifests", [])} if kind == "index" else {digest}
        if published != {entry["digest"] for entry in records}:
            raise ValueError(f"{immutable} does not reference exactly the scanned image digests")
        log(f"{immutable} -> {digest} ({kind})")
        applied = [immutable_tag]
        skipped = []
        for tag in tags:
            existing = registry.inspect(f"{repository}:{tag}")
            move, note = tag_decision(tag, None if existing is None else existing_release(existing), source_sha, version, registry)
            if move:
                log(f"{repository}:{tag} moves ({note})")
                applied.append(tag)
            else:
                log(f"{repository}:{tag} kept: {note}", warning=True)
                skipped.append({"tag": tag, "reason": note})
        if len(applied) > 1:
            registry.docker(
                "create", "--prefer-index=false",
                *[argument for tag in applied[1:] for argument in ("--tag", f"{repository}:{tag}")],
                f"{repository}@{digest}",
            )
        images.append({
            "repository": repository,
            "cloud": records[0]["cloud"],
            "workload": records[0]["workload"],
            "kind": kind,
            "digest": digest,
            "platforms": sorted(entry["platform"] for entry in records),
            "tags": applied,
            "skipped_tags": skipped,
        })
    return indexes(document, ref, images)


def indexes(document, ref, images):
    document = validate_manifest(document)
    if not isinstance(ref, str) or not ref.startswith("refs/"):
        raise ValueError("ref must be a fully qualified Git ref")
    if not isinstance(images, list):
        raise ValueError("index images must be a list")
    build = document["build"]
    immutable_tag = f"sha-{document['source_sha']}-run-{build['run_id']}-{build['run_attempt']}"
    channels = channel_tags(ref)
    groups = {}
    for image in document["images"]:
        groups.setdefault(image["repository"], []).append(image)
    results = {}
    for entry in images:
        if not isinstance(entry, dict) or entry.get("repository") not in groups:
            raise ValueError("index record must be an object naming a released repository")
        repository = entry["repository"]
        if repository in results:
            raise ValueError(f"duplicate index record for {repository}")
        records = groups[repository]
        digest = entry.get("digest")
        if not isinstance(digest, str) or not DIGEST.fullmatch(digest):
            raise ValueError(f"index record for {repository} needs a SHA-256 digest")
        tags = entry.get("tags")
        skipped = entry.get("skipped_tags")
        if not isinstance(tags, list) or not all(isinstance(tag, str) for tag in tags) or not tags or tags[0] != immutable_tag:
            raise ValueError(f"index record for {repository} must list the immutable tag first")
        if not isinstance(skipped, list) or not all(
            isinstance(item, dict) and isinstance(item.get("tag"), str) and isinstance(item.get("reason"), str) and item["reason"]
            and set(item) == {"tag", "reason"} for item in skipped
        ):
            raise ValueError(f"index record for {repository} has a malformed skipped tag")
        moved = tags[1:]
        kept = [item["tag"] for item in skipped]
        if len(set(moved + kept)) != len(moved) + len(kept) or sorted(moved + kept) != sorted(channels):
            raise ValueError(f"index record for {repository} does not account for every channel tag")
        if kind_for(records) == "manifest" and digest != records[0]["digest"]:
            raise ValueError(f"index record for {repository} must reuse the single-platform digest")
        expected = {
            "repository": repository,
            "cloud": records[0]["cloud"],
            "workload": records[0]["workload"],
            "kind": kind_for(records),
            "digest": digest,
            "platforms": sorted(record["platform"] for record in records),
            "tags": tags,
            "skipped_tags": skipped,
        }
        if json.dumps(entry, sort_keys=True) != json.dumps(expected, sort_keys=True):
            fields = sorted(set(entry) ^ set(expected) | {
                key for key in expected if key in entry
                and json.dumps(entry[key], sort_keys=True) != json.dumps(expected[key], sort_keys=True)
            })
            raise ValueError(f"index record for {repository} does not match the release: {', '.join(fields)}")
        results[repository] = expected
    missing = sorted(groups.keys() - results.keys())
    if missing:
        raise ValueError(f"missing index records: {', '.join(missing)}")
    return {
        "schema_version": SCHEMA_VERSION,
        "registry": document["registry"],
        "owner": document["owner"],
        "source_sha": document["source_sha"],
        "build": build,
        "ref": ref,
        "images": [results[repository] for repository in sorted(results)],
    }


def kind_for(records):
    return "index" if len(records) > 1 else "manifest"


def validate_indexes(document, release):
    if not isinstance(document, dict):
        raise ValueError("index manifest must be an object")
    expected = indexes(release, document.get("ref"), document.get("images", []))
    if json.dumps(document, sort_keys=True) != json.dumps(expected, sort_keys=True):
        raise ValueError("index manifest does not match the release")
    return expected


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    matrix_parser = commands.add_parser("matrix", help="print the GitHub Actions build matrix")
    matrix_parser.add_argument("--cloud", choices=CLOUDS, default="all")
    record_parser = commands.add_parser("record", help="record one successfully pushed image digest")
    record_parser.add_argument("--target", choices=sorted(TARGET_BY_ID), required=True)
    record_parser.add_argument("--digest", required=True)
    manifest_parser = commands.add_parser("manifest", help="validate and merge a complete set of image records")
    manifest_parser.add_argument("--cloud", choices=CLOUDS, default="all")
    manifest_parser.add_argument("--records-dir", type=Path, required=True)
    tags_parser = commands.add_parser("tags", help="print the channel tags and version for a Git ref")
    tags_parser.add_argument("--ref", required=True)
    publish_parser = commands.add_parser("publish", help="create release references from a validated manifest and apply guarded tags")
    publish_parser.add_argument("--manifest", type=Path, required=True)
    publish_parser.add_argument("--ref", required=True)
    publish_parser.add_argument("--github-repository", required=True)
    indexes_parser = commands.add_parser("indexes", help="validate a published index manifest against its release")
    indexes_parser.add_argument("--manifest", type=Path, required=True)
    indexes_parser.add_argument("--path", type=Path, required=True)
    for command in (record_parser, manifest_parser):
        command.add_argument("--owner", required=True)
    for command in (record_parser, manifest_parser, publish_parser):
        command.add_argument("--source-sha", required=True)
        command.add_argument("--run-id", required=True)
        command.add_argument("--run-attempt", required=True)
    for command in (record_parser, manifest_parser, publish_parser, indexes_parser):
        command.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == "matrix":
            result = matrix(args.cloud)
        elif args.command == "record":
            result = record(args.target, args.owner, args.source_sha, args.digest, args.run_id, args.run_attempt)
        elif args.command == "manifest":
            result = manifest(args.cloud, args.owner, args.source_sha, args.run_id, args.run_attempt, read_records(args.records_dir))
        elif args.command == "tags":
            result = {"ref": args.ref, "version": release_version(args.ref), "tags": channel_tags(args.ref)}
        elif args.command == "publish":
            if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.github_repository):
                raise ValueError("GitHub repository must be owner/name")
            release = validate_manifest(read_json(args.manifest, "release manifest"), args.source_sha, args.run_id, args.run_attempt)
            result = publish(release, args.ref, args.github_repository, Registry())
        else:
            release = validate_manifest(read_json(args.manifest, "release manifest"))
            result = validate_indexes(read_json(args.path, "index manifest"), release)
        output = json.dumps(result, sort_keys=True, separators=(",", ":")) + "\n"
        if getattr(args, "output", None):
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(output, encoding="utf-8")
        else:
            print(output, end="")
    except (OSError, ValueError) as error:
        print(f"container images: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
