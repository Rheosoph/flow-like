"""Build, describe for signing and publish the runtime packs (llama.cpp, MLX) that standalone agents install."""

import argparse
import contextlib
import gzip
import hashlib
import importlib.util
import io
import itertools
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import tarfile
import tempfile
import time
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener

_spec = importlib.util.spec_from_file_location("standalone_release", Path(__file__).with_name("standalone_release.py"))
release = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(release)

_spec = importlib.util.spec_from_file_location("runtime_toolchain", Path(__file__).with_name("runtime_toolchain.py"))
toolchain = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(toolchain)

LLAMACPP_BUILD, LLAMACPP_BUILD_NUMBER = "b11429", 11429
LLAMACPP_COMMIT = "d81235049384534c167caea52b85a694f6103d14"
LLAMACPP_DOWNLOADS = "https://github.com/ggml-org/llama.cpp/releases/download"
# Official v0.6.0 selects b11429. GitHub's asset digests were checked on 2026-10-06.
UPSTREAM_ARCHIVES = {
    "macos-arm64": (11971406, "740288ec6887be94280a5dfa25b5e23a78285cab104519e6c7e218904ee82459"),
    "macos-x64": (11487431, "29ac3ea02be6bd143e824973f2cc5fa74bc4094393a9eaab0ff6814f19dd8522"),
    "ubuntu-x64": (17693462, "f6d25dde8f51133143d1453da4fd5f73b145127177612a283bf7995957af3392"),
    "ubuntu-vulkan-x64": (31636673, "632c4e98feba2b94407a2130e3133e0c3aefb0ea1ab41337e926d8bfafdd0b74"),
}
JWS_TYPE = release.RUNTIME_MANIFEST_JWS_TYPE
LISTING, FALLBACK, NOTICES = "pack.json", "fallback", "THIRD-PARTY-NOTICES.txt"
DEBIAN_DOCS = Path("/usr/share/doc")
MAX_PACK_FILES, MAX_PACK_BYTES, MAX_UNPACKED_BYTES = 64, 2 * 1024**3, 4 * 1024**3
MAX_COMPACT_JWS_BYTES, MAX_SAFE_INTEGER = 16384, 9007199254740991
# The management envelope also carries ids, timestamps and the install command.
MAX_FORWARDED_MANIFEST_BYTES = 15 * 1024
GLIBC_FLOOR = (2, 35)
GLIBC_LIBRARIES = frozenset({"libc.so.6", "libm.so.6", "libdl.so.2", "libpthread.so.0", "librt.so.1", "libresolv.so.2",
                             "libutil.so.1", "libmvec.so.1", "ld-linux-x86-64.so.2", "ld-linux-aarch64.so.1"})
SYSTEM_MACHO_PREFIXES = ("/usr/lib/", "/System/Library/")
HOST_TARGETS = {("Linux", "x86_64"): "x86_64-unknown-linux-gnu", ("Linux", "aarch64"): "aarch64-unknown-linux-gnu",
                ("Darwin", "x86_64"): "x86_64-apple-darwin", ("Darwin", "arm64"): "aarch64-apple-darwin"}

# Each library ships once under the name the loader asks for: the macOS install name, the
# Linux soname of a core library, or the plain name ggml uses to open a backend plugin.
MAC_LIBRARIES = ("libggml-base.0.dylib", "libggml-blas.0.dylib", "libggml-cpu.0.dylib", "libggml-rpc.0.dylib",
                 "libggml.0.dylib", "libllama-common.0.dylib", "libllama-server-impl.dylib", "libllama.0.dylib",
                 "libmtmd.0.dylib")
LINUX_LIBRARIES = ("libggml-base.so.0", "libggml.so.0", "libllama-common.so.0", "libllama-server-impl.so",
                   "libllama.so.0", "libmtmd.so.0")
X64_CPU_BACKENDS = tuple(f"libggml-cpu-{variant}.so" for variant in (
    "alderlake", "cannonlake", "cascadelake", "cooperlake", "haswell", "icelake", "ivybridge", "piledriver",
    "sandybridge", "sapphirerapids", "skylakex", "sse42", "x64", "zen4"))
ARM64_CPU_BACKENDS = tuple(f"libggml-cpu-{variant}.so" for variant in (
    "armv8.0_1", "armv8.2_1", "armv8.2_2", "armv8.2_3", "armv8.6_1", "armv8.6_2", "armv9.2_1", "armv9.2_2"))
