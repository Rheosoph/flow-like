import importlib.util
from pathlib import Path
import tempfile
import unittest


spec = importlib.util.spec_from_file_location("check_rust_toolchain", Path(__file__).parents[1] / "check_rust_toolchain.py")
toolchain = importlib.util.module_from_spec(spec)
spec.loader.exec_module(toolchain)


class RustToolchainTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.root = Path(self.folder.name)
        self.write("rust-toolchain.toml", '[toolchain]\nchannel = "beta-2026-09-27"\n')

    def write(self, name, text):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def errors(self):
        return toolchain.check_repository(self.root)

    def test_root_change_invalidates_otherwise_matching_explicit_pins(self):
        self.write("mise.toml", '[tools]\nrust = "beta-2026-09-27"\n')
        self.write("apple/project.yml", "settings:\n  RUSTUP_TOOLCHAIN: beta-2026-09-27\n")
        self.write("apple/project.pbxproj", 'RUSTUP_TOOLCHAIN = "beta-2026-09-27";\n')
        self.assertEqual(self.errors(), [])
        self.write("rust-toolchain.toml", '[toolchain]\nchannel = "1.100.0"\n')
        self.assertEqual(len(self.errors()), 3)

    def test_nested_toolchains_and_mise_table_overrides_must_match(self):
        self.write("templates/mise.toml", '[tools.rust]\nversion = "stable"\n')
        self.write("examples/rust-toolchain", "1.97.1\n")
        self.assertEqual(len(self.errors()), 2)

    def test_generated_and_dependency_directories_are_not_product_pins(self):
        for directory in ("target", "node_modules", "packages/vendor"):
            self.write(f"{directory}/rust-toolchain", "unrelated-third-party-compiler")
        self.assertEqual(self.errors(), [])

    def test_nested_checkout_is_not_part_of_repository_configuration(self):
        self.write("worktrees/other/.git", "gitdir: /somewhere/else")
        self.write("worktrees/other/rust-toolchain", "1.97.1")
        self.assertEqual(self.errors(), [])

    def test_known_build_workflow_requires_central_setup(self):
        name = ".github/workflows/release-native.yml"
        self.write(name, "steps:\n  - uses: actions/checkout@sha\n")
        self.assertIn("missing", self.errors()[0])
        self.write(name, f"steps:\n  - uses: {toolchain.SETUP_ACTION}\n")
        self.assertEqual(self.errors(), [])

    def test_new_ci_workflow_cannot_reintroduce_independent_rust_action(self):
        self.write(".github/workflows/new.yml", "steps:\n  - uses: dtolnay/rust-toolchain@stable\n    with:\n      toolchain: beta-2026-09-27\n")
        self.assertEqual(len(self.errors()), 1)
        self.assertIn("root compiler pin", self.errors()[0])

    def test_exported_template_pin_and_prerequisite_prose_are_checked(self):
        self.write("templates/wasm-node-rust/.github/workflows/build.yml", "steps:\n  - uses: dtolnay/rust-toolchain@stable\n    with:\n      toolchain: 1.97.1\n")
        self.write("templates/wasm-node-rust/README.md", "Requires Rust 1.97.1, matching mise.toml.\n")
        self.assertEqual(len(self.errors()), 2)

    def test_bootstrap_image_version_does_not_override_installed_root_pin(self):
        self.write("Dockerfile", "FROM rust:1.97.1-bookworm AS base\nCOPY rust-toolchain.toml /rust-toolchain.toml\nRUN rustup toolchain install\nFROM base AS builder\nWORKDIR /app\nRUN cargo build --release\nFROM debian:bookworm-slim\nCOPY --from=builder /app/target/release/app /app\n")
        self.assertEqual(self.errors(), [])

    def test_json_copy_and_installer_with_continuations_are_supported(self):
        self.write("Dockerfile", 'FROM debian:bookworm\nCOPY ["rust-toolchain.toml", "/rust-toolchain.toml"]\nRUN curl https://sh.rustup.rs | sh -s -- -y --default-toolchain none && \\\n    /root/.cargo/bin/rustup toolchain install && \\\n    /root/.cargo/bin/cargo build --release\n')
        self.assertEqual(self.errors(), [])

    def test_pin_in_an_unrelated_docker_stage_does_not_cover_builder(self):
        self.write("Dockerfile", "FROM debian AS unrelated\nCOPY rust-toolchain.toml /rust-toolchain.toml\nRUN rustup toolchain install\nFROM rust:1.97.1 AS builder\nRUN cargo build --release\n")
        self.assertIn("must use", self.errors()[0])

    def test_install_must_follow_pin_and_precede_compilation(self):
        for body in (
            "RUN rustup toolchain install\nCOPY rust-toolchain.toml /rust-toolchain.toml\nRUN cargo build\n",
            "COPY rust-toolchain.toml /rust-toolchain.toml\nRUN cargo build && rustup toolchain install\n",
        ):
            with self.subTest(body=body):
                self.write("Dockerfile", "FROM rust:1.97.1\n" + body)
                self.assertTrue(self.errors())

    def test_explicit_shell_selection_cannot_restore_a_stale_compiler(self):
        for command in (
            "rustup toolchain install 1.97.1",
            "rustup toolchain install --profile minimal --component rustfmt 1.97.1",
            "rustup default stable",
            "cargo +1.97.1 build",
        ):
            with self.subTest(command=command):
                self.write("Dockerfile", "FROM rust:1.97.1\nCOPY rust-toolchain.toml /rust-toolchain.toml\nRUN rustup toolchain install\nRUN " + command + "\n")
                self.assertIn("explicit compiler selection", self.errors()[0])

    def test_copy_from_another_image_does_not_prove_repository_pin(self):
        self.write("Dockerfile", "FROM rust:1.97.1\nCOPY --from=external rust-toolchain.toml /rust-toolchain.toml\nRUN rustup toolchain install\nRUN cargo build\n")
        self.assertTrue(self.errors())

    def test_package_context_exception_still_rejects_stale_compiler(self):
        recipe = 'FROM rust:1.97.1\nENV RUSTUP_TOOLCHAIN={pin}\nRUN rustup toolchain install "$RUSTUP_TOOLCHAIN" --profile minimal\nRUN cargo install sea-orm-cli\n'
        self.write(toolchain.PACKAGE_DOCKER, recipe.format(pin="beta-2026-09-27"))
        self.assertEqual(self.errors(), [])
        self.write(toolchain.PACKAGE_DOCKER, recipe.format(pin="1.97.1"))
        self.assertEqual(len(self.errors()), 1)
        self.assertIn("expected", self.errors()[0])


if __name__ == "__main__":
    unittest.main()
