#!/usr/bin/env python3
"""Check that compiler overrides follow the repository's Rust toolchain."""

import argparse
import json
import os
from pathlib import Path, PurePosixPath
import re
import shlex
import sys
import tomllib


SETUP_ACTION = "./.github/actions/setup-rust"
CENTRALIZED_CI = (
    ".github/actions/setup-environment/action.yml",
    ".github/workflows/clippy.yml",
    ".github/workflows/compile-guardrails.yml",
    ".github/workflows/release-native.yml",
    ".github/workflows/standalone-release.yml",
    ".github/workflows/standalone-release-renew.yml",
)
PACKAGE_DOCKER = "packages/api/Dockerfile.tools"
SKIP_DIRECTORIES = {".git", ".agents", ".claude", ".codex", "node_modules", "target", ".next", ".turbo", ".venv", "vendor"}
RUST_COMMAND = re.compile(r"\bcargo(?:\s+\+\S+)?\s+(?:build|check|test|install|rustc|clippy|fmt|fetch|metadata|chef)\b")
RUST_INSTALL = re.compile(r"(?:^|[/\s])rustup\s+toolchain\s+install\b")
OVERRIDE = re.compile(
    r"^[ \t]*(?:(?:ENV|export)[ \t]+)?RUSTUP_TOOLCHAIN(?:[ \t]*[:=][ \t]*|[ \t]+)"
    r"[\"']?([^\s\"';#]+)", re.MULTILINE
)


def configuration_files(root):
    for directory, names, files in os.walk(root):
        names[:] = sorted(name for name in names if name not in SKIP_DIRECTORIES
                          and not (Path(directory) / name / ".git").exists())
        for name in sorted(files):
            path = Path(directory) / name
            relative = path.relative_to(root).as_posix()
            if (name in {"mise.toml", ".mise.toml", "rust-toolchain.toml", "rust-toolchain"}
                    or "Dockerfile" in name or path.suffix == ".pbxproj"
                    or path.suffix in {".yml", ".yaml"}
                    or relative == "templates/wasm-node-rust/README.md"):
                yield path