# Nothing else in an engine process loads these, so the pack's copies are always used.
BUNDLED = ("libcrypto.so.3", "libgomp.so.1", "libssl.so.3")
# GPU drivers load into the engine and need the host's own C++ runtime and Vulkan loader.
FALLBACK_LIBRARIES = ("libgcc_s.so.1", "libstdc++.so.6")
LLAMACPP_PACKS = {
    "llamacpp-macos-arm64-metal": {"target": "aarch64-apple-darwin", "backend": "metal", "archive": "macos-arm64",
                                   "files": MAC_LIBRARIES + ("libggml-metal.0.dylib",)},
    "llamacpp-macos-x64-cpu": {"target": "x86_64-apple-darwin", "backend": "cpu", "archive": "macos-x64",
                               "files": MAC_LIBRARIES},
    "llamacpp-linux-x64-cpu": {"target": "x86_64-unknown-linux-gnu", "backend": "cpu", "archive": "ubuntu-x64",
                               "files": LINUX_LIBRARIES + X64_CPU_BACKENDS, "bundled": BUNDLED,
                               "fallback": FALLBACK_LIBRARIES},
    "llamacpp-linux-x64-vulkan": {"target": "x86_64-unknown-linux-gnu", "backend": "vulkan",
                                  "archive": "ubuntu-vulkan-x64",
                                  "files": LINUX_LIBRARIES + X64_CPU_BACKENDS + ("libggml-vulkan.so",),
                                  "bundled": BUNDLED, "fallback": FALLBACK_LIBRARIES + ("libvulkan.so.1",)},
    # Upstream's arm64 build needs GLIBC_2.38, so CI compiles the pinned commit on ubuntu-22.04-arm
    # with the source-pinned GCC toolchain; its shared runtimes use the same glibc floor.
    "llamacpp-linux-arm64-cpu": {"target": "aarch64-unknown-linux-gnu", "backend": "cpu", "archive": None,
                                 "files": LINUX_LIBRARIES + ARM64_CPU_BACKENDS, "bundled": BUNDLED,
                                 "fallback": FALLBACK_LIBRARIES, "toolchain": toolchain.GCC_VERSION},
}
# The helper finds SwiftPM bundles beside itself and Swift back-deployment libraries in ../lib.
MLX_ENTRYPOINT = "bin/flow-like-mlx-service"
MLX_METALLIB = "bin/mlx-swift_Cmlx.bundle/Contents/Resources/default.metallib"


def run(*argv, env=None):
    return subprocess.run(argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                          timeout=120, env=env)


def tool(*argv, env=None):
    result = run(*argv, env=env)
    if result.returncode:
        raise ValueError(f"{Path(argv[0]).name} {' '.join(argv[1:-1])} failed for {argv[-1]}: {result.stdout.strip()[-400:]}")
    return result.stdout


def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write(path, data, mode=0o644):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    path.chmod(mode)


def pin():
    return f"build={LLAMACPP_BUILD}\nnumber={LLAMACPP_BUILD_NUMBER}\ncommit={LLAMACPP_COMMIT}"


def upstream_asset(pack):
    asset = LLAMACPP_PACKS[pack]["archive"]
    if asset is None:
        raise ValueError(f"{pack} is compiled from llama.cpp {LLAMACPP_COMMIT}; it has no upstream archive")
    return asset, f"llama-{LLAMACPP_BUILD}-bin-{asset}.tar.gz"


def verify_archive(path, asset):
    size, digest = UPSTREAM_ARCHIVES[asset]
    if path.is_symlink() or not path.is_file() or path.stat().st_size != size or sha256_file(path) != digest:
        raise ValueError(f"{path} is not the pinned llama.cpp {LLAMACPP_BUILD} {asset} archive "
                         f"({size} bytes, sha256 {digest})")


class HttpsRedirects(HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, newurl):
        if urlsplit(newurl).scheme != "https":
            raise ValueError(f"Refusing a redirect to {newurl}; upstream archives are fetched over HTTPS only")
        return super().redirect_request(request, fp, code, message, headers, newurl)


def fetch(pack, output, opener=None):
    asset, name = upstream_asset(pack)
    size = UPSTREAM_ARCHIVES[asset][0]
    output.mkdir(parents=True, exist_ok=True)
    destination = output / name
    request = Request(f"{LLAMACPP_DOWNLOADS}/{LLAMACPP_BUILD}/{name}",
                      headers={"User-Agent": "flow-like-runtime-packs/1", "Accept-Encoding": "identity"})
    with tempfile.NamedTemporaryFile(dir=output, prefix=".download-", delete=False) as partial:
        try:
            with (opener or build_opener(HttpsRedirects)).open(request, timeout=120) as response:
                received = 0
                while chunk := response.read(1024 * 1024):
                    received += len(chunk)
                    if received > size:
                        raise ValueError(f"{name} is larger than its pinned {size} bytes")
                    partial.write(chunk)
            partial.close()
            verify_archive(Path(partial.name), asset)
            Path(partial.name).replace(destination)
        finally:
            Path(partial.name).unlink(missing_ok=True)
    return str(destination)


