"""Pinned Linux build inputs and the cached native GCC used by the arm64 runtime pack."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener

# Official Docker Hub ubuntu:22.04 multi-platform index, resolved on 2026-10-06.
IMAGE = "ubuntu:22.04@sha256:5ec03bb3441e8b0bf3b4f9cd4629a1ae763010dc3035bb8da3ae6cf026486401"
SNAPSHOT = "20261005T000000Z"
GCC_VERSION = "14.3.0"
LIBRARIES = ("libgcc_s.so.1", "libstdc++.so.6", "libgomp.so.1")
# GNU release sha512.sum files and GCC 14.3.0's contrib/prerequisites.sha512.
SOURCES = {
    "gcc-14.3.0.tar.xz": ("https://gcc.gnu.org/pub/gcc/releases/gcc-14.3.0/",
        "cb4e3259640721bbd275c723fe4df53d12f9b1673afb3db274c22c6aa457865dccf2d6ea20b4fd4c591f6152e6d4b87516c402015900f06ce9d43af66d3b7a93"),
    "binutils-2.44.tar.xz": ("https://sourceware.org/pub/binutils/releases/",
        "b85d3bbc0e334cf67a96219d3c7c65fbf3e832b2c98a7417bf131f3645a0307057ec81cd2b29ff2563cec53e3d42f73e2c60cc5708e80d4a730efdcc6ae14ad7"),
    "gmp-6.2.1.tar.bz2": ("https://gcc.gnu.org/pub/gcc/infrastructure/",
        "8904334a3bcc5c896ececabc75cda9dec642e401fb5397c4992c4fabea5e962c9ce8bd44e8e4233c34e55c8010cc28db0545f5f750cbdbb5f00af538dc763be9"),
    "mpfr-4.1.0.tar.bz2": ("https://gcc.gnu.org/pub/gcc/infrastructure/",
        "410208ee0d48474c1c10d3d4a59decd2dfa187064183b09358ec4c4666e34d74383128436b404123b831e585d81a9176b24c7ced9d913967c5fce35d4040a0b4"),
    "mpc-1.2.1.tar.gz": ("https://gcc.gnu.org/pub/gcc/infrastructure/",
        "3279f813ab37f47fdcc800e4ac5f306417d07f539593ca715876e43e04896e1d5bceccfb288ef2908a3f24b760747d0dbd0392a24b9b341bc3e12082e5c836ee"),
    "isl-0.24.tar.bz2": ("https://gcc.gnu.org/pub/gcc/infrastructure/",
        "aab3bddbda96b801d0f56d2869f943157aad52a6f6e6a61745edd740234c635c38231af20bc3f1a08d416a5e973a90e18249078ed8e4ae2f1d5de57658738e95"),
}
MAX_SOURCE_BYTES = 128 * 1024**2


def digest(path, algorithm="sha256"):
    result = hashlib.new(algorithm)
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def identity():
    return hashlib.sha256(json.dumps({"image": IMAGE, "snapshot": SNAPSHOT, "sources": SOURCES,
        "recipe": digest(Path(__file__)),
        "build": digest(Path(__file__).with_name("build_linux_runtime.sh"))}, sort_keys=True).encode()).hexdigest()


class HttpsRedirects(HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, newurl):
        if urlsplit(newurl).scheme != "https":
            raise ValueError("Compiler source downloads must stay on HTTPS")
        return super().redirect_request(request, fp, code, message, headers, newurl)


def fetch(name, destination, opener=None):
    base, expected = SOURCES[name]
    if destination.is_file() and not destination.is_symlink() and digest(destination, "sha512") == expected:
        return
    with tempfile.NamedTemporaryFile(dir=destination.parent, prefix=".source-", delete=False) as partial:
        try:
            request = Request(base + name, headers={"Accept-Encoding": "identity"})
            with (opener or build_opener(HttpsRedirects)).open(request, timeout=120) as response:
                total = 0
                while block := response.read(1024 * 1024):
                    total += len(block)
                    if total > MAX_SOURCE_BYTES:
                        raise ValueError(f"Compiler source {name} exceeds {MAX_SOURCE_BYTES} bytes")
                    partial.write(block)
            partial.close()
            if digest(Path(partial.name), "sha512") != expected:
                raise ValueError(f"Compiler source {name} does not match its pinned SHA512")
            Path(partial.name).replace(destination)
        finally:
            Path(partial.name).unlink(missing_ok=True)


def command(*args, cwd=None, env=None):
    subprocess.run([str(arg) for arg in args], cwd=cwd, env=env, check=True)


def output(*args, env=None):
    return subprocess.check_output([str(arg) for arg in args], env=env, text=True).strip()


def manifest(prefix):
    value = json.loads((prefix / "runtime-toolchain.json").read_text())
    if value.get("identity") != identity() or value.get("version") != GCC_VERSION:
        raise ValueError("Cached runtime compiler does not match the pinned build inputs")
    return value


def library_path(prefix):
    prefix = prefix.resolve()
    paths = {(prefix / entry["path"]).resolve().parent for entry in manifest(prefix)["libraries"].values()}
    if not paths or any(not path.is_relative_to(prefix) for path in paths):
        raise ValueError("Runtime compiler libraries must stay inside the cache")
    return ":".join(str(path) for path in sorted(paths))


def library_notice(name, source):
    configured = os.environ.get("RUNTIME_TOOLCHAIN_ROOT")
    if not configured:
        raise ValueError("The arm64 pack requires RUNTIME_TOOLCHAIN_ROOT from the pinned GCC build")
    prefix = Path(configured).resolve()
    entry = manifest(prefix)["libraries"][name]
    expected = (prefix / entry["path"]).resolve()
    if not expected.is_relative_to(prefix) or source.resolve() != expected or digest(source) != entry["sha256"]:
        raise ValueError(f"{name} is not from the pinned GCC {GCC_VERSION} build")
    notices = prefix / "share/runtime-licenses"
    text = (f"Source: {SOURCES['gcc-14.3.0.tar.xz'][0]}gcc-14.3.0.tar.xz\n"
            f"SHA512: {SOURCES['gcc-14.3.0.tar.xz'][1]}\n\n"
            + (notices / "COPYING3").read_text() + "\n" + (notices / "COPYING.RUNTIME").read_text())
    return f"GCC {GCC_VERSION} ({name}; GPLv3 with GCC Runtime Library Exception)", text


def build(prefix, work, jobs):
    prefix, work = prefix.resolve(), work.resolve()
    marker = prefix / "runtime-toolchain.json"
    if marker.exists():
        value = manifest(prefix)
        for name, entry in value["libraries"].items():
            source = prefix / entry["path"]
            if not source.resolve().is_relative_to(prefix) or digest(source) != entry["sha256"]:
                raise ValueError(f"Cached {name} was changed after the compiler build")
        if output(prefix / "bin/gcc", "-dumpfullversion") != GCC_VERSION:
            raise ValueError("Cached GCC does not report its pinned version")
        return
    if any(prefix.iterdir()):
        raise ValueError("The runtime compiler cache is incomplete; use an empty prefix")
    work.mkdir(parents=True, exist_ok=True)
    environment = {**os.environ, "CC": "gcc-12", "CXX": "g++-12", "CFLAGS": "-O2", "CXXFLAGS": "-O2",
                   "CONFIG_SHELL": "/bin/bash", "SOURCE_DATE_EPOCH": "1748822400"}
    for name in SOURCES:
        path = work / name
        fetch(name, path)
        # Only hash-verified GNU release archives reach tar.
        command("tar", "-xf", path, "--no-same-owner", "-C", work)
    gcc = work / "gcc-14.3.0"
    for component in ("gmp", "mpfr", "mpc", "isl"):
        archive = next(name for name in SOURCES if name.startswith(component + "-"))
        (gcc / component).symlink_to(work / archive.split(".tar.")[0], target_is_directory=True)
    binutils_build = work / "binutils-build"
    binutils_build.mkdir()
    command(work / "binutils-2.44/configure", f"--prefix={prefix}", "--disable-nls", "--disable-werror",
            "--disable-gprofng", "--without-debuginfod", cwd=binutils_build, env=environment)
    command("make", f"-j{jobs}", cwd=binutils_build, env=environment)
    command("make", "install", cwd=binutils_build, env=environment)
    gcc_build = work / "gcc-build"
    gcc_build.mkdir()
    command(gcc / "configure", f"--prefix={prefix}", f"--libdir={prefix / 'lib'}", "--enable-languages=c,c++",
            "--disable-bootstrap", "--disable-multilib", "--enable-multiarch", "--disable-nls", "--disable-libsanitizer",
            "--disable-libquadmath", "--without-zstd", "--enable-checking=release",
            f"--with-as={prefix / 'bin/as'}", f"--with-ld={prefix / 'bin/ld'}", cwd=gcc_build, env=environment)
    command("make", f"-j{jobs}", cwd=gcc_build, env=environment)
    command("make", "install-strip", cwd=gcc_build, env=environment)
    if output(prefix / "bin/gcc", "-dumpfullversion") != GCC_VERSION:
        raise ValueError("Built GCC does not report its pinned version")
    libraries = {}
    for name in LIBRARIES:
        source = Path(output(prefix / "bin/g++", f"-print-file-name={name}")).resolve()
        if not source.is_file() or not source.is_relative_to(prefix):
            raise ValueError(f"GCC did not build its own {name}")
        libraries[name] = {"path": str(source.relative_to(prefix)), "sha256": digest(source)}
    # Exercise native multiarch headers, CRT objects, shared C++ and OpenMP before caching.
    probe = work / "compiler-probe.cpp"
    probe.write_text("#include <iostream>\n#include <omp.h>\n"
                     "int main() { std::cout << omp_get_max_threads() << '\\n'; }\n")
    command(prefix / "bin/g++", "-std=c++17", "-fopenmp", probe, "-o", work / "compiler-probe", env=environment)
    runtime_dirs = sorted({str((prefix / entry["path"]).parent) for entry in libraries.values()})
    command(work / "compiler-probe", env={**environment, "LD_LIBRARY_PATH": ":".join(runtime_dirs), "OMP_NUM_THREADS": "2"})
    notices = prefix / "share/runtime-licenses"
    notices.mkdir(parents=True)
    for name in ("COPYING3", "COPYING.RUNTIME"):
        shutil.copyfile(gcc / name, notices / name)
    marker.write_text(json.dumps({"identity": identity(), "version": GCC_VERSION, "libraries": libraries}, sort_keys=True))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("pins", "build", "library-path"))
    parser.add_argument("--prefix", type=Path, default=Path("/toolchain"))
    parser.add_argument("--work", type=Path, default=Path("/scratch/runtime-toolchain-build"))
    parser.add_argument("--jobs", type=int, default=os.cpu_count() or 2)
    args = parser.parse_args()
    if args.action == "pins":
        print(f"image={IMAGE}\nsnapshot={SNAPSHOT}\nkey={identity()}")
    elif args.action == "library-path":
        print(library_path(args.prefix))
    else:
        if args.jobs < 1:
            parser.error("--jobs must be positive")
        args.prefix.mkdir(parents=True, exist_ok=True)
        build(args.prefix, args.work, args.jobs)


if __name__ == "__main__":
    main()
