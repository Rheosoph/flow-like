"""Exercise the release workflow's build and runtime validation sequence, and the renewal workflow."""

import json
import os
from pathlib import Path
import re
import struct
import subprocess
import sys
import tempfile
import textwrap
import tomllib
import unittest


REPOSITORY = Path(__file__).resolve().parents[3]
WORKFLOW = REPOSITORY / ".github/workflows/standalone-release.yml"
RENEWAL = REPOSITORY / ".github/workflows/standalone-release-renew.yml"
TARGET = "aarch64-apple-darwin"
BASE = "https://cdn.example/standalone"
KEYS = json.dumps(["A" * 43])


def job_body(name, workflow=WORKFLOW):
    match = re.search(
        rf"^  {re.escape(name)}:\n(.*?)(?=^  [a-z][a-z-]*:\n|\Z)",
        workflow.read_text(), re.MULTILINE | re.DOTALL,
    )
    if not match:
        raise AssertionError(f"Missing workflow job: {name}")
    return match.group(1)


def step(job, start, workflow=WORKFLOW):
    for text in re.split(r"^      - ", job_body(job, workflow), flags=re.MULTILINE):
        if text.startswith(start + "\n"):
            return text
    raise AssertionError(f"Missing workflow step: {start}")


def script(text):
    return textwrap.dedent(text.split("        run: |\n", 1)[1])


def build_script():
    return script(step("binaries", "name: Build the release binary and validate its runtime metadata"))


def run_script(text, directory, environment):
    return subprocess.run(
        ["bash", "--noprofile", "--norc", "-eo", "pipefail", "-c", text],
        cwd=directory, env={**os.environ, **environment}, capture_output=True, text=True, check=False,
    )


def triggers(workflow):
    block = re.search(r"^on:\n(.*?)(?=^\S)", workflow.read_text(), re.MULTILINE | re.DOTALL).group(1)
    return re.findall(r"^  ([a-z_]+):", block, re.MULTILINE)


def allows_job(name, results, reuse_run_id="", cancelled=False):
    """Evaluate the release's job gates with explicit dependency outcomes."""
    body = job_body(name)
    match = re.search(r"^    if: (.*?)(?=^    [a-z]|\Z)", body, re.MULTILINE | re.DOTALL)
    condition = match.group(1).strip()
    if condition.startswith(">-"):
        condition = condition[2:].strip()
    condition = condition.removeprefix("${{").removesuffix("}}").strip()
    condition = condition.replace("needs.prepare.outputs.reuse_run_id", repr(reuse_run_id))
    condition = re.sub(r"needs\.([a-z-]+)\.result", lambda value: repr(results[value[1]]), condition)
    condition = condition.replace("!cancelled()", repr(not cancelled))
    condition = condition.replace("&&", " and ").replace("||", " or ")
    condition = " ".join(condition.split())
    return eval(condition, {"__builtins__": {}}, {})


