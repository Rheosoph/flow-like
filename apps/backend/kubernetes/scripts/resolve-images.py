#!/usr/bin/env python3
"""Pin the published Flow-Like images to digests for the Helm chart without Docker."""
import argparse
import base64
import json
import os
from pathlib import Path
import re
import urllib.error
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_REGISTRY = "ghcr.io/rheosoph"
TAG_PATTERN = re.compile(r"[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}")
DIGEST_PATTERN = re.compile(r"sha256:[a-f0-9]{64}")
MANIFEST_TYPES = ", ".join([
    "application/vnd.oci.image.index.v1+json",
    "application/vnd.docker.distribution.manifest.list.v2+json",
    "application/vnd.oci.image.manifest.v1+json",
    "application/vnd.docker.distribution.manifest.v2+json",
])
REPOSITORIES = {
    "api": "flow-like-kubernetes-api",
    "web": "flow-like-kubernetes-web",
    "executor": "flow-like-kubernetes-executor",
    "execution-manager": "flow-like-kubernetes-execution-manager",
    "migration": "flow-like-kubernetes-migration",
    "sink-trigger": "flow-like-kubernetes-sink-trigger",
    "runtime": "flow-like-docker-compose-runtime",
    "compiler": "flow-like-docker-compose-compiler",
    "signaling": "flow-like-docker-compose-signaling",
    "object-store-init": "flow-like-docker-compose-object-store-init",
    "audit-worker": "flow-like-audit-worker",
}
IMAGE_PATHS = {
    "audit-worker": [("audit", "image")],
    "api": [("api", "image")],
    "web": [("web", "image")],
    "executor": [("executor", "image"), ("executorPool", "image")],
    "execution-manager": [("executionManager", "image")],
    "migration": [("database", "migration", "image")],
    "sink-trigger": [("sinkServices", "image")],
    "runtime": [("executionManager", "queueBridge", "image")],
    "compiler": [("compiler", "image")],
    "signaling": [("signaling", "image")],
    "object-store-init": [("rustfs", "bootstrap", "image")],
}
NODE_SELECTOR_PATHS = [("audit",), ("api",), ("web",), ("compiler",), ("signaling",), ("executionManager",), ("executionManager", "sandbox"), ("monitoring", "prometheus"), ("monitoring", "grafana")]


def split_registry(registry):
    host, _, prefix = registry.strip("/").partition("/")
    if not host or "://" in registry:
        raise ValueError("--registry must be host[/namespace], for example ghcr.io/rheosoph")
    return host, prefix


def credentials():
    token = os.environ.get("GHCR_TOKEN", "")
    if not token:
        return None
    user = os.environ.get("GHCR_USER", "token")
    return "Basic " + base64.b64encode(f"{user}:{token}".encode()).decode()


def parse_challenge(header):
    scheme, _, rest = header.partition(" ")
    if scheme.lower() != "bearer":
        raise ValueError("registry requested unsupported authentication " + scheme)
    fields = dict(re.findall(r'(\w+)="([^"]*)"', rest))
    if "realm" not in fields:
        raise ValueError("registry authentication challenge lacks a realm")
    return fields


def head(url, headers, opener=urllib.request.urlopen):
    request = urllib.request.Request(url, headers=headers, method="HEAD")
    with opener(request) as response:
        return response.status, dict(response.headers)


def bearer_token(host, challenge, repository, opener=urllib.request.urlopen):
    query = {key: challenge[key] for key in ("service", "scope") if key in challenge}
    query.setdefault("scope", f"repository:{repository}:pull")
    url = challenge["realm"] + "?" + urllib.parse.urlencode(query)
    headers = {}
    auth = credentials()
    if auth and urllib.parse.urlsplit(challenge["realm"]).hostname == host:
        headers["Authorization"] = auth
    with opener(urllib.request.Request(url, headers=headers)) as response:
        body = json.loads(response.read().decode())
    token = body.get("token") or body.get("access_token")
    if not token:
        raise ValueError("registry token endpoint returned no token")
    return token


