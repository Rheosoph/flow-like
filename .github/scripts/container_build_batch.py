#!/usr/bin/env python3
"""Publish helper images sequentially while keeping each image's scan and digest checks."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

import container_images


class BuildError(Exception):
    pass


class CredentialCleanupError(BuildError):
    pass


def run(arguments, *, env, capture=False, input=None, check=True, timeout=1800):
    result = subprocess.run(arguments, env=env, text=True, input=input,
                            capture_output=capture, timeout=timeout)
    if check and result.returncode:
        # Scanner reports and registry errors can contain sensitive content.
        # Report the operation; secret scan metadata is summarized separately.
        raise BuildError(f"{arguments[0]} {arguments[1]} failed ({result.returncode})")
    return result


def build_one(entry, identity, token, env, records_dir, temp_dir):
    target = entry["id"]
    platform = entry["platform"]
    repository = f"ghcr.io/{identity['owner']}/{entry['image_suffix']}"
    tag = f"sha-{identity['sha']}-{platform.split('/')[1]}-run-{identity['run_id']}-{identity['attempt']}"
    image = f"{repository}:{tag}"
    scripts = Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory(prefix=f"{target}-", dir=temp_dir, ignore_cleanup_errors=True) as directory:
        output = Path(directory)
        command = ["docker", "buildx", "build", "--file", entry["dockerfile"],
                   "--platform", platform, "--load", "--provenance=false", "--sbom=false",
                   "--tag", image, "--build-arg", f"CARGO_BUILD_JOBS={entry['cargo_build_jobs']}",
                   "--build-arg", "CRDB_CA_URL=", "--build-arg", f"AUDIT_FEATURES={entry['audit_features'] or 'aws'}",
                   "--label", f"org.opencontainers.image.source=https://github.com/{identity['repository']}",
                   "--label", f"org.opencontainers.image.revision={identity['sha']}",
                   "--label", "org.opencontainers.image.licenses=BUSL-1.1"]
        if entry["layer_cache"]:
            cache = f"type=gha,version=2,scope=container-v1-{target}"
            command.extend(["--cache-from", cache, "--cache-to", f"{cache},mode=max"])
        command.append(entry["context"])
        try:
            run(command, env=env)
            if entry["workload"] == "signaling":
                run(["bash", "apps/backend/signaling/tests/image-smoke.sh", image, platform], env=env, timeout=120)
            strings = output / "strings"
            run([sys.executable, str(scripts / "container_publication.py"), "extract",
                 "--image", image, "--platform", platform, "--output-dir", str(strings)], env=env)
            for scan_type, source in (("image", image), ("fs", str(strings))):
                report = output / f"{scan_type}-secrets.json"
                result = run(["trivy", scan_type, "--scanners", "secret", "--exit-code", "1",
                              "--format", "json", "--output", str(report), source], env=env, check=False)
                if result.returncode:
                    summarize = [sys.executable, str(scripts / "container_publication.py"),
                                 "report-secrets", "--path", str(report)]
                    if scan_type == "fs":
                        summarize.append("--compiled-text")
                    run(summarize, env=env, check=False)
                    raise BuildError(f"{target}: {scan_type} secret scan failed")

            # Credentials are absent during builds and scanning. Every image
            # independently earns permission to authenticate and publish.
            try:
                run(["docker", "login", "ghcr.io", "--username", identity["actor"], "--password-stdin"],
                    env=env, input=token + "\n", capture=True, timeout=120)
                run(["docker", "push", image], env=env)
                raw = json.loads(run(["docker", "buildx", "imagetools", "inspect", image, "--raw"],
                                     env=env, capture=True, timeout=120).stdout)
                config = run(["docker", "image", "inspect", image, "--format", "{{.Id}}"],
                             env=env, capture=True, timeout=120).stdout.strip()
                if raw.get("config", {}).get("digest") != config or "manifests" in raw:
                    raise BuildError(f"{target}: pushed manifest differs from the scanned image")
                metadata = json.loads(run(["docker", "buildx", "imagetools", "inspect", image,
                                           "--format", "{{json .Manifest}}"], env=env, capture=True, timeout=120).stdout)
                record = container_images.record(target, identity["owner"], identity["sha"], metadata["digest"],
                                                  identity["run_id"], identity["attempt"])
                records_dir.mkdir(parents=True, exist_ok=True)
                (records_dir / f"{target}.json").write_text(json.dumps(record) + "\n", encoding="utf-8")
            finally:
                try:
                    run(["docker", "logout", "ghcr.io"], env=env, capture=True, timeout=120)
                except (BuildError, OSError, subprocess.SubprocessError) as error:
                    raise CredentialCleanupError("cannot clear registry credentials; stopping the batch") from error
        finally:
            # Keep shared BuildKit layers, but release each loaded image's disk.
            # Best-effort disk cleanup must not replace a credential cleanup error.
            try:
                run(["docker", "image", "rm", image], env=env, capture=True, check=False, timeout=120)
            except (BuildError, OSError, subprocess.SubprocessError):
                print(f"::warning::{target}: could not remove the local image", file=sys.stderr, flush=True)


def publish_batch(targets, identity, token, env, records_dir, temp_dir):
    entries = [container_images.TARGET_BY_ID[target] for target in targets]
    if not entries or len(set(targets)) != len(targets):
        raise ValueError("helper batch must contain distinct targets")
    if any(entry["workload"] not in container_images.HELPER_WORKLOADS for entry in entries):
        raise ValueError("helper batch contains a non-helper workload")
    if len({entry["platform"] for entry in entries}) != 1:
        raise ValueError("helper batch mixes architectures")
    failures = []
    for entry in entries:
        print(f"::group::Build and publish {entry['id']}", flush=True)
        try:
            build_one(entry, identity, token, env, records_dir, temp_dir)
        except CredentialCleanupError:
            raise
        except (BuildError, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
            failures.append(entry["id"])
            # Keep diagnostics free of scanner contents or process environments.
            message = str(error) if isinstance(error, BuildError) else type(error).__name__
            print(f"::error::{entry['id']}: {message}", file=sys.stderr, flush=True)
        finally:
            print("::endgroup::", flush=True)
    return failures


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--records-dir", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        env = dict(os.environ)
        token = env.pop("GITHUB_TOKEN")
        env.pop("GH_TOKEN", None)
        if not token:
            raise ValueError("publication token is missing")
        owner, _ = container_images.build_identity(env["OWNER"], env["GITHUB_SHA"],
                                                   env["GITHUB_RUN_ID"], env["GITHUB_RUN_ATTEMPT"])
        identity = {"owner": owner, "sha": env["GITHUB_SHA"], "run_id": env["GITHUB_RUN_ID"],
                    "attempt": env["GITHUB_RUN_ATTEMPT"], "repository": env["GITHUB_REPOSITORY"],
                    "actor": env["GITHUB_ACTOR"]}
        targets = json.loads(env["BATCH_TARGETS"])
        if not isinstance(targets, list) or not all(isinstance(target, str) for target in targets):
            raise ValueError("batch targets must be an array of IDs")
        failures = publish_batch(targets, identity, token, env, args.records_dir, env["RUNNER_TEMP"])
        if failures:
            print("Failed helper images: " + ", ".join(failures), file=sys.stderr)
            return 1
    except CredentialCleanupError as error:
        print(str(error), file=sys.stderr)
        return 1
    except (KeyError, ValueError, OSError) as error:
        print(f"Invalid helper build input: {type(error).__name__}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
