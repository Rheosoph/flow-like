"""Validate completed release builds before reusing their artifacts in a new sequence."""

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import subprocess
from urllib.error import HTTPError
from urllib.request import HTTPRedirectHandler, Request, build_opener


WORKFLOW = ".github/workflows/standalone-release.yml"
TARGETS = ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "aarch64-apple-darwin")
PACKS = ("llamacpp-macos-arm64-metal", "llamacpp-linux-x64-cpu", "llamacpp-linux-x64-vulkan",
         "llamacpp-linux-arm64-cpu", "mlx-macos-arm64-metal")
# These files change packaging and its validation without changing compiled inputs.
PACKAGING_CHANGES = frozenset({
    WORKFLOW,
    "apps/standalone/packaging/Dockerfile.playground",
    ".github/scripts/check-standalone-container-deps.sh",
    ".github/scripts/standalone_release_reuse.py",
    ".github/scripts/tests/test_standalone_release_workflow.py",
    ".github/scripts/tests/test_standalone_release_reuse.py",
    ".github/scripts/tests/test_runtime_packs.py",
})
MAX_RESPONSE_BYTES = 8 * 1024 * 1024


class NoRedirects(HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, newurl):
        raise ValueError("GitHub release metadata must not redirect")


class GitHub:
    def __init__(self, repository, token):
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
            raise ValueError("GITHUB_REPOSITORY must identify one owner and repository")
        if not token:
            raise ValueError("Artifact reuse requires GITHUB_TOKEN with actions: read")
        self.repository, self.token = repository, token
        self.opener = build_opener(NoRedirects)

    def get(self, path):
        request = Request(f"https://api.github.com/repos/{self.repository}/{path}", headers={
            "Authorization": f"Bearer {self.token}", "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "flow-like-release-reuse",
        })
        try:
            with self.opener.open(request, timeout=60) as response:
                body = response.read(MAX_RESPONSE_BYTES + 1)
        except HTTPError as error:
            raise ValueError(f"Cannot read GitHub release metadata (HTTP {error.code}); check actions: read access") from None
        if len(body) > MAX_RESPONSE_BYTES:
            raise ValueError("GitHub release metadata exceeds its size limit")
        value = json.loads(body)
        if not isinstance(value, dict):
            raise ValueError("Expected an object from GitHub release metadata")
        return value

    def collection(self, path, field):
        values, expected = [], None
        separator = "&" if "?" in path else "?"
        for page in range(1, 101):
            response = self.get(f"{path}{separator}per_page=100&page={page}")
            count, batch = response.get("total_count"), response.get(field)
            if (type(count) is not int or not 0 <= count <= 10000 or not isinstance(batch, list)
                    or len(batch) > 100 or any(not isinstance(item, dict) for item in batch)):
                raise ValueError(f"Invalid GitHub {field} collection")
            if expected is not None and count != expected:
                raise ValueError(f"Source run {field} changed while checking reuse")
            expected = count
            values.extend(batch)
            if len(values) == expected:
                return values
            if len(values) > expected or len(batch) < 100:
                break
        raise ValueError(f"Incomplete GitHub {field} collection")


def validate_run(run, repository, run_id):
    source_repo, head_repo = run.get("repository") or {}, run.get("head_repository") or {}
    if (run.get("id") != run_id or source_repo.get("full_name", "").lower() != repository.lower()
            or type(source_repo.get("id")) is not int or head_repo.get("id") != source_repo["id"]):
        raise ValueError("Source run must belong to this repository and cannot come from a fork")
    if (run.get("path") != WORKFLOW or run.get("head_branch") != "dev"
            or run.get("event") != "workflow_dispatch" or run.get("status") != "completed"):
        raise ValueError("Reuse requires a completed standalone release dispatched on dev")
    sha = run.get("head_sha")
    if not isinstance(sha, str) or not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("Source run must identify an exact Git commit")
    return sha


