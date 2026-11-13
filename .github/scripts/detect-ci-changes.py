#!/usr/bin/env python3
"""Select CI workloads without GitHub's changed-file API pagination limits."""

import functools
import json
import os
from pathlib import Path, PurePosixPath
import posixpath
import re
import subprocess
import tomllib


REPO = Path(__file__).resolve().parents[2]
# Skip Rust only for known frontend trees and prose. Unknown inputs still build.
FRONTEND_TREES = (
    "apps/book/", "apps/docs/", "apps/website/", "apps/web/",
    "apps/embedded/", "apps/extension/", "apps/translation/",
    "packages/ui/", "packages/locales/", "packages/widget-sdk/",
    "packages/widget-bundler/", "packages/js-video-url-parser/",
    "packages/dexie-tauri-adapter/frontend/",
)
PROSE_SUFFIXES = {".md", ".mdx", ".rst"}
UI_TREES = (
    "packages/ui/", "packages/locales/", "packages/widget-sdk/", "packages/widget-bundler/",
    "apps/docs/scripts/", "apps/docs/src/content/", "scripts/biome-changed.mjs", "patches/",
)
UI_FILES = {"package.json", "bun.lock", "bunfig.toml", "biome.json"}
CHECKS = ("rust", "bun", "dsql", "browser", "ui", "azure_migrations", "compiler", "containers")
# Reusable workflows select only their consumers. The coordinator and detector
# affect every selection, and unknown CI helpers remain conservative.
WORKFLOW_CHECKS = {
    "ci.yml": CHECKS,
    "tests.yml": ("rust", "bun", "ui", "azure_migrations", "compiler"),
    "clippy.yml": ("rust", "dsql", "compiler"),
    "compile-guardrails.yml": ("rust", "compiler"),
    "browser.yml": ("browser", "compiler"),
    "compiler-reproducibility.yml": ("compiler", "containers"),
    "containers.yml": ("compiler", "containers"),
    "platform-sdks.yml": ("compiler",),
    "industrial-protocols.yml": ("compiler",),
    "inspection-ml.yml": ("compiler",),
    "alpha-release.yml": ("compiler",),
    "release-native.yml": ("compiler",),
    "standalone-release.yml": ("compiler",),
    "standalone-release-renew.yml": ("compiler",),
    "audit.yml": (),
    "cache-prune.yml": (),
    "fossa-scan.yml": (),
    "fossa-test.yml": (),
}
ACTION_CHECKS = {
    "detect-changes": CHECKS,
    "setup-rust": ("rust", "browser", "compiler", "containers"),
    "setup-environment": ("rust", "compiler"),
    "setup-namespace-cache": ("compiler",),
}
RUST_CI_SCRIPTS = {
    "check-datafusion-alignment.py", "ensure-spa-headers.sh", "run-with-resource-log.sh",
}
COMPILER_CI_SCRIPTS = {
    "check_compiler_reproducibility.py", "check_rust_toolchain.py", "check_windows_alignment.py",
    "tests/test_compiler_reproducibility.py", "tests/test_rust_toolchain.py",
    "tests/test_setup_rust_toolchain.py", "tests/test_windows_alignment.py",
}
# These helpers have their own workflows or are exercised by container validation.
# They do not change the workspace test, lint, browser, or frontend jobs.
OTHER_CI_SCRIPTS = {
    "build_linux_runtime.sh", "check-container-native-deps.sh", "check_ml_platform_features.py",
    "codesign-with-timeout.sh", "configure-standalone-release.ts", "container_build_batch.py", "container_images.py",
    "container_publication.py", "finalize-updater-json.sh", "install-release-tools.ts",
    "notarytool.py", "ort-glibc-compat.c", "prepare-appimage-tools.sh", "prepare-ort-glibc-compat.sh",
    "prepare-vulkan-sdk.sh", "prune_caches.py", "public_secret_fixtures.json", "public_secret_fixtures.py",
    "retry-release-upload.sh", "runtime_packs.py", "runtime_toolchain.py", "standalone_release.py",
    "verify-appimage-bundle.sh", "verify-linux-glibc.sh", "verify-updater-signature.ts",
}
CONTAINER_FILES = {
    ".dockerignore", "LICENSE", "flow-like.config.json", "Cargo.toml", "Cargo.lock",
    "rust-toolchain.toml", "bun.lock", "bunfig.toml", "package.json",
}
CONTAINER_TREES = (
    "packages/", "apps/backend/", "apps/web/", ".cargo/", "scripts/", "tools/", "libs/", "patches/",
    "apps/website/src/content/legal/payments/",
    "apps/docs/public/flow/icons/", "apps/desktop/public/flow/icons/", "apps/embedded/public/flow/icons/",
)
NON_CONTAINER_TREES = (
    "docs/", "books/", "apps/book/", "apps/docs/", "apps/website/", "apps/desktop/",
    "apps/embedded/", "apps/extension/", "apps/translation/", "assets/",
)
AZURE_MIGRATION_TREES = ("apps/backend/azure/migration/", "packages/api/prisma/", "patches/")
AZURE_MIGRATION_FILES = {"package.json", "bun.lock", "bunfig.toml", ".npmrc"}
# The real-browser jobs follow the browser crate, the automation nodes, every local crate either one
# depends on (any target or dependency kind, read from the manifests so the set cannot drift), and
# the toolchain.
BROWSER_ROOTS = ("packages/browser", "packages/catalog/automation")
BROWSER_TREES = (".cargo/",)
BROWSER_FILES = {
    "Cargo.lock", "Cargo.toml", "rust-toolchain.toml",
}
DEPENDENCY_KINDS = ("dependencies", "dev-dependencies", "build-dependencies")


