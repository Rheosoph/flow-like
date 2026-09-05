#!/usr/bin/env python3
"""Generate a new deployment configuration without displaying or replacing secrets."""
import argparse
import base64
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import time
from urllib.parse import quote, urlsplit

ROOT = Path(__file__).resolve().parents[1]


def encoded(value):
    return base64.urlsafe_b64encode(value).decode().rstrip("=")


def generate(template, mode, web_origin, api_url, s3_endpoint):
    values = {}
    for key in ["POSTGRES_PASSWORD", "REDIS_API_PASSWORD", "REDIS_RUNTIME_PASSWORD",
                "REDIS_SIGNALING_PASSWORD", "REDIS_SINK_PASSWORD", "REDIS_METRICS_PASSWORD",
                "RUSTFS_ROOT_PASSWORD", "AWS_SECRET_ACCESS_KEY", "STS_ISSUER_SECRET_KEY",
                "EXECUTION_MANAGER_TOKEN", "SINK_SECRET", "MAINTENANCE_TOKEN", "GRAFANA_ADMIN_PASSWORD"]:
        values[key] = secrets.token_hex(32)
    values["SINK_TOKEN_ENCRYPTION_KEY"] = base64.b64encode(secrets.token_bytes(32)).decode()
    for key in ["RUSTFS_ROOT_USER", "AWS_ACCESS_KEY_ID", "STS_ISSUER_ACCESS_KEY"]:
        values[key] = secrets.token_hex(10)
    private = subprocess.run(["openssl", "genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256"],
                             capture_output=True, check=True).stdout
    public = subprocess.run(["openssl", "pkey", "-pubout"], input=private, capture_output=True, check=True).stdout
    values["BACKEND_KEY"] = base64.b64encode(private).decode()
    values["BACKEND_PUB"] = base64.b64encode(public).decode()
    values["DATABASE_URL"] = f"postgresql://flowlike:{quote(values['POSTGRES_PASSWORD'], safe='')}@postgres:5432/flowlike"
    header = encoded(json.dumps({"alg": "HS256", "typ": "JWT"}, separators=(",", ":")).encode())
    payload = encoded(json.dumps({"sub": "sink-trigger", "iss": "flow-like", "sink_types": ["cron", "discord", "telegram"],
                                 "iat": int(time.time())}, separators=(",", ":")).encode())
    message = f"{header}.{payload}"
    signature = encoded(hmac.new(values["SINK_SECRET"].encode(), message.encode(), hashlib.sha256).digest())
    values["SINK_TRIGGER_JWT"] = f"{message}.{signature}"
    values.update({"NEXT_PUBLIC_API_URL": api_url, "PUBLIC_API_URL": api_url,
                   "NEXT_PUBLIC_REDIRECT_URL": web_origin.rstrip("/") + "/callback",
                   "NEXT_PUBLIC_REDIRECT_LOGOUT_URL": web_origin.rstrip("/") + "/",
                   "REALTIME_ALLOWED_ORIGINS": web_origin, "CORS_ALLOWED_ORIGINS": web_origin, "S3_PUBLIC_ENDPOINT": s3_endpoint,
                   "S3_GATEWAY_ALIAS": urlsplit(s3_endpoint).hostname if urlsplit(s3_endpoint).scheme == "http" and urlsplit(s3_endpoint).port == 9000 else "object-gateway",
                   "COMPILER_ALLOWED_STORAGE_HOSTS": s3_endpoint + ",http://object-store:9000"})
    if mode == "trusted":
        values.update({"EXECUTION_ISOLATION_MODE": "trusted_shared", "COMPOSE_PROFILES": "trusted",
                       "EXECUTOR_URL": "http://runtime-gateway:9000"})
    return re.sub(r"^([A-Z][A-Z0-9_]*)=.*$", lambda m: f"{m[1]}={values[m[1]]}" if m[1] in values else m[0], template, flags=re.M)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / ".env")
    parser.add_argument("--mode", choices=["per-run", "trusted"], default="per-run")
    parser.add_argument("--web-origin", default="http://localhost:3001")
    parser.add_argument("--api-url", default="http://localhost:8080")
    parser.add_argument("--s3-endpoint", default="http://s3.localhost:9000")
    args = parser.parse_args()
    for value in [args.web_origin, args.api_url, args.s3_endpoint]:
        url = urlsplit(value)
        if url.scheme not in {"http", "https"} or not url.hostname or url.username or url.password or url.path not in {"", "/"} or url.query or url.fragment or "\n" in value:
            parser.error("URLs must be HTTP(S) origins without credentials, query, or path")
    if args.output.exists() or args.output.is_symlink():
        parser.error("Output already exists; refusing to replace deployment secrets")
    data = generate((ROOT / ".env.example").read_text(), args.mode, args.web_origin.rstrip("/"), args.api_url.rstrip("/"), args.s3_endpoint.rstrip("/"))
    fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as target:
        target.write(data)
    print(f"Created {args.output} with mode 0600. Review URLs, then run scripts/preflight.py.")


if __name__ == "__main__":
    main()
