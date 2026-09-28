#!/usr/bin/env python3
"""Compare optimized compiler output from independent clean builds."""

import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


FIXTURE = Path(__file__).resolve().parent / "tests/fixtures/compiler-reproducibility"


def build_environment(target_dir):
    # A compiler cache or inherited optimization workaround can hide a regression.
    env = {
        key: value for key, value in os.environ.items()
        if not key.startswith("CARGO_PROFILE_")
    }
    env.update({
        "RUSTC_WRAPPER": "",
        "RUSTC_WORKSPACE_WRAPPER": "",
        "RUSTFLAGS": "",
        "CARGO_ENCODED_RUSTFLAGS": "",
        "CARGO_INCREMENTAL": "0",
        "CARGO_BUILD_BUILD_DIR": str(target_dir),
    })
    return env


def check_reproducibility(target, attempts, output_dir):
    if attempts < 2:
        raise ValueError("At least two independent builds are required")
    output_dir = Path(output_dir).resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    if any(output_dir.iterdir()):
        raise ValueError(f"Output directory must be empty: {output_dir}")

    target_dir = output_dir / "target"
    env = build_environment(target_dir)
    version = subprocess.check_output(["rustc", "--version", "--verbose"], env=env, text=True)
    print(version, end="", flush=True)
    print(f"Checking {target}; artifacts: {output_dir}", flush=True)
    reference = None
    reference_digest = None

    for attempt in range(1, attempts + 1):
        # Reuse the path, but delete every crate's output before each compilation.
        if target_dir.exists():
            shutil.rmtree(target_dir)
        subprocess.run([
            "cargo", "rustc", "--locked", "--release", "--lib",
            "--manifest-path", str(FIXTURE / "Cargo.toml"),
            "--target", target, "--target-dir", str(target_dir),
            "--", "--emit=obj",
        ], cwd=FIXTURE, env=env, check=True)

        # Compare complete compiler objects. This also works without a target linker.
        deps = target_dir / target / "release/deps"
        objects = list(deps.glob("compiler_reproducibility-*.o"))
        objects += list(deps.glob("compiler_reproducibility-*.obj"))
        if len(objects) != 1:
            raise RuntimeError(f"Expected one fixture object in {deps}, found {len(objects)}")
        data = objects[0].read_bytes()
        if not data:
            raise RuntimeError("Compiler produced an empty fixture object")
        saved = output_dir / f"build-{attempt}{objects[0].suffix}"
        saved.write_bytes(data)
        digest = hashlib.sha256(data).hexdigest()
        print(f"Build {attempt}/{attempts}: {len(data)} bytes, SHA-256 {digest}", flush=True)

        if reference is None:
            reference, reference_digest = data, digest
        elif data != reference:
            raise RuntimeError(
                f"Compiler output differs for {target}: build 1 ({reference_digest}) "
                f"and build {attempt} ({digest}). Objects retained in {output_dir}"
            )

    shutil.rmtree(target_dir)
    print(f"All {attempts} clean optimized builds are byte-identical for {target}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, help="Installed Rust target triple")
    parser.add_argument("--attempts", type=int, default=4)
    parser.add_argument("--output-dir", type=Path, help="Empty directory for compared objects")
    args = parser.parse_args()
    output_dir = args.output_dir or Path(tempfile.mkdtemp(prefix="compiler-reproducibility-"))
    try:
        check_reproducibility(args.target, args.attempts, output_dir)
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
