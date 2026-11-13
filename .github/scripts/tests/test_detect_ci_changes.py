"""Exercise CI selection against changed build inputs and real Git histories."""

import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "detect-ci-changes.py"
SPEC = importlib.util.spec_from_file_location("detect_ci_changes", SCRIPT)
changes = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(changes)


class ClassificationTests(unittest.TestCase):
    def assert_checks(self, paths, *enabled):
        self.assertEqual(changes.classify(paths), {name: name in enabled for name in changes.CHECKS})

    def test_frontend_and_docs_do_not_compile_rust(self):
        self.assert_checks([
            "README.md", "apps/docs/public/screenshot.png",
            "apps/desktop/app/page.tsx", "packages/ui/components/button.tsx",
        ], "ui", "containers")

    def test_ui_checks_follow_the_ui_package_and_its_inputs(self):
        for path in (
            "packages/ui/components/button.tsx", "packages/locales/locales/en/common.json",
            "bun.lock", "package.json", "apps/docs/src/content/docs/studio/flowscript.md",
        ):
            with self.subTest(path=path):
                self.assertTrue(changes.classify([path])["ui"])
        for path in ("packages/core/src/lib.rs", "apps/website/src/index.astro", "README.md"):
            with self.subTest(path=path):
                self.assertFalse(changes.classify([path])["ui"])

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

    def test_shared_detection_and_unknown_helpers_run_every_check(self):
        for path in (
            ".github/workflows/ci.yml", ".github/scripts/detect-ci-changes.py",
            ".github/scripts/tests/test_detect_ci_changes.py", ".github/actions/detect-changes/action.yml",
            ".github/actions/new-shared-setup/action.yml", ".github/scripts/new-build-input.py",
            ".github/workflows/new-workflow.yml",
        ):
            with self.subTest(path=path):
                self.assertTrue(all(changes.classify([path]).values()))

    def test_workflow_and_action_changes_select_their_consumers(self):
        cases = (
            (".github/actions/setup-environment/action.yml", ("rust", "compiler")),
            (".github/actions/setup-rust/action.yml", ("rust", "browser", "compiler", "containers")),
            (".github/workflows/tests.yml", ("rust", "bun", "ui", "azure_migrations", "compiler")),
            (".github/workflows/clippy.yml", ("rust", "dsql", "compiler")),
            (".github/workflows/compile-guardrails.yml", ("rust", "compiler")),
            (".github/workflows/browser.yml", ("browser", "compiler")),
            (".github/workflows/compiler-reproducibility.yml", ("compiler", "containers")),
            (".github/workflows/containers.yml", ("compiler", "containers")),
            (".github/workflows/platform-sdks.yml", ("compiler",)),
        )
        for path, enabled in cases:
            with self.subTest(path=path):
                self.assert_checks([path], *enabled)

    def test_unrelated_workflows_do_not_start_builds(self):
        for path in (
            ".github/workflows/cache-prune.yml", ".github/workflows/fossa-scan.yml",
            ".github/workflows/fossa-test.yml", ".github/workflows/audit.yml",
        ):
            with self.subTest(path=path):
                self.assert_checks([path])

    def test_helper_edits_keep_their_validation_without_unrelated_suites(self):
        self.assert_checks([".github/scripts/container_images.py"], "containers")
        self.assert_checks([".github/scripts/prune_caches.py"], "containers")
        self.assert_checks([".github/scripts/tests/test_container_images.py"], "containers")
        self.assert_checks([".github/scripts/check_rust_toolchain.py"], "compiler", "containers")
        self.assert_checks([".github/scripts/run-with-resource-log.sh"], "rust", "containers")

    def test_azure_migrations_follow_their_inputs(self):
        for path in (
            "apps/backend/azure/migration/migrate.ts", "apps/backend/azure/migration/package.json",
            "packages/api/prisma/schema/bot.prisma", "packages/api/prisma/pre-push.ts",
            "package.json", "bun.lock", "bunfig.toml", ".npmrc", "patches/example.patch",
        ):
            with self.subTest(path=path):
                self.assertTrue(changes.classify([path])["azure_migrations"])
        for path in ("apps/desktop/src-tauri/src/main.rs", "packages/api/src/lib.rs", "Cargo.lock"):
            with self.subTest(path=path):
                self.assertFalse(changes.classify([path])["azure_migrations"])

    def test_compiler_checks_follow_configuration_inputs(self):
        for path in (
            "Cargo.lock", "packages/core/Cargo.toml", "rust-toolchain.toml", "examples/rust-toolchain",
            "mise.toml", "apps/tool/.mise.toml", "apps/backend/gcp/api/Dockerfile",
            "examples/custom.Dockerfile", "examples/Dockerfile.tools", ".cargo/config.toml",
            "apps/desktop/src-tauri/gen/apple/project.pbxproj",
            ".github/scripts/tests/fixtures/compiler-reproducibility/src/main.rs",
        ):
            with self.subTest(path=path):
                self.assertTrue(changes.classify([path])["compiler"])
        for path in ("packages/core/src/lib.rs", "apps/docs/src/index.md", "apps/web/app/page.tsx"):
            with self.subTest(path=path):
                self.assertFalse(changes.classify([path])["compiler"])

    def test_container_checks_follow_build_and_validation_inputs(self):
        for path in (
            ".dockerignore", "LICENSE", "flow-like.config.json", "Cargo.toml", "Cargo.lock",
            "rust-toolchain.toml", "bun.lock", "bunfig.toml", "package.json", ".cargo/config.toml",
            "packages/core/src/lib.rs", "apps/backend/docker-compose/runtime/Dockerfile",
            "apps/web/app/page.tsx", "apps/website/src/content/legal/payments/en.md",
            "packages/core/src/prompts/system.md", ".github/scripts/tests/test_prune_caches.py",
            "scripts/sync-node-icons.ts", "tools/runtime/build.py", "libs/platform/rust/src/lib.rs",
            "patches/next-themes@0.4.6.patch", "tsconfig.json", "new-library/build-input.json",
            "apps/docs/package.json", "apps/book/package.json", "apps/desktop/src-tauri/Cargo.toml",
            "assets/mcp-inspector.html", "apps/docs/public/flow/icons/custom.svg",
            "apps/desktop/public/flow/icons/custom.svg", "apps/embedded/public/flow/icons/custom.svg",
        ):
            with self.subTest(path=path):
                self.assertTrue(changes.classify([path])["containers"])
        for path in (
            "README.md", "packages/core/README.md", "apps/backend/guide.rst",
            "apps/docs/src/content/docs/guide.md", "apps/website/src/pages/index.astro",
            "apps/desktop/src-tauri/src/main.rs", "apps/book/src/chapter.md", "docs/diagram.png",
            "apps/docs/public/screenshot.png", "assets/marketing.png", "CONTRIBUTING.md",
        ):
            with self.subTest(path=path):
                self.assertFalse(changes.classify([path])["containers"])

    def test_browser_inputs_run_browser_checks(self):
        for path in (
            "packages/browser/src/launch/cft.rs", "packages/catalog/automation/tests/browser_nodes_e2e.rs",
            "packages/catalog/core/src/lib.rs", "packages/core/runtime/src/lib.rs",
            "packages/types/proto/protobufs/board.proto", "packages/storage/src/lib.rs",
            "packages/storage/files/src/lib.rs", "packages/model-provider/protocol/src/lib.rs",
            "packages/catalog/std-ui/src/lib.rs", "packages/catalog/data/support/src/lib.rs",
            "packages/catalog-macros/src/lib.rs", "packages/core/contracts/src/lib.rs",
            "Cargo.lock", "Cargo.toml",
            "rust-toolchain.toml", ".cargo/config.toml", ".github/workflows/browser.yml",
            ".github/actions/setup-rust/action.yml", ".github/scripts/detect-ci-changes.py",
        ):
            with self.subTest(path=path):
                self.assertTrue(changes.classify([path])["browser"])

    def test_other_rust_and_workflow_inputs_skip_browser_checks(self):
        for path in (
            "packages/api/src/lib.rs", "packages/catalog/data/Cargo.toml",
            "packages/catalog/data/src/lib.rs", "packages/core/src/lib.rs",
            "packages/core/editor/src/lib.rs", "apps/desktop/src-tauri/src/main.rs",
            ".github/workflows/tests.yml", "packages/browser-extension/src/lib.rs",
            "packages/browser/README.md", "packages/catalog/automation/tests/fixtures/README.md",
        ):
            with self.subTest(path=path):
                self.assertFalse(changes.classify([path])["browser"])

    @unittest.skipUnless(shutil.which("cargo"), "cargo is not installed")
    def test_browser_crates_are_the_local_dependencies_cargo_reads(self):
        # Reading manifests needs no registry, and runners preinstall the stable toolchain.
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--offline", "--format-version", "1"],
            cwd=changes.REPO, env={**os.environ, "RUSTUP_TOOLCHAIN": "stable"}, timeout=300,
        ))
        members = {Path(package["manifest_path"]).parent: package for package in metadata["packages"]}
        seen, pending = set(), [changes.REPO / crate for crate in changes.BROWSER_ROOTS]
        while pending:
            crate = pending.pop()
            if crate not in seen:
                seen.add(crate)
                pending.extend(Path(dependency["path"]) for dependency in members[crate]["dependencies"] if dependency.get("path"))
        self.assertEqual({crate.relative_to(changes.REPO).as_posix() for crate in seen}, changes.browser_crates())

    def test_unavailable_comparison_falls_back_to_all_checks(self):
        for event_name, payload in (("push", {"before": "0" * 40}), ("workflow_dispatch", {}), ("schedule", {})):
            with self.subTest(event=event_name), tempfile.TemporaryDirectory() as temp:
                event = Path(temp) / "event.json"
                output = Path(temp) / "output"
                event.write_text(json.dumps(payload))
                with patch.dict(os.environ, {"GITHUB_EVENT_NAME": event_name, "GITHUB_EVENT_PATH": str(event), "GITHUB_OUTPUT": str(output)}):
                    changes.main()
                self.assertEqual(output.read_text(), "".join(f"{check}=true\n" for check in changes.CHECKS))

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