def archive_reader(archive):
    prefix = f"llama-{LLAMACPP_BUILD}/"
    members = {member.name[len(prefix):]: member for member in archive.getmembers() if member.name.startswith(prefix)}

    def read(name):
        for _ in range(8):
            member = members.get(name)
            if member is None:
                raise ValueError(f"The llama.cpp archive has no {name}; upstream packaging changed")
            if member.isfile():
                return archive.extractfile(member).read()
            if not member.issym() or "/" in member.linkname:
                raise ValueError(f"{name} in the llama.cpp archive is neither a file nor a sibling link")
            name = member.linkname
        raise ValueError(f"{name} in the llama.cpp archive is a link loop")
    return read


def directory_reader(root):
    root = root.resolve()

    def read(name):
        path = (root / name).resolve()
        if path.parent != root or not path.is_file():
            raise ValueError(f"{root} has no regular file {name} beside the build outputs")
        return path.read_bytes()
    return read


@contextlib.contextmanager
def llamacpp_files(spec, source):
    if spec["archive"] is None:
        if not source.is_dir():
            raise ValueError(f"Expected the llama.cpp build output directory, got {source}")
        yield directory_reader(source)
        return
    verify_archive(source, spec["archive"])
    with tarfile.open(source, "r:gz") as archive:
        yield archive_reader(archive)


def elf(path):
    with path.open("rb") as stream:
        return stream.read(4) == b"\x7fELF"


def macho(path):
    with path.open("rb") as stream:
        return stream.read(4) in (b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"\xca\xfe\xba\xbe")


def staged_files(stage):
    files = sorted((path for path in stage.rglob("*") if not path.is_dir()), key=lambda path: path.relative_to(stage).as_posix())
    for path in files:
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"{path} must be a regular file; packs hold no links")
    return files


def dynamic_section(text):
    """NEEDED libraries and the run path from `readelf -d --wide`."""
    needed = re.findall(r"\(NEEDED\)\s+Shared library: \[([^\]]+)\]", text)
    runpaths = re.findall(r"\((?:RUNPATH|RPATH)\)\s+Library (?:runpath|rpath): \[([^\]]*)\]", text)
    return needed, ":".join(runpaths) if runpaths else None


def symbol_versions(text):
    """Needed versions per library, and the versions a library defines, from `readelf --version-info --wide`."""
    needs, defines, section, library = {}, set(), None, None
    for line in text.splitlines():
        if line.startswith("Version needs section"):
            section = "needs"
        elif line.startswith("Version definition section"):
            section = "definitions"
        elif line.startswith("Version symbols section"):
            section = None
        elif section == "needs" and (match := re.search(r"\bFile: (\S+)", line)):
            library = match.group(1)
            needs.setdefault(library, set())
        elif section == "needs" and library and (match := re.search(r"\bName: (\S+)", line)):
            needs[library].add(match.group(1))
        elif section == "definitions" and "Flags: BASE" not in line and (match := re.search(r"\bName: (\S+)", line)):
            defines.add(match.group(1))
    return needs, defines


def glibc_violations(needs, floor=GLIBC_FLOOR):
    """GLIBC versions above the floor; tags like GLIBC_ABI_DT_RELR count, as in verify-linux-glibc.sh."""
    violations = set()
    for names in needs.values():
        for name in names:
            if not name.startswith("GLIBC_"):
                continue
            version = name[len("GLIBC_"):]
            if not re.fullmatch(r"\d+(?:\.\d+)+", version) or tuple(map(int, version.split("."))) > floor:
                violations.add(name)
    return sorted(violations)


def ldd_paths(text):
    return {match.group(1): match.group(2) for match in re.finditer(r"^\s*(\S+) => (/\S+)", text, re.MULTILINE)}


def merged_usr_alias(path):
    """The file's other name on a merged-/usr system, where /lib, /bin and /sbin link into /usr."""
    text = path.as_posix()
    return Path(text[len("/usr"):] if text.startswith("/usr/") else f"/usr{text}")


def debian_package(path):
    """The installed package that owns a library file, as `dpkg-query -S` names it (`libgcc-s1:amd64`).

    dpkg finds a file only under the path its package registered, and a merged-/usr runner reaches
    it under both: libgcc-s1 registers /lib/<triplet>/libgcc_s.so.1, libstdc++6 /usr/lib/<triplet>."""
    alias = merged_usr_alias(path)
    for candidate in (path, alias):
        result = run("dpkg-query", "-S", str(candidate))
        if not result.returncode:
            return result.stdout.split(": ", 1)[0].strip()
    raise ValueError(f"No installed Debian package owns {path} or {alias}: {result.stdout.strip()[-400:]}")


def debian_version(package):
    return tool("dpkg-query", "--show", "--showformat=${Version}", package).strip()