class StandaloneReleaseWorkflowTests(unittest.TestCase):
    def test_container_dependencies_gate_every_expensive_build(self):
        check = job_body("container-check")
        self.assertIn("needs: prepare", check)
        self.assertIn("runs-on: namespace-profile-linux-amd-small-24", check)
        self.assertIn("check-standalone-container-deps.sh linux/amd64,linux/arm64", check)
        for job in ["frontend", "runtime-packs"]:
            self.assertIn("needs: [prepare, container-check]", job_body(job))
        self.assertIn("needs: [prepare, frontend]", job_body("binaries"))

    def test_artifact_reuse_skips_compilation_and_keeps_all_release_gates(self):
        results = dict.fromkeys([
            "prepare", "container-check", "binaries", "runtime-packs", "container",
            "worker-smoke", "signed-bundle", "runtime-manifest", "runtime-publish",
        ], "success")
        downstream = ["container", "worker-smoke", "signed-bundle", "runtime-manifest", "runtime-publish", "publish"]
        for reuse in ["", "43471505292"]:
            for job in ["frontend", "binaries", "runtime-packs"]:
                self.assertEqual(allows_job(job, results, reuse), not bool(reuse))
            results.update(binaries="skipped" if reuse else "success",
                           **{"runtime-packs": "skipped" if reuse else "success"})
            for job in downstream:
                with self.subTest(reuse=reuse, job=job):
                    self.assertTrue(allows_job(job, results, reuse))
                    self.assertFalse(allows_job(job, results, reuse, cancelled=True))
                    # Explicit status checks allow skipped build ancestors only in reuse mode.
                    self.assertIn("!cancelled()", job_body(job))
        for job, prerequisite in [
            ("container", "prepare"), ("container", "container-check"), ("container", "binaries"),
            ("worker-smoke", "container"), ("signed-bundle", "worker-smoke"),
            ("runtime-manifest", "container-check"), ("runtime-manifest", "runtime-packs"),
            ("runtime-publish", "runtime-manifest"), ("publish", "signed-bundle"),
            ("publish", "runtime-publish"),
        ]:
            for outcome in ["failure", "cancelled"]:
                with self.subTest(job=job, prerequisite=prerequisite, outcome=outcome):
                    self.assertFalse(allows_job(job, {**results, prerequisite: outcome}, "43471505292"))
        for job in ["container", "signed-bundle", "runtime-manifest"]:
            self.assertFalse(allows_job(job, results), "Skipped builds require validated reuse")

    def test_recovery_downloads_only_the_validated_artifact_ids_from_the_source_run(self):
        self.assertIn("actions: read", WORKFLOW.read_text().split("jobs:")[0])
        validate = step("prepare", "id: reuse")
        self.assertIn("--run-id \"$RELEASE_REUSE_RUN_ID\"", validate)
        for job, output in [("container", "linux_artifact_ids"), ("signed-bundle", "binary_artifact_ids"),
                            ("runtime-manifest", "runtime_artifact_ids")]:
            download = step(job, "uses: actions/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093")
            self.assertIn(f"artifact-ids: ${{{{ needs.prepare.outputs.{output} }}}}", download)
            self.assertIn("run-id: ${{ needs.prepare.outputs.reuse_run_id || github.run_id }}", download)
            self.assertIn("github-token: ${{ github.token }}", download)

    def test_incomplete_or_duplicate_runtime_downloads_cannot_reach_signing(self):
        check = step("runtime-manifest", "name: Require the complete runtime pack set before signing")
        job = job_body("runtime-manifest")
        self.assertLess(job.index(check), job.index("name: Build the runtime manifest signer"))
        packs = [
            {"runtime": "llamacpp", "target": "aarch64-apple-darwin", "backend": "metal"},
            {"runtime": "llamacpp", "target": "x86_64-unknown-linux-gnu", "backend": "cpu"},
            {"runtime": "llamacpp", "target": "x86_64-unknown-linux-gnu", "backend": "vulkan"},
            {"runtime": "llamacpp", "target": "aarch64-unknown-linux-gnu", "backend": "cpu"},
            {"runtime": "mlx", "target": "aarch64-apple-darwin", "backend": "metal"},
        ]
        with tempfile.TemporaryDirectory() as folder:
            directory = Path(folder)
            (directory / "runtime-bundle").mkdir()
            for entries in [packs, [], *[packs[:i] + packs[i + 1:] for i in range(len(packs))], packs + packs[:1]]:
                (directory / "runtime-bundle/test.json").write_text(json.dumps({"packs": entries}))
                result = run_script(script(check), directory, {"PYTHONPATH": str(REPOSITORY / ".github/scripts")})
                self.assertEqual(result.returncode == 0, entries == packs, result.stdout + result.stderr)

    def test_native_worker_smokes_gate_signing_the_exact_published_digest(self):
        job = job_body("worker-smoke")
        self.assertIn("needs: [prepare, container]", job)
        for platform, runner in [("linux/amd64", "namespace-profile-linux-amd-large-24"),
                                 ("linux/arm64", "namespace-profile-linux-arm-large-24")]:
            self.assertIn(f"platform: {platform}\n            runner: {runner}", job)
        self.assertIn("runs-on: ${{ matrix.runner }}", job)
        self.assertIn("RELEASE_IMAGE: ${{ needs.prepare.outputs.image }}@${{ needs.container.outputs.digest }}", job)
        self.assertIn('standalone_worker_smoke.py --image "$RELEASE_IMAGE" --platform "$RELEASE_PLATFORM"', job)
        self.assertNotIn("STANDALONE_RELEASE_SIGNING_KEY", job)
        self.assertIn("needs: [prepare, binaries, container, worker-smoke]", job_body("signed-bundle"))
        self.assertIn("test_standalone_release*.py", job_body("prepare"))
        self.assertTrue(Path(__file__).with_name("test_standalone_release_worker.py").is_file())

    def run_build(self, failure=""):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        directory = Path(folder.name)
        commands = directory / "commands"
        commands.mkdir()
        shim = f"#!{sys.executable}\n" + textwrap.dedent('''\
            import json
            import os
            from pathlib import Path
            import sys

            operation = "cargo" if Path(sys.argv[0]).name == "cargo" else "describe"
            with open(os.environ["COMMAND_LOG"], "a") as log:
                log.write(json.dumps([operation, sys.argv[1:]]) + "\\n")
            if os.environ["SIMULATE_FAILURE"] == operation:
                sys.exit(7)
            if operation == "cargo":
                binary = Path(os.environ["CARGO_TARGET_DIR"]) / os.environ["RELEASE_TARGET"] / "release/flow-like-standalone"
                binary.parent.mkdir(parents=True)
                binary.write_bytes(b"release binary")
            else:
                assert sys.argv[2] == "describe", sys.argv
                binary = Path(sys.argv[sys.argv.index("--binary") + 1])
                assert binary.read_bytes() == b"release binary"
                output = Path(sys.argv[sys.argv.index("--output") + 1])
                output.mkdir()
                (output / ("flow-like-standalone-" + os.environ["RELEASE_TARGET"])).write_bytes(binary.read_bytes())
            ''')
        for name in ["cargo", "python3"]:
            executable = commands / name
            executable.write_text(shim)
            executable.chmod(0o755)
        command_log = directory / "commands.jsonl"
        result = subprocess.run(
            ["bash", "--noprofile", "--norc", "-eo", "pipefail", "-c", build_script()],
            env={
                **os.environ, "PATH": str(commands) + os.pathsep + os.environ["PATH"],
                "RUNNER_TEMP": str(directory), "RELEASE_TARGET": TARGET,
                "CARGO_TARGET_DIR": str(directory / "target"),
                "COMMAND_LOG": str(command_log), "SIMULATE_FAILURE": failure,
            },
            capture_output=True, text=True,
        )
        calls = [json.loads(line) for line in command_log.read_text().splitlines()]
        return result, directory / "release-artifacts", calls

    def test_builds_once_then_validates_the_result_before_preparing_artifacts(self):
        result, artifacts, calls = self.run_build()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([operation for operation, _ in calls], ["cargo", "describe"])
        self.assertEqual(calls[0][1], [
            "build", "--locked", "--release", "--package", "flow-like-standalone",
            "--features", "frontend", "--target", TARGET,
        ])
        self.assertEqual((artifacts / f"flow-like-standalone-{TARGET}").read_bytes(), b"release binary")

    def test_failed_compilation_stops_before_runtime_validation_or_artifact_preparation(self):
        result, artifacts, calls = self.run_build(failure="cargo")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual([operation for operation, _ in calls], ["cargo"])
        self.assertFalse(artifacts.exists())

    def test_failed_runtime_validation_never_prepares_artifacts(self):
        result, artifacts, calls = self.run_build(failure="describe")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual([operation for operation, _ in calls], ["cargo", "describe"])
        self.assertFalse(artifacts.exists())

    def test_macos_build_uses_a_static_onnx_archive_for_apple_silicon(self):
        setup = step("binaries", "name: Configure macOS ONNX Runtime")
        self.assertIn("if: runner.os == 'macOS'", setup)
        job = job_body("binaries")
        self.assertLess(job.index(setup), job.index("name: Restore compiled release dependencies"))
        with tempfile.TemporaryDirectory() as folder:
            environment_file = Path(folder) / "environment"
            result = run_script(script(setup), REPOSITORY, {
                "GITHUB_WORKSPACE": str(REPOSITORY), "GITHUB_ENV": str(environment_file),
            })
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            environment = dict(line.split("=", 1) for line in environment_file.read_text().splitlines())
        self.assertEqual(environment["MACOSX_DEPLOYMENT_TARGET"], "14.0")
        archive = Path(environment["ORT_LIB_LOCATION"]) / "libonnxruntime.a"
        with archive.open("rb") as binary:
            magic, count = struct.unpack(">II", binary.read(8))
            self.assertEqual(magic, 0xCAFEBABE, "Expected the vendored universal ONNX Runtime archive")
            architectures = [struct.unpack(">IIIII", binary.read(20)) for _ in range(count)]
            arm64 = [entry for entry in architectures if entry[0] == 0x0100000C]
            self.assertEqual(len(arm64), 1, "ONNX Runtime must contain Apple Silicon code")
            for _, _, offset, _, _ in arm64:
                binary.seek(offset)
                self.assertEqual(binary.read(8), b"!<arch>\n", "The release needs a static ONNX archive")

    def test_cache_covers_the_actual_target_directory_and_native_environment(self):
        job = job_body("binaries")
        target = re.search(r"^      CARGO_TARGET_DIR: (.+)$", job, re.MULTILINE).group(1)
        workspace = re.search(r"^          workspaces: '(.+)'$", job, re.MULTILINE).group(1)
        root, cache_target = workspace.split(" -> ")
        expected = Path("/workspace") / root / cache_target
        self.assertEqual(Path(target.replace("${{ github.workspace }}", "/workspace")), expected)
        cache_key = re.search(r"^          shared-key: (.+)$", job, re.MULTILINE).group(1)
        self.assertIn("${{ matrix.target }}", cache_key)
        environment = re.search(r"^          env-vars: '(.+)'$", job, re.MULTILINE).group(1).split()
        self.assertTrue({"CARGO", "CC", "CXX", "CMAKE", "RUST", "ORT", "ImageOS", "ImageVersion", "SDKROOT"} <= set(environment))


