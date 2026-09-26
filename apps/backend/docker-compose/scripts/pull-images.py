#!/usr/bin/env python3
"""Pull the published Compose images for one tag and pin their registry digests without changing deployment secrets."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
from preflight import DIGEST_PIN_PATTERN, IMAGE_TAG_PATTERN, IMAGE_WORKLOADS, ROOT, SANDBOX_SOURCES, ensure_private, read_env, write_env

DEFAULT_REGISTRY = "ghcr.io/rheosoph"
LOGIN_HINT = "Private packages and forks require `docker login ghcr.io` with a read:packages token before pulling; also check that the tag was published for this registry."


def pull(repository, tag):
    reference = f"{repository}:{tag}"
    result = subprocess.run(["docker", "pull", reference], capture_output=True, text=True)
    if result.returncode:
        detail = result.stderr.strip().splitlines()[-1:] or ["no error output"]
        raise RuntimeError(f"Pulling {reference} failed: {detail[0]}\n{LOGIN_HINT}")
    digest = re.search(r"^Digest: (sha256:[a-f0-9]{64})\s*$", result.stdout, re.M)
    if digest:
        return f"{repository}@{digest.group(1)}"
    result = subprocess.run(["docker", "image", "inspect", "--format", "{{json .RepoDigests}}", reference], capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f"Docker could not inspect {reference} after pulling it")
    for entry in json.loads(result.stdout):
        if entry.partition("@")[0] == repository and re.fullmatch(DIGEST_PIN_PATTERN, entry):
            return entry
    raise RuntimeError(f"Docker recorded no registry digest for {reference}; pull it from a registry rather than loading it from an archive")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--env-file", type=Path, default=ROOT / ".env")
    parser.add_argument("--tag", help="Image tag such as dev, 1.2.3, or an immutable sha-<sha>-run-<run>-<attempt> tag; defaults to FLOW_LIKE_IMAGE_TAG from the env file, then dev")
    parser.add_argument("--registry", default=DEFAULT_REGISTRY, help="Registry and owner prefix holding flow-like-docker-compose-<workload> repositories")
    args = parser.parse_args()
    path = args.env_file.absolute()
    try:
        ensure_private(path)
        values = read_env(path)
    except (ValueError, OSError) as error:
        parser.error(str(error))
    tag = args.tag or values.get("FLOW_LIKE_IMAGE_TAG") or "dev"
    if not re.fullmatch(IMAGE_TAG_PATTERN, tag):
        parser.error("Tag must contain only letters, digits, '_', '.', '-' and start with a letter, digit or '_'")
    registry = args.registry.strip().rstrip("/")
    if not re.fullmatch(r"[A-Za-z0-9.\-]+(?::\d+)?(?:/[a-z0-9._\-]+)+", registry):
        parser.error("Registry must be a host followed by the package owner, such as ghcr.io/rheosoph")
    updates = {}
    try:
        for key, workload in IMAGE_WORKLOADS.items():
            repository = "flow-like-audit-worker" if workload == "audit-worker" else f"flow-like-docker-compose-{workload}"
            updates[key] = pull(f"{registry}/{repository}", tag)
    except (RuntimeError, OSError, ValueError) as error:
        print(f"Pull failed: {error}", file=sys.stderr)
        return 1
    for key, image_key in SANDBOX_SOURCES.items():
        updates[key] = updates[image_key]
    updates["FLOW_LIKE_IMAGE_TAG"] = tag
    write_env(path, updates)
    print(f"Pinned {len(IMAGE_WORKLOADS)} published images from {registry} at tag {tag}. Deployment secrets were preserved.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