def debian_notice(package, names):
    """A bundled package's copyright notice, titled with the exact version the pack carries."""
    name = package.split(":", 1)[0]
    return (f"{name} {debian_version(package)} (Ubuntu package copyright; bundled as {', '.join(names)})",
            (DEBIAN_DOCS / name / "copyright").read_text())


def llamacpp_title(spec):
    """The llama.cpp notice's title; a compiled pack also names its pinned compiler."""
    title = f"llama.cpp {LLAMACPP_BUILD} (MIT)"
    if spec["archive"] is not None:
        return title
    return f"{title}, compiled from {LLAMACPP_COMMIT} with GCC {spec['toolchain']}"


def bundle_host_libraries(stage, spec):
    """Copies the runner's libraries into the pack and returns each owning package's copyright notice."""
    resolved = {}
    for path in staged_files(stage):
        if elf(path):
            resolved.update(ldd_paths(tool("ldd", str(path))))
    packages, source_notices = {}, []
    for directory, names in ((stage, spec.get("bundled", ())), (stage / FALLBACK, spec.get("fallback", ()))):
        for name in names:
            if name not in resolved:
                raise ValueError(f"Install {name} on the build runner; the {spec['target']} pack bundles it")
            source = Path(resolved[name]).resolve()
            if spec.get("toolchain") and name in toolchain.LIBRARIES:
                source_notices.append(toolchain.library_notice(name, source))
            else:
                packages.setdefault(debian_package(source), []).append(name)
            write(directory / name, source.read_bytes())
    for path in staged_files(stage):
        if elf(path) and dynamic_section(tool("readelf", "-d", "--wide", str(path)))[1] != "$ORIGIN":
            tool("patchelf", "--set-rpath", "$ORIGIN", str(path))
    return [debian_notice(package, names) for package, names in sorted(packages.items())] + source_notices


def write_notices(stage, notices):
    write(stage / NOTICES, "\n\n".join(f"# {title}\n\n{text.strip()}" for title, text in notices).encode() + b"\n")


def elf_problems(name, path, provided):
    needed, runpath = dynamic_section(tool("readelf", "-d", "--wide", str(path)))
    needs, defines = symbol_versions(tool("readelf", "--version-info", "--wide", str(path)))
    problems = [] if runpath == "$ORIGIN" else [f"{name} has run path {runpath!r} instead of $ORIGIN"]
    problems += [f"{name} needs {library}, which is neither in the pack nor part of glibc"
                 for library in needed if library not in provided and library not in GLIBC_LIBRARIES]
    problems += [f"{name} needs {version}, above the GLIBC_{'.'.join(map(str, GLIBC_FLOOR))} floor"
                 for version in glibc_violations(needs)]
    return problems, needs, defines


def version_problems(versions, provided):
    """Symbol versions a file needs from a bundled library that the bundled copy does not define."""
    problems = []
    for name, (needs, _) in versions.items():
        for library, needed in sorted(needs.items()):
            defined = versions.get(provided.get(library), (None, None))[1]
            if defined is not None:
                problems += [f"{name} needs {version} from {library}, which the bundled copy does not define"
                             for version in sorted(needed - defined)]
    return problems


def check_linux(stage):
    """Every ELF loads from the pack or glibc, stays under the glibc floor and finds its versions in bundled copies."""
    files = {path.relative_to(stage).as_posix(): path for path in staged_files(stage)}
    provided = {name.rsplit("/", 1)[-1]: name for name in files
                if "/" not in name or (name.startswith(f"{FALLBACK}/") and name.count("/") == 1)}
    problems = [f"{name} is in the pack root and in {FALLBACK}/" for name in files if f"{FALLBACK}/{name}" in files]
    problems += [f"{name} is not an ELF file" for name, path in files.items() if name != NOTICES and not elf(path)]
    versions = {}
    for name, path in files.items():
        if elf(path):
            found, needs, defines = elf_problems(name, path, provided)
            problems += found
            versions[name] = (needs, defines)
    problems += version_problems(versions, provided)
    if problems:
        raise ValueError("Linux runtime pack is not self-contained:\n" + "\n".join(problems))


def macho_dependencies(text):
    return [line.strip().split(" (compatibility version", 1)[0] for line in text.splitlines()[1:] if line[:1].isspace()]


def macho_rpaths(text):
    return re.findall(r"cmd LC_RPATH\n\s+cmdsize \d+\n\s+path (.+?) \(offset \d+\)", text)


def rpath_file(stage, path, rpaths, name):
    # Pack executables and the libraries they load share a directory tree, so both prefixes
    # resolve against the loading file's directory.
    for rpath in rpaths:
        prefix = next((p for p in ("@loader_path", "@executable_path") if rpath == p or rpath.startswith(p + "/")), None)
        if prefix is None:
            continue
        candidate = Path(os.path.normpath(os.path.join(path.parent, rpath[len(prefix):].lstrip("/"), name)))
        if candidate.is_relative_to(stage) and candidate.is_file():
            return candidate
    return None


