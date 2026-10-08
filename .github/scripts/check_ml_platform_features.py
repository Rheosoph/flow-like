#!/usr/bin/env python3
"""Check Cargo's selected ML backends and the matching Rust cfgs without GPU hardware."""

import argparse
import os
from pathlib import Path
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]
BACKENDS = {"flex": "cpu", "wgpu": "wgpu", "cuda": "cuda", "rocm": "rocm"}
TARGETS = [
    ("x86_64-unknown-linux-gnu", "linux", "x86_64", "gnu", {"cpu", "cuda", "rocm", "wgpu"}),
    ("x86_64-pc-windows-msvc", "windows", "x86_64", "msvc", {"cpu", "cuda", "rocm", "wgpu"}),
    ("aarch64-apple-darwin", "macos", "aarch64", "", {"cpu", "wgpu"}),
    ("x86_64-apple-darwin", "macos", "x86_64", "", {"cpu", "wgpu"}),
    ("aarch64-unknown-linux-gnu", "linux", "aarch64", "gnu", {"cpu", "wgpu"}),
    ("aarch64-pc-windows-msvc", "windows", "aarch64", "msvc", {"cpu", "wgpu"}),
    ("x86_64-unknown-linux-musl", "linux", "x86_64", "musl", {"cpu", "wgpu"}),
    ("x86_64-pc-windows-gnu", "windows", "x86_64", "gnu", {"cpu", "wgpu"}),
    ("aarch64-apple-ios", "ios", "aarch64", "", {"cpu"}),
    ("aarch64-linux-android", "android", "aarch64", "", {"cpu"}),
    ("wasm32-unknown-unknown", "unknown", "wasm32", "", {"cpu"}),
]


def cargo_features(package, target, features, offline, depth):
    command = [
        "cargo", "tree", "--locked", "--package", package,
        "--no-default-features", "--target", target, "--edges", "normal,build",
        "--depth", str(depth), "--prefix", "none", "--format", "{p}|{f}",
    ]
    if features:
        command += ["--features", ",".join(features)]
    if offline:
        command.append("--offline")
    output = subprocess.check_output(command, cwd=ROOT, text=True)
    packages = {}
    for line in output.splitlines():
        dependency, _, enabled = line.partition("|")
        packages[dependency.split(" v", 1)[0]] = set(enabled.removesuffix(" (*)").split(",")) - {""}
    return packages


def cargo_backends(target, features, offline):
    packages = cargo_features("flow-like-ml-burn", target, features, offline, 1)
    return {BACKENDS[item] for item in packages.get("burn", set()) if item in BACKENDS}


def check_product_features(offline):
    target = "x86_64-unknown-linux-gnu"
    native = cargo_features("flow-like-ml-burn", target, ["training-auto"], offline, 10)
    cuda = native["cudarc"]
    assert "fallback-dynamic-loading" in cuda and "fallback-latest" in cuda
    assert not cuda.intersection({"dynamic-linking", "static-linking"}), "CUDA must load drivers at runtime"
    for feature in ["server-local-metadata", "executor-local-metadata", "remote-metadata"]:
        packages = cargo_features("flow-like-catalog", target, [feature], offline, 3)
        enabled = packages["flow-like-catalog"]
        expected = feature != "remote-metadata"
        assert ("training-metadata" in enabled) == expected, f"{feature}: incorrect training metadata"
        assert "training-auto" not in enabled and "burn" not in packages, f"{feature}: enables a training engine"
    for feature in ["desktop", "server-local-ml", "executor-local-ml", "local-runtime-ml"]:
        packages = cargo_features("flow-like-catalog", target, [feature], offline, 0)
        assert "training-auto" in packages["flow-like-catalog"], f"{feature}: missing automatic training"
    for feature in ["mobile", "mobile-apple", "server", "executor", "local-runtime", "local-ml"]:
        packages = cargo_features("flow-like-catalog", target, [feature], offline, 0)
        assert "training-auto" not in packages["flow-like-catalog"], f"{feature}: unexpectedly enables automatic training"
    for feature in ["default", "frontend", "training-auto"]:
        packages = cargo_features("flow-like-standalone", target, [feature], offline, 1)
        assert "training-auto" in packages["flow-like-catalog"], f"standalone {feature}: missing automatic training"
    packages = cargo_features("flow-like-standalone", target, ["training-cpu"], offline, 1)
    assert "training-cpu" in packages["flow-like-catalog"]
    assert "training-auto" not in packages["flow-like-catalog"]
    print("Product defaults, standalone CPU opt-out, and API metadata isolation passed", flush=True)


def build_cfgs(executable, os_name, architecture, target_env, features):
    env = {key: value for key, value in os.environ.items() if not key.startswith("CARGO_FEATURE_")}
    env.update({
        "CARGO_CFG_TARGET_OS": os_name,
        "CARGO_CFG_TARGET_ARCH": architecture,
        "CARGO_CFG_TARGET_ENV": target_env,
    })
    for feature in features:
        env["CARGO_FEATURE_" + feature.upper().replace("-", "_")] = "1"
    output = subprocess.check_output([str(executable)], env=env, text=True)
    prefix = "cargo:rustc-cfg=ml_backend_"
    return {line[len(prefix):] for line in output.splitlines() if line.startswith(prefix)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true")
    arguments = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="ml-feature-matrix-") as directory:
        executable = Path(directory) / ("build-cfg.exe" if os.name == "nt" else "build-cfg")
        subprocess.run(
            ["rustc", "--edition=2024", str(ROOT / "packages/ml/burn/build.rs"), "-o", str(executable)],
            cwd=ROOT, check=True,
        )
        for target, os_name, architecture, target_env, automatic in TARGETS:
            for features, expected in [(["training-auto"], automatic), (["cpu"], {"cpu"}), ([], set())]:
                selected = cargo_backends(target, features, arguments.offline)
                cfgs = build_cfgs(executable, os_name, architecture, target_env, features)
                assert selected == expected, f"{target} {features}: Cargo selected {selected}, expected {expected}"
                assert cfgs == selected, f"{target} {features}: Rust cfgs {cfgs} disagree with Cargo {selected}"
            print(f"{target}: automatic={','.join(sorted(automatic))}; CPU and metadata isolation passed", flush=True)
    check_product_features(arguments.offline)


if __name__ == "__main__":
    main()
