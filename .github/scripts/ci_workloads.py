#!/usr/bin/env python3
"""Route shared CI checks and require every selected workload to succeed."""

import argparse
import json
import os
from pathlib import Path


WORKLOADS = ("tests", "lint", "boundaries", "browser", "containers", "compiler", "alpha")
CHANGE_FLAGS = (
    "rust", "bun", "ui", "dsql", "browser", "azure_migrations", "containers", "compiler",
)
CLOUDS = ("all", "aws", "gcp", "azure", "docker-compose", "kubernetes", "self-hosted")


def boolean(value, name):
    """Accept Python flags and the string flags GitHub puts in action outputs."""
    if type(value) is bool:
        return value
    if value == "true":
        return True
    if value == "false":
        return False
    raise ValueError(f"{name} must be true or false")


def release_ref(ref):
    return ref in ("refs/heads/dev", "refs/heads/main", "refs/heads/alpha") or ref.startswith(
        ("refs/tags/v", "refs/tags/beta-v")
    )


def plan(event_name, ref, base_ref, inputs, changed):
    """Select jobs without letting a manual checks run start publication."""
    if not isinstance(inputs, dict) or not isinstance(changed, dict):
        raise ValueError("CI inputs and change flags must be objects")
    missing = set(CHANGE_FLAGS) - changed.keys()
    if missing:
        raise ValueError(f"Missing change flags: {', '.join(sorted(missing))}")
    flags = {key: boolean(changed[key], f"changed.{key}") for key in CHANGE_FLAGS}
    core = containers = alpha = False
    if event_name == "pull_request":
        core = base_ref == "dev"
        containers = flags["containers"]
    elif event_name == "push":
        core = ref == "refs/heads/dev"
        alpha = ref == "refs/heads/alpha"
        containers = release_ref(ref) and (not core or flags["containers"])
    elif event_name == "workflow_dispatch":
        scope = inputs.get("scope") or "checks"
        if scope == "checks":
            core = True
        elif scope == "containers":
            if not release_ref(ref):
                raise ValueError("Container publication requires dev, main, alpha, a v* tag, or a beta-v* tag")
            cloud = inputs.get("cloud") or "all"
            if cloud not in CLOUDS:
                raise ValueError(f"Unknown container cloud: {cloud}")
            containers = True
        else:
            raise ValueError(f"Unknown CI scope: {scope}")
    else:
        raise ValueError(f"Unsupported CI event: {event_name}")

    return {
        "tests": core and any(flags[key] for key in ("rust", "bun", "ui", "azure_migrations")),
        "lint": core and (flags["rust"] or flags["dsql"]),
        "boundaries": core and flags["rust"],
        "browser": core and flags["browser"],
        "containers": containers,
        "compiler": containers or alpha or (core and flags["compiler"]),
        "alpha": alpha,
    }


def check_results(needs):
    """Reject missing, failed, cancelled, or unexpectedly skipped dependencies."""
    if not isinstance(needs, dict):
        return ["CI job results must be an object"]
    errors = []
    expected_jobs = {"changes", *WORKLOADS}
    for job in sorted(expected_jobs - needs.keys()):
        errors.append(f"Missing CI job result: {job}")
    for job in sorted(needs.keys() - expected_jobs):
        errors.append(f"Unplanned CI job result: {job}")
    changes = needs.get("changes")
    if not isinstance(changes, dict) or changes.get("result") != "success":
        errors.append("Change detection and workload planning did not succeed")
        return errors
    outputs = changes.get("outputs")
    if not isinstance(outputs, dict):
        errors.append("Change detection did not provide workload plans")
        return errors
    for job in WORKLOADS:
        try:
            selected = boolean(outputs.get(job), f"changes.outputs.{job}")
        except ValueError as error:
            errors.append(str(error))
            continue
        expected = "success" if selected else "skipped"
        result = needs.get(job)
        actual = result.get("result") if isinstance(result, dict) else None
        if actual != expected:
            errors.append(f"{job}: expected {expected}, got {actual or 'no result'}")
    return errors


def annotation(message):
    escaped = str(message).replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    print(f"::error::{escaped}")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", nargs="?", choices=("plan", "gate"), default="plan")
    args = parser.parse_args(argv)
    try:
        if args.command == "gate":
            errors = check_results(json.loads(os.environ["NEEDS_JSON"]))
            for error in errors:
                annotation(error)
            if not errors:
                print("Every selected CI workload succeeded.")
            return int(bool(errors))
        selected = plan(
            os.environ["GITHUB_EVENT_NAME"],
            os.environ["GITHUB_REF"],
            os.environ.get("GITHUB_BASE_REF", ""),
            {"scope": os.environ.get("INPUT_SCOPE", ""), "cloud": os.environ.get("INPUT_CLOUD", "")},
            json.loads(os.environ["CHANGES_JSON"]),
        )
        output = "".join(f"{key}={str(value).lower()}\n" for key, value in selected.items())
        with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as handle:
            handle.write(output)
        print(output, end="")
        return 0
    except (KeyError, OSError, TypeError, ValueError) as error:
        annotation(f"CI {args.command} failed: {error}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