def validate_jobs(jobs):
    requirements = []
    for target in TARGETS:
        steps = ["Build the release binary and validate its runtime metadata"]
        if target.endswith("linux-gnu"):
            steps.append("Assemble Linux container runtime from the exact binary and resolved libraries")
        requirements.append((f"binaries ({target}, ", steps))
    for pack in PACKS:
        if "linux" in pack:
            step = "Build and check Linux packs in the frozen Ubuntu 22.04 environment"
        elif pack.startswith("mlx-"):
            step = "Build the MLX helper and pack it with its resources and Swift back-deployment libraries"
        else:
            step = "Pack llama.cpp after bundling, closure, glibc floor and start checks"
        requirements.append((f"runtime-packs ({pack}, ", [step]))
    for prefix, required in requirements:
        matches = [job for job in jobs if isinstance(job.get("name"), str) and job["name"].startswith(prefix)]
        if len(matches) != 1 or matches[0].get("status") != "completed" or matches[0].get("conclusion") != "success":
            raise ValueError(f"Source run must have one successful build job for {prefix.rstrip(', ')}")
        steps = matches[0].get("steps") or []
        passed = {step.get("name") for step in steps
                  if step.get("status") == "completed" and step.get("conclusion") == "success"}
        if not set(required) <= passed or not any(isinstance(name, str) and name.startswith("Run actions/upload-artifact@") for name in passed):
            raise ValueError(f"Source job {matches[0]['name']} did not build and upload its artifacts; reuse an original build run")


def select_artifacts(artifacts, run, now=None):
    now = now or datetime.now(timezone.utc)
    names = [f"standalone-{target}" for target in TARGETS] + [f"runtime-pack-{pack}" for pack in PACKS]
    selected = {}
    for name in names:
        matches = [artifact for artifact in artifacts if artifact.get("name") == name]
        if len(matches) != 1:
            raise ValueError(f"Source run must contain exactly one {name} artifact")
        artifact = matches[0]
        source = artifact.get("workflow_run") or {}
        if (source.get("id") != run["id"] or source.get("head_sha") != run["head_sha"]
                or source.get("head_branch") != "dev" or source.get("repository_id") != run["repository"]["id"]
                or source.get("head_repository_id") != run["repository"]["id"]):
            raise ValueError(f"Artifact {name} does not belong to the validated source commit")
        try:
            expires = datetime.fromisoformat(artifact["expires_at"].replace("Z", "+00:00"))
        except (KeyError, TypeError, ValueError, AttributeError):
            raise ValueError(f"Artifact {name} has no valid expiry") from None
        if artifact.get("expired") is not False or expires.tzinfo is None or expires <= now:
            raise ValueError(f"Artifact {name} has expired; build a fresh release")
        if (type(artifact.get("id")) is not int or artifact["id"] <= 0
                or type(artifact.get("size_in_bytes")) is not int or artifact["size_in_bytes"] <= 0
                or not isinstance(artifact.get("digest"), str)
                or not re.fullmatch(r"sha256:[0-9a-f]{64}", artifact["digest"])):
            raise ValueError(f"Artifact {name} must have an immutable ID, nonempty archive and SHA256 digest")
        selected[name] = artifact["id"]
    if len(set(selected.values())) != len(selected):
        raise ValueError("Source artifact IDs must be distinct")
    return selected


def workflow_build_inputs(text):
    environment = re.search(r"^env:\n(.*?)(?=^[^ \n]|\Z)", text, re.MULTILINE | re.DOTALL)
    if environment is None:
        raise ValueError("Release workflow must retain its global build environment")
    inputs = {"env": environment.group(1)}
    for name in ("frontend", "binaries", "runtime-packs"):
        job = re.search(rf"^  {name}:\n(.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)", text, re.MULTILINE | re.DOTALL)
        if job is None:
            raise ValueError(f"Release workflow must retain the {name} build job")
        # Reuse changes job scheduling; runner, toolchain, flags and steps remain build inputs.
        inputs[name] = re.sub(r"^    (?:needs|if):[^\n]*\n", "", job.group(1), flags=re.MULTILINE)
    return inputs


