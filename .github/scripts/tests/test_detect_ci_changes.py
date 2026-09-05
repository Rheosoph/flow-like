"""Exercise CI selection against changed build inputs and real Git histories."""

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "detect-ci-changes.py"
SPEC = importlib.util.spec_from_file_location("detect_ci_changes", SCRIPT)
changes = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(changes)


class ClassificationTests(unittest.TestCase):
    def test_frontend_and_docs_do_not_compile_rust(self):
        result = changes.classify([
            "README.md", "apps/docs/public/screenshot.png",
            "apps/desktop/app/page.tsx", "packages/ui/components/button.tsx",
        ])
        self.assertEqual(result, {"rust": False, "bun": False, "dsql": False})

    def test_native_configuration_fixtures_and_unknown_inputs_compile(self):
        for path in (
            "flow-like.config.json", "apps/desktop/src-tauri/tauri.conf.json",
            "packages/types/proto/protobufs/board.proto",
            "packages/core/tests/fixtures/board.json", "new-component/input.dat",
            "apps/desktop/scripts/sync-version.ts", ".cargo/config.toml",
        ):
            with self.subTest(path=path):
                self.assertTrue(changes.classify([path])["rust"])

    def test_rust_inputs_override_frontend_directory_exclusions(self):
        for path in ("apps/web/new/Cargo.toml", "apps/docs/example.rs", "packages/ui/data.proto", "apps/book/examples/greeting.flow", "apps/book/examples/greeting.flowscript"):
            with self.subTest(path=path):
                self.assertTrue(changes.classify([path])["rust"])

    def test_package_inputs_run_security_checks(self):
        for path in ("package.json", "apps/desktop/package.json", "bun.lock", ".npmrc", "bunfig.toml", "patches/next-themes@0.4.6.patch"):
            with self.subTest(path=path):
                self.assertTrue(changes.classify([path])["bun"])

    def test_migration_generator_and_sql_run_dsql(self):
        for path in ("packages/api/scripts/dsql-migration.ts", "packages/api/prisma/migrations-dsql/123/migration.sql"):
            with self.subTest(path=path):
                self.assertTrue(changes.classify([path])["dsql"])

    def test_workflow_changes_run_every_check(self):
        self.assertTrue(all(changes.classify([".github/actions/setup-environment/action.yml"]).values()))

    def test_unavailable_comparison_falls_back_to_all_checks(self):
        with tempfile.TemporaryDirectory() as temp:
            event = Path(temp) / "event.json"
            output = Path(temp) / "output"
            event.write_text(json.dumps({"before": "0" * 40}))
            with patch.dict(os.environ, {"GITHUB_EVENT_NAME": "push", "GITHUB_EVENT_PATH": str(event), "GITHUB_OUTPUT": str(output)}):
                changes.main()
            self.assertEqual(output.read_text(), "rust=true\nbun=true\ndsql=true\n")

    def test_full_diff_includes_renamed_and_late_files(self):
        # A Rust input after hundreds of frontend files must not be truncated.
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            def git(*args):
                return subprocess.check_output(["git", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", *args], cwd=root)
            git("init", "-q")
            (root / "packages/core").mkdir(parents=True)
            (root / "packages/core/input.json").write_text("{}")
            git("add", ".")
            git("-c", "user.name=CI", "-c", "user.email=ci@example.invalid", "commit", "-qm", "base")
            base = git("rev-parse", "HEAD").decode().strip()
            (root / "apps/web").mkdir(parents=True)
            (root / "packages/core/input.json").rename(root / "apps/web/input.json")
            for index in range(350):
                (root / f"apps/web/page-{index}.tsx").write_text("export default null;")
            git("add", ".")
            git("-c", "user.name=CI", "-c", "user.email=ci@example.invalid", "commit", "-qm", "move")
            previous = Path.cwd()
            try:
                os.chdir(root)
                paths = changes.changed_paths("pull_request", {"pull_request": {"base": {"sha": base}}})
            finally:
                os.chdir(previous)
            self.assertIn("packages/core/input.json", paths)
            self.assertIn("apps/web/input.json", paths)
            self.assertEqual(len(paths), 352)
            self.assertTrue(changes.classify(paths)["rust"])


if __name__ == "__main__":
    unittest.main()
