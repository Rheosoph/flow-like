"""Exercise the release workflow's build and runtime validation sequence."""

import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest


WORKFLOW = Path(__file__).resolve().parents[2] / "workflows/standalone-release.yml"
TARGET = "aarch64-apple-darwin"


def job_body(name):
    match = re.search(
        rf"^  {re.escape(name)}:\n(.*?)(?=^  [a-z][a-z-]*:\n|\Z)",
        WORKFLOW.read_text(), re.MULTILINE | re.DOTALL,
    )
    if not match:
        raise AssertionError(f"Missing workflow job: {name}")
    return match.group(1)


def build_script():
    for step in re.split(r"^      - ", job_body("binaries"), flags=re.MULTILINE):
        if step.startswith("name: Build the release binary and validate its runtime metadata\n"):
            return textwrap.dedent(step.split("        run: |\n", 1)[1])
    raise AssertionError("Missing release build and runtime validation step")


class StandaloneReleaseWorkflowTests(unittest.TestCase):
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
        self.assertTrue({"CARGO", "CC", "CXX", "CMAKE", "RUST", "ImageOS", "ImageVersion", "SDKROOT"} <= set(environment))


if __name__ == "__main__":
    unittest.main()
