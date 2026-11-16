"""Artifact recovery must identify its original source and require completed builds."""

import copy
from datetime import datetime, timezone
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
from urllib.request import Request


SPEC = importlib.util.spec_from_file_location("release_reuse", Path(__file__).parents[1] / "standalone_release_reuse.py")
release_reuse = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release_reuse)
REPOSITORY = "Example/Flow-Like"
RUN_ID = 123456
SHA = "a" * 40
METADATA = {"source_sha": SHA, "packaging_sha": "c" * 40, "version": "1.2.3", "epoch": "1780000000"}
WORKFLOW = """env:
  CARGO_PROFILE_RELEASE_DEBUG: '0'

jobs:
  frontend:
    needs: prepare
    runs-on: frontend-runner
    steps:
      - run: bun run build:frontend

  binaries:
    needs: [prepare, frontend]
    runs-on: binary-runner
    env:
      CC: clang
    steps:
      - run: cargo build --release --features frontend

  runtime-packs:
    needs: prepare
    runs-on: runtime-runner
    steps:
      - run: build-runtime
"""


def run_fixture():
    return {
        "id": RUN_ID, "repository": {"id": 42, "full_name": REPOSITORY}, "head_repository": {"id": 42},
        "path": release_reuse.WORKFLOW, "head_branch": "dev", "event": "workflow_dispatch",
        "status": "completed", "conclusion": "failure", "head_sha": SHA, "run_attempt": 1,
    }


def job_fixture(name, steps):
    return {"name": name, "status": "completed", "conclusion": "success", "steps": [
        {"name": step, "status": "completed", "conclusion": "success"}
        for step in [*steps, "Run actions/upload-artifact@pinned"]
    ]}


def jobs_fixture():
    jobs = []
    for target in release_reuse.TARGETS:
        steps = ["Build the release binary and validate its runtime metadata"]
        if target.endswith("linux-gnu"):
            steps.append("Assemble Linux container runtime from the exact binary and resolved libraries")
        jobs.append(job_fixture(f"binaries ({target}, runner)", steps))
    for pack in release_reuse.PACKS:
        if "linux" in pack:
            step = "Build and check Linux packs in the frozen Ubuntu 22.04 environment"
        elif pack.startswith("mlx-"):
            step = "Build the MLX helper and pack it with its resources and Swift back-deployment libraries"
        else:
            step = "Pack llama.cpp after bundling, closure, glibc floor and start checks"
        jobs.append(job_fixture(f"runtime-packs ({pack}, runner)", [step]))
    return jobs


def artifacts_fixture():
    names = [f"standalone-{target}" for target in release_reuse.TARGETS]
    names += [f"runtime-pack-{pack}" for pack in release_reuse.PACKS]
    return [{
        "id": index + 1, "name": name, "size_in_bytes": 1234, "digest": "sha256:" + "b" * 64,
        "expired": False, "expires_at": "2100-01-01T00:00:00Z",
        "workflow_run": {"id": RUN_ID, "head_sha": SHA, "head_branch": "dev",
                         "repository_id": 42, "head_repository_id": 42},
    } for index, name in enumerate(names)]


