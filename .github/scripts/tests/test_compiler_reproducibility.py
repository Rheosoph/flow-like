import contextlib
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location(
    "check_compiler_reproducibility",
    Path(__file__).parents[1] / "check_compiler_reproducibility.py",
)
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


class CompilerReproducibilityTests(unittest.TestCase):
    def linked_builds(self, directory, outputs):
        target = "aarch64-apple-darwin"
        builds, executions = [], []

        def run(command, **options):
            if command[0] != "cargo":
                self.assertEqual(options["timeout"], 15)
                executions.append(command[0])
                return subprocess.CompletedProcess(command, 0)
            target_dir = Path(command[command.index("--target-dir") + 1])
            self.assertFalse(target_dir.exists())
            self.assertEqual(command[1], "build")
            self.assertIn("--bin", command)
            self.assertNotIn("--emit=obj", command)
            self.assertEqual(options["env"]["RUSTFLAGS"], "--remap-path-prefix=/checkout=/src")
            self.assertEqual(options["env"]["CARGO_PROFILE_RELEASE_STRIP"], "debuginfo")
            self.assertEqual(options["env"]["RUSTC_WRAPPER"], "")
            release = target_dir / target / "release"
            release.mkdir(parents=True)
            # Identical objects must not conceal a changed linked executable.
            (release / "fixture.o").write_bytes(b"same object")
            (release / "compiler-reproducibility").write_bytes(outputs[len(builds)])
            builds.append(target_dir)
            return subprocess.CompletedProcess(command, 0)

        with patch.object(check.subprocess, "check_output", return_value="rustc test\n"), \
                patch.object(check.subprocess, "run", side_effect=run), \
                patch.dict(os.environ, {
                    "RUSTFLAGS": "--remap-path-prefix=/checkout=/src",
                    "CARGO_PROFILE_RELEASE_STRIP": "debuginfo",
                    "RUSTC_WRAPPER": "sccache",
                }), contextlib.redirect_stdout(io.StringIO()):
            check.check_reproducibility(target, len(outputs), directory, linked=True)
        self.assertEqual(len(builds), len(executions))

    def test_links_runs_and_compares_independent_executables_with_release_flags(self):
        with tempfile.TemporaryDirectory() as directory:
            self.linked_builds(directory, [b"linked executable"] * 3)
            self.assertEqual(len(list(Path(directory).glob("build-*"))), 3)

    def test_rejects_linker_changes_even_when_objects_are_identical(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(RuntimeError, "Linked executable differs"):
                self.linked_builds(directory, [b"executable1", b"executable2"])
            self.assertNotEqual((Path(directory) / "build-1").read_bytes(),
                                (Path(directory) / "build-2").read_bytes())

    def run_builds(self, directory, outputs):
        target = "x86_64-pc-windows-msvc"
        builds = []

        def compile_fixture(command, **options):
            target_dir = Path(command[command.index("--target-dir") + 1])
            # A prior dependency's output must be gone too, not just the fixture.
            self.assertFalse(target_dir.exists())
            self.assertEqual(options["env"]["RUSTC_WRAPPER"], "")
            self.assertEqual(options["env"]["RUSTC_WORKSPACE_WRAPPER"], "")
            self.assertEqual(options["env"]["CARGO_ENCODED_RUSTFLAGS"], "")
            self.assertEqual(options["env"]["CARGO_INCREMENTAL"], "0")
            self.assertNotIn("CARGO_PROFILE_RELEASE_OPT_LEVEL", options["env"])
            self.assertIn("--locked", command)
            self.assertIn("--release", command)
            deps = target_dir / target / "release/deps"
            deps.mkdir(parents=True)
            (deps / "dependency.rlib").write_bytes(b"previous compiled dependency")
            (deps / "compiler_reproducibility-fixture.o").write_bytes(outputs[len(builds)])
            builds.append(target_dir)
            return subprocess.CompletedProcess(command, 0)

        with patch.object(check.subprocess, "check_output", return_value="rustc test\n"), \
                patch.object(check.subprocess, "run", side_effect=compile_fixture), \
                patch.dict(os.environ, {
                    "RUSTC_WRAPPER": "sccache",
                    "CARGO_ENCODED_RUSTFLAGS": "-Copt-level=0",
                    "CARGO_PROFILE_RELEASE_OPT_LEVEL": "0",
                }), contextlib.redirect_stdout(io.StringIO()):
            check.check_reproducibility(target, len(outputs), directory)
        self.assertEqual(len(set(builds)), 1)

    def test_rebuilds_dependencies_and_keeps_identical_objects(self):
        with tempfile.TemporaryDirectory() as directory:
            self.run_builds(directory, [b"optimized object"] * 3)
            root = Path(directory)
            self.assertFalse((root / "target").exists())
            self.assertEqual(len(list(root.glob("build-*.o"))), 3)

    def test_rejects_one_changed_byte_and_retains_both_objects(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(RuntimeError, "Compiler output differs"):
                self.run_builds(directory, [b"optimized object1", b"optimized object2"])
            self.assertNotEqual(
                (Path(directory) / "build-1.o").read_bytes(),
                (Path(directory) / "build-2.o").read_bytes(),
            )

    def test_rejects_one_build_and_preserves_existing_output(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, "At least two"):
                check.check_reproducibility("target", 1, directory)
            marker = Path(directory) / "keep"
            marker.write_text("keep")
            with self.assertRaisesRegex(ValueError, "must be empty"):
                check.check_reproducibility("target", 4, directory)
            self.assertEqual(marker.read_text(), "keep")


if __name__ == "__main__":
    unittest.main()
