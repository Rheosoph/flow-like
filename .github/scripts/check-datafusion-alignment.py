#!/usr/bin/env python3
"""Reject incompatible Arrow or DataFusion runtimes in the workspace lockfile."""

from pathlib import Path
import sys
import tomllib
from urllib.parse import parse_qs, urlsplit

LANCE_VERSION = "13.0.0"


def is_patched_lance_source(source: str) -> bool:
    if source == "path":
        return True
    if not source.startswith("git+"):
        return False
    url = urlsplit(source.removeprefix("git+"))
    revision = parse_qs(url.query).get("rev", [""])[0]
    return (
        url.scheme == "https"
        and url.netloc.lower() == "github.com"
        and url.path.removesuffix(".git").lower() == "/rheosoph/lance"
        and len(revision) == 40
        and all(character in "0123456789abcdef" for character in revision.lower())
        and url.fragment == revision
    )


def check_alignment(lockfile: Path) -> list[str]:
    packages = tomllib.loads(lockfile.read_text())["package"]
    versions: dict[str, set[str]] = {}
    identities: dict[str, set[tuple[str, str]]] = {}
    for package in packages:
        versions.setdefault(package["name"], set()).add(package["version"])
        identities.setdefault(package["name"], set()).add(
            (package["version"], package.get("source", "path"))
        )

    errors = []
    # Cargo resolves optional providers in the lockfile, so this also covers the
    # providers that a particular CI build has not enabled.
    required = (
        "lancedb",
        "lance",
        "lance-graph",
        "lance-graph-catalog",
        "datafusion-table-providers",
        "datafusion-federation",
        "deltalake-core",
        "iceberg-datafusion",
        "datafusion",
        "datafusion-common",
        "datafusion-expr",
        "datafusion-physical-plan",
        "arrow",
        "arrow-array",
        "arrow-schema",
        "arrow-odbc",
        "odbc-api",
    )
    for name in required:
        resolved = identities.get(name, set())
        if len(resolved) != 1:
            errors.append(f"{name}: expected one runtime package, found {sorted(resolved)}")

    for family in (
        ("datafusion", "datafusion-common", "datafusion-expr", "datafusion-physical-plan"),
        ("arrow", "arrow-array", "arrow-schema"),
    ):
        resolved = set().union(*(versions.get(name, set()) for name in family))
        if len(resolved) != 1:
            errors.append(f"{', '.join(family)} must share a version; found {sorted(resolved)}")

    # Path and registry copies of the same Lance version have incompatible Rust
    # types. Every Lance workspace crate must come from the patched fork.
    lance_sources = {
        package.get("source", "path")
        for package in packages
        if package["name"] == "lance" and package["version"] == LANCE_VERSION
    }
    for package in packages:
        name = package["name"]
        if (
            (
                (name == "fsst" or name.startswith("lance-"))
                and package["version"] == LANCE_VERSION
            )
            or name in ("lance-arrow-scalar", "lance-arrow-stats")
        ) and package.get("source", "path") not in lance_sources:
            errors.append(f"{name} must share the patched Lance source")

    # Lance needs the fork's create-only fix.
    for package in packages:
        if (
            package["name"] == "lance"
            and package["version"] == LANCE_VERSION
            and not is_patched_lance_source(package.get("source", "path"))
        ):
            errors.append(
                f"Lance {LANCE_VERSION} requires a local checkout or a commit-pinned "
                "Rheosoph fork preserving create-only commits"
            )
    return errors


def main() -> int:
    lockfile = (
        Path(sys.argv[1])
        if len(sys.argv) > 1
        else Path(__file__).resolve().parents[2] / "Cargo.lock"
    )
    errors = check_alignment(lockfile)
    if errors:
        print("DataFusion provider compatibility check failed:", file=sys.stderr)
        for error in errors:
            print(f"  {error}", file=sys.stderr)
        return 1
    print("Lance, graph, SQL, federation, Delta, and Iceberg share one Arrow/DataFusion runtime.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