def instructions(text):
    """Read Docker instructions, including shell continuations."""
    pending = ""
    start = 0
    for number, line in enumerate(text.splitlines(), 1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if not pending:
            start = number
        pending += line.rstrip().removesuffix("\\") + " "
        if line.rstrip().endswith("\\"):
            continue
        kind, _, value = pending.strip().partition(" ")
        yield start, kind.upper(), value.strip()
        pending = ""
    if pending:
        yield start, *pending.strip().split(maxsplit=1)


def copied_pin(value, workdir):
    if value.startswith("["):
        parts = json.loads(value)
    else:
        parts = shlex.split(value)
    if any(part.startswith("--from") for part in parts):
        return False
    parts = [part for part in parts if not part.startswith("--")]
    if len(parts) < 2 or "rust-toolchain.toml" not in [part.lstrip("./") for part in parts[:-1]]:
        return False
    destination = PurePosixPath(parts[-1])
    if not destination.is_absolute():
        destination = PurePosixPath(workdir) / destination
    # Installing at / keeps the pin visible after later WORKDIR instructions.
    return destination in {PurePosixPath("/"), PurePosixPath("/rust-toolchain.toml")}


def check_docker(relative, text):
    errors = []
    stages = {}
    state = None
    package_context = relative == PACKAGE_DOCKER
    for line, kind, value in instructions(text):
        if kind == "FROM":
            words = shlex.split(value)
            words = [word for word in words if not word.startswith("--")]
            base = words[0]
            inherited = stages.get(base.lower(), {})
            state = {
                "pin": inherited.get("pin", False),
                "installed": inherited.get("installed", False),
                "override": inherited.get("override", False),
                "workdir": inherited.get("workdir", "/"),
            }
            if len(words) > 2 and words[-2].lower() == "as":
                stages[words[-1].lower()] = state
        elif state is not None:
            if kind == "WORKDIR":
                state["workdir"] = str(PurePosixPath(state["workdir"]) / value)
            elif kind == "COPY" and copied_pin(value, state["workdir"]):
                state["pin"] = True
            elif kind == "ENV" and re.match(r"RUSTUP_TOOLCHAIN(?:=|\s)", value):
                state["override"] = True
            elif kind == "RUN":
                install = RUST_INSTALL.search(value)
                cargo = RUST_COMMAND.search(value)
                if install:
                    if state["pin"] or (package_context and state["override"]):
                        state["installed"] = True
                    else:
                        errors.append(f"{relative}:{line}: install Rust after copying the root toolchain pin")
                if cargo and (not state["installed"] or (install and cargo.start() < install.start())):
                    errors.append(f"{relative}:{line}: Rust compilation must use the installed repository toolchain")
    return errors


def check_repository(root):
    root = Path(root)
    expected = tomllib.loads((root / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    errors = []

    def check_pin(relative, value, label):
        if value != expected:
            errors.append(f"{relative}: {label} is {value!r}; expected {expected!r} from rust-toolchain.toml")

    for path in configuration_files(root):
        relative = path.relative_to(root).as_posix()
        text = path.read_text()
        if path.name in {"mise.toml", ".mise.toml"}:
            rust = tomllib.loads(text).get("tools", {}).get("rust")
            if rust is not None:
                check_pin(relative, rust.get("version") if isinstance(rust, dict) else rust, "Rust version")
        elif path.name == "rust-toolchain.toml":
            check_pin(relative, tomllib.loads(text)["toolchain"]["channel"], "Rust channel")
        elif path.name == "rust-toolchain":
            check_pin(relative, text.strip(), "Rust channel")

        for match in OVERRIDE.finditer(text):
            value = match.group(1)
            if not value.startswith("$"):
                check_pin(relative, value, "RUSTUP_TOOLCHAIN")

        for pattern in (
            r"\bcargo\s+\+([^\s\"';]+)",
            r"\brustup\s+(?:default|override\s+set)[ \t]+[\"']?([^\s\"';&|]+)",
            r"\brustup\s+toolchain\s+(?:install|add)[ \t]+"
            r"(?:(?:--(?:profile|component|target)(?:=|[ \t]+)[^\s]+|--(?:force|no-self-update))[ \t]+)*"
            r"[\"']?([^\s\"';&|]+)",
            r"--default-toolchain[=\s]+[\"']?([^\s\"';&|]+)",
        ):
            for pin in re.findall(pattern, text):
                if not pin.startswith(("$", "-")) and pin != "none":
                    check_pin(relative, pin, "explicit compiler selection")

        if path.suffix in {".yml", ".yaml"}:
            uses = re.findall(r"^\s*-?\s*uses:\s*[\"']?([^\s\"'#]+)", text, re.MULTILINE)
            if relative.startswith(".github/") and any("rust-toolchain@" in action for action in uses):
                errors.append(f"{relative}: use {SETUP_ACTION} to read the root compiler pin")
            if relative in CENTRALIZED_CI and SETUP_ACTION not in uses:
                errors.append(f"{relative}: missing {SETUP_ACTION}")
            if any("rust-toolchain@" in action for action in uses):
                pins = re.findall(r"^\s*toolchain:\s*[\"']?([^\s\"'#]+)", text, re.MULTILINE)
                if not pins:
                    errors.append(f"{relative}: exported template must explicitly pin its Rust compiler")
                for pin in pins:
                    check_pin(relative, pin, "Rust action toolchain")

        if relative == "templates/wasm-node-rust/README.md":
            for pin in re.findall(r"\bRust\s+`?((?:\d+\.\d+\.\d+)|(?:(?:beta|nightly)-\d{4}-\d{2}-\d{2}))", text):
                check_pin(relative, pin, "documented Rust compiler")
        if "Dockerfile" in path.name:
            errors.extend(check_docker(relative, text))
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    errors = check_repository(args.root)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print("Rust compiler configuration follows rust-toolchain.toml.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
