#!/usr/bin/env python3
"""Build and pin the local runner/helper images without changing deployment secrets."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
from preflight import ROOT, read_env

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--env-file", type=Path, default=ROOT / ".env")
args = parser.parse_args()
path = args.env_file.absolute()
if path.is_symlink() or path.stat().st_mode & 0o077:
    parser.error("Use a private regular deployment env file (chmod 600)")
values = read_env(path)
env = os.environ.copy()
for key in values:
    env.pop(key, None)
compose = ["docker", "compose", "--env-file", str(path)]
subprocess.run(compose + ["build", "runtime", "execution-manager"], cwd=ROOT, env=env, check=True)
pins = {}
for key, image_key in [("SANDBOX_IMAGE", "RUNTIME_IMAGE"), ("SANDBOX_GATEWAY_IMAGE", "EXECUTION_MANAGER_IMAGE")]:
    result = subprocess.run(["docker", "image", "inspect", "--format", "{{json .Id}}", values[image_key]], capture_output=True, text=True, check=True)
    digest = json.loads(result.stdout)
    if not re.fullmatch(r"sha256:[a-f0-9]{64}", digest):
        raise RuntimeError("Docker did not return an immutable image ID")
    pins[key] = digest
text = re.sub(r"^(SANDBOX_IMAGE|SANDBOX_GATEWAY_IMAGE)=.*$", lambda m: f"{m[1]}={pins[m[1]]}", path.read_text(), flags=re.M)
fd, temporary = tempfile.mkstemp(prefix=".env-pin-", dir=path.parent)
try:
    with os.fdopen(fd, "w") as target:
        target.write(text)
        target.flush()
        os.fsync(target.fileno())
    os.replace(temporary, path)
finally:
    if os.path.exists(temporary):
        os.unlink(temporary)
print("Pinned the locally built runner and gateway images. Deployment secrets were preserved.")
