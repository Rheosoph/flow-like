#!/usr/bin/env python3
"""Read current app metadata from S3 into a private local export directory."""

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess


CURRENT_SOURCE = re.compile(
    r"(?:manifest\.app|[A-Za-z0-9_-]+\.(?:board|widget)|"
    r"events/[A-Za-z0-9_-]+\.event|_[A-Za-z0-9_-]+/[A-Za-z0-9_-]+\.page)\Z"
)


def relative_key(key, prefix):
    if not key.startswith(prefix):
        raise ValueError("S3 listing returned an object outside the requested app prefix")
    relative = key[len(prefix):]
    if not relative or relative.startswith("/") or "\\" in relative:
        raise ValueError("S3 listing returned an invalid relative object key")
    if any(part in ("", ".", "..") for part in relative.split("/")):
        raise ValueError("S3 object key contains an unsafe path component")
    if PurePosixPath(relative).is_absolute():
        raise ValueError("S3 object key must be relative")
    return relative


def aws_read(operation, args, *, region):
    result = subprocess.run(["aws", "s3api", operation, "--region", region, "--output", "json", *args],
                            capture_output=True, text=True)
    if result.returncode:
        # Authentication details and any AWS CLI diagnostics stay out of fixture reports.
        raise RuntimeError(f"AWS read {operation} failed with exit status {result.returncode}")
    return json.loads(result.stdout)


def export(bucket, app_id, region, output, workers):
    prefix = f"apps/{app_id}/"
    listing = aws_read("list-objects-v2", ["--bucket", bucket, "--prefix", prefix], region=region)
    selected = []
    for item in listing.get("Contents", []):
        relative = relative_key(item["Key"], prefix)
        if CURRENT_SOURCE.fullmatch(relative):
            if item["Size"] > 128 * 1024 * 1024:
                raise ValueError("A source metadata object exceeds the 128 MiB limit")
            selected.append((item, relative))
    if not any(relative == "manifest.app" for _, relative in selected):
        raise ValueError("No current app manifest was found")
    if len(selected) > 500:
        raise ValueError("This benchmark export is limited to 500 current source objects")
    output.mkdir(parents=True, exist_ok=True, mode=0o700)
    output.chmod(0o700)

    def download(selected_object):
        item, relative = selected_object
        target = output / relative
        if output not in target.resolve().parents:
            raise ValueError("Export path escapes the output directory")
        target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        temporary = target.with_name(target.name + ".partial")
        if temporary.is_symlink():
            raise ValueError("Export temporary path must not be a symbolic link")
        try:
            aws_read("get-object", ["--bucket", bucket, "--key", item["Key"],
                                    "--if-match", item["ETag"], str(temporary)], region=region)
            temporary.chmod(0o600)
            content = temporary.read_bytes()
            if len(content) != item["Size"]:
                raise ValueError(f"Object size changed during export: {relative}")
            temporary.replace(target)
            return {"path": relative, "size": len(content), "sha256": hashlib.sha256(content).hexdigest()}
        finally:
            temporary.unlink(missing_ok=True)

    with ThreadPoolExecutor(max_workers=workers) as pool:
        objects = sorted(pool.map(download, selected), key=lambda item: item["path"])
    inventory = {"app_id": app_id, "bucket": bucket, "region": region, "prefix": prefix,
                 "selection": "Current manifest, boards, widgets, events and pages only",
                 "customer_rows_downloaded": False, "objects": objects}
    path = output / "export-inventory.json"
    path.write_text(json.dumps(inventory, indent=2) + "\n")
    path.chmod(0o600)
    return path, len(objects)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bucket", required=True)
    parser.add_argument("--app-id", required=True)
    parser.add_argument("--region", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--workers", type=int, default=4)
    args = parser.parse_args(argv)
    if not re.fullmatch(r"[A-Za-z0-9_-]+", args.app_id):
        parser.error("App ID must contain only letters, digits, hyphens or underscores")
    if not 1 <= args.workers <= 16:
        parser.error("Use between 1 and 16 download workers")
    os.umask(0o077)
    path, count = export(args.bucket, args.app_id, args.region, args.output.resolve(), args.workers)
    print(json.dumps({"inventory": str(path), "current_source_objects": count}))


if __name__ == "__main__":
    main()
