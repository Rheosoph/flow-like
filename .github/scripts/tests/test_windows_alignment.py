import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


spec = importlib.util.spec_from_file_location("check_windows_alignment", Path(__file__).parents[1] / "check_windows_alignment.py")
alignment = importlib.util.module_from_spec(spec)
spec.loader.exec_module(alignment)

REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"


def package(name, version, dependencies=(), source=REGISTRY):
    return dict(name=name, version=version, dependencies=list(dependencies), source=source)


class WindowsAlignmentTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.lockfile = Path(self.folder.name) / "Cargo.lock"
        self.packages = [
            package("windows-core", "0.61.2"),
            package("windows-core", "0.62.2"),
            package("windows", "0.61.3", ["windows-core 0.61.2"]),
            package("windows", "0.62.2", ["windows-core 0.62.2"]),
        ]

    def errors(self, *consumers):
        self.lockfile.write_text("version = 4\n" + "".join(
            "\n[[package]]\n" + "".join(f"{key} = {json.dumps(value)}\n" for key, value in item.items())
            for item in [*self.packages, *consumers]
        ))
        return alignment.check_alignment(self.lockfile)

    def test_rejects_wmi_with_incompatible_direct_windows_core(self):
        errors = self.errors(package("wmi", "0.18.4", ["windows 0.61.3", "windows-core 0.62.2"]))
        self.assertEqual(len(errors), 1)
        self.assertIn("wmi 0.18.4", errors[0])
        self.assertIn("windows-core 0.61.2", errors[0])
        self.assertIn("windows-core 0.62.2", errors[0])

    def test_rejects_allocator_with_incompatible_d3d12_types(self):
        errors = self.errors(
            package("gpu-allocator", "0.28.0", ["windows 0.61.3"]),
            package("wgpu-hal", "30.0.1", ["windows 0.62.2", "windows-core 0.62.2", "gpu-allocator"]),
        )
        self.assertEqual(len(errors), 1)
        self.assertIn("wgpu-hal 30.0.1", errors[0])
        self.assertIn("gpu-allocator 0.28.0", errors[0])

    def test_allows_independent_versions_and_different_windows_core_patch_versions(self):
        self.assertEqual(self.errors(
            package("wmi", "0.18.4", ["windows 0.61.3", "windows-core 0.61.2"]),
            package("gpu-allocator", "0.28.0", ["windows 0.62.2"]),
            package("wgpu-hal", "30.0.1", ["windows 0.62.2", "windows-core 0.62.2", "gpu-allocator"]),
        ), [])

    def test_accepts_unique_unversioned_dependencies(self):
        self.packages = [
            package("windows-core", "0.61.2"),
            package("windows", "0.61.3", ["windows-core"]),
        ]
        self.assertEqual(self.errors(package("wmi", "0.18.4", ["windows", "windows-core"])), [])

    def test_distinguishes_sources_even_when_versions_match(self):
        fork = "git+https://example.com/windows-rs#abc123"
        self.packages.append(package("windows-core", "0.61.2", source=fork))
        self.packages[2]["dependencies"] = [f"windows-core 0.61.2 ({REGISTRY})"]
        errors = self.errors(package("wmi", "0.18.4", ["windows 0.61.3", f"windows-core 0.61.2 ({fork})"]))
        self.assertEqual(len(errors), 1)
        self.assertIn(fork, errors[0])
        self.assertIn(REGISTRY, errors[0])

    def test_rejects_ambiguous_lockfile_references(self):
        errors = self.errors(package("wmi", "0.18.4", ["windows", "windows-core 0.61.2"]))
        self.assertEqual(len(errors), 1)
        self.assertIn("cannot resolve windows uniquely", errors[0])


if __name__ == "__main__":
    unittest.main()
