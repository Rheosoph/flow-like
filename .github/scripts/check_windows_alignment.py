#!/usr/bin/env python3
"""Reject incompatible Windows COM types at shared dependency boundaries."""

from pathlib import Path
import sys
import tomllib


def identity(package: dict) -> tuple[str, str, str]:
    return package["name"], package["version"], package.get("source", "path")


def describe(package: dict) -> str:
    name, version, source = identity(package)
    return f"{name} {version} ({source})"


def dependency(package: dict, name: str, packages: list[dict]) -> dict | None:
    references = [
        reference.split(maxsplit=2)
        for reference in package.get("dependencies", [])
        if reference.split()[0] == name
    ]
    if not references:
        return None
    if len(references) != 1:
        raise ValueError(f"{describe(package)} has multiple direct {name} dependencies")
    reference = references[0]
    candidates = [
        candidate for candidate in packages
        if candidate["name"] == name
        and (len(reference) < 2 or candidate["version"] == reference[1])
        and (len(reference) < 3 or candidate.get("source", "path") == reference[2].removeprefix("(").removesuffix(")"))
    ]
    if len(candidates) != 1:
        raise ValueError(f"{describe(package)}: cannot resolve {' '.join(reference)} uniquely")
    return candidates[0]


def check_alignment(lockfile: Path) -> list[str]:
    packages = tomllib.loads(lockfile.read_text())["package"]
    errors = []
    for package in packages:
        try:
            windows = dependency(package, "windows", packages)
            if windows is None:
                continue
            core = dependency(package, "windows-core", packages)
            if core is not None:
                windows_core = dependency(windows, "windows-core", packages)
                if windows_core is None:
                    errors.append(f"{describe(windows)} has no windows-core dependency")
                elif identity(core) != identity(windows_core):
                    errors.append(
                        f"{describe(package)} uses {describe(core)}, but its "
                        f"{describe(windows)} uses {describe(windows_core)}"
                    )
            # The allocator's D3D12 API receives interfaces owned by wgpu-hal.
            if package["name"] == "wgpu-hal":
                allocator = dependency(package, "gpu-allocator", packages)
                if allocator is not None:
                    allocator_windows = dependency(allocator, "windows", packages)
                    if allocator_windows is not None and identity(windows) != identity(allocator_windows):
                        errors.append(
                            f"{describe(package)} uses {describe(windows)}, but its "
                            f"{describe(allocator)} uses {describe(allocator_windows)}"
                        )
        except ValueError as error:
            errors.append(str(error))
    return errors


def main() -> int:
    lockfile = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[2] / "Cargo.lock"
    errors = check_alignment(lockfile)
    if errors:
        print("Windows dependency alignment check failed:", file=sys.stderr)
        for error in errors:
            print(f"  {error}", file=sys.stderr)
        return 1
    print("Windows COM dependencies agree at the checked interface boundaries.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
