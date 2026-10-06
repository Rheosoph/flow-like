"""The source-built runtime compiler must not drift to runner or PPA libraries."""

import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from urllib.request import Request

spec = importlib.util.spec_from_file_location("runtime_toolchain", Path(__file__).parents[1] / "runtime_toolchain.py")
toolchain = importlib.util.module_from_spec(spec)
spec.loader.exec_module(toolchain)


class Opener:
    def __init__(self, body):
        self.body, self.requests = body, []

    def open(self, request, timeout):
        self.requests.append((request.full_url, timeout))
        return io.BytesIO(self.body)


class ToolchainTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.root = Path(self.folder.name)
        self.prefix = self.root / "toolchain"
        self.prefix.mkdir()

    def cache(self):
        libraries = {}
        for name in toolchain.LIBRARIES:
            path = self.prefix / "lib" / name
            path.parent.mkdir(exist_ok=True)
            path.write_bytes(b"\x7fELF pinned " + name.encode())
            libraries[name] = {"path": "lib/" + name, "sha256": toolchain.digest(path)}
        for name in ("COPYING3", "COPYING.RUNTIME"):
            path = self.prefix / "share/runtime-licenses" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(name + " text")
        value = {"identity": toolchain.identity(), "version": toolchain.GCC_VERSION, "libraries": libraries}
        (self.prefix / "runtime-toolchain.json").write_text(json.dumps(value))
        return value

    def test_download_checks_hash_before_replacing_a_cached_archive(self):
        data = b"pinned source"
        pins = {"test.tar.xz": ("https://gnu.example/", hashlib.sha512(data).hexdigest())}
        path = self.root / "test.tar.xz"
        path.write_bytes(b"older archive")
        with patch.dict(toolchain.SOURCES, pins):
            with self.assertRaisesRegex(ValueError, "pinned SHA512"):
                toolchain.fetch("test.tar.xz", path, Opener(b"changed source"))
            self.assertEqual(path.read_bytes(), b"older archive")
            self.assertEqual(list(self.root.glob(".source-*")), [])
            opener = Opener(data)
            toolchain.fetch("test.tar.xz", path, opener)
            self.assertEqual(path.read_bytes(), data)
            self.assertEqual(opener.requests, [("https://gnu.example/test.tar.xz", 120)])
            toolchain.fetch("test.tar.xz", path, opener)
            self.assertEqual(len(opener.requests), 1, "only verified cached bytes avoid downloading again")

    def test_download_limits_bytes_and_rejects_transport_downgrade(self):
        with patch.object(toolchain, "MAX_SOURCE_BYTES", 4):
            with self.assertRaisesRegex(ValueError, "exceeds"):
                toolchain.fetch("gcc-14.3.0.tar.xz", self.root / "source", Opener(b"too long"))
        self.assertFalse((self.root / "source").exists())
        with self.assertRaisesRegex(ValueError, "HTTPS"):
            toolchain.HttpsRedirects().redirect_request(Request("https://gnu.example/"), None, 302, "", {}, "http://gnu.example/file")

    def test_cache_reuse_checks_recipe_version_and_runtime_bytes(self):
        self.cache()
        with patch.object(toolchain, "command") as command, patch.object(toolchain, "fetch") as fetch, \
                patch.object(toolchain, "output", return_value=toolchain.GCC_VERSION):
            toolchain.build(self.prefix, self.root / "work", 2)
            command.assert_not_called()
            fetch.assert_not_called()
            (self.prefix / "lib/libgomp.so.1").write_bytes(b"development OpenMP runtime")
            with self.assertRaisesRegex(ValueError, "changed after"):
                toolchain.build(self.prefix, self.root / "work", 2)
        self.cache()
        with patch.object(toolchain, "identity", return_value="changed build recipe"):
            with self.assertRaisesRegex(ValueError, "pinned build inputs"):
                toolchain.build(self.prefix, self.root / "work", 2)
        with patch.object(toolchain, "output", return_value="16.0.0"):
            with self.assertRaisesRegex(ValueError, "pinned version"):
                toolchain.build(self.prefix, self.root / "work", 2)

    def test_library_provenance_rejects_ambient_and_mutated_runtime_libraries(self):
        self.cache()
        source = self.prefix / "lib/libstdc++.so.6"
        with patch.dict(os.environ, {"RUNTIME_TOOLCHAIN_ROOT": str(self.prefix)}):
            title, notice = toolchain.library_notice(source.name, source)
            self.assertIn("GCC 14.3.0", title)
            self.assertIn("COPYING.RUNTIME text", notice)
            self.assertIn(toolchain.SOURCES["gcc-14.3.0.tar.xz"][1], notice)
            foreign = self.root / source.name
            foreign.write_bytes(source.read_bytes())
            with self.assertRaisesRegex(ValueError, "not from the pinned GCC"):
                toolchain.library_notice(source.name, foreign)
            source.write_bytes(b"newer compiler runtime")
            with self.assertRaisesRegex(ValueError, "not from the pinned GCC"):
                toolchain.library_notice(source.name, source)

    def test_library_provenance_cannot_escape_the_cache_through_a_symlink(self):
        self.cache()
        source = self.prefix / "lib/libgcc_s.so.1"
        foreign = self.root / "foreign-library"
        source.rename(foreign)
        source.symlink_to(foreign)
        with patch.dict(os.environ, {"RUNTIME_TOOLCHAIN_ROOT": str(self.prefix)}):
            with self.assertRaisesRegex(ValueError, "not from the pinned GCC"):
                toolchain.library_notice(source.name, source)

    def test_incomplete_cache_is_not_reused(self):
        (self.prefix / "bin").mkdir()
        with self.assertRaisesRegex(ValueError, "incomplete"):
            toolchain.build(self.prefix, self.root / "work", 2)

    def test_build_keeps_shared_runtime_and_openmp_with_only_c_and_cxx(self):
        work = self.root / "work"
        calls = []

        def command(*argv, cwd=None, env=None):
            calls.append((tuple(map(str, argv)), cwd, env))
            if argv[0] == "tar":
                name = Path(argv[2]).name.split(".tar.")[0]
                (work / name).mkdir()
                if name == "gcc-14.3.0":
                    for license_name in ("COPYING3", "COPYING.RUNTIME"):
                        (work / name / license_name).write_text(license_name)
            if argv[:2] == ("make", "install-strip"):
                (self.prefix / "lib").mkdir()
                for name in toolchain.LIBRARIES:
                    (self.prefix / "lib" / name).write_bytes(name.encode())

        def output(*argv):
            return toolchain.GCC_VERSION if argv[-1] == "-dumpfullversion" else str(self.prefix / "lib" / argv[-1].split("=")[1])

        with patch.object(toolchain, "fetch") as fetch, patch.object(toolchain, "command", side_effect=command), \
                patch.object(toolchain, "output", side_effect=output):
            toolchain.build(self.prefix, work, 2)
        self.assertEqual({call.args[0] for call in fetch.call_args_list}, set(toolchain.SOURCES))
        gcc = next(argv for argv, _, _ in calls if argv[0].endswith("gcc-14.3.0/configure"))
        self.assertIn("--enable-languages=c,c++", gcc)
        self.assertIn("--disable-bootstrap", gcc)
        self.assertIn("--enable-multiarch", gcc)
        self.assertNotIn("--disable-shared", gcc)
        self.assertNotIn("--disable-libgomp", gcc)
        self.assertIn(f"--with-as={self.prefix.resolve() / 'bin/as'}", gcc)
        probe = next(argv for argv, _, _ in calls if argv[0].endswith("/bin/g++"))
        self.assertIn("-fopenmp", probe)
        self.assertTrue(any(argv[0].endswith("/compiler-probe") for argv, _, _ in calls))
        value = toolchain.manifest(self.prefix)
        self.assertEqual(set(value["libraries"]), set(toolchain.LIBRARIES))


if __name__ == "__main__":
    unittest.main()