def check_macos(stage):
    problems = []
    for path in staged_files(stage):
        if not macho(path):
            continue
        name = path.relative_to(stage).as_posix()
        rpaths = macho_rpaths(tool("otool", "-l", str(path)))
        for dependency in macho_dependencies(tool("otool", "-L", str(path))):
            if dependency.startswith("@rpath/") and rpath_file(stage, path, rpaths, dependency[len("@rpath/"):]):
                continue
            if dependency.startswith(SYSTEM_MACHO_PREFIXES):
                continue
            problems.append(f"{name} loads {dependency}, which is neither in the pack nor part of macOS")
    if problems:
        raise ValueError("macOS runtime pack is not self-contained:\n" + "\n".join(problems))


def require_host(target, host):
    if (host or HOST_TARGETS.get((platform.system(), platform.machine()))) != target:
        raise ValueError(f"Build the {target} pack on a {target} runner so its entrypoint can be started before packing")


def smoke(stage, target, host=None):
    require_host(target, host)
    environments = [{"PATH": "/usr/bin:/bin"}]
    if (stage / FALLBACK).is_dir():
        environments.append({**environments[0], "LD_LIBRARY_PATH": str(stage / FALLBACK)})
    for environment in environments:
        output = tool(str(stage / "llama-server"), "--version", env=environment)
        if f"(build {LLAMACPP_BUILD_NUMBER}," not in output:
            raise ValueError(f"llama-server reports {output.strip()[:200]!r}, not build {LLAMACPP_BUILD_NUMBER}")


def pack_name(value):
    return f"{value['runtime']}-{value['build']}-{value['target']}-{value['backend']}.tar.gz"


def listing(stage, runtime, build, target, backend, entrypoint):
    files, folded, total = [], set(), 0
    for path in staged_files(stage):
        name = path.relative_to(stage).as_posix()
        size = path.stat().st_size
        total += size
        if (not re.fullmatch(r"[A-Za-z0-9_+-][A-Za-z0-9._+-]*(?:/[A-Za-z0-9_+-][A-Za-z0-9._+-]*)*", name)
                or name.lower() == LISTING or name.lower() in folded or size == 0):
            raise ValueError(f"Runtime pack file {name!r} has an unsafe name, collides with another file or is empty")
        folded.add(name.lower())
        files.append({"path": name, "size": size, "sha256": sha256_file(path), "executable": name == entrypoint})
    if not 0 < len(files) <= MAX_PACK_FILES or total > MAX_UNPACKED_BYTES:
        raise ValueError(f"Runtime pack holds {len(files)} files and {total} bytes; the limits are {MAX_PACK_FILES} and {MAX_UNPACKED_BYTES}")
    if not any(entry["executable"] for entry in files):
        raise ValueError(f"Runtime pack entrypoint {entrypoint} is missing")
    return {"version": 1, "runtime": runtime, "build": build, "target": target, "backend": backend,
            "entrypoint": entrypoint, "files": files}


def add_member(archive, name, stream, size, mode, epoch):
    info = tarfile.TarInfo(name)
    info.size, info.mode, info.mtime = size, mode, epoch
    info.uid = info.gid = 0
    info.uname = info.gname = ""
    archive.addfile(info, stream)