class SourceRunTests(unittest.TestCase):
    def test_accepts_a_failed_release_when_its_original_builds_completed(self):
        self.assertEqual(release_reuse.validate_run(run_fixture(), REPOSITORY.lower(), RUN_ID), SHA)
        release_reuse.validate_jobs(jobs_fixture())
        selected = release_reuse.select_artifacts(artifacts_fixture(), run_fixture())
        self.assertEqual(len(selected), 8)

    def test_refuses_other_repositories_forks_workflows_branches_events_and_incomplete_runs(self):
        changes = [
            {"repository": {"id": 42, "full_name": "Other/Flow-Like"}},
            {"head_repository": {"id": 43}}, {"id": RUN_ID + 1},
            {"path": ".github/workflows/pull-request.yml"}, {"head_branch": "feature"},
            {"event": "pull_request"}, {"status": "in_progress"}, {"head_sha": "dev"},
        ]
        for change in changes:
            with self.subTest(change=change), self.assertRaises(ValueError):
                release_reuse.validate_run({**run_fixture(), **change}, REPOSITORY, RUN_ID)

    def test_each_binary_and_runtime_build_must_finish_successfully_and_upload(self):
        for index in range(8):
            for failure in ("missing", "failed", "skipped", "missing-build", "missing-upload", "duplicate"):
                with self.subTest(index=index, failure=failure):
                    jobs = jobs_fixture()
                    if failure == "missing":
                        del jobs[index]
                    elif failure == "duplicate":
                        jobs.append(copy.deepcopy(jobs[index]))
                    elif failure == "failed":
                        jobs[index]["conclusion"] = "failure"
                    elif failure == "skipped":
                        jobs[index]["steps"][0]["conclusion"] = "skipped"
                    elif failure == "missing-build":
                        jobs[index]["steps"] = jobs[index]["steps"][-1:]
                    else:
                        jobs[index]["steps"].pop()
                    with self.assertRaises(ValueError):
                        release_reuse.validate_jobs(jobs)

    def test_missing_duplicate_expired_or_unbound_artifacts_are_rejected(self):
        modifications = [
            ("expired", True), ("expires_at", "2000-01-01T00:00:00Z"),
            ("expires_at", "2100-01-01T00:00:00"), ("expires_at", "invalid"),
            ("id", 0), ("id", True), ("size_in_bytes", 0),
            ("digest", None), ("digest", "sha256:unverified"),
            ("workflow_run", {**artifacts_fixture()[0]["workflow_run"], "head_sha": "c" * 40}),
            ("workflow_run", {**artifacts_fixture()[0]["workflow_run"], "head_repository_id": 99}),
        ]
        for field, value in modifications:
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                artifacts = artifacts_fixture()
                artifacts[0][field] = value
                release_reuse.select_artifacts(artifacts, run_fixture())
        for artifacts in (artifacts_fixture()[1:], artifacts_fixture() + artifacts_fixture()[:1]):
            with self.assertRaisesRegex(ValueError, "exactly one"):
                release_reuse.select_artifacts(artifacts, run_fixture())
        artifacts = artifacts_fixture()
        artifacts[1]["id"] = artifacts[0]["id"]
        with self.assertRaisesRegex(ValueError, "distinct"):
            release_reuse.select_artifacts(artifacts, run_fixture())

    def test_expiry_is_checked_even_before_github_marks_the_archive_expired(self):
        with self.assertRaisesRegex(ValueError, "expired"):
            release_reuse.select_artifacts(artifacts_fixture(), run_fixture(), datetime(2100, 1, 1, tzinfo=timezone.utc))

    def test_reuse_outputs_immutable_ids_and_the_original_build_epoch(self):
        api = release_reuse.GitHub(REPOSITORY, "test-token")
        with patch.object(api, "get", return_value=run_fixture()), \
                patch.object(api, "collection", side_effect=[jobs_fixture(), artifacts_fixture()]), \
                patch.object(release_reuse, "source_metadata", return_value=METADATA) as epoch:
            outputs = release_reuse.reuse(api, RUN_ID, Path("/checkout"), "1.2.3")
        self.assertEqual(outputs, {
            "reuse_run_id": str(RUN_ID), "binary_artifact_ids": "1,2,3", "linux_artifact_ids": "1,2",
            "runtime_artifact_ids": "4,5,6,7,8", **METADATA,
        })
        epoch.assert_called_once_with(SHA, Path("/checkout"), REPOSITORY)

    def test_summary_identifies_source_and_each_selected_artifact(self):
        api = release_reuse.GitHub(REPOSITORY, "test-token")
        with tempfile.TemporaryDirectory() as folder:
            summary = Path(folder) / "summary"
            with patch.object(api, "get", return_value=run_fixture()), \
                    patch.object(api, "collection", side_effect=[jobs_fixture(), artifacts_fixture()]), \
                    patch.object(release_reuse, "source_metadata", return_value=METADATA):
                release_reuse.reuse(api, RUN_ID, Path("/checkout"), "1.2.3", summary)
            text = summary.read_text()
        self.assertIn(f"https://github.com/{REPOSITORY}/actions/runs/{RUN_ID}", text)
        self.assertIn(SHA, text)
        self.assertIn("original build epoch `1780000000`", text)
        self.assertIn("still gate publication", text)
        self.assertIn("Binaries, embedded frontend and runtime packs remain at source commit", text)
        self.assertIn("Later source changes are not included", text)
        self.assertIn(METADATA["packaging_sha"], text)
        self.assertIn("version `1.2.3`", text)
        for artifact in artifacts_fixture():
            self.assertIn(f"| {artifact['name']} | {artifact['id']} |", text)

    def test_mismatched_agent_version_stops_before_writing_summary_or_outputs(self):
        api = release_reuse.GitHub(REPOSITORY, "test-token")
        with tempfile.TemporaryDirectory() as folder:
            summary = Path(folder) / "summary"
            with patch.object(api, "get", return_value=run_fixture()) as get, \
                    patch.object(api, "collection", side_effect=[jobs_fixture(), artifacts_fixture()]), \
                    patch.object(release_reuse, "source_metadata", return_value=METADATA), \
                    self.assertRaisesRegex(ValueError, "agent version 1.2.3.*expects 2.0.0"):
                release_reuse.reuse(api, RUN_ID, Path("/checkout"), "2.0.0", summary)
            self.assertFalse(summary.exists())
            self.assertEqual(get.call_count, 1)

    def test_a_source_rerun_during_validation_is_rejected(self):
        api = release_reuse.GitHub(REPOSITORY, "test-token")
        with patch.object(api, "get", side_effect=[run_fixture(), {**run_fixture(), "run_attempt": 2}]), \
                patch.object(api, "collection", side_effect=[jobs_fixture(), artifacts_fixture()]), \
                patch.object(release_reuse, "source_metadata", return_value=METADATA), \
                self.assertRaisesRegex(ValueError, "changed"):
            release_reuse.reuse(api, RUN_ID, Path("/checkout"), "1.2.3")


