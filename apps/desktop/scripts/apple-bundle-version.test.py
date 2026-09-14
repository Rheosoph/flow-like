import importlib.util
import json
import plistlib
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("versions", Path(__file__).with_name("apple-bundle-version.py"))
versions = importlib.util.module_from_spec(spec)
spec.loader.exec_module(versions)


class AppleBundleVersionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.config = self.directory / "tauri.conf.json"
        self.config.write_text(json.dumps({"version": "1.2.3"}))

    def test_mac_uses_merged_config_and_explicit_build_number(self):
        (self.directory / "tauri.macos.conf.json").write_text(json.dumps({"version": "2.3.4"}))
        result = versions.bundle_versions(self.config, "macOS", config_override=json.dumps({
            "bundle": {"macOS": {"bundleVersion": "42"}},
        }))
        self.assertEqual(result, {"CFBundleShortVersionString": "2.3.4", "CFBundleVersion": "42"})

    def test_ios_copies_host_version_and_build_number_without_changing_template(self):
        host = self.directory / "Host.plist"
        host.write_bytes(plistlib.dumps({"CFBundleShortVersionString": "4.5.6", "CFBundleVersion": "4.5.6.123"}))
        template = self.directory / "Template.plist"
        original = plistlib.dumps({"CFBundleShortVersionString": "$(MARKETING_VERSION)",
                                  "CFBundleVersion": "$(CURRENT_PROJECT_VERSION)",
                                  "CFBundleIdentifier": "$(PRODUCT_BUNDLE_IDENTIFIER)"})
        template.write_bytes(original)
        output = self.directory / "Derived" / "Info.plist"
        versions.write_extension_info(template, output, versions.bundle_versions(self.config, "iOS", host))
        self.assertEqual(plistlib.loads(output.read_bytes()), {
            "CFBundleShortVersionString": "4.5.6", "CFBundleVersion": "4.5.6.123",
            "CFBundleIdentifier": "$(PRODUCT_BUNDLE_IDENTIFIER)",
        })
        self.assertEqual(template.read_bytes(), original)

    def test_unresolved_host_settings_fall_back_to_tauri(self):
        host = self.directory / "Host.plist"
        host.write_bytes(plistlib.dumps({"CFBundleShortVersionString": "$(MARKETING_VERSION)",
                                        "CFBundleVersion": "${CURRENT_PROJECT_VERSION}"}))
        self.assertEqual(versions.bundle_versions(self.config, "iOS", host), {
            "CFBundleShortVersionString": "1.2.3", "CFBundleVersion": "1.2.3",
        })


if __name__ == "__main__":
    unittest.main()
