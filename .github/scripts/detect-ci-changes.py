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
CHECKS = ("rust", "bun", "dsql", "browser")
# The real-browser jobs follow the browser crate, the automation nodes, every local crate either one
# depends on (any target or dependency kind, read from the manifests so the set cannot drift), and
# the toolchain.
BROWSER_ROOTS = ("packages/browser", "packages/catalog/automation")
BROWSER_TREES = (".cargo/", ".github/actions/")
BROWSER_FILES = {
    "Cargo.lock", "Cargo.toml", "rust-toolchain.toml",
    ".github/workflows/browser.yml", ".github/scripts/detect-ci-changes.py",
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


def classify(paths):
    result = dict.fromkeys(CHECKS, False)
    for path in paths:
        file = PurePosixPath(path)
        result["browser"] |= is_browser_input(path)
        if path.startswith(".github/"):
            result.update(rust=True, bun=True, dsql=True)
            continue

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