def manifest(crate):
    with open(REPO / crate / "Cargo.toml", "rb") as handle:
        return tomllib.load(handle)


def path_dependencies(crate, workspace_dependencies):
    document = manifest(crate)
    for section in (document, *document.get("target", {}).values()):
        for kind in DEPENDENCY_KINDS:
            for name, spec in section.get(kind, {}).items():
                base = crate
                if isinstance(spec, dict) and spec.get("workspace"):
                    base, spec = ".", workspace_dependencies.get(name)
                if isinstance(spec, dict) and "path" in spec:
                    yield posixpath.normpath(posixpath.join(base, spec["path"]))


@functools.cache
def browser_crates():
    workspace_dependencies = manifest(".")["workspace"].get("dependencies", {})
    crates, pending = set(), list(BROWSER_ROOTS)
    while pending:
        crate = pending.pop()
        if crate not in crates:
            crates.add(crate)
            pending.extend(path_dependencies(crate, workspace_dependencies))
    return frozenset(crates)


def owning_crate(path):
    for parent in PurePosixPath(path).parents:
        if (REPO / parent / "Cargo.toml").is_file():
            return parent.as_posix()
    return None


def is_browser_input(path):
    if PurePosixPath(path).suffix in PROSE_SUFFIXES:
        return False
    if path in BROWSER_FILES or path.startswith(BROWSER_TREES):
        return True
    return owning_crate(path) in browser_crates()


def ci_checks(path):
    """Select consumers of CI definitions without treating every workflow as shared."""
    if path.startswith(".github/workflows/"):
        return WORKFLOW_CHECKS.get(path.removeprefix(".github/workflows/"), CHECKS)
    if path.startswith(".github/actions/"):
        action = path.removeprefix(".github/actions/").split("/", 1)[0]
        return ACTION_CHECKS.get(action, CHECKS)
    if path.startswith(".github/scripts/"):
        script = path.removeprefix(".github/scripts/")
        if script in {"detect-ci-changes.py", "tests/test_detect_ci_changes.py"}:
            return CHECKS
        if script in RUST_CI_SCRIPTS:
            return ("rust", "containers")
        if script in COMPILER_CI_SCRIPTS or script.startswith("tests/fixtures/compiler-reproducibility/"):
            return ("compiler", "containers")
        if script in OTHER_CI_SCRIPTS or script.startswith("tests/"):
            # Container validation runs the complete helper test suite.
            return ("containers",)
    return CHECKS


def is_compiler_input(path):
    file = PurePosixPath(path)
    return (
        file.name in {"Cargo.toml", "Cargo.lock", "mise.toml", ".mise.toml"}
        or file.name.startswith(("rust-toolchain", "Dockerfile"))
        or file.name.endswith(".Dockerfile")
        or path.startswith((".cargo/", "apps/desktop/src-tauri/gen/apple/"))
    )