def write_pack(stage, value, epoch, output):
    """The listing first, then every file in listing order; owners, times and gzip header are fixed."""
    if not isinstance(epoch, int) or epoch < 0:
        raise ValueError("Expected SOURCE_DATE_EPOCH as a non-negative whole number of seconds")
    output.mkdir(parents=True, exist_ok=True)
    path = output / pack_name(value)
    body = json.dumps(value, separators=(",", ":")).encode()
    with path.open("xb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed, \
            tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
        add_member(archive, LISTING, io.BytesIO(body), len(body), 0o644, epoch)
        for entry in value["files"]:
            with (stage / entry["path"]).open("rb") as stream:
                add_member(archive, entry["path"], stream, entry["size"], 0o755 if entry["executable"] else 0o644, epoch)
    if path.stat().st_size > MAX_PACK_BYTES:
        path.unlink()
        raise ValueError(f"{path.name} exceeds the {MAX_PACK_BYTES}-byte runtime pack limit")
    return {"pack": path.name, "size": path.stat().st_size, "sha256": sha256_file(path), "files": len(value["files"])}


def llamacpp(pack, source, epoch, output, host=None):
    spec = LLAMACPP_PACKS[pack]
    with tempfile.TemporaryDirectory(prefix="runtime-pack-") as folder:
        stage = Path(folder) / "stage"
        with llamacpp_files(spec, source) as read:
            write(stage / "llama-server", read("llama-server"), 0o755)
            for name in spec["files"]:
                write(stage / name, read(name))
            notices = [(llamacpp_title(spec), read("LICENSE").decode())]
        if spec["target"].endswith("-linux-gnu"):
            notices += bundle_host_libraries(stage, spec)
            write_notices(stage, notices)
            check_linux(stage)
        else:
            write_notices(stage, notices)
            check_macos(stage)
        smoke(stage, spec["target"], host)
        value = listing(stage, "llamacpp", LLAMACPP_BUILD, spec["target"], spec["backend"], "llama-server")
        return write_pack(stage, value, epoch, output)


def swift_back_deployment(helper, toolchain):
    """Swift libraries the helper loads through @rpath that macOS 14 and 15 lack, from Xcode's back-deployment set."""
    copies = {}
    for dependency in macho_dependencies(tool("otool", "-L", str(helper))):
        if dependency.startswith("@rpath/libswift"):
            name = dependency[len("@rpath/"):]
            matches = sorted(toolchain.glob(f"swift-*/macosx/{name}"))
            if len(matches) != 1:
                raise ValueError(f"Expected one {name} under {toolchain}/swift-*/macosx, found {len(matches)}")
            copies[name] = matches[0]
    return copies


def mlx_smoke(stage, host=None):
    # Hosted runners may lack a Metal device, so only loading the helper and its libraries must succeed.
    require_host("aarch64-apple-darwin", host)
    result = run(str(stage / MLX_ENTRYPOINT), env={"PATH": "/usr/bin:/bin"})
    if result.returncode < 0 or "Library not loaded" in result.stdout:
        raise ValueError(f"The MLX helper does not load: {result.stdout.strip()[-400:]}")


def mlx(executable, resources, toolchain, notices, build, epoch, output, host=None):
    with tempfile.TemporaryDirectory(prefix="runtime-pack-") as folder:
        stage = Path(folder) / "stage"
        if executable.is_symlink() or not executable.is_file():
            raise ValueError(f"Expected the MLX helper executable at {executable}")
        helper = stage / MLX_ENTRYPOINT
        write(helper, executable.read_bytes(), 0o755)
        write(stage / NOTICES, notices.read_bytes())
        for path in staged_files(resources):
            write(helper.parent / path.relative_to(resources), path.read_bytes())
        if not (stage / MLX_METALLIB).is_file():
            raise ValueError(f"{resources} has no mlx-swift_Cmlx.bundle with default.metallib; MLX could not load its kernels")
        for name, path in swift_back_deployment(helper, toolchain).items():
            write(stage / "lib" / name, path.read_bytes())
        check_macos(stage)
        mlx_smoke(stage, host)
        value = listing(stage, "mlx", build, "aarch64-apple-darwin", "metal", MLX_ENTRYPOINT)
        return write_pack(stage, value, epoch, output)


def check_members(archive, members, files, name):
    for entry, member in itertools.zip_longest(files, members):
        if entry is None or member is None:
            raise ValueError(f"{name} holds other entries than its {LISTING} lists")
        mode = 0o755 if entry["executable"] else 0o644
        if (member.name != entry["path"] or not member.isfile() or member.size != entry["size"]
                or member.mode != mode or hashlib.sha256(archive.extractfile(member).read()).hexdigest() != entry["sha256"]):
            raise ValueError(f"{name} member {member.name} differs from its {LISTING} entry")


def describe_pack(path):
    """Re-read a built pack completely before its digests are signed."""
    if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= MAX_PACK_BYTES:
        raise ValueError(f"{path} is not a bounded regular runtime pack")
    with tarfile.open(path, "r:gz") as archive:
        members = iter(archive)
        first = next(members, None)
        if first is None or first.name != LISTING or not first.isfile() or first.size > 1024 * 1024:
            raise ValueError(f"{path.name} must start with its {LISTING} listing")
        value = json.loads(archive.extractfile(first).read())
        if set(value) != {"version", "runtime", "build", "target", "backend", "entrypoint", "files"} or value["version"] != 1:
            raise ValueError(f"{path.name} has an unexpected {LISTING} shape")
        check_members(archive, members, value["files"], path.name)
    if path.name != pack_name(value):
        raise ValueError(f"{path.name} must be named {pack_name(value)}")
    return {"runtime": value["runtime"], "build": value["build"], "target": value["target"], "backend": value["backend"],
            "size": path.stat().st_size, "sha256": sha256_file(path), "entrypoint": value["entrypoint"],
            "files": value["files"]}


def encoded_length(size):
    return -(-size * 4 // 3)


def compact_size(body):
    """Exact length of the compact JWS the Rust signer makes from this manifest JSON."""
    header = json.dumps({"alg": "EdDSA", "typ": JWS_TYPE, "kid": "A" * 43}, separators=(",", ":"))
    return encoded_length(len(header)) + 1 + encoded_length(len(body)) + 1 + 86


def whole(value):
    return isinstance(value, int) and not isinstance(value, bool)


def manifest(packs, base_url, sequence, output, validity_days=release.DEFAULT_RELEASE_LIFETIME_DAYS, issued_at=None):
    """One unsigned manifest per target, small enough to forward in an install command."""
    base_url = release.secure_prefix(base_url)
    if not whole(sequence) or not 0 < sequence <= MAX_SAFE_INTEGER:
        raise ValueError("Expected a positive runtime manifest sequence")
    dates = release.validity(validity_days, issued_at)
    targets = {}
    for path in sorted(packs.glob("*.tar.gz")):
        described = describe_pack(path)
        entry = {name: described[name] for name in ("runtime", "build", "target", "backend")}
        entry.update(url=f"{base_url}/{path.name}", size=described["size"], sha256=described["sha256"],
                     entrypoint=described["entrypoint"], files=described["files"])
        targets.setdefault(entry["target"], []).append(entry)
    if not targets:
        raise ValueError(f"{packs} holds no runtime packs")
    sizes = {}
    for target, entries in sorted(targets.items()):
        body = json.dumps({"version": 1, "sequence": sequence, **dates, "packs": entries}, separators=(",", ":"))
        sizes[target] = compact_size(body)
        limit = min(MAX_COMPACT_JWS_BYTES, MAX_FORWARDED_MANIFEST_BYTES)
        if sizes[target] > limit:
            raise ValueError(f"The {target} runtime manifest would sign to {sizes[target]} bytes, "
                             f"above the {limit}-byte JWS limit; publish fewer packs or files per target")
        (output / f"{target}.json").write_text(body)
    return sizes


def verified_manifest(compact, public_keys, current=True):
    """Signature, type, sequence and lifetime as `verified_release` checks a release, and a pack list."""
    value = release.verified_release(compact, public_keys, current, JWS_TYPE)
    if not isinstance(value.get("packs"), list) or not value["packs"]:
        raise ValueError("Invalid signed runtime manifest packs: expected a nonempty list")
    return value


def signed_manifests(artifacts, public_keys):
    manifests = {}
    for path in sorted(artifacts.glob("*.jws")):
        if path.stem not in release.TARGETS or path.is_symlink() or not path.is_file() or path.stat().st_size > MAX_COMPACT_JWS_BYTES:
            raise ValueError(f"{path.name} is not a bounded signed runtime manifest of a release target")
        compact = path.read_bytes()
        value = verified_manifest(compact, public_keys)
        if {pack.get("target") for pack in value["packs"]} != {path.stem}:
            raise ValueError(f"{path.name} lists packs of another target")
        manifests[path.stem] = (path, compact, value)
    if not manifests or len({value["sequence"] for _, _, value in manifests.values()}) != 1:
        raise ValueError("Expected signed runtime manifests that share one sequence")
    return manifests


def immutable_uploads(artifacts, base_url, prefix, manifests, sequence):
    """(key, local file, public URL, size, sha256) for every pack and per-sequence manifest."""
    folder = f"releases/{sequence}/runtimes"
    uploads = []
    for target, (path, compact, value) in manifests.items():
        for pack in value["packs"]:
            name = pack_name(pack)
            local = artifacts / name
            if (local.is_symlink() or not local.is_file() or pack.get("url") != f"{base_url}/{folder}/{name}"
                    or local.stat().st_size != pack.get("size") or sha256_file(local) != pack.get("sha256")):
                raise ValueError(f"{name} differs from its signed size, digest or immutable URL")
            uploads.append((f"{prefix}/{folder}/{name}", local, pack["url"], pack["size"], pack["sha256"]))
        uploads.append((f"{prefix}/{folder}/{target}.jws", path, f"{base_url}/{folder}/{target}.jws",
                        len(compact), hashlib.sha256(compact).hexdigest()))
    return uploads


def stable_heads(store, prefix, manifests, sequence, public_keys):
    """Each target's stable manifest as read before any upload; none may be newer or differ at this sequence."""
    heads = {}
    for target, (_, compact, _) in manifests.items():
        stable = store.read(f"{prefix}/runtimes/{target}.jws")
        if stable:
            old = verified_manifest(stable["body"], public_keys, current=False)["sequence"]
            if old > sequence or (old == sequence and stable["body"] != compact):
                raise ValueError(f"The {target} runtime manifest would roll back or replace signed sequence {old}")
        heads[target] = stable
    return heads


def upload_immutable(store, uploads, verify_public):
    for key, path, url, size, digest in uploads:
        try:
            store.put(key, path)
        except FileExistsError:
            pass  # A retry may find the exact immutable bytes already committed.
        stored = store.read(key, size, retain=False)
        if not stored or stored["size"] != size or stored["sha256"] != digest:
            raise ValueError(f"Immutable runtime object {key} differs from the signed bundle")
        verify_public(url, size, digest)


def publish_bundle(artifacts, base_url, prefix, public_keys, store, verify_public=release.public_readback):
    """Packs and per-sequence manifests are immutable; each target's stable manifest advances last."""
    base_url = release.secure_prefix(base_url)
    if not re.fullmatch(r"[A-Za-z0-9_-]+(?:/[A-Za-z0-9_-]+)*", prefix):
        raise ValueError("Release prefix must contain safe, nonempty path segments")
    if not isinstance(public_keys, list) or not 1 <= len(public_keys) <= 8:
        raise ValueError("Configure one to eight trusted release public keys")
    manifests = signed_manifests(artifacts, public_keys)
    sequence = next(iter(manifests.values()))[2]["sequence"]
    uploads = immutable_uploads(artifacts, base_url, prefix, manifests, sequence)
    previous = stable_heads(store, prefix, manifests, sequence, public_keys)
    upload_immutable(store, uploads, verify_public)
    for target, (path, compact, _) in manifests.items():
        release.promote(store, f"{prefix}/runtimes/{target}.jws", path, compact, previous[target], public_keys, JWS_TYPE)
        verify_public(f"{base_url}/runtimes/{target}.jws", len(compact), hashlib.sha256(compact).hexdigest())
    return {"sequence": sequence, "manifests": {target: f"{base_url}/runtimes/{target}.jws" for target in manifests}}


def publish(artifacts, base_url, bucket, prefix, public_keys, endpoint=None):
    return publish_bundle(artifacts, base_url, prefix, json.loads(public_keys), release.S3Store(bucket, endpoint))


def summary(artifacts, public_keys):
    manifests = signed_manifests(artifacts, json.loads(public_keys))
    rows = []
    for target, (_, _, value) in sorted(manifests.items()):
        until = time.strftime("%Y-%m-%d", time.gmtime(value["expires_at"]))
        for pack in value["packs"]:
            rows.append(f"| {target} | {pack['runtime']} {pack['build']} | {pack['backend']} | "
                        f"{pack['size'] / 1024**2:.1f} MiB | {len(pack['files'])} | {until} |")
    sequence = next(iter(manifests.values()))[2]["sequence"]
    return "\n".join([f"### Runtime packs {sequence} published", "",
                      "| Target | Runtime | Backend | Archive | Files | Valid until |", "|---|---|---|---|---|---|", *rows, "",
                      "Each target's signed list is `runtimes/<target>.jws` next to `release.jws`."])


def main(argv=None):
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("pin")
    fetch_parser = commands.add_parser("fetch")
    fetch_parser.add_argument("--pack", choices=[name for name, spec in LLAMACPP_PACKS.items() if spec["archive"]], required=True)
    fetch_parser.add_argument("--output", type=Path, required=True)
    llamacpp_parser = commands.add_parser("llamacpp")
    llamacpp_parser.add_argument("--pack", choices=LLAMACPP_PACKS, required=True)
    llamacpp_parser.add_argument("--source", type=Path, required=True)
    mlx_parser = commands.add_parser("mlx")
    mlx_parser.add_argument("--executable", type=Path, required=True)
    mlx_parser.add_argument("--resources", type=Path, required=True)
    mlx_parser.add_argument("--toolchain", type=Path, required=True)
    mlx_parser.add_argument("--notices", type=Path, required=True)
    mlx_parser.add_argument("--build", required=True)
    for build_parser in (llamacpp_parser, mlx_parser):
        build_parser.add_argument("--epoch", type=int, required=True)
        build_parser.add_argument("--output", type=Path, required=True)
    manifest_parser = commands.add_parser("manifest")
    manifest_parser.add_argument("--packs", type=Path, required=True)
    manifest_parser.add_argument("--base-url", required=True)
    manifest_parser.add_argument("--sequence", type=int, required=True)
    manifest_parser.add_argument("--validity-days", type=int, default=release.DEFAULT_RELEASE_LIFETIME_DAYS)
    manifest_parser.add_argument("--output", type=Path, required=True)
    publish_parser = commands.add_parser("publish")
    publish_parser.add_argument("--artifacts", type=Path, required=True)
    publish_parser.add_argument("--base-url", required=True)
    publish_parser.add_argument("--bucket", required=True)
    publish_parser.add_argument("--prefix", required=True)
    publish_parser.add_argument("--public-keys", required=True)
    publish_parser.add_argument("--endpoint")
    summary_parser = commands.add_parser("summary")
    summary_parser.add_argument("--artifacts", type=Path, required=True)
    summary_parser.add_argument("--public-keys", required=True)
    args = vars(parser.parse_args(argv))
    result = globals()[args.pop("command")](**args)
    print(result if isinstance(result, str) else json.dumps(result, separators=(",", ":")))


if __name__ == "__main__":
    main()