def resolve_digest(host, repository, tag, opener=urllib.request.urlopen):
    url = f"https://{host}/v2/{repository}/manifests/{tag}"
    headers = {"Accept": MANIFEST_TYPES}
    try:
        status, response_headers = head(url, headers, opener)
    except urllib.error.HTTPError as error:
        challenge = error.headers.get("WWW-Authenticate", "") if error.code == 401 else ""
        error.close()
        if not challenge:
            raise
        headers["Authorization"] = "Bearer " + bearer_token(host, parse_challenge(challenge), repository, opener)
        status, response_headers = head(url, headers, opener)
    digest = {key.lower(): value for key, value in response_headers.items()}.get("docker-content-digest", "")
    if status != 200 or not DIGEST_PATTERN.fullmatch(digest):
        raise ValueError(f"{host}/{repository}:{tag} returned no sha256 manifest digest")
    return digest


def resolve_all(registry, tag, opener=urllib.request.urlopen):
    host, prefix = split_registry(registry)
    resolved = {}
    for component, name in REPOSITORIES.items():
        path = f"{prefix}/{name}" if prefix else name
        try:
            digest = resolve_digest(host, path, tag, opener)
        except urllib.error.HTTPError as error:
            error.close()
            hint = " Private packages and forks need GHCR_TOKEN (a read:packages token) in the environment." if error.code in (401, 403, 404) else ""
            raise ValueError(f"{host}/{path}:{tag} returned HTTP {error.code}.{hint}") from None
        except urllib.error.URLError as error:
            raise ValueError(f"{host}/{path}:{tag} is unreachable: {error.reason}") from None
        resolved[component] = (f"{host}/{path}", digest)
    return resolved


def image_values(resolved, tag, existing=None, pull_secrets=(), arch=None):
    values = dict(existing or {})
    values.setdefault("global", {})["imageRegistry"] = ""
    for component, (repository, digest) in resolved.items():
        for path in IMAGE_PATHS[component]:
            target = values
            for key in path[:-1]:
                target = target.setdefault(key, {})
            target[path[-1]] = {"repository": repository, "tag": tag, "digest": digest, "pullPolicy": "IfNotPresent"}
    repository, digest = resolved["executor"]
    values.setdefault("executionManager", {}).setdefault("sandbox", {})["image"] = f"{repository}@{digest}"
    if pull_secrets:
        values["global"]["imagePullSecrets"] = [{"name": name} for name in pull_secrets]
    if arch:
        for path in NODE_SELECTOR_PATHS:
            target = values
            for key in path:
                target = target.setdefault(key, {})
            target.setdefault("nodeSelector", {})["kubernetes.io/arch"] = arch
    return values


def write_values(path, values):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        json.dump(values, handle, indent=2)
        handle.write("\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", default="dev", help="published tag: dev, main, alpha, beta, a version or an immutable sha-... tag")
    parser.add_argument("--registry", default=DEFAULT_REGISTRY, help="registry host and namespace holding the flow-like-* repositories")
    parser.add_argument("--output", type=Path, default=ROOT / ".generated" / "values-images.yaml")
    parser.add_argument("--pull-secret", action="append", default=[], metavar="NAME", help="docker-registry Secret name for global.imagePullSecrets (repeatable)")
    parser.add_argument("--arch", choices=["amd64", "arm64"], help="pin every workload with a nodeSelector to this node architecture")
    args = parser.parse_args()
    if not TAG_PATTERN.fullmatch(args.tag):
        raise ValueError("--tag must match [A-Za-z0-9_][A-Za-z0-9_.-]{0,127}")
    for name in args.pull_secret:
        if not re.fullmatch(r"[a-z0-9]([-a-z0-9.]*[a-z0-9])?", name) or len(name) > 253:
            raise ValueError(f"--pull-secret {name!r} is not a valid Secret name")
    resolved = resolve_all(args.registry, args.tag)
    existing = json.loads(args.output.read_text(encoding="utf-8")) if args.output.exists() else {}
    write_values(args.output, image_values(resolved, args.tag, existing, args.pull_secret, args.arch))
    for component, (repository, digest) in resolved.items():
        print(f"{component}: {repository}@{digest}")
    print(f"Wrote image values to {args.output}")


if __name__ == "__main__":
    try:
        main()
    except ValueError as error:
        raise SystemExit(str(error)) from None