def validate_workflow_build_inputs(source, current):
    before, after = workflow_build_inputs(source), workflow_build_inputs(current)
    changed = [name for name in before if before[name] != after[name]]
    if changed:
        raise ValueError("Release workflow build inputs differ from the source run; start a fresh build: " + ", ".join(changed))


def source_epoch(sha, directory, repository):
    exists = subprocess.run(["git", "cat-file", "-e", f"{sha}^{{commit}}"], cwd=directory,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    if exists.returncode:
        subprocess.run(["git", "-c", "credential.helper=", "-c", "http.extraHeader=", "fetch",
                        "--no-tags", "--depth=1", f"https://github.com/{repository}.git", sha],
                       cwd=directory, env={**os.environ, "GIT_TERMINAL_PROMPT": "0"}, check=True, timeout=300)
    changed = subprocess.check_output(
        ["git", "diff", "--no-ext-diff", "--name-only", "-z", "--no-renames", sha, "HEAD", "--"], cwd=directory,
    ).decode().split("\0")
    refused = sorted(set(changed) - PACKAGING_CHANGES - {""})
    if refused:
        raise ValueError("Compiled inputs differ from the source run; start a fresh build: " + ", ".join(refused))
    if WORKFLOW in changed:
        try:
            workflows = [subprocess.check_output(["git", "show", f"{ref}:{WORKFLOW}"], cwd=directory, text=True)
                         for ref in (sha, "HEAD")]
        except subprocess.CalledProcessError:
            raise ValueError("Both release commits must contain the release workflow") from None
        validate_workflow_build_inputs(*workflows)
    epoch = subprocess.check_output(["git", "show", "-s", "--format=%ct", sha], cwd=directory, text=True).strip()
    if not re.fullmatch(r"[0-9]+", epoch):
        raise ValueError("Source commit has no valid build epoch")
    return epoch


def reuse(api, run_id, directory, summary=None):
    run = api.get(f"actions/runs/{run_id}")
    sha = validate_run(run, api.repository, run_id)
    validate_jobs(api.collection(f"actions/runs/{run_id}/jobs?filter=latest", "jobs"))
    artifacts = select_artifacts(api.collection(f"actions/runs/{run_id}/artifacts", "artifacts"), run)
    epoch = source_epoch(sha, directory, api.repository)
    latest = api.get(f"actions/runs/{run_id}")
    if validate_run(latest, api.repository, run_id) != sha or latest.get("run_attempt") != run.get("run_attempt"):
        raise ValueError("Source run changed while checking reuse")
    binary_ids = [str(artifacts[f"standalone-{target}"]) for target in TARGETS]
    if summary:
        rows = ["### Reused standalone build artifacts", "",
                f"Source: [run {run_id}](https://github.com/{api.repository}/actions/runs/{run_id}), commit `{sha}`.", "",
                f"The original build epoch `{epoch}` is retained. New containers, worker checks and signed manifests still gate publication.", "",
                "| Artifact | ID |", "| --- | --- |"]
        rows.extend(f"| {name} | {artifact_id} |" for name, artifact_id in artifacts.items())
        with Path(summary).open("a") as output:
            output.write("\n".join(rows) + "\n")
    return {
        "reuse_run_id": str(run_id), "binary_artifact_ids": ",".join(binary_ids),
        "linux_artifact_ids": ",".join(binary_ids[:2]),
        "runtime_artifact_ids": ",".join(str(artifacts[f"runtime-pack-{pack}"]) for pack in PACKS),
        "epoch": epoch,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-id", required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"[1-9][0-9]{0,19}", args.run_id):
        parser.error("--run-id must be a positive GitHub Actions run ID")
    api = GitHub(os.environ["GITHUB_REPOSITORY"], os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN"))
    outputs = reuse(api, int(args.run_id), Path.cwd(), os.environ.get("GITHUB_STEP_SUMMARY"))
    for name, value in outputs.items():
        print(f"{name}={value}")


if __name__ == "__main__":
    main()
