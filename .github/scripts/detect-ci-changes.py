#!/usr/bin/env python3
"""Select CI workloads without GitHub's changed-file API pagination limits."""

import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess


# Skip Rust only for known frontend trees and prose. Unknown inputs still build.
FRONTEND_TREES = (
    "apps/book/", "apps/docs/", "apps/website/", "apps/web/",
    "apps/embedded/", "apps/extension/", "apps/translation/",
    "packages/ui/", "packages/locales/", "packages/widget-sdk/",
    "packages/widget-bundler/", "packages/js-video-url-parser/",
    "packages/dexie-tauri-adapter/frontend/",
)
PROSE_SUFFIXES = {".md", ".mdx", ".rst"}


def classify(paths):
    result = dict.fromkeys(("rust", "bun", "dsql"), False)
    for path in paths:
        file = PurePosixPath(path)
        if path.startswith(".github/"):
            return dict.fromkeys(result, True)

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
        result = dict.fromkeys(("rust", "bun", "dsql"), True)

    output = "".join(f"{name}={str(enabled).lower()}\n" for name, enabled in result.items())
    print(output, end="")
    with open(os.environ["GITHUB_OUTPUT"], "a") as handle:
        handle.write(output)


if __name__ == "__main__":
    main()
