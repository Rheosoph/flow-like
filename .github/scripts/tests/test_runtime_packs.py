"""Runtime pack building, checks, manifests, publishing and the release workflow jobs that run them."""

import base64
import gzip
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tempfile
import textwrap
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("runtime_packs", Path(__file__).parents[1] / "runtime_packs.py")
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)

REPOSITORY = Path(__file__).resolve().parents[3]
WORKFLOW = REPOSITORY / ".github/workflows/standalone-release.yml"
EPOCH = 1759600000
BASE = "https://cdn.example/standalone"
VERSION = "version: 0.6.0-dev (build 11429, commit d81235049)\nbuilt with GNU 14.3.0 for Linux aarch64\n"

READELF_DYNAMIC = """
Dynamic section at offset 0x2d58 contains 30 entries:
  Tag        Type                         Name/Value
 0x0000000000000001 (NEEDED)             Shared library: [libllama-server-impl.so]
 0x0000000000000001 (NEEDED)             Shared library: [libstdc++.so.6]
 0x0000000000000001 (NEEDED)             Shared library: [libc.so.6]
 0x000000000000001d (RUNPATH)            Library runpath: [$ORIGIN]
 0x000000000000000c (INIT)               0x1000
"""
READELF_VERSIONS = """
Version symbols section '.gnu.version' contains 43 entries:
 Addr: 0x0000000000000b28  Offset: 0x000b28  Link: 6 (.dynsym)
  000:   0 (*local*)       2 (GLIBCXX_3.4)   3 (GLIBC_2.2.5)   4 (GLIBC_2.39)

Version definition section '.gnu.version_d' contains 3 entries:
 Addr: 0x00000000000a4c30  Offset: 0x0a4c30  Link: 5 (.dynstr)
  000000: Rev: 1  Flags: BASE  Index: 1  Cnt: 1  Name: libllama.so.0
  0x001c: Rev: 1  Flags: none  Index: 2  Cnt: 1  Name: LLAMA_1.0
  0x0038: Rev: 1  Flags: none  Index: 3  Cnt: 2  Name: LLAMA_1.1
  0x0054: Parent 1: LLAMA_1.0

Version needs section '.gnu.version_r' contains 3 entries:
 Addr: 0x0000000000000b70  Offset: 0x000b70  Link: 7 (.dynstr)
  000000: Version: 1  File: libgcc_s.so.1  Cnt: 1
  0x0010:   Name: GCC_3.0  Flags: none  Version: 9
  0x0020: Version: 1  File: libc.so.6  Cnt: 2
  0x0030:   Name: GLIBC_2.34  Flags: none  Version: 10
  0x0040:   Name: GLIBC_2.2.5  Flags: none  Version: 3
  0x0050: Version: 1  File: libstdc++.so.6  Cnt: 2
  0x0060:   Name: GLIBCXX_3.4.21  Flags: none  Version: 7
  0x0070:   Name: CXXABI_1.3.9  Flags: none  Version: 2
"""
LDD = """\tlinux-vdso.so.1 (0x00007ffd5a1f2000)
\tlibllama-server-impl.so => /tmp/stage/libllama-server-impl.so (0x00007f0000000000)
\tlibstdc++.so.6 => /lib/x86_64-linux-gnu/libstdc++.so.6 (0x00007f0000001000)
\tlibvulkan.so.1 => not found
\t/lib64/ld-linux-x86-64.so.2 (0x00007f0000002000)
"""
OTOOL_L = """/stage/llama-server:
\t@rpath/libllama-server-impl.dylib (compatibility version 0.0.0, current version 0.0.0)
\t/usr/lib/libc++.1.dylib (compatibility version 1.0.0, current version 1900.180.0)
\t/usr/lib/librdma.dylib (compatibility version 1.0.0, current version 1.0.0, weak)
"""
OTOOL_RPATHS = """Load command 14
          cmd LC_RPATH
      cmdsize 32
         path /usr/lib/swift (offset 12)
Load command 15
          cmd LC_RPATH
      cmdsize 40
         path @executable_path/../lib (offset 12)
"""


def done(argv, stdout="", returncode=0):
    return subprocess.CompletedProcess(argv, returncode, stdout=stdout)


def readelf_dynamic(info):
    lines = ["Dynamic section at offset 0x2d58 contains 3 entries:", "  Tag        Type                         Name/Value"]
    lines += [f" 0x0000000000000001 (NEEDED)             Shared library: [{name}]" for name in info["needed"]]
    if info["runpath"] is not None:
        lines.append(f" 0x000000000000001d (RUNPATH)            Library runpath: [{info['runpath']}]")
    return "\n".join(lines) + "\n"


def readelf_versions(name, info):
    lines = []
    if info["defines"]:
        lines += ["Version definition section '.gnu.version_d' contains 2 entries:",
                  f"  000000: Rev: 1  Flags: BASE  Index: 1  Cnt: 1  Name: {name}"]
        lines += [f"  0x001c: Rev: 1  Flags: none  Index: 2  Cnt: 1  Name: {version}" for version in sorted(info["defines"])]
        lines.append("")
    if info["needs"]:
        lines.append("Version needs section '.gnu.version_r' contains 1 entries:")
        for library, versions in info["needs"].items():
            lines.append(f"  000000: Version: 1  File: {library}  Cnt: {len(versions)}")
            lines += [f"  0x0010:   Name: {version}  Flags: none  Version: 2" for version in sorted(versions)]
    return "\n".join(lines) + "\n"


PACKAGES = {"libstdc++.so.6": "libstdc++6", "libgcc_s.so.1": "libgcc-s1", "libgomp.so.1": "libgomp1",
            "libssl.so.3": "libssl3", "libcrypto.so.3": "libssl3"}
PACKAGE_VERSIONS = {"libstdc++6:arm64": "12.3.0-1ubuntu1~22.04.3", "libgcc-s1:arm64": "12.3.0-1ubuntu1~22.04.3",
                    "libgomp1:arm64": "12.3.0-1ubuntu1~22.04.3", "libssl3:arm64": "3.0.2-0ubuntu1.20"}


def fake_dpkg(argv, owners, versions):
    """`dpkg-query -S` finds only a registered path, as dpkg does; `--show` prints an installed version."""
    if argv[1] == "-S":
        owner = owners.get(argv[-1])
        return done(argv, f"{owner}: {argv[-1]}\n") if owner else \
            done(argv, f"dpkg-query: no path found matching pattern {argv[-1]}\n", 1)
    version = versions.get(argv[-1])
    return done(argv, version) if version else done(argv, f"dpkg-query: no packages found matching {argv[-1]}\n", 1)


class FakeLinuxHost:
    """ldd, readelf, patchelf, dpkg-query and llama-server for fake ELF files described by their file name."""

    def __init__(self, root):
        self.host = root / "host"
        self.host.mkdir()
        self.docs = root / "doc"
        for package in set(PACKAGES.values()):
            stage(self.docs, {f"{package}/copyright": f"Copyright notice of {package}\n".encode()})
        self.info, self.calls = {}, []
        self.owners, self.versions = {}, dict(PACKAGE_VERSIONS)
        self.version = VERSION

    def library(self, name, needed=(), runpath="$ORIGIN", needs=None, defines=(), host=False):
        self.info[name] = {"needed": list(needed), "runpath": runpath, "needs": needs or {}, "defines": set(defines)}
        if host:
            path = self.host / name
            path.write_bytes(b"\x7fELF host " + name.encode())
            self.owners[str(path.resolve())] = f"{PACKAGES[name]}:arm64"

    def registered_under_alias(self, name):
        """dpkg knows the file only by its other merged-/usr name, as libgcc-s1 registers /lib/<triplet>."""
        path = str((self.host / name).resolve())
        self.owners[f"/usr{path}"] = self.owners.pop(path)

    def ldd(self, path):
        seen, pending, lines = set(), list(self.info[path.name]["needed"]), ["\tlinux-vdso.so.1 (0x1)"]
        while pending:
            name = pending.pop(0)
            if name in seen:
                continue
            seen.add(name)
            if name in runtime.GLIBC_LIBRARIES:
                lines.append(f"\t{name} => /lib/aarch64-linux-gnu/{name} (0x1)")
                continue
            found = next((folder / name for folder in (path.parent, self.host) if (folder / name).is_file()), None)
            lines.append(f"\t{name} => {found} (0x1)" if found else f"\t{name} => not found")
            if found:
                pending += self.info[name]["needed"]
        return "\n".join(lines) + "\n"

    def run(self, *argv, env=None):
        command, path = Path(argv[0]).name, Path(argv[-1])
        self.calls.append((command, argv[1:], env))
        if command == "llama-server":
            return done(argv, self.version)
        if command == "dpkg-query":
            return fake_dpkg(argv, self.owners, self.versions)
        info = self.info[path.name]
        if command == "ldd":
            return done(argv, self.ldd(path))
        if command == "patchelf":
            info["runpath"] = argv[2]
            return done(argv)
        if argv[1] == "-d":
            return done(argv, readelf_dynamic(info))
        return done(argv, readelf_versions(path.name, info))


