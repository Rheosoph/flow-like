#!/usr/bin/env python3
"""Generate local Kubernetes Secrets and matching Helm values without cluster writes."""
import argparse
import base64
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import tempfile
import urllib.parse


def required(name):
    value = os.environ.get(name, "")
    if not value:
        raise ValueError(f"{name} is required")
    return value


def keypair():
    private = os.environ.get("BACKEND_KEY")
    public = os.environ.get("BACKEND_PUB")
    if bool(private) != bool(public):
        raise ValueError("BACKEND_KEY and BACKEND_PUB must be supplied together")
    if private:
        return private, public
    with tempfile.TemporaryDirectory(prefix="flow-like-k8s-keys-") as directory:
        path = Path(directory) / "private.pem"
        subprocess.run(["openssl", "genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256", "-out", str(path)], check=True, capture_output=True)
        public = subprocess.run(["openssl", "pkey", "-in", str(path), "-pubout"], check=True, capture_output=True).stdout
        return base64.b64encode(path.read_bytes()).decode(), base64.b64encode(public).decode()


def origin(value, name):
    url = urllib.parse.urlsplit(value)
    if url.scheme not in ("http", "https") or not url.hostname or url.path not in ("", "/") or url.username or url.query or url.fragment:
        raise ValueError(f"{name} must be an HTTP(S) origin")
    return value.rstrip("/")


def generate(namespace, release):
    for name, value in [("namespace", namespace), ("release", release)]:
        if len(value) > 40 or not re.fullmatch(r"[a-z0-9](?:[a-z0-9-]*[a-z0-9])?", value):
            raise ValueError(f"{name} must be a DNS label of at most 40 characters")
    objects = []

    def secret(suffix, data):
        name = f"{release}-{suffix}"
        objects.append({"apiVersion": "v1", "kind": "Secret", "metadata": {"name": name, "namespace": namespace, "labels": {"app.kubernetes.io/part-of": "flow-like"}}, "type": "Opaque", "stringData": data})
        return name

    private, public = keypair()
    jwt = secret("backend-jwt", {"BACKEND_KEY": private, "BACKEND_PUB": public, "BACKEND_KID": os.environ.get("BACKEND_KID", "backend-es256-v1")})
    system = secret("api-config", {key: os.environ.get(key) or secrets.token_hex(32) for key in ["SINK_TOKEN_ENCRYPTION_KEY", "SINK_SECRET", "MAINTENANCE_TOKEN"]})
    execution = secret("execution", {"EXECUTION_MANAGER_TOKEN": os.environ.get("EXECUTION_MANAGER_TOKEN") or secrets.token_hex(32)})
    web = origin(os.environ.get("PUBLIC_WEB_URL", "http://localhost:3001"), "PUBLIC_WEB_URL")
    api = origin(os.environ.get("PUBLIC_API_URL", "http://localhost:8080"), "PUBLIC_API_URL")
    bundled = os.environ.get("RUSTFS_ENABLED", "true").lower() == "true"
    public_s3 = origin(os.environ.get("S3_PUBLIC_ENDPOINT", f"http://{release}-object-gateway.{namespace}.svc.cluster.local:9000") if bundled else required("S3_PUBLIC_ENDPOINT"), "S3_PUBLIC_ENDPOINT")
    values = {"fullnameOverride": release, "jwt": {"existingSecret": jwt}, "api": {"existingSecret": system, "publicUrl": api, "corsAllowedOrigins": [web, "tauri://localhost", "http://tauri.localhost", "https://tauri.localhost"]}, "execution": {"existingSecret": execution}, "storage": {"provider": "s3", "s3": {"publicEndpoint": public_s3}}}
    values["rustfs"] = {"enabled": bundled}
    storage = {}
    for key in ["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "STS_ISSUER_ACCESS_KEY", "STS_ISSUER_SECRET_KEY"]:
        storage[key] = (os.environ.get(key) or secrets.token_hex(16 if key.endswith("ACCESS_KEY") or key.endswith("KEY_ID") else 32)) if bundled else required(key)
    values["storage"]["s3"]["existingSecret"] = secret("storage", storage)
    if bundled:
        values["rustfs"]["existingSecret"] = secret("rustfs-root", {"RUSTFS_ROOT_USER": os.environ.get("RUSTFS_ROOT_USER") or secrets.token_hex(16), "RUSTFS_ROOT_PASSWORD": os.environ.get("RUSTFS_ROOT_PASSWORD") or secrets.token_hex(32)})
    else:
        values["storage"]["s3"].update({"internalEndpoint": origin(required("S3_INTERNAL_ENDPOINT"), "S3_INTERNAL_ENDPOINT"), "stsEndpoint": origin(required("STS_ENDPOINT_URL"), "STS_ENDPOINT_URL"), "runtimeCredentialsProvider": os.environ.get("S3_STS_PROVIDER", "rustfs")})
    if os.environ.get("DATABASE_URL"):
        values["database"] = {"type": "external", "external": {"existingSecret": secret("database", {"DATABASE_URL": required("DATABASE_URL")}), "provider": os.environ.get("DATABASE_PROVIDER", "postgresql")}}
    password = os.environ.get("REDIS_PASSWORD") or secrets.token_hex(32)
    redis_url = os.environ.get("REDIS_URL")
    if redis_url:
        parsed = urllib.parse.urlsplit(redis_url)
        if parsed.scheme not in ("redis", "rediss") or not parsed.hostname or not parsed.password:
            raise ValueError("REDIS_URL must be authenticated redis:// or rediss://")
        values["redis"] = {"enabled": False, "externalExistingSecret": secret("redis", {"REDIS_URL": redis_url})}
    else:
        redis_url = f"redis://:{urllib.parse.quote(password, safe='')}@{release}-redis-master:6379"
        values["redis"] = {"auth": {"existingSecret": secret("redis", {"REDIS_PASSWORD": password, "REDIS_URL": redis_url})}}
    if os.environ.get("OPENROUTER_API_KEY"):
        values["llm"] = {"openrouter": {"existingSecret": secret("openrouter", {"OPENROUTER_API_KEY": required("OPENROUTER_API_KEY"), "OPENROUTER_ENDPOINT": os.environ.get("OPENROUTER_ENDPOINT", "https://openrouter.ai/api")})}}
    return {"apiVersion": "v1", "kind": "List", "items": objects}, values


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--namespace", default=os.environ.get("K8S_NAMESPACE", "flow-like"))
    parser.add_argument("--release", default=os.environ.get("RELEASE", "flow-like"))
    parser.add_argument("--output-dir", type=Path, default=Path(__file__).resolve().parents[1] / ".generated")
    args = parser.parse_args()
    os.umask(0o077)
    paths = [args.output_dir / "secrets.yaml", args.output_dir / "values-generated.yaml"]
    if any(path.exists() for path in paths):
        raise ValueError("Generated files already exist. Reuse them; use a new output directory for a deliberate credential rotation.")
    objects, values = generate(args.namespace, args.release)
    args.output_dir.mkdir(parents=True, mode=0o700, exist_ok=True)
    for path, content in zip(paths, [objects, values]):
        with path.open("x", encoding="utf-8") as handle:
            json.dump(content, handle, indent=2)
            handle.write("\n")
    print(f"Wrote private configuration to {args.output_dir}. No cluster resources were changed.")
    print("Review values-generated.yaml, apply secrets.yaml, then deploy with that values file.")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, subprocess.CalledProcessError) as error:
        raise SystemExit(str(error) if isinstance(error, ValueError) else "OpenSSL key generation failed") from None
