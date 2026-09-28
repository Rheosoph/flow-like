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


def build_environment(target_dir, linked=False):
    # Disable caches; the object check also excludes inherited compiler workarounds.
    env = {
        key: value for key, value in os.environ.items()
        if linked or not key.startswith("CARGO_PROFILE_")
    }
    env.update({
        "RUSTC_WRAPPER": "",
        "RUSTC_WORKSPACE_WRAPPER": "",
        "CARGO_INCREMENTAL": "0",
        "CARGO_BUILD_BUILD_DIR": str(target_dir),
    })
    if not linked:
        env.update({"RUSTFLAGS": "", "CARGO_ENCODED_RUSTFLAGS": ""})
    return env


def check_reproducibility(target, attempts, output_dir, linked=False):
    if attempts < 2:
        raise ValueError("At least two independent builds are required")
    output_dir = Path(output_dir).resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    if any(output_dir.iterdir()):
        raise ValueError(f"Output directory must be empty: {output_dir}")

    target_dir = output_dir / "target"
    env = build_environment(target_dir, linked)
    version = subprocess.check_output(["rustc", "--version", "--verbose"], env=env, text=True)
    print(version, end="", flush=True)
    print(f"Checking {target}; artifacts: {output_dir}", flush=True)
    reference = None
    reference_digest = None

    for attempt in range(1, attempts + 1):
        # Reuse the path, but delete every crate's output before each compilation.
        if target_dir.exists():
            shutil.rmtree(target_dir)
        command = ["cargo", "build" if linked else "rustc", "--locked", "--release"]
        command += ["--bin", "compiler-reproducibility"] if linked else ["--lib"]
        command += [
            "--manifest-path", str(FIXTURE / "Cargo.toml"),
            "--target", target, "--target-dir", str(target_dir),
        ]
        if not linked:
            command += ["--", "--emit=obj"]
        subprocess.run(command, cwd=FIXTURE, env=env, check=True)

        # Object checks also work without a target linker. Linked checks need a native runner.
        deps = target_dir / target / "release/deps"
        if linked:
            suffix = ".exe" if "windows" in target else ""
            objects = [target_dir / target / "release" / f"compiler-reproducibility{suffix}"]
        else:
            objects = list(deps.glob("compiler_reproducibility-*.o"))
            objects += list(deps.glob("compiler_reproducibility-*.obj"))
        if len(objects) != 1:
            raise RuntimeError(f"Expected one fixture object in {deps}, found {len(objects)}")
        data = objects[0].read_bytes()
        if not data:
            raise RuntimeError("Compiler produced an empty fixture output")
        saved = output_dir / f"build-{attempt}{objects[0].suffix}"
        saved.write_bytes(data)
        if linked:
            # Exercise the executable on its native runner, including its loader.
            subprocess.run([str(objects[0])], env=env, check=True, timeout=15)
        digest = hashlib.sha256(data).hexdigest()
        print(f"Build {attempt}/{attempts}: {len(data)} bytes, SHA-256 {digest}", flush=True)

        if reference is None:
            reference, reference_digest = data, digest
        elif data != reference:
            raise RuntimeError(
                f"{'Linked executable' if linked else 'Compiler output'} differs for {target}: "
                f"build 1 ({reference_digest}) and build {attempt} ({digest}). "
                f"Outputs retained in {output_dir}"
            )

    shutil.rmtree(target_dir)
    print(f"All {attempts} clean optimized builds are byte-identical for {target}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, help="Installed Rust target triple")
    parser.add_argument("--attempts", type=int, default=4)
    parser.add_argument("--output-dir", type=Path, help="Empty directory for compared outputs")
    parser.add_argument("--linked", action="store_true",
                        help="Build and run native executables with the caller's release flags")
    args = parser.parse_args()
    output_dir = args.output_dir or Path(tempfile.mkdtemp(prefix="compiler-reproducibility-"))
    try:
        check_reproducibility(args.target, args.attempts, output_dir, args.linked)
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