class GitInputTests(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.root = Path(folder.name)
        self.git("init", "--quiet")
        self.git("config", "user.name", "Release Tests")
        self.git("config", "user.email", "release-tests@example.invalid")
        self.git("config", "commit.gpgsign", "false")
        self.git("config", "core.hooksPath", "/dev/null")
        self.write("apps/standalone/Cargo.toml", '[package]\nname = "flow-like-standalone"\nversion = "1.2.3"\n')
        self.write("Cargo.lock", 'version = 4\n[[package]]\nname = "flow-like-standalone"\nversion = "1.2.3"\n')
        self.write("apps/standalone/src/main.rs", "fn main() {}\n")
        self.write("apps/standalone/packaging/Dockerfile.playground", "FROM debian\n")
        self.write(release_reuse.WORKFLOW, WORKFLOW)
        self.commit()
        self.sha = self.git("rev-parse", "HEAD").strip()
        self.epoch = self.git("show", "-s", "--format=%ct", "HEAD").strip()

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.root, text=True, stderr=subprocess.PIPE)

    def write(self, name, contents):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(contents)

    def commit(self):
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "fixture")

    def test_current_source_workflow_and_version_changes_keep_the_selected_build_metadata(self):
        changes = {
            "apps/standalone/packaging/Dockerfile.playground": "FROM debian\nRUN install-egl\n",
            "apps/standalone/src/main.rs": "fn main() { changed(); }\n",
            "packages/ui/components/interfaces/chat-default/chat.tsx": "new chat behavior\n",
            "packages/locales/locales/en/bots.json": '{"bots": "updated"}\n',
            "apps/standalone/Cargo.toml": '[package]\nversion = "9.9.9"\n',
            "Cargo.lock": 'version = 4\n[[package]]\nname = "flow-like-standalone"\nversion = "9.9.9"\n',
            release_reuse.WORKFLOW: WORKFLOW.replace("--features frontend", "--features frontend,experimental"),
        }
        for name, contents in changes.items():
            self.write(name, contents)
        self.commit()
        current = self.git("rev-parse", "HEAD").strip()
        self.write("Cargo.lock", "an unrelated worktree edit\n")
        self.assertEqual(release_reuse.source_metadata(self.sha, self.root, REPOSITORY), {
            "source_sha": self.sha, "packaging_sha": current, "version": "1.2.3", "epoch": self.epoch,
        })

    def test_source_version_must_be_a_bounded_literal(self):
        manifests = ['invalid = [', '[package]\n', 'version = "1.2.3"\n']
        manifests += [f'[package]\nname = "flow-like-standalone"\nversion = {json.dumps(value)}\n'
                      for value in ("", "1.2.3\nspoofed=value", "x" * 65, "1.2.3/path", 123)]
        manifests += ['[package]\nname = "flow-like-standalone"\n[package.version]\nworkspace = true\n',
                      '[package]\nname = "other"\nversion = "1.2.3"\n']
        for manifest in manifests:
            with self.subTest(manifest=manifest):
                self.write("apps/standalone/Cargo.toml", manifest)
                self.commit()
                invalid_sha = self.git("rev-parse", "HEAD").strip()
                with self.assertRaisesRegex(ValueError, "Source"):
                    release_reuse.source_metadata(invalid_sha, self.root, REPOSITORY)
                self.git("reset", "--hard", self.sha)

    def test_source_lock_must_match_the_agent_version_once(self):
        entry = '[[package]]\nname = "flow-like-standalone"\nversion = "1.2.3"\n'
        locks = ["invalid = [", "version = 4\n", "[package]\n", entry + entry,
                 entry.replace("1.2.3", "1.2.4"), entry.replace("flow-like-standalone", "other"),
                 entry + 'source = "registry+https://example.invalid"\n']
        for lock in locks:
            with self.subTest(lock=lock):
                self.write("Cargo.lock", lock)
                self.commit()
                invalid_sha = self.git("rev-parse", "HEAD").strip()
                with self.assertRaisesRegex(ValueError, "Source"):
                    release_reuse.source_metadata(invalid_sha, self.root, REPOSITORY)
                self.git("reset", "--hard", self.sha)

    def test_source_metadata_never_falls_back_to_files_from_current_checkout(self):
        (self.root / "apps/standalone/Cargo.toml").unlink()
        self.commit()
        missing_sha = self.git("rev-parse", "HEAD").strip()
        self.git("reset", "--hard", self.sha)
        with self.assertRaisesRegex(ValueError, "Source commit must contain valid"):
            release_reuse.source_metadata(missing_sha, self.root, REPOSITORY)

    def test_source_sha_is_validated_before_running_git(self):
        for sha in ("dev", "HEAD", "--help", "a" * 39, "a" * 40 + "\n"):
            with self.subTest(sha=sha), patch.object(release_reuse.subprocess, "run") as run, \
                    self.assertRaisesRegex(ValueError, "exact Git commit"):
                release_reuse.source_metadata(sha, self.root, REPOSITORY)
            run.assert_not_called()


