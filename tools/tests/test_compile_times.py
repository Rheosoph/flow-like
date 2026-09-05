"""Exercise the timing runner's build modes and source restoration without compiling Rust."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
RUNNER = ROOT / "tools/compile-times.sh"


class CompileTimesTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="flow-like-timing-test-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.calls = self.root / "calls.jsonl"
        self.source = self.root / "node.rs"
        self.source.write_text("pub const VALUE: usize = 1;\n")
        os.utime(self.source, ns=(1_700_000_000_000_000_000,) * 2)
        self.modified = self.source.stat().st_mtime_ns
        cargo = self.bin / "cargo"
        cargo.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, pathlib, sys\n"
            "if sys.argv[1:] == ['--version']:\n"
            "    print('cargo test-stub'); sys.exit(0)\n"
            "path = pathlib.Path(os.environ['TIMING_TEST_CALLS'])\n"
            "with path.open('a') as out:\n"
            "    out.write(json.dumps({'args': sys.argv[1:], "
            "'incremental': os.environ.get('CARGO_INCREMENTAL')}) + '\\n')\n"
            "timings = pathlib.Path(os.environ['CARGO_TARGET_DIR']) / 'cargo-timings'\n"
            "timings.mkdir(parents=True, exist_ok=True)\n"
            "(timings / 'cargo-timing.html').write_text('timing fixture')\n"
            "if os.environ.get('TIMING_TEST_FAIL_SECOND') and len(path.read_text().splitlines()) == 2:\n"
            "    sys.exit(9)\n"
        )
        cargo.chmod(0o755)
        self.env = {
            **os.environ,
            "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
            "TIMING_TEST_CALLS": str(self.calls),
            "CARGO_INCREMENTAL": "1",
        }

    def run_timing(self, *args):
        return subprocess.run(
            [
                "bash", str(RUNNER), "--target-root", str(self.root / "target"),
                "--run-id", "test", "--incremental-source", str(self.source),
                *args, "core",
            ],
            cwd=ROOT, env=self.env, capture_output=True, text=True,
        )

    def read_calls(self):
        return [json.loads(line) for line in self.calls.read_text().splitlines()]

    def test_build_respects_profile_defaults_and_restores_source(self):
        result = self.run_timing("--command", "build", "--profile", "ci")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        calls = self.read_calls()
        self.assertEqual(len(calls), 3)
        for call in calls:
            self.assertEqual(call["args"][0], "build")
            self.assertEqual(call["args"][-2:], ["--profile", "ci"])
            self.assertIsNone(call["incremental"])
        self.assertEqual(self.source.stat().st_mtime_ns, self.modified)
        self.assertEqual(self.source.read_text(), "pub const VALUE: usize = 1;\n")
        report = self.root / "target/reports/test/core"
        self.assertTrue((report / "cold.html").exists())
        self.assertTrue((report / "incremental.html").exists())

    def test_explicit_incremental_setting_and_default_check(self):
        result = self.run_timing("--incremental", "0", "--phase", "cold")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        call, = self.read_calls()
        self.assertEqual(call["args"][0], "check")
        self.assertEqual(call["incremental"], "0")

    def test_failed_incremental_build_restores_source_and_reports_failure(self):
        self.env["TIMING_TEST_FAIL_SECOND"] = "1"
        result = self.run_timing("--command", "build", "--phase", "incremental")
        self.assertEqual(result.returncode, 9, result.stdout + result.stderr)
        self.assertEqual(self.source.stat().st_mtime_ns, self.modified)
        summary = (self.root / "target/reports/test/summary.tsv").read_text()
        self.assertTrue(summary.rstrip().endswith("\t9"), summary)

    def test_invalid_command_does_not_build(self):
        result = self.run_timing("--command", "run")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.calls.exists())


if __name__ == "__main__":
    unittest.main()
