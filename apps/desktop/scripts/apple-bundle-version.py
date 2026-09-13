import argparse
import json
import os
import plistlib
import re
from pathlib import Path


def merge_config(base, override):
    for key, value in override.items():
        if value is None:
            base.pop(key, None)
        elif isinstance(value, dict):
            base[key] = merge_config(base.get(key, {}) if isinstance(base.get(key), dict) else {}, value)
        else:
            base[key] = value
    return base


def bundle_versions(config_path, platform, container_info=None, config_override=None):
    config_path = Path(config_path)
    config = json.loads(config_path.read_text())
    platform_config = config_path.with_name(f"tauri.{platform.lower()}.conf.json")
    if platform_config.exists():
        merge_config(config, json.loads(platform_config.read_text()))
    if config_override:
        merge_config(config, json.loads(config_override))
    version = config.get("version")
    if isinstance(version, str) and (config_path.parent / version).is_file():
        version = json.loads((config_path.parent / version).read_text()).get("version")
    if version is None:
        cargo = (config_path.parent / "Cargo.toml").read_text()
        package = re.search(r"(?ms)^\[package\]\s*(.*?)(?=^\[|\Z)", cargo)
        declared = re.search(r'^version\s*=\s*"([^"]+)"', package[1], re.M) if package else None
        version = declared[1] if declared else None
    build = config.get("bundle", {}).get(platform, {}).get("bundleVersion", version)
    versions = {"CFBundleShortVersionString": version, "CFBundleVersion": build}
    # Tauri writes the actual iOS version, including --build-number, before invoking Xcode.
    if container_info:
        with Path(container_info).open("rb") as source:
            host = plistlib.load(source)
        for key in versions:
            value = host.get(key)
            if isinstance(value, str) and value and "$" not in value:
                versions[key] = value
    for key, value in versions.items():
        if not isinstance(value, str) or not value or "$" in value:
            raise ValueError(f"Cannot resolve {key} from {config_path}.")
    return versions


def write_extension_info(template, output, versions):
    with Path(template).open("rb") as source:
        info = plistlib.load(source)
    info.update(versions)
    destination = Path(output)
    destination.parent.mkdir(parents=True, exist_ok=True)
    data = plistlib.dumps(info, sort_keys=False)
    if not destination.exists() or destination.read_bytes() != data:
        destination.write_bytes(data)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", required=True)
    parser.add_argument("--platform", required=True, choices=["macOS", "iOS"])
    parser.add_argument("--container-info")
    parser.add_argument("--template")
    parser.add_argument("--output")
    args = parser.parse_args()
    versions = bundle_versions(args.config, args.platform, args.container_info, os.environ.get("TAURI_CONFIG"))
    if args.template and args.output:
        write_extension_info(args.template, args.output, versions)
    elif args.template or args.output:
        parser.error("--template and --output must be used together")
    else:
        print(json.dumps(versions))