class StandaloneReleaseValidityTests(unittest.TestCase):
    def directory(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        return Path(folder.name)

    def prepare(self, validity_days, repository=REPOSITORY):
        outputs = self.directory() / "outputs"
        result = run_script(script(step("prepare", "id: inputs")), repository, {
            "RELEASE_SEQUENCE": "3", "RELEASE_VALIDITY_DAYS": validity_days,
            "RELEASE_ARTIFACT_BASE": BASE, "RELEASE_CONFIGURED_BASE": BASE, "RELEASE_BUCKET": "releases",
            "RELEASE_PREFIX": "standalone", "RELEASE_PUBLIC_KEYS": KEYS, "RELEASE_REGION": "auto",
            "GITHUB_REPOSITORY": "Example/Flow-Like", "GITHUB_OUTPUT": str(outputs),
        })
        written = outputs.read_text().splitlines() if outputs.exists() else []
        return result, dict(line.split("=", 1) for line in written)

    def test_prepare_hands_over_the_validity_and_the_agent_version(self):
        with (REPOSITORY / "apps/standalone/Cargo.toml").open("rb") as manifest:
            version = tomllib.load(manifest)["package"]["version"]
        for days in ["365", "30", "1825"]:
            result, outputs = self.prepare(days)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual({name: outputs[name] for name in ["sequence", "base", "image", "validity_days", "version"]}, {
                "sequence": "3", "base": BASE, "image": "ghcr.io/example/flow-like-standalone",
                "validity_days": days, "version": version,
            })
        self.assertIn('--validity-days "$RELEASE_VALIDITY_DAYS"', step("signed-bundle", "name: Bind binary hashes and exact container digest"))
        self.assertIn("RELEASE_VALIDITY_DAYS: ${{ needs.prepare.outputs.validity_days }}", job_body("signed-bundle"))
        preflight = step("prepare", "name: Refuse a sequence that is already published or partly uploaded, and an agent version that was not bumped")
        self.assertIn("RELEASE_VERSION: ${{ steps.inputs.outputs.version }}", preflight)
        self.assertIn('--release-version "$RELEASE_VERSION"', preflight)

    def test_prepare_refuses_a_stale_or_missing_agent_lock_entry_before_anything_is_built(self):
        repository = self.directory()
        for name in ["apps/standalone/Cargo.toml", ".github/scripts/standalone_release.py"]:
            destination = repository / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes((REPOSITORY / name).read_bytes())
        with (repository / "apps/standalone/Cargo.toml").open("rb") as manifest:
            version = tomllib.load(manifest)["package"]["version"]
        for name, locked_version in [("flow-like-standalone", "0.0.0"), ("unrelated", version)]:
            with self.subTest(name=name, locked_version=locked_version):
                (repository / "Cargo.lock").write_text(
                    f'version = 4\n\n[[package]]\nname = "{name}"\nversion = "{locked_version}"\n'
                )
                result, outputs = self.prepare("365", repository)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(f"Cargo.lock must contain flow-like-standalone {version}", result.stderr)
                self.assertIn("cargo update --workspace", result.stderr)
                self.assertEqual(outputs, {})

    def test_prepare_refuses_a_validity_outside_the_range_before_anything_is_built(self):
        for days, reason in [("0", "between 1 and 1825 days"), ("1826", "between 1 and 1825 days"),
                             ("a year", "Invalid validity_days"), ("", "Invalid validity_days")]:
            result, outputs = self.prepare(days)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(reason, result.stderr)
            self.assertEqual(outputs, {})

    def test_signing_workflows_start_only_by_hand_and_never_run_together(self):
        for workflow in [WORKFLOW, RENEWAL]:
            text = workflow.read_text()
            self.assertEqual(triggers(workflow), ["workflow_dispatch"])
            self.assertIn("\nconcurrency:\n  group: standalone-release\n  cancel-in-progress: false\n", text)
            self.assertEqual(re.findall(r"^    if: (github\.ref .+)$", text, re.MULTILINE), ["github.ref == 'refs/heads/dev'"])
        self.assertRegex(RENEWAL.read_text(), r"\n  # schedule:\n  #   - cron: '0 6 1 \* \*'\n")
        self.assertIn("RELEASE_ONLY_IF_DAYS_LEFT: ${{ github.event_name == 'schedule' && '60' || inputs.only_if_days_left }}", job_body("sign", RENEWAL))

    def test_summaries_follow_a_successful_publish(self):
        for workflow, options in [(WORKFLOW, '--artifacts release-bundle --public-keys "$RELEASE_PUBLIC_KEYS"'),
                                  (RENEWAL, '--artifacts release-renewal --public-keys "$RELEASE_PUBLIC_KEYS" --renewed')]:
            last = re.split(r"^      - ", job_body("publish", workflow), flags=re.MULTILINE)[-1]
            self.assertIn(f'standalone_release.py summary {options} >> "$GITHUB_STEP_SUMMARY"', last)
            self.assertNotIn("        if:", last)

    def test_renewal_uses_the_release_jobs_key_and_publisher_handling(self):
        sign = job_body("sign", RENEWAL)
        self.assertEqual(sign.count("secrets."), 1)
        self.assertLess(sign.index("cargo build --locked --package flow-like-device-protocol --example sign-release"),
                        sign.index("secrets.STANDALONE_RELEASE_SIGNING_KEY"))
        self.assertLess(sign.index("--example sign-runtime-manifest"), sign.index("secrets.STANDALONE_RELEASE_SIGNING_KEY"))
        self.assertNotIn("id-token", sign)
        self.assertNotIn("STANDALONE_RELEASE_SIGNING_KEY", job_body("publish", RENEWAL))
        for job in ["sign", "publish"]:
            self.assertIn("\n    environment: standalone-release\n", job_body(job, RENEWAL))
        runtime_signing = script(step("runtime-manifest", "name: Sign the runtime manifests using the configured private release key"))
        self.assertEqual(
            script(step("sign", "name: Sign the renewed manifests using the configured private release key", RENEWAL)),
            script(step("signed-bundle", "name: Sign the release manifest using the configured private release key")).replace("release-bundle/", "release-renewal/")
            + "shopt -s nullglob\nfor manifest in " + runtime_signing.split("\nfor manifest in ", 1)[1].replace("runtime-bundle/", "release-renewal/runtimes/"),
        )
        identity = "name: Require exactly one publisher identity"
        self.assertEqual(step("publish", identity, RENEWAL), step("publish", identity))
        credentials = "name: Obtain short-lived AWS publisher credentials"
        self.assertEqual(step("publish", credentials, RENEWAL), step("publish", credentials).replace("standalone-release-${{", "standalone-release-renew-${{"))
        publish = step("publish", "name: Verify anonymous images, publish binaries, then conditionally advance the signed manifest")
        renew = step("publish", "name: Replace the stable signed manifests only if nothing but their dates changed", RENEWAL)
        self.assertIn("release.renew_publish(Path('release-renewal')", renew)
        self.assertEqual(
            renew.split("\n", 1)[1].replace("needs.prepare.outputs.base", "vars.STANDALONE_RELEASE_PUBLIC_BASE_URL"),
            publish.split("\n", 1)[1].replace("needs.prepare.outputs.base", "vars.STANDALONE_RELEASE_PUBLIC_BASE_URL")
            .replace("release.publish(Path('release-bundle')", "release.renew_publish(Path('release-renewal')"),
        )

    def renew_manifest(self, environment, outcome="written"):
        directory = self.directory()
        commands = directory / "commands"
        commands.mkdir()
        shim = commands / "python3"
        shim.write_text(f"#!{sys.executable}\n" + textwrap.dedent('''\
            import json
            import os
            from pathlib import Path
            import sys

            Path(os.environ["COMMAND_LOG"]).write_text(json.dumps(sys.argv[1:]))
            if os.environ["OUTCOME"] == "failed":
                sys.exit(7)
            if os.environ["OUTCOME"] == "written":
                Path(sys.argv[sys.argv.index("--output") + 1]).write_text("{}")
            '''))
        shim.chmod(0o755)
        result = run_script(script(step("sign", "id: manifest", RENEWAL)), directory, {
            "PATH": str(commands) + os.pathsep + os.environ["PATH"], "COMMAND_LOG": str(directory / "command.json"),
            "OUTCOME": outcome, "GITHUB_OUTPUT": str(directory / "outputs"), "RELEASE_BASE": BASE,
            "RELEASE_PUBLIC_KEYS": KEYS, "RELEASE_VALIDITY_DAYS": "365", **environment,
        })
        outputs = directory / "outputs"
        return result, json.loads((directory / "command.json").read_text()), outputs.read_text() if outputs.exists() else ""

    def test_renewal_names_the_release_and_signs_only_what_was_written(self):
        expected = [".github/scripts/standalone_release.py", "renew-manifest", "--base-url", BASE, "--public-keys", KEYS,
                    "--validity-days", "365", "--output", "release-renewal/release.json",
                    "--runtime-output", "release-renewal/runtimes"]
        result, command, outputs = self.renew_manifest({"RELEASE_SEQUENCE": "3", "RELEASE_ONLY_IF_DAYS_LEFT": ""})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((command, outputs), (expected + ["--sequence", "3"], "renewed=true\n"))
        result, command, outputs = self.renew_manifest({"RELEASE_SEQUENCE": "", "RELEASE_ONLY_IF_DAYS_LEFT": "60"}, "skipped")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((command, outputs), (expected + ["--only-if-days-left", "60"], ""))
        result, _, outputs = self.renew_manifest({"RELEASE_SEQUENCE": "3", "RELEASE_ONLY_IF_DAYS_LEFT": ""}, "failed")
        self.assertEqual((result.returncode, outputs), (7, ""))
        sign = job_body("sign", RENEWAL)
        self.assertEqual(sign.count("        if: steps.manifest.outputs.renewed == 'true'\n"), 2)
        self.assertIn("\n    if: needs.sign.outputs.renewed == 'true'\n", job_body("publish", RENEWAL))

    def sign_renewal(self, runtime_targets):
        directory = self.directory()
        signers = directory / "release-signer/debug/examples"
        signers.mkdir(parents=True)
        for name in ["sign-release", "sign-runtime-manifest"]:
            (signers / name).write_text(f"#!{sys.executable}\n" + textwrap.dedent('''\
                import json
                import os
                from pathlib import Path
                import sys

                key, manifest, output = sys.argv[1:]
                assert Path(key).read_text() == "test signing key"
                with open(os.environ["COMMAND_LOG"], "a") as log:
                    log.write(json.dumps([Path(sys.argv[0]).name, manifest, output]) + "\\n")
                Path(output).write_text("signed " + Path(manifest).read_text())
                '''))
            (signers / name).chmod(0o755)
        renewal = directory / "release-renewal"
        renewal.mkdir()
        (renewal / "release.json").write_text("release")
        for target in runtime_targets:
            (renewal / "runtimes").mkdir(exist_ok=True)
            (renewal / "runtimes" / f"{target}.json").write_text(target)
        log = directory / "signed.jsonl"
        log.touch()
        result = run_script(script(step("sign", "name: Sign the renewed manifests using the configured private release key", RENEWAL)), directory, {
            "RUNNER_TEMP": str(directory), "STANDALONE_RELEASE_SIGNING_KEY": "test signing key", "COMMAND_LOG": str(log),
        })
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse((directory / "standalone-signing-key").exists())
        return [json.loads(line) for line in log.read_text().splitlines()]

    def test_renewal_signs_the_release_and_every_runtime_pack_list_it_copied(self):
        release_call = ["sign-release", "release-renewal/release.json", "release-renewal/release.jws"]
        self.assertEqual(self.sign_renewal([]), [release_call])
        self.assertEqual(self.sign_renewal(["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"]), [release_call] + [
            ["sign-runtime-manifest", f"release-renewal/runtimes/{target}.json", f"release-renewal/runtimes/{target}.jws"]
            for target in ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu"]])


if __name__ == "__main__":
    unittest.main()