class GitHubMetadataTests(unittest.TestCase):
    def test_request_uses_fixed_api_host_and_keeps_the_token_in_the_header(self):
        api = release_reuse.GitHub(REPOSITORY, "test-token")
        with patch.object(api.opener, "open", return_value=io.BytesIO(json.dumps(run_fixture()).encode())) as opened:
            self.assertEqual(api.get(f"actions/runs/{RUN_ID}")["id"], RUN_ID)
        request = opened.call_args.args[0]
        self.assertEqual(request.full_url, f"https://api.github.com/repos/{REPOSITORY}/actions/runs/{RUN_ID}")
        self.assertEqual(request.get_header("Authorization"), "Bearer test-token")
        with self.assertRaisesRegex(ValueError, "redirect"):
            release_reuse.NoRedirects().redirect_request(Request(request.full_url), None, 302, "", {}, "https://other.invalid")

    def test_collections_include_all_pages_and_reject_changing_or_incomplete_results(self):
        api = release_reuse.GitHub(REPOSITORY, "test-token")
        first = {"total_count": 101, "jobs": [{"id": index} for index in range(100)]}
        last = {"total_count": 101, "jobs": [{"id": 100}]}
        with patch.object(api, "get", side_effect=[first, last]) as get:
            self.assertEqual(len(api.collection("actions/runs/1/jobs?filter=latest", "jobs")), 101)
        self.assertEqual(get.call_args.args[0], "actions/runs/1/jobs?filter=latest&per_page=100&page=2")
        for pages in ([first, {**last, "total_count": 102}], [{"total_count": 2, "jobs": [{"id": 1}]}]):
            with patch.object(api, "get", side_effect=pages), self.assertRaises(ValueError):
                api.collection("actions/runs/1/jobs", "jobs")

    def test_metadata_size_and_repository_path_are_bounded(self):
        for repository in ("owner/repo/extra", "https://other.invalid", "owner/repo?token=secret"):
            with self.assertRaises(ValueError):
                release_reuse.GitHub(repository, "test-token")
        api = release_reuse.GitHub(REPOSITORY, "test-token")
        with patch.object(api.opener, "open", return_value=io.BytesIO(b"12345")), \
                patch.object(release_reuse, "MAX_RESPONSE_BYTES", 4), self.assertRaisesRegex(ValueError, "size limit"):
            api.get("actions/runs/1")


if __name__ == "__main__":
    unittest.main()
