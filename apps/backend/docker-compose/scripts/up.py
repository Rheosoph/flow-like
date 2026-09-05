#!/usr/bin/env python3
"""Check deployment prerequisites, then start the selected Compose services."""
import argparse
import os
from pathlib import Path
import subprocess
from preflight import ROOT, read_env, run

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--env-file", type=Path, default=ROOT / ".env")
parser.add_argument("--build", action="store_true")
args = parser.parse_args()
args.env_file = args.env_file.absolute()
args.config_only = False
run(args)
env = os.environ.copy()
for key in read_env(args.env_file):
    env.pop(key, None)
command = ["docker", "compose", "--env-file", str(args.env_file), "up", "-d"]
if args.build:
    command.append("--build")
subprocess.run(command, cwd=ROOT, env=env, check=True)