def arm64_build_outputs(root, host):
    """A llama.cpp build/bin directory with soname links, and a host with the libraries packs bundle."""
    output = root / "build" / "bin"
    output.mkdir(parents=True)
    (output / "LICENSE").write_text("MIT License\n\nCopyright (c) 2023-2026 The ggml authors\n")
    glibc = {"libc.so.6": {"GLIBC_2.34", "GLIBC_2.17"}}
    cxx = {"libstdc++.so.6": {"GLIBCXX_3.4.30", "CXXABI_1.3.13"}, "libgcc_s.so.1": {"GCC_3.0"}}
    versioned = {"libllama-common.so.0": "libllama-common.so.0.4.0", "libllama.so.0": "libllama.so.0.4.0",
                 "libmtmd.so.0": "libmtmd.so.0.4.0", "libggml.so.0": "libggml.so.0.23.0",
                 "libggml-base.so.0": "libggml-base.so.0.23.0"}
    for name in ("llama-server", *runtime.LINUX_LIBRARIES, *runtime.ARM64_CPU_BACKENDS):
        real = versioned.get(name, name)
        (output / real).write_bytes(b"\x7fELF " + real.encode())
        if real != name:
            (output / name).symlink_to(real)
            (output / name.rsplit(".", 1)[0]).symlink_to(name)
    host.library("llama-server", ["libllama-server-impl.so", "libstdc++.so.6", "libgcc_s.so.1", "libc.so.6"],
                 needs={**glibc, **cxx})
    host.library("libllama-server-impl.so", ["libllama-common.so.0", "libmtmd.so.0", "libllama.so.0", "libggml.so.0",
                                             "libggml-base.so.0", "libssl.so.3", "libcrypto.so.3", "libstdc++.so.6",
                                             "libm.so.6", "libc.so.6"],
                 needs={**glibc, **cxx, "libcrypto.so.3": {"OPENSSL_3.0.0"}, "libssl.so.3": {"OPENSSL_3.0.0"}})
    for name in ("libllama-common.so.0", "libllama.so.0", "libmtmd.so.0", "libggml.so.0"):
        host.library(name, ["libggml-base.so.0", "libstdc++.so.6", "libc.so.6"], needs={**glibc, **cxx})
    host.library("libggml-base.so.0", ["libgomp.so.1", "libstdc++.so.6", "libm.so.6", "libc.so.6"],
                 needs={**glibc, **cxx, "libgomp.so.1": {"GOMP_4.0", "OMP_1.0"}})
    for name in runtime.ARM64_CPU_BACKENDS:
        host.library(name, ["libggml-base.so.0", "libgomp.so.1", "libstdc++.so.6", "libc.so.6"],
                     needs={**glibc, "libgomp.so.1": {"GOMP_4.0"}})
    host.library("libstdc++.so.6", ["libgcc_s.so.1", "libm.so.6", "libc.so.6"], runpath=None, host=True,
                 needs={"libc.so.6": {"GLIBC_2.34"}, "libgcc_s.so.1": {"GCC_3.0"}},
                 defines={"GLIBCXX_3.4", "GLIBCXX_3.4.30", "CXXABI_1.3", "CXXABI_1.3.13"})
    host.library("libgcc_s.so.1", ["libc.so.6"], runpath=None, host=True, needs={"libc.so.6": {"GLIBC_2.17"}},
                 defines={"GCC_3.0"})
    host.registered_under_alias("libgcc_s.so.1")
    host.library("libgomp.so.1", ["libc.so.6"], runpath=None, host=True, needs={"libc.so.6": {"GLIBC_2.34"}},
                 defines={"GOMP_4.0", "OMP_1.0"})
    host.library("libssl.so.3", ["libcrypto.so.3", "libc.so.6"], runpath=None, host=True,
                 needs={"libcrypto.so.3": {"OPENSSL_3.0.0"}}, defines={"OPENSSL_3.0.0"})
    host.library("libcrypto.so.3", ["libc.so.6"], runpath=None, host=True, needs={"libc.so.6": {"GLIBC_2.33"}},
                 defines={"OPENSSL_3.0.0"})
    return output


def members(path):
    with tarfile.open(path, "r:gz") as archive:
        return [(member.name, member.mode, member.uid, member.gid, member.uname, member.gname, member.mtime,
                 archive.extractfile(member).read()) for member in archive]


def stage(root, files, executable=None):
    for name, data in files.items():
        runtime.write(root / name, data, 0o755 if name == executable else 0o644)
    return root


def build_pack(folder, files, runtime_kind="llamacpp", build="b11429", target="x86_64-unknown-linux-gnu",
               backend="cpu", entrypoint="llama-server", output=None):
    root = stage(Path(folder) / f"stage-{target}-{backend}", files, entrypoint)
    value = runtime.listing(root, runtime_kind, build, target, backend, entrypoint)
    return runtime.write_pack(root, value, EPOCH, output or Path(folder) / "packs")