def is_container_input(path):
    # READMEs and reStructuredText guides do not feed the images. Other Markdown
    # may be embedded prompts or web content, so keep those build inputs covered.
    file = PurePosixPath(path)
    if file.name.lower() == "readme.md" or file.suffix == ".rst":
        return False
    # Frozen Bun installation resolves every workspace manifest, including the
    # docs and desktop apps. Rust likewise resolves workspace members first.
    if file.name in {"package.json", "bun.lock", "bun.lockb", "bunfig.toml", "Cargo.toml", "Cargo.lock"}:
        return True
    if path in CONTAINER_FILES or path == "assets/mcp-inspector.html" or path.startswith(CONTAINER_TREES):
        return True
    if path.startswith(NON_CONTAINER_TREES):
        return False
    if file.suffix in PROSE_SUFFIXES:
        return False
    # Images use the root build context. New libraries, scripts and root build
    # configuration must stay covered until they are known to be unrelated.
    return True


def classify(paths):
    result = dict.fromkeys(CHECKS, False)
    for path in paths:
        file = PurePosixPath(path)
        if path.startswith(".github/"):
            for check in ci_checks(path):
                result[check] = True
            continue

        result["browser"] |= is_browser_input(path)
        result["ui"] |= path in UI_FILES or path.startswith(UI_TREES)
        result["azure_migrations"] |= path in AZURE_MIGRATION_FILES or path.startswith(AZURE_MIGRATION_TREES)
        result["compiler"] |= is_compiler_input(path)
        result["containers"] |= is_container_input(path)

        package_input = file.name in {"package.json", "bun.lock", "bun.lockb", "bunfig.toml", ".npmrc"}
        result["bun"] |= package_input or path.startswith("patches/") or path == "tests/package-manager-security.e2e.test.ts"
        result["dsql"] |= path.startswith("packages/api/prisma/migrations-dsql/") or path == "packages/api/scripts/dsql-migration.ts"

        # Rust files and build manifests always win, including in frontend trees.
        if file.suffix in {".rs", ".proto", ".wit", ".flow", ".flowscript"} or file.name in {"Cargo.toml", "Cargo.lock", "build.rs"}:
            result["rust"] = True
        elif path.startswith(("docs/", "books/")) or file.suffix in PROSE_SUFFIXES:
            continue
        elif path.startswith(FRONTEND_TREES):
            continue
        elif path.startswith("apps/desktop/") and not path.startswith(("apps/desktop/src-tauri/", "apps/desktop/scripts/")):
            continue
        elif package_input or path.startswith("patches/"):
            continue
        elif path.startswith("templates/widget-"):
            continue
        else:
            result["rust"] = True
    return result


def changed_paths(event_name, event):
    if event_name == "pull_request":
        base = event["pull_request"]["base"]["sha"]
    elif event_name == "push":
        base = event["before"]
    else:
        raise ValueError("This event requires the full set of checks")
    if not re.fullmatch(r"[0-9a-fA-F]{40}", base) or set(base) == {"0"}:
        raise ValueError("No usable base revision")

    present = subprocess.run(["git", "cat-file", "-e", f"{base}^{{commit}}"], capture_output=True)
    if present.returncode:
        subprocess.run(["git", "fetch", "--no-tags", "--depth=1", "origin", base], check=True, timeout=120)
    # Include both sides of renames so moving an input out of a Rust tree still
    # triggers a build. Checkout retains the PR merge commit for the actual tests.
    diff = subprocess.check_output(["git", "diff", "--no-renames", "--name-only", "-z", base, "HEAD", "--"], timeout=60)
    return [name.decode("utf-8", errors="surrogateescape") for name in diff.split(b"\0") if name]


def main():
    try:
        event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
        result = classify(changed_paths(os.environ["GITHUB_EVENT_NAME"], event))
    except (KeyError, OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"::warning::Could not compare CI inputs; running all checks ({type(error).__name__}).")
        result = dict.fromkeys(CHECKS, True)

    output = "".join(f"{name}={str(enabled).lower()}\n" for name, enabled in result.items())
    print(output, end="")
    with open(os.environ["GITHUB_OUTPUT"], "a") as handle:
        handle.write(output)


if __name__ == "__main__":
    main()