def rewrite(path, entries):
    """A pack with chosen members, written like write_pack does."""
    path.unlink()
    with path.open("xb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed, \
            tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
        for name, data, mode in entries:
            runtime.add_member(archive, name, io.BytesIO(data), len(data), mode, EPOCH)


def b64(value):
    return base64.urlsafe_b64encode(value).decode().rstrip("=")


class Signer:
    def __init__(self, directory):
        self.directory = directory
        private = directory / "private.pem"
        subprocess.run(["openssl", "genpkey", "-algorithm", "ED25519", "-out", str(private)], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        public = subprocess.check_output(["openssl", "pkey", "-in", str(private), "-pubout", "-outform", "DER"],
                                         stderr=subprocess.DEVNULL)[-32:]
        self.key = b64(public)

    def sign(self, value, typ=runtime.JWS_TYPE):
        jwk = json.dumps({"crv": "Ed25519", "kty": "OKP", "x": self.key}, separators=(",", ":")).encode()
        header = {"alg": "EdDSA", "typ": typ, "kid": b64(hashlib.sha256(jwk).digest())}
        body = value if isinstance(value, str) else json.dumps(value, separators=(",", ":"))
        message = f"{b64(json.dumps(header, separators=(',', ':')).encode())}.{b64(body.encode())}".encode()
        (self.directory / "message").write_bytes(message)
        signature = subprocess.check_output(["openssl", "pkeyutl", "-sign", "-rawin", "-inkey", str(self.directory / "private.pem"),
                                             "-in", str(self.directory / "message")], stderr=subprocess.DEVNULL)
        return message + b"." + b64(signature).encode()


class MemoryStore:
    def __init__(self):
        self.objects, self.writes, self.race = {}, [], None

    def read(self, key, maximum=16384, retain=True):
        value = self.objects.get(key)
        if value is None:
            return None
        if len(value) > maximum:
            raise ValueError("too large")
        digest = hashlib.sha256(value).hexdigest()
        return {"etag": digest, "sha256": digest, "size": len(value), "body": value if retain else None}

    def put(self, key, path, *, etag=None, immutable=True):
        if not immutable and self.race:
            self.objects[key] = self.race
        previous = self.objects.get(key)
        if (etag is None and previous is not None) or (etag is not None and (previous is None or hashlib.sha256(previous).hexdigest() != etag)):
            raise FileExistsError()
        self.objects[key] = path.read_bytes()
        self.writes.append((key, immutable))


class ParserTests(unittest.TestCase):
    def test_readelf_dynamic_section_lists_needed_libraries_and_run_path(self):
        self.assertEqual(runtime.dynamic_section(READELF_DYNAMIC),
                         (["libllama-server-impl.so", "libstdc++.so.6", "libc.so.6"], "$ORIGIN"))
        self.assertEqual(runtime.dynamic_section("Shared library: none\n"), ([], None))
        both = READELF_DYNAMIC + " 0x000000000000000f (RPATH)              Library rpath: [/home/runner/lib]\n"
        self.assertEqual(runtime.dynamic_section(both)[1], "$ORIGIN:/home/runner/lib")

    def test_readelf_versions_separate_needs_per_library_from_definitions(self):
        needs, defines = runtime.symbol_versions(READELF_VERSIONS)
        self.assertEqual(needs, {"libgcc_s.so.1": {"GCC_3.0"}, "libc.so.6": {"GLIBC_2.34", "GLIBC_2.2.5"},
                                 "libstdc++.so.6": {"GLIBCXX_3.4.21", "CXXABI_1.3.9"}})
        self.assertEqual(defines, {"LLAMA_1.0", "LLAMA_1.1"})

    def test_glibc_floor_allows_2_35_and_rejects_newer_or_unnumbered_tags(self):
        needs = {"libc.so.6": {"GLIBC_2.2.5", "GLIBC_2.34", "GLIBC_2.35"}, "libstdc++.so.6": {"GLIBCXX_3.4.32"}}
        self.assertEqual(runtime.glibc_violations(needs), [])
        needs["libm.so.6"] = {"GLIBC_2.38", "GLIBC_2.35.1", "GLIBC_ABI_DT_RELR", "GLIBC_PRIVATE"}
        self.assertEqual(runtime.glibc_violations(needs), ["GLIBC_2.35.1", "GLIBC_2.38", "GLIBC_ABI_DT_RELR", "GLIBC_PRIVATE"])
        self.assertEqual(runtime.glibc_violations(runtime.symbol_versions(READELF_VERSIONS)[0]), [])
        self.assertEqual(runtime.glibc_violations({"libc.so.6": {"GLIBC_2.39"}}, floor=(2, 39)), [])

    def test_ldd_keeps_resolved_paths_only(self):
        self.assertEqual(runtime.ldd_paths(LDD), {"libllama-server-impl.so": "/tmp/stage/libllama-server-impl.so",
                                                  "libstdc++.so.6": "/lib/x86_64-linux-gnu/libstdc++.so.6"})

    def test_dpkg_finds_the_owner_under_the_merged_usr_name_its_package_registered(self):
        # Jammy: ldd's /lib/x86_64-linux-gnu/libgcc_s.so.1 resolves through the /lib link into /usr/lib.
        owners = {"/lib/x86_64-linux-gnu/libgcc_s.so.1": "libgcc-s1:amd64",
                  "/usr/lib/x86_64-linux-gnu/libstdc++.so.6.0.30": "libstdc++6:amd64"}
        queries = []

        def dpkg(*argv, env=None):
            queries.append(argv[-1])
            return fake_dpkg(argv, owners, {})
        with patch.object(runtime, "run", dpkg):
            self.assertEqual(runtime.debian_package(Path("/usr/lib/x86_64-linux-gnu/libgcc_s.so.1")), "libgcc-s1:amd64")
            self.assertEqual(queries, ["/usr/lib/x86_64-linux-gnu/libgcc_s.so.1", "/lib/x86_64-linux-gnu/libgcc_s.so.1"])
            self.assertEqual(runtime.debian_package(Path("/usr/lib/x86_64-linux-gnu/libstdc++.so.6.0.30")), "libstdc++6:amd64")
            self.assertEqual(runtime.debian_package(Path("/lib/x86_64-linux-gnu/libstdc++.so.6.0.30")), "libstdc++6:amd64")
            with self.assertRaisesRegex(ValueError, "No installed Debian package owns /usr/lib/x86_64-linux-gnu/libfoo.so.1 "
                                                    "or /lib/x86_64-linux-gnu/libfoo.so.1: dpkg-query: no path found"):
                runtime.debian_package(Path("/usr/lib/x86_64-linux-gnu/libfoo.so.1"))

    def test_otool_dependencies_and_run_paths(self):
        self.assertEqual(runtime.macho_dependencies(OTOOL_L),
                         ["@rpath/libllama-server-impl.dylib", "/usr/lib/libc++.1.dylib", "/usr/lib/librdma.dylib"])
        self.assertEqual(runtime.macho_rpaths(OTOOL_RPATHS), ["/usr/lib/swift", "@executable_path/../lib"])


class PackArchiveTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.root = Path(self.folder.name)

    def test_packs_are_byte_identical_listing_first_sorted_with_normalized_owners_times_and_modes(self):
        files = {"llama-server": b"server", "libggml.so.0": b"ggml", "fallback/libstdc++.so.6": b"cxx", "lib-a.so": b"a"}
        first = build_pack(self.folder.name, files, output=self.root / "one")
        second = build_pack(self.folder.name, files, output=self.root / "two")
        one, two = self.root / "one" / first["pack"], self.root / "two" / second["pack"]
        self.assertEqual(one.read_bytes(), two.read_bytes())
        self.assertEqual(first["pack"], "llamacpp-b11429-x86_64-unknown-linux-gnu-cpu.tar.gz")
        self.assertEqual(first["sha256"], hashlib.sha256(one.read_bytes()).hexdigest())
        header = one.read_bytes()[:10]
        self.assertEqual((header[3], header[4:8]), (0, b"\0\0\0\0"), "gzip header must carry no name or time")
        entries = members(one)
        self.assertEqual([entry[0] for entry in entries],
                         ["pack.json", "fallback/libstdc++.so.6", "lib-a.so", "libggml.so.0", "llama-server"])
        self.assertTrue(all(entry[2:7] == (0, 0, "", "", EPOCH) for entry in entries))
        self.assertEqual({entry[0]: entry[1] for entry in entries}["llama-server"], 0o755)
        self.assertTrue(all(entry[1] == 0o644 for entry in entries if entry[0] != "llama-server"))
        listing = json.loads(entries[0][7])
        self.assertEqual(list(listing), ["version", "runtime", "build", "target", "backend", "entrypoint", "files"])
        self.assertEqual(listing["files"][0], {"path": "fallback/libstdc++.so.6", "size": 3,
                                               "sha256": hashlib.sha256(b"cxx").hexdigest(), "executable": False})
        self.assertEqual([entry["executable"] for entry in listing["files"]], [False, False, False, True])

    def test_listing_sorts_paths_as_strings_not_path_parts(self):
        root = stage(self.root / "stage", {"a-b": b"1", "a/b": b"2", "llama-server": b"3"}, "llama-server")
        value = runtime.listing(root, "llamacpp", "b11429", "x86_64-unknown-linux-gnu", "cpu", "llama-server")
        self.assertEqual([entry["path"] for entry in value["files"]], ["a-b", "a/b", "llama-server"])

    def test_listing_refuses_links_empty_files_reserved_or_colliding_names_and_a_missing_entrypoint(self):
        cases = [({"pack.json": b"x"}, "unsafe name"), ({"PACK.JSON": b"x"}, "unsafe name"),
                 ({".hidden": b"x"}, "unsafe name"), ({"has space": b"x"}, "unsafe name"),
                 ({"empty": b""}, "empty"), ({"lib/.hidden": b"x"}, "unsafe name")]
        for index, (extra, reason) in enumerate(cases):
            root = stage(self.root / f"stage{index}", {"llama-server": b"server", **extra}, "llama-server")
            with self.assertRaisesRegex(ValueError, reason):
                runtime.listing(root, "llamacpp", "b11429", "x86_64-unknown-linux-gnu", "cpu", "llama-server")
        root = stage(self.root / "case", {"Lib.so": b"x", "lib.so": b"x", "llama-server": b"server"}, "llama-server")
        # Case-insensitive file systems cannot hold both names, so the walk is given both.
        with patch.object(runtime, "staged_files", return_value=[root / "Lib.so", root / "lib.so", root / "llama-server"]), \
                self.assertRaisesRegex(ValueError, "collides"):
            runtime.listing(root, "llamacpp", "b11429", "x86_64-unknown-linux-gnu", "cpu", "llama-server")
        root = stage(self.root / "link", {"llama-server": b"server"}, "llama-server")
        (root / "libggml.so").symlink_to("llama-server")
        with self.assertRaisesRegex(ValueError, "packs hold no links"):
            runtime.listing(root, "llamacpp", "b11429", "x86_64-unknown-linux-gnu", "cpu", "llama-server")
        root = stage(self.root / "noentry", {"libggml.so": b"x"})
        with self.assertRaisesRegex(ValueError, "entrypoint llama-server is missing"):
            runtime.listing(root, "llamacpp", "b11429", "x86_64-unknown-linux-gnu", "cpu", "llama-server")
        root = stage(self.root / "many", {f"lib{index:02}.so": b"x" for index in range(runtime.MAX_PACK_FILES)} | {"llama-server": b"s"})
        with self.assertRaisesRegex(ValueError, "the limits are 64"):
            runtime.listing(root, "llamacpp", "b11429", "x86_64-unknown-linux-gnu", "cpu", "llama-server")

    def test_describe_pack_rereads_every_member_against_the_listing(self):
        built = build_pack(self.folder.name, {"llama-server": b"server", "libggml.so.0": b"ggml"})
        path = self.root / "packs" / built["pack"]
        described = runtime.describe_pack(path)
        self.assertEqual((described["size"], described["sha256"]), (built["size"], built["sha256"]))
        self.assertEqual([entry["path"] for entry in described["files"]], ["libggml.so.0", "llama-server"])
        listing = members(path)[0][7]
        for entries, reason in [
            ([("pack.json", listing, 0o644), ("libggml.so.0", b"GGML", 0o644), ("llama-server", b"server", 0o755)], "differs"),
            ([("pack.json", listing, 0o644), ("libggml.so.0", b"ggml", 0o755), ("llama-server", b"server", 0o755)], "differs"),
            ([("pack.json", listing, 0o644), ("llama-server", b"server", 0o755), ("libggml.so.0", b"ggml", 0o644)], "differs"),
            ([("pack.json", listing, 0o644), ("libggml.so.0", b"ggml", 0o644)], "other entries"),
            ([("pack.json", listing, 0o644), ("libggml.so.0", b"ggml", 0o644), ("llama-server", b"server", 0o755),
              ("extra", b"x", 0o644)], "other entries"),
            ([("libggml.so.0", b"ggml", 0o644), ("pack.json", listing, 0o644)], "must start with"),
        ]:
            rewrite(path, entries)
            with self.assertRaisesRegex(ValueError, reason):
                runtime.describe_pack(path)
        rewrite(path, [("pack.json", listing, 0o644), ("libggml.so.0", b"ggml", 0o644), ("llama-server", b"server", 0o755)])
        renamed = path.with_name("llamacpp-b11429-x86_64-unknown-linux-gnu-vulkan.tar.gz")
        path.rename(renamed)
        with self.assertRaisesRegex(ValueError, "must be named llamacpp-b11429-x86_64-unknown-linux-gnu-cpu.tar.gz"):
            runtime.describe_pack(renamed)


class UpstreamTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.root = Path(self.folder.name)

    def test_every_upstream_pack_is_pinned_by_size_and_digest(self):
        archives = {spec["archive"] for spec in runtime.LLAMACPP_PACKS.values() if spec["archive"]}
        self.assertEqual(archives, set(runtime.UPSTREAM_ARCHIVES))
        for size, digest in runtime.UPSTREAM_ARCHIVES.values():
            self.assertTrue(0 < size < 64 * 1024**2 and re.fullmatch(r"[0-9a-f]{64}", digest))
        self.assertEqual([name for name, spec in runtime.LLAMACPP_PACKS.items() if not spec["archive"]],
                         ["llamacpp-linux-arm64-cpu"])
        self.assertEqual([name for name, spec in runtime.LLAMACPP_PACKS.items() if spec.get("toolchain")],
                         ["llamacpp-linux-arm64-cpu"], "a pack compiled on the runner names its compiler packages")
        self.assertEqual(runtime.pin().splitlines(), ["build=b11429", "number=11429",
                                                      "commit=d81235049384534c167caea52b85a694f6103d14"])
        script = (REPOSITORY / "apps/desktop/scripts/update-llama-server.ts").read_text()
        self.assertIn(f'const PINNED_TAG = "{runtime.LLAMACPP_BUILD}"', script, "packs and the desktop pin one build")

    def test_desktop_script_pins_the_same_upstream_bytes(self):
        script = (REPOSITORY / "apps/desktop/scripts/update-llama-server.ts").read_text()
        block = re.search(r"^const ARCHIVE_PINS\b.*?^};$", script, re.MULTILINE | re.DOTALL).group(0)
        desktop = {name: (int(size), digest) for name, size, digest in
                   re.findall(r'"([^"]+)": \{\s*size: (\d+),\s*sha256: "([0-9a-f]{64})",?\s*\}', block)}
        downloads = {name.replace("{TAG}", runtime.LLAMACPP_BUILD) for name in re.findall(r'assetName: "([^"]+)"', script)}
        self.assertEqual(len(downloads), 5)
        self.assertLessEqual(downloads, set(desktop), "the desktop script pins every archive of its build")
        packs = {runtime.upstream_asset(pack)[1]: runtime.UPSTREAM_ARCHIVES[spec["archive"]]
                 for pack, spec in runtime.LLAMACPP_PACKS.items() if spec["archive"]}
        shared = sorted(set(packs) & downloads)
        self.assertEqual(shared, [f"llama-{runtime.LLAMACPP_BUILD}-bin-{asset}.tar.gz"
                                  for asset in ("macos-arm64", "macos-x64", "ubuntu-vulkan-x64")])
        for name in shared:
            self.assertEqual(desktop[name], packs[name], f"{name} has other pins in update-llama-server.ts than in runtime_packs.py")

    def fake_archive(self, files, links):
        path = self.root / "upstream.tar.gz"
        with tarfile.open(path, "w:gz") as archive:
            directory = tarfile.TarInfo("llama-b11429")
            directory.type = tarfile.DIRTYPE
            archive.addfile(directory)
            for name, data in files.items():
                info = tarfile.TarInfo(f"llama-b11429/{name}")
                info.size = len(data)
                archive.addfile(info, io.BytesIO(data))
            for name, target in links.items():
                info = tarfile.TarInfo(f"llama-b11429/{name}")
                info.type, info.linkname = tarfile.SYMTYPE, target
                archive.addfile(info)
        return path

    def test_fetch_streams_the_pinned_archive_and_refuses_other_bytes(self):
        data = b"pinned archive bytes"

        class Opener:
            def __init__(self, body):
                self.body, self.requests = body, []

            def open(self, request, timeout):
                self.requests.append(request.full_url)
                stream = io.BytesIO(self.body)
                stream.__enter__, stream.__exit__ = lambda *_: stream, lambda *_: None
                return stream
        pins = {"ubuntu-x64": (len(data), hashlib.sha256(data).hexdigest())}
        with patch.dict(runtime.UPSTREAM_ARCHIVES, pins):
            opener = Opener(data)
            path = Path(runtime.fetch("llamacpp-linux-x64-cpu", self.root / "out", opener))
            self.assertEqual(path.read_bytes(), data)
            self.assertEqual(opener.requests, ["https://github.com/ggml-org/llama.cpp/releases/download/b11429/llama-b11429-bin-ubuntu-x64.tar.gz"])
            for body, reason in [(data + b"!", "larger than its pinned"), (b"X" * len(data), "is not the pinned")]:
                with self.assertRaisesRegex(ValueError, reason):
                    runtime.fetch("llamacpp-linux-x64-cpu", self.root / "bad", Opener(body))
            self.assertEqual(list((self.root / "bad").iterdir()), [], "a refused download leaves nothing behind")
        with self.assertRaisesRegex(ValueError, "compiled from llama.cpp"):
            runtime.fetch("llamacpp-linux-arm64-cpu", self.root / "out")
        with self.assertRaisesRegex(ValueError, "HTTPS only"):
            runtime.HttpsRedirects().redirect_request(None, None, 302, "Found", {}, "http://objects.example/archive")

    def test_archive_reader_follows_sibling_links_only(self):
        path = self.fake_archive({"libggml.0.23.0.dylib": b"ggml"},
                                 {"libggml.0.dylib": "libggml.0.23.0.dylib", "libggml.dylib": "libggml.0.dylib",
                                  "escape.dylib": "../outside.dylib", "loop.dylib": "loop.dylib"})
        with tarfile.open(path, "r:gz") as archive:
            read = runtime.archive_reader(archive)
            self.assertEqual(read("libggml.dylib"), b"ggml")
            for name, reason in [("missing.dylib", "upstream packaging changed"), ("escape.dylib", "sibling link"),
                                 ("loop.dylib", "link loop")]:
                with self.assertRaisesRegex(ValueError, reason):
                    read(name)

    def test_directory_reader_resolves_links_inside_the_build_output_only(self):
        output = self.root / "bin"
        output.mkdir()
        (output / "libllama.so.0.4.0").write_bytes(b"llama")
        (output / "libllama.so.0").symlink_to("libllama.so.0.4.0")
        (self.root / "secret").write_bytes(b"secret")
        (output / "libescape.so").symlink_to("../secret")
        read = runtime.directory_reader(output)
        self.assertEqual(read("libllama.so.0"), b"llama")
        with self.assertRaisesRegex(ValueError, "no regular file libescape.so"):
            read("libescape.so")

    def test_macos_packs_ship_upstream_files_under_their_install_names_from_the_pinned_archive_only(self):
        names = ("llama-server", *runtime.MAC_LIBRARIES, "libggml-metal.0.dylib")
        files = {name.replace(".0.dylib", ".0.23.0.dylib"): b"\xcf\xfa\xed\xfe" + name.encode() for name in names}
        links = {name: name.replace(".0.dylib", ".0.23.0.dylib") for name in names if name.endswith(".0.dylib")}
        archive = self.fake_archive({**files, "LICENSE": b"MIT License\n", "llama-cli": b"\xcf\xfa\xed\xfecli"}, links)
        pins = {"macos-arm64": (archive.stat().st_size, runtime.sha256_file(archive))}

        def otool(*argv, env=None):
            name = Path(argv[-1]).name
            if argv[1] == "-l":
                return done(argv, "Load command 9\n          cmd LC_RPATH\n      cmdsize 32\n         path @loader_path (offset 12)\n")
            if name == "llama-server":
                return done(argv, f"{argv[-1]}:\n" + "".join(f"\t@rpath/{library} (compatibility version 0.0.0, current version 0.0.0)\n"
                                                            for library in names[1:]) + "\t/usr/lib/libc++.1.dylib (compatibility version 1.0.0)\n")
            if argv[0].endswith("llama-server"):
                return done(argv, "version: 0.6.0-dev (build 11429, commit d81235049)\n")
            return done(argv, f"{argv[-1]}:\n\t@rpath/{name} (compatibility version 0.0.0, current version 0.0.0)\n"
                              "\t/System/Library/Frameworks/Metal.framework/Versions/A/Metal (compatibility version 1.0.0)\n")
        with patch.dict(runtime.UPSTREAM_ARCHIVES, pins), patch.object(runtime, "run", otool):
            built = runtime.llamacpp("llamacpp-macos-arm64-metal", archive, EPOCH, self.root / "packs", host="aarch64-apple-darwin")
            entries = {entry[0]: entry[7] for entry in members(self.root / "packs" / built["pack"])}
            self.assertEqual(set(entries) - {"pack.json", "THIRD-PARTY-NOTICES.txt"}, set(names), "only the server's files ship")
            self.assertEqual(entries["libggml.0.dylib"], b"\xcf\xfa\xed\xfelibggml.0.dylib")
            self.assertEqual(entries["THIRD-PARTY-NOTICES.txt"], b"# llama.cpp b11429 (MIT)\n\nMIT License\n")
            with self.assertRaisesRegex(ValueError, "Build the aarch64-apple-darwin pack on a aarch64-apple-darwin runner"):
                runtime.llamacpp("llamacpp-macos-arm64-metal", archive, EPOCH, self.root / "other", host="x86_64-apple-darwin")
        with self.assertRaisesRegex(ValueError, "is not the pinned llama.cpp b11429 macos-arm64 archive"):
            runtime.llamacpp("llamacpp-macos-arm64-metal", archive, EPOCH, self.root / "unpinned", host="aarch64-apple-darwin")

    def test_macos_closure_resolves_rpath_against_the_pack_and_rejects_foreign_libraries(self):
        root = stage(self.root / "stage", {"bin/helper": b"\xcf\xfa\xed\xfehelper", "lib/libswiftCompatibilitySpan.dylib": b"\xcf\xfa\xed\xfespan",
                                           "bin/Cmlx.bundle/default.metallib": b"MTLB"}, "bin/helper")
        dependencies = ["@rpath/libswiftCompatibilitySpan.dylib", "/usr/lib/swift/libswiftCore.dylib"]

        def otool(*argv, env=None):
            if argv[1] == "-l":
                return done(argv, OTOOL_RPATHS if argv[-1].endswith("helper") else "")
            listed = dependencies if argv[-1].endswith("helper") else ["/usr/lib/swift/libswiftCompatibilitySpan.dylib"]
            return done(argv, f"{argv[-1]}:\n" + "".join(f"\t{name} (compatibility version 1.0.0)\n" for name in listed))
        with patch.object(runtime, "run", otool):
            runtime.check_macos(root)
            dependencies.append("/opt/homebrew/lib/libomp.dylib")
            dependencies.append("@rpath/libmissing.dylib")
            with self.assertRaisesRegex(ValueError, "libomp.dylib, which is neither[^\n]*\n.*libmissing.dylib"):
                runtime.check_macos(root)


class LinuxPackTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.root = Path(self.folder.name)
        self.host = FakeLinuxHost(self.root)
        self.source = arm64_build_outputs(self.root, self.host)
        stage(self.host.host, {"share/runtime-licenses/COPYING3": b"GPLv3\n",
                               "share/runtime-licenses/COPYING.RUNTIME": b"GCC Runtime Library Exception\n"})
        self.compiler = {"identity": runtime.toolchain.identity(), "version": runtime.toolchain.GCC_VERSION,
                         "libraries": {name: {"path": name, "sha256": runtime.sha256_file(self.host.host / name)}
                                       for name in runtime.toolchain.LIBRARIES}}
        (self.host.host / "runtime-toolchain.json").write_text(json.dumps(self.compiler))

    def build(self, output="packs", host="aarch64-unknown-linux-gnu"):
        with patch.object(runtime, "run", self.host.run), patch.object(runtime, "DEBIAN_DOCS", self.host.docs), \
                patch.dict(os.environ, {"RUNTIME_TOOLCHAIN_ROOT": str(self.host.host)}):
            return runtime.llamacpp("llamacpp-linux-arm64-cpu", self.source, EPOCH, self.root / output, host=host)

    def test_linux_pack_bundles_host_libraries_sets_origin_and_starts_with_and_without_fallback(self):
        built = self.build()
        entries = members(self.root / "packs" / built["pack"])
        listing = json.loads(entries[0][7])
        self.assertEqual(built["pack"], "llamacpp-b11429-aarch64-unknown-linux-gnu-cpu.tar.gz")
        self.assertEqual([entry["path"] for entry in listing["files"]], sorted(
            ["THIRD-PARTY-NOTICES.txt", "fallback/libgcc_s.so.1", "fallback/libstdc++.so.6", "libcrypto.so.3",
             "libgomp.so.1", "libssl.so.3", "llama-server", *runtime.LINUX_LIBRARIES, *runtime.ARM64_CPU_BACKENDS]))
        contents = {entry[0]: entry[7] for entry in entries}
        self.assertEqual(contents["libllama.so.0"], b"\x7fELF libllama.so.0.4.0")
        self.assertEqual(contents["fallback/libstdc++.so.6"], b"\x7fELF host libstdc++.so.6")
        notices = contents["THIRD-PARTY-NOTICES.txt"].decode()
        self.assertTrue(notices.startswith(
            f"# llama.cpp b11429 (MIT), compiled from {runtime.LLAMACPP_COMMIT} with GCC 14.3.0\n\nMIT License\n"))
        self.assertIn("# libssl3 3.0.2-0ubuntu1.20 (Ubuntu package copyright; bundled as libcrypto.so.3, libssl.so.3)\n\n"
                      "Copyright notice of libssl3\n", notices)
        self.assertEqual(sorted(re.findall(r"^# (\S+) (\S+) \(Ubuntu", notices, re.MULTILINE)),
                         [("libssl3", "3.0.2-0ubuntu1.20")])
        for name in runtime.toolchain.LIBRARIES:
            self.assertIn(f"GCC 14.3.0 ({name}; GPLv3 with GCC Runtime Library Exception)", notices)
        self.assertIn(runtime.toolchain.SOURCES["gcc-14.3.0.tar.xz"][1], notices)
        patched = sorted(Path(argv[-1]).name for command, argv, _ in self.host.calls if command == "patchelf")
        self.assertEqual(patched, ["libcrypto.so.3", "libgcc_s.so.1", "libgomp.so.1", "libssl.so.3", "libstdc++.so.6"])
        starts = [env for command, _, env in self.host.calls if command == "llama-server"]
        self.assertEqual(len(starts), 2)
        self.assertNotIn("LD_LIBRARY_PATH", starts[0])
        self.assertTrue(starts[1]["LD_LIBRARY_PATH"].endswith("/stage/fallback"))
        self.assertEqual(self.build("again")["sha256"], built["sha256"], "a rebuild is byte-identical")

    def test_linux_pack_refuses_glibc_above_the_floor(self):
        self.host.info["libggml-base.so.0"]["needs"]["libc.so.6"] = {"GLIBC_2.38"}
        with self.assertRaisesRegex(ValueError, "libggml-base.so.0 needs GLIBC_2.38, above the GLIBC_2.35 floor"):
            self.build()
        self.host.info["libggml-base.so.0"]["needs"]["libc.so.6"] = {"GLIBC_ABI_DT_RELR"}
        with self.assertRaisesRegex(ValueError, "needs GLIBC_ABI_DT_RELR"):
            self.build()

    def test_linux_pack_refuses_libraries_that_are_neither_packed_nor_glibc(self):
        self.host.info["libllama-common.so.0"]["needed"].append("libcurl.so.4")
        with self.assertRaisesRegex(ValueError, "libllama-common.so.0 needs libcurl.so.4, which is neither in the pack nor part of glibc"):
            self.build()

    def test_linux_pack_refuses_versions_the_bundled_copy_lacks(self):
        self.host.info["llama-server"]["needs"]["libstdc++.so.6"] = {"GLIBCXX_3.4.32"}
        with self.assertRaisesRegex(ValueError, "llama-server needs GLIBCXX_3.4.32 from libstdc\\+\\+.so.6, which the bundled copy does not define"):
            self.build()

    def test_linux_pack_requires_bundled_libraries_on_the_runner_and_patches_foreign_run_paths(self):
        self.host.info["libggml.so.0"]["runpath"] = "/home/runner/work/llama.cpp/build/bin"
        self.build()
        self.assertIn("libggml.so.0", [Path(argv[-1]).name for command, argv, _ in self.host.calls if command == "patchelf"])
        (self.host.host / "libgomp.so.1").unlink()
        with self.assertRaisesRegex(ValueError, "Install libgomp.so.1 on the build runner"):
            self.build("missing")

    def test_linux_pack_refuses_a_changed_compiler_runtime_or_unowned_system_library(self):
        (self.host.host / "libgomp.so.1").write_bytes(b"\x7fELF changed development runtime")
        with self.assertRaisesRegex(ValueError, "libgomp.so.1 is not from the pinned GCC"):
            self.build()
        (self.host.host / "libgomp.so.1").write_bytes(b"\x7fELF host libgomp.so.1")
        self.host.owners.pop(str((self.host.host / "libssl.so.3").resolve()))
        with self.assertRaisesRegex(ValueError, "No installed Debian package owns .*libssl.so.3"):
            self.build("unowned")

    def test_linux_pack_must_report_the_pinned_build_on_its_own_platform(self):
        self.host.version = "version: 0.4.0 (build 1, commit d81235049)"
        with self.assertRaisesRegex(ValueError, "not build 11429"):
            self.build()
        with self.assertRaisesRegex(ValueError, "on a aarch64-unknown-linux-gnu runner"):
            self.build("elsewhere", host="x86_64-unknown-linux-gnu")
        with self.assertRaisesRegex(ValueError, "build output directory"):
            runtime.llamacpp("llamacpp-linux-arm64-cpu", self.root / "missing", EPOCH, self.root / "x", host="aarch64-unknown-linux-gnu")

    def test_upstream_linux_packs_bundle_the_vulkan_loader_only_as_a_fallback(self):
        cpu, vulkan = runtime.LLAMACPP_PACKS["llamacpp-linux-x64-cpu"], runtime.LLAMACPP_PACKS["llamacpp-linux-x64-vulkan"]
        self.assertEqual(set(vulkan["files"]) - set(cpu["files"]), {"libggml-vulkan.so"})
        self.assertEqual(set(vulkan["fallback"]) - set(cpu["fallback"]), {"libvulkan.so.1"})
        self.assertNotIn("libggml-rpc.so", vulkan["files"], "Linux packs leave out the RPC backend plugin")
        self.assertEqual(len(runtime.X64_CPU_BACKENDS), 14)


class MlxPackTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.root = Path(self.folder.name)
        self.executable = self.root / "flow-like-mlx-service-aarch64-apple-darwin"
        self.executable.write_bytes(b"\xcf\xfa\xed\xfemlx")
        self.resources = self.root / "mlx-resources"
        stage(self.resources, {"mlx-swift_Cmlx.bundle/Contents/Resources/default.metallib": b"MTLB",
                               "mlx-swift_Cmlx.bundle/Contents/Info.plist": b"<plist/>",
                               "swift-transformers_Hub.bundle/Contents/Resources/t5_tokenizer_config.json": b"{}"})
        self.toolchain = self.root / "toolchain"
        stage(self.toolchain, {"swift-6.2/macosx/libswiftCompatibilitySpan.dylib": b"\xcf\xfa\xed\xfespan",
                               "swift-6.2/iphoneos/libswiftCompatibilitySpan.dylib": b"\xcf\xfa\xed\xfeios"})
        self.notices = self.root / "THIRD-PARTY-NOTICES.txt"
        self.notices.write_text("# Third-Party Notices — Apple MLX Swift runtime\n")
        self.started = done([], "MLX error: [metal::Device] Unable to load device\n", 255)

    def run_tool(self, *argv, env=None):
        if argv[0].endswith("bin/flow-like-mlx-service"):
            return self.started
        if argv[1] == "-l":
            return done(argv, OTOOL_RPATHS if argv[-1].endswith("flow-like-mlx-service") else "")
        if argv[-1].endswith("flow-like-mlx-service"):
            return done(argv, f"{argv[-1]}:\n\t/usr/lib/swift/libswiftCore.dylib (compatibility version 1.0.0)\n"
                              "\t@rpath/libswiftCompatibilitySpan.dylib (compatibility version 0.0.0)\n")
        return done(argv, f"{argv[-1]}:\n\t/usr/lib/swift/libswiftCompatibilitySpan.dylib (compatibility version 0.0.0)\n")

    def build(self, output="packs"):
        with patch.object(runtime, "run", self.run_tool):
            return runtime.mlx(self.executable, self.resources, self.toolchain, self.notices, "g71d368e71f1a", EPOCH,
                               self.root / output, host="aarch64-apple-darwin")

    def test_mlx_pack_puts_bundles_beside_the_helper_and_back_deployment_libraries_in_lib(self):
        built = self.build()
        self.assertEqual(built["pack"], "mlx-g71d368e71f1a-aarch64-apple-darwin-metal.tar.gz")
        entries = members(self.root / "packs" / built["pack"])
        listing = json.loads(entries[0][7])
        self.assertEqual(listing["entrypoint"], "bin/flow-like-mlx-service")
        self.assertEqual({entry[0]: entry[7] for entry in entries}["THIRD-PARTY-NOTICES.txt"], self.notices.read_bytes())
        self.assertEqual([(entry["path"], entry["executable"]) for entry in listing["files"]], [
            ("THIRD-PARTY-NOTICES.txt", False),
            ("bin/flow-like-mlx-service", True),
            ("bin/mlx-swift_Cmlx.bundle/Contents/Info.plist", False),
            ("bin/mlx-swift_Cmlx.bundle/Contents/Resources/default.metallib", False),
            ("bin/swift-transformers_Hub.bundle/Contents/Resources/t5_tokenizer_config.json", False),
            ("lib/libswiftCompatibilitySpan.dylib", False)])

    def test_mlx_pack_refuses_a_helper_that_does_not_load_or_misses_its_kernels(self):
        self.started = done([], "dyld[42]: Library not loaded: @rpath/libswiftCompatibilitySpan.dylib\n", -6)
        with self.assertRaisesRegex(ValueError, "does not load: dyld"):
            self.build()
        (self.toolchain / "swift-6.2/macosx/libswiftCompatibilitySpan.dylib").unlink()
        with self.assertRaisesRegex(ValueError, "Expected one libswiftCompatibilitySpan.dylib"):
            self.build("again")
        (self.resources / "mlx-swift_Cmlx.bundle/Contents/Resources/default.metallib").unlink()
        with self.assertRaisesRegex(ValueError, "no mlx-swift_Cmlx.bundle with default.metallib"):
            self.build("third")


class ManifestTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.root = Path(self.folder.name)
        self.packs = self.root / "packs"
        for backend in ("vulkan", "cpu"):
            build_pack(self.folder.name, {"llama-server": b"server", f"libggml-{backend}.so": b"x" * 100}, backend=backend)
        build_pack(self.folder.name, {"llama-server": b"mac", "libggml.0.dylib": b"g"}, target="aarch64-apple-darwin",
                   backend="metal")

    def test_one_unsigned_manifest_per_target_in_the_signed_field_order(self):
        sizes = runtime.manifest(self.packs, f"{BASE}/releases/9/runtimes", 9, self.root, validity_days=30, issued_at=1000)
        self.assertEqual(set(sizes), {"x86_64-unknown-linux-gnu", "aarch64-apple-darwin"})
        body = (self.root / "x86_64-unknown-linux-gnu.json").read_text()
        value = json.loads(body, object_pairs_hook=lambda pairs: [key for key, _ in pairs] and dict(pairs))
        self.assertEqual(list(value), ["version", "sequence", "issued_at", "expires_at", "packs"])
        self.assertEqual((value["sequence"], value["issued_at"], value["expires_at"]), (9, 1000, 1000 + 30 * 86400))
        self.assertEqual([pack["backend"] for pack in value["packs"]], ["cpu", "vulkan"])
        self.assertEqual(list(value["packs"][0]), ["runtime", "build", "target", "backend", "url", "size", "sha256", "entrypoint", "files"])
        self.assertEqual(list(value["packs"][0]["files"][0]), ["path", "size", "sha256", "executable"])
        self.assertEqual(value["packs"][1]["url"], f"{BASE}/releases/9/runtimes/llamacpp-b11429-x86_64-unknown-linux-gnu-vulkan.tar.gz")
        archive = self.packs / "llamacpp-b11429-x86_64-unknown-linux-gnu-vulkan.tar.gz"
        self.assertEqual((value["packs"][1]["size"], value["packs"][1]["sha256"]), (archive.stat().st_size, runtime.sha256_file(archive)))
        self.assertEqual(body, json.dumps(value, separators=(",", ":")))
        signed = Signer(self.root).sign(body)
        self.assertEqual(len(signed), sizes["x86_64-unknown-linux-gnu"], "the size estimate is the signed length")

    def test_manifest_refuses_an_oversized_list_bad_inputs_and_tampered_packs(self):
        with patch.object(runtime, "MAX_COMPACT_JWS_BYTES", 1000), self.assertRaisesRegex(ValueError, "would sign to [0-9]+ bytes, above the 1000-byte JWS limit"):
            runtime.manifest(self.packs, f"{BASE}/releases/9/runtimes", 9, self.root)
        with patch.object(runtime, "compact_size", return_value=15 * 1024 + 1), self.assertRaisesRegex(ValueError, "above the 15360-byte JWS limit"):
            runtime.manifest(self.packs, f"{BASE}/releases/9/runtimes", 9, self.root)
        for base, sequence, reason in [("http://cdn.example/x", 9, "direct HTTPS prefix"), (f"{BASE}/", 9, "direct HTTPS prefix"),
                                       (f"{BASE}/runtimes", 0, "positive runtime manifest sequence")]:
            with self.assertRaisesRegex(ValueError, reason):
                runtime.manifest(self.packs, base, sequence, self.root)
        with self.assertRaisesRegex(ValueError, "between 1 and 1825 days"):
            runtime.manifest(self.packs, f"{BASE}/runtimes", 9, self.root, validity_days=1826)
        with self.assertRaisesRegex(ValueError, "holds no runtime packs"):
            runtime.manifest(self.root / "empty", f"{BASE}/runtimes", 9, self.root)
        archive = self.packs / "llamacpp-b11429-aarch64-apple-darwin-metal.tar.gz"
        rewrite(archive, [("pack.json", members(archive)[0][7], 0o644), ("libggml.0.dylib", b"G", 0o644), ("llama-server", b"mac", 0o755)])
        with self.assertRaisesRegex(ValueError, "differs from its pack.json entry"):
            runtime.manifest(self.packs, f"{BASE}/runtimes", 9, self.root)


class PublishTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.root = Path(self.folder.name)
        self.bundle = self.root / "bundle"
        self.signer = Signer(self.root)
        self.keys = [self.signer.key]
        for backend in ("cpu", "vulkan"):
            build_pack(self.folder.name, {"llama-server": b"server", f"libggml-{backend}.so": b"x"}, backend=backend, output=self.bundle)
        build_pack(self.folder.name, {"llama-server": b"mac", "libggml.0.dylib": b"g"}, target="aarch64-apple-darwin",
                   backend="metal", output=self.bundle)

    def signed(self, sequence, typ=runtime.JWS_TYPE, **validity):
        runtime.manifest(self.bundle, f"{BASE}/releases/{sequence}/runtimes", sequence, self.bundle, **validity)
        for target in ("x86_64-unknown-linux-gnu", "aarch64-apple-darwin"):
            compact = self.signer.sign((self.bundle / f"{target}.json").read_text(), typ)
            (self.bundle / f"{target}.jws").write_bytes(compact)
        return {target: (self.bundle / f"{target}.jws").read_bytes() for target in ("x86_64-unknown-linux-gnu", "aarch64-apple-darwin")}

    def publish(self, store, verify=lambda *_: None):
        return runtime.publish_bundle(self.bundle, BASE, "standalone", self.keys, store, verify)

    def test_packs_and_sequence_manifests_upload_and_read_back_before_each_stable_list_advances(self):
        signed = self.signed(9)
        store, verified = MemoryStore(), []

        def readback(url, size, digest):
            if url.startswith(f"{BASE}/runtimes/"):
                immutable = [earlier for earlier in verified if not earlier.startswith(f"{BASE}/runtimes/")]
                self.assertEqual(len(immutable), 5, "all immutable objects come first")
            else:
                self.assertFalse(any(key.startswith("standalone/runtimes/") for key in store.objects))
            verified.append(url)
        result = self.publish(store, readback)
        self.assertEqual(result, {"sequence": 9, "manifests": {target: f"{BASE}/runtimes/{target}.jws" for target in signed}})
        self.assertEqual(sorted(key for key, immutable in store.writes if immutable), [
            "standalone/releases/9/runtimes/aarch64-apple-darwin.jws",
            "standalone/releases/9/runtimes/llamacpp-b11429-aarch64-apple-darwin-metal.tar.gz",
            "standalone/releases/9/runtimes/llamacpp-b11429-x86_64-unknown-linux-gnu-cpu.tar.gz",
            "standalone/releases/9/runtimes/llamacpp-b11429-x86_64-unknown-linux-gnu-vulkan.tar.gz",
            "standalone/releases/9/runtimes/x86_64-unknown-linux-gnu.jws"])
        self.assertEqual(sorted(store.writes[-2:]), sorted((f"standalone/runtimes/{target}.jws", False) for target in signed))
        for target, compact in signed.items():
            self.assertEqual(store.objects[f"standalone/runtimes/{target}.jws"], compact)
        writes = list(store.writes)
        self.publish(store)
        self.assertEqual(store.writes, writes, "a retry finds every object committed")

    def test_rollback_replacement_and_concurrent_publishers_cannot_move_a_stable_list(self):
        store = MemoryStore()
        self.signed(9)
        self.publish(store)
        stable = dict(store.objects)
        self.signed(8)
        with self.assertRaisesRegex(ValueError, "roll back or replace signed sequence 9"):
            self.publish(store)
        self.signed(9, validity_days=30)
        with self.assertRaisesRegex(ValueError, "roll back or replace signed sequence 9"):
            self.publish(store)
        self.assertEqual({key: store.objects[key] for key in stable}, stable)
        newer = self.signed(12)["aarch64-apple-darwin"]
        store.race = newer
        self.signed(11)
        with self.assertRaisesRegex(ValueError, "changed concurrently"):
            self.publish(store)

    def test_untrusted_wrong_type_mixed_or_altered_bundles_upload_nothing(self):
        cases = []
        self.signed(9, typ="flow-like-standalone-release+jws")
        cases.append("Unexpected runtime manifest signature type")
        for reason in cases:
            store = MemoryStore()
            with self.assertRaisesRegex(ValueError, reason):
                self.publish(store)
            self.assertEqual(store.writes, [])
        self.signed(9)
        with self.assertRaisesRegex(ValueError, "not trusted"):
            runtime.publish_bundle(self.bundle, BASE, "standalone", [b64(bytes(range(32)))], MemoryStore())
        linux = (self.bundle / "x86_64-unknown-linux-gnu.jws").read_bytes()
        self.signed(10)
        (self.bundle / "x86_64-unknown-linux-gnu.jws").write_bytes(linux)
        with self.assertRaisesRegex(ValueError, "share one sequence"):
            self.publish(MemoryStore())
        self.signed(9)
        (self.bundle / "aarch64-apple-darwin.jws").write_bytes(linux)
        with self.assertRaisesRegex(ValueError, "lists packs of another target"):
            self.publish(MemoryStore())
        self.signed(9)
        (self.bundle / "llamacpp-b11429-x86_64-unknown-linux-gnu-cpu.tar.gz").write_bytes(b"changed")
        store = MemoryStore()
        with self.assertRaisesRegex(ValueError, "differs from its signed size, digest or immutable URL"):
            self.publish(store)
        self.assertEqual(store.writes, [])

    def test_verification_checks_signature_time_and_lifetime(self):
        compact = self.signed(9)["aarch64-apple-darwin"]
        self.assertEqual(runtime.verified_manifest(compact, self.keys)["sequence"], 9)
        flipped = compact[:-2] + (b"AA" if compact[-2:] != b"AA" else b"BA")
        with self.assertRaises(ValueError):
            runtime.verified_manifest(flipped, self.keys)
        expiry = runtime.verified_manifest(compact, self.keys)["expires_at"]
        with patch.object(runtime.time, "time", return_value=expiry):
            with self.assertRaisesRegex(ValueError, "expired"):
                runtime.verified_manifest(compact, self.keys)
            self.assertEqual(runtime.verified_manifest(compact, self.keys, current=False)["sequence"], 9)
        value = json.loads((self.bundle / "aarch64-apple-darwin.json").read_text())
        value["expires_at"] = value["issued_at"] + 1826 * 86400
        with self.assertRaisesRegex(ValueError, "lifetime"):
            runtime.verified_manifest(self.signer.sign(value), self.keys)

    def test_summary_names_each_target_pack_and_end_date(self):
        self.signed(9, issued_at=None)
        text = runtime.summary(self.bundle, json.dumps(self.keys))
        self.assertIn("### Runtime packs 9 published", text)
        self.assertRegex(text, r"\| x86_64-unknown-linux-gnu \| llamacpp b11429 \| vulkan \| 0\.0 MiB \| 2 \| \d{4}-\d{2}-\d{2} \|")


def job_body(name):
    match = re.search(rf"^  {re.escape(name)}:\n(.*?)(?=^  [a-z][a-z-]*:\n|\Z)", WORKFLOW.read_text(), re.MULTILINE | re.DOTALL)
    if not match:
        raise AssertionError(f"Missing workflow job: {name}")
    return match.group(1)


def step(job, start):
    for text in re.split(r"^      - ", job_body(job), flags=re.MULTILINE):
        if text.startswith(start + "\n"):
            return text
    raise AssertionError(f"Missing workflow step: {start}")


def script(text):
    return textwrap.dedent(text.split("        run: |\n", 1)[1])


class WorkflowTests(unittest.TestCase):
    def test_matrix_builds_every_pack_on_its_own_platform(self):
        job = job_body("runtime-packs")
        matrix = dict(re.findall(r"^          - pack: (\S+)\n            runner: (\S+)$", job, re.MULTILINE))
        self.assertEqual(set(matrix), set(runtime.LLAMACPP_PACKS) | {"mlx-macos-arm64-metal"})
        runners = {"aarch64-apple-darwin": "macos-15", "x86_64-apple-darwin": "macos-15-intel",
                   "x86_64-unknown-linux-gnu": "ubuntu-22.04", "aarch64-unknown-linux-gnu": "ubuntu-22.04-arm"}
        for pack, spec in runtime.LLAMACPP_PACKS.items():
            self.assertEqual(matrix[pack], runners[spec["target"]], pack)
        self.assertEqual(matrix["mlx-macos-arm64-metal"], "macos-15")
        self.assertIn("needs: prepare\n", job)
        self.assertNotIn("secrets.", job)
        self.assertNotIn("environment:", job)

    def test_arm64_source_build_takes_its_commit_from_the_pin(self):
        build = (REPOSITORY / ".github/scripts/build_linux_runtime.sh").read_text()
        self.assertIn("runtime_packs.py pin | sed -n 's/^commit=//p'", build)
        self.assertNotRegex(build, r"\b[0-9a-f]{40}\b")
        self.assertIn('test "$(git -C /scratch/llama.cpp rev-parse HEAD)" = "$LLAMACPP_COMMIT"', build)
        self.assertIn("-DCMAKE_INSTALL_RPATH='$ORIGIN'", build)

    def test_arm64_source_build_compiles_with_the_toolchain_its_notices_record(self):
        build = (REPOSITORY / ".github/scripts/build_linux_runtime.sh").read_text()
        self.assertEqual(runtime.LLAMACPP_PACKS["llamacpp-linux-arm64-cpu"]["toolchain"], runtime.toolchain.GCC_VERSION)
        self.assertIn("python3 .github/scripts/runtime_toolchain.py build", build)
        self.assertIn("export RUNTIME_TOOLCHAIN_ROOT=/toolchain", build)
        self.assertIn("runtime_toolchain.py library-path", build)
        loader_config = re.search(r"> /etc/ld\.so\.conf\.d/([^\s/]+\.conf)", build)
        self.assertIsNotNone(loader_config)
        self.assertLess(loader_config.group(1), "aarch64-linux-gnu.conf",
                        "The pinned GCC runtime must precede Ubuntu's system runtime in the loader cache")
        self.assertIn("\n  ldconfig\n", build)
        self.assertIn("-DCMAKE_C_COMPILER=/toolchain/bin/gcc -DCMAKE_CXX_COMPILER=/toolchain/bin/g++", build)
        self.assertNotIn("ppa:", job_body("runtime-packs") + build)

    def test_linux_build_uses_a_frozen_container_and_snapshot(self):
        container = step("runtime-packs", "name: Build and check Linux packs in the frozen Ubuntu 22.04 environment")
        self.assertIn('"$RUNTIME_IMAGE" bash /work/.github/scripts/build_linux_runtime.sh', container)
        self.assertRegex(runtime.toolchain.IMAGE, r"^ubuntu:22\.04@sha256:[0-9a-f]{64}$")
        self.assertIn("--env UBUNTU_SNAPSHOT", container)
        self.assertIn("runtime-gcc-arm64-${{ steps.linux-inputs.outputs.key }}", job_body("runtime-packs"))
        self.assertNotIn("restore-keys:", job_body("runtime-packs"))

    def run_fetch(self, outcome):
        with tempfile.TemporaryDirectory() as folder:
            directory = Path(folder)
            commands = directory / "commands"
            commands.mkdir()
            (commands / "python3").write_text(f"#!{sys.executable}\nimport sys\nprint('/tmp/upstream/archive.tar.gz')\nsys.exit({outcome})\n")
            (commands / "python3").chmod(0o755)
            environment = directory / "environment"
            environment.touch()
            result = subprocess.run(["bash", "--noprofile", "--norc", "-eo", "pipefail", "-c",
                                     script(step("runtime-packs", "name: Download the pinned upstream llama.cpp archive and check its digest"))],
                                    env={**os.environ, "PATH": f"{commands}{os.pathsep}{os.environ['PATH']}", "GITHUB_ENV": str(environment),
                                         "RUNNER_TEMP": folder, "RUNTIME_PACK": "llamacpp-linux-x64-cpu"},
                                    capture_output=True, text=True, check=False)
            return result.returncode, environment.read_text()

    def test_a_failed_download_stops_the_job_before_packing(self):
        self.assertEqual(self.run_fetch(0), (0, "LLAMACPP_SOURCE=/tmp/upstream/archive.tar.gz\n"))
        self.assertEqual(self.run_fetch(7), (7, ""))

    def test_signing_uses_the_release_jobs_key_handling_after_the_signer_is_built(self):
        job = job_body("runtime-manifest")
        self.assertIn("\n    environment: standalone-release\n", job)
        self.assertEqual(job.count("secrets."), 1)
        self.assertLess(job.index("--example sign-runtime-manifest"), job.index("secrets.STANDALONE_RELEASE_SIGNING_KEY"))
        self.assertLess(job.index("test_runtime*.py"), job.index("runtime_packs.py manifest"))
        ours = script(step("runtime-manifest", "name: Sign the runtime manifests using the configured private release key"))
        theirs = script(step("signed-bundle", "name: Sign the release manifest using the configured private release key"))
        prefix = "trap 'rm -f \"$RUNNER_TEMP/standalone-signing-key\"' EXIT\n"
        self.assertEqual(ours.split(prefix)[0], theirs.split(prefix)[0])
        self.assertIn('"$RUNNER_TEMP/release-signer/debug/examples/sign-runtime-manifest" "$RUNNER_TEMP/standalone-signing-key" "$manifest" "${manifest%.json}.jws"', ours)

    def test_runtime_publication_precedes_agents_with_the_same_rollback_floor(self):
        job = job_body("runtime-publish")
        self.assertIn("needs: [prepare, runtime-manifest]\n", job)
        self.assertIn("needs: [prepare, signed-bundle, runtime-publish]\n", job_body("publish"))
        self.assertIn("\n    environment: standalone-release\n", job)
        self.assertNotIn("STANDALONE_RELEASE_SIGNING_KEY", job)
        identity = "name: Require exactly one publisher identity"
        self.assertEqual(step("runtime-publish", identity), step("publish", identity))
        credentials = "name: Obtain short-lived AWS publisher credentials"
        self.assertEqual(step("runtime-publish", credentials),
                         step("publish", credentials).replace("standalone-release-${{", "standalone-runtime-packs-${{"))
        ours = step("runtime-publish", "name: Publish runtime packs, then conditionally advance each target's signed runtime manifest")
        theirs = step("publish", "name: Verify anonymous images, publish binaries, then conditionally advance the signed manifest")
        environment = re.compile(r"^        env:\n(.*?)^        run:", re.MULTILINE | re.DOTALL)
        self.assertEqual(environment.search(ours).group(1), environment.search(theirs).group(1))
        self.assertIn("runtime.publish(Path('runtime-bundle')", ours)
        last = re.split(r"^      - ", job, flags=re.MULTILINE)[-1]
        self.assertIn('runtime_packs.py summary --artifacts runtime-bundle --public-keys "$RELEASE_PUBLIC_KEYS" >> "$GITHUB_STEP_SUMMARY"', last)

    def test_commands_reach_their_functions_with_the_documented_options(self):
        with patch.object(runtime, "manifest", return_value={"aarch64-apple-darwin": 1}) as manifest, \
                patch("sys.stdout", new_callable=io.StringIO) as output:
            runtime.main(["manifest", "--packs", "p", "--base-url", BASE, "--sequence", "3", "--validity-days", "30", "--output", "o"])
        manifest.assert_called_once_with(packs=Path("p"), base_url=BASE, sequence=3, validity_days=30, output=Path("o"))
        self.assertEqual(output.getvalue(), '{"aarch64-apple-darwin":1}\n')
        with patch.object(runtime, "mlx", return_value={}) as mlx, patch("sys.stdout", new_callable=io.StringIO):
            runtime.main(["mlx", "--executable", "e", "--resources", "r", "--toolchain", "t", "--notices", "n", "--build", "g1",
                          "--epoch", "5", "--output", "o"])
        mlx.assert_called_once_with(executable=Path("e"), resources=Path("r"), toolchain=Path("t"), notices=Path("n"),
                                    build="g1", epoch=5, output=Path("o"))
        with patch("sys.stdout", new_callable=io.StringIO) as output:
            runtime.main(["pin"])
        self.assertEqual(output.getvalue(), runtime.pin() + "\n")


if __name__ == "__main__":
    unittest.main()
