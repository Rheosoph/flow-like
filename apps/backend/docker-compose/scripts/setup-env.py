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
AUDIT_KMS_SETTINGS = ("AUDIT_KMS_KEY_ID", "AUDIT_KMS_PROVIDER", "AUDIT_KMS_REGION", "AUDIT_KID",
                      "AUDIT_KMS_AWS_ACCESS_KEY_ID", "AUDIT_KMS_AWS_SECRET_ACCESS_KEY",
                      "AUDIT_VAULT_ADDR", "AUDIT_VAULT_TOKEN", "AUDIT_VAULT_TOKEN_FILE",
                      "AUDIT_VAULT_CA_FILE")
VAULT_PROVIDERS = {"vault", "openbao", "transit"}
STRIPE_SETTINGS = ("STRIPE_SECRET_KEY", "STRIPE_WEBHOOK_SECRET", "STRIPE_CONNECT_WEBHOOK_SECRET",
                   "STRIPE_CONNECT_WEBHOOK_SECRET_PREVIOUS", "STRIPE_MARKETPLACE_WEBHOOK_SECRET",
                   "STRIPE_MARKETPLACE_WEBHOOK_SECRET_PREVIOUS")


def encoded(value):
    return base64.urlsafe_b64encode(value).decode().rstrip("=")


def p256_private_key():
    """PKCS#8 PEM, the form both BACKEND_KEY and AUDIT_SIGNING_KEY expect."""
    return subprocess.run(["openssl", "genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256"],
                          capture_output=True, check=True).stdout


def unique_json_keys(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("Duplicate JSON key")
        result[key] = value
    return result


def audit_key_values(audit_kms):
    """A key in a key service replaces the generated AUDIT_SIGNING_KEY; the worker refuses both."""
    audit_kms = {key: value for key, value in (audit_kms or {}).items() if value}
    for key, value in audit_kms.items():
        if not re.fullmatch(r"[^\s'\"$\\]+", value):
            raise ValueError(f"{key} must be a single value without whitespace, quotes, '$' or '\\'")
    if not audit_kms.get("AUDIT_KMS_KEY_ID"):
        if set(audit_kms) - {"AUDIT_KID"}:
            raise ValueError("AUDIT_KMS_* settings require AUDIT_KMS_KEY_ID")
        # A separate key: the audit worker signs epochs with it, never tokens.
        private = p256_private_key()
        public = subprocess.run(["openssl", "pkey", "-pubout"], input=private, capture_output=True, check=True).stdout.decode()
        kid = audit_kms.get("AUDIT_KID") or "audit-es256-" + secrets.token_hex(8)
        return {**audit_kms, "AUDIT_SIGNING_KEY": base64.b64encode(private).decode(), "AUDIT_KID": kid,
                "AUDIT_VERIFYING_KEYS": "'" + json.dumps({kid: public}, separators=(",", ":")) + "'"}
    if audit_kms.get("AUDIT_VAULT_ADDR") or audit_kms.get("AUDIT_KMS_PROVIDER", "").lower() in VAULT_PROVIDERS:
        if not audit_kms.get("AUDIT_VAULT_ADDR", "").startswith(("http://", "https://")):
            raise ValueError("A Vault audit key needs AUDIT_VAULT_ADDR, for example https://vault:8200")
        if not (audit_kms.get("AUDIT_VAULT_TOKEN") or audit_kms.get("AUDIT_VAULT_TOKEN_FILE")):
            raise ValueError("A Vault audit key needs AUDIT_VAULT_TOKEN or AUDIT_VAULT_TOKEN_FILE")
        return audit_kms
    if bool(audit_kms.get("AUDIT_KMS_AWS_ACCESS_KEY_ID")) != bool(audit_kms.get("AUDIT_KMS_AWS_SECRET_ACCESS_KEY")):
        raise ValueError("Set both AUDIT_KMS_AWS_ACCESS_KEY_ID and AUDIT_KMS_AWS_SECRET_ACCESS_KEY or neither")
    return audit_kms


def generate(template, mode, web_origin, api_url, s3_endpoint, runtime_config=None, audit_kms=None, stripe=None):
    values = {}
    for key in STRIPE_SETTINGS:
        value = (stripe or {}).get(key, "")
        if value and not re.fullmatch(r"[A-Za-z0-9_]+", value):
            raise ValueError(f"{key} must contain only letters, digits and underscores")
        if value:
            values[key] = value
    runtime_config = dict(runtime_config or {})
    for key, value in runtime_config.items():
        if value and not value.strip():
            raise ValueError(f"{key} must not contain only whitespace")
        if key != "FLOW_LIKE_CONFIG_JSON" and value != value.strip():
            raise ValueError(f"{key} must not have surrounding whitespace")
    sources = [key for key in ("FLOW_LIKE_CONFIG_FILE", "FLOW_LIKE_CONFIG_JSON", "FLOW_LIKE_CONFIG_SECRET_REF") if runtime_config.get(key, "")]
    if len(sources) > 1:
        raise ValueError("Select only one nonempty API runtime config source")
    if runtime_config.get("FLOW_LIKE_CONFIG_JSON", ""):
        try:
            parsed = json.loads(runtime_config["FLOW_LIKE_CONFIG_JSON"], object_pairs_hook=unique_json_keys)
            if not isinstance(parsed, dict):
                raise ValueError()
        except (ValueError, TypeError):
            raise ValueError("FLOW_LIKE_CONFIG_JSON must contain a JSON object") from None
        runtime_config["FLOW_LIKE_CONFIG_JSON"] = json.dumps(parsed, separators=(",", ":"))
    if any(key in sources for key in ("FLOW_LIKE_CONFIG_JSON", "FLOW_LIKE_CONFIG_SECRET_REF")):
        runtime_config["FLOW_LIKE_CONFIG_FILE"] = ""
    # Single-quoted dotenv values preserve dollar signs and JSON quotes as data.
    values.update({key: "'" + value.replace("'", "\\'") + "'" if value else "" for key, value in runtime_config.items()})
    for key in ["POSTGRES_PASSWORD", "REDIS_API_PASSWORD", "REDIS_RUNTIME_PASSWORD",
                "REDIS_SIGNALING_PASSWORD", "REDIS_SINK_PASSWORD", "REDIS_METRICS_PASSWORD",
                "RUSTFS_ROOT_PASSWORD", "AWS_SECRET_ACCESS_KEY", "STS_ISSUER_SECRET_KEY",
                "AUDIT_BUCKET_SECRET_ACCESS_KEY", "EXECUTION_MANAGER_TOKEN", "SINK_SECRET", "MAINTENANCE_TOKEN", "GRAFANA_ADMIN_PASSWORD"]:
        values[key] = secrets.token_hex(32)
    values["SINK_TOKEN_ENCRYPTION_KEY"] = base64.b64encode(secrets.token_bytes(32)).decode()
    for key in ["RUSTFS_ROOT_USER", "AWS_ACCESS_KEY_ID", "STS_ISSUER_ACCESS_KEY", "AUDIT_BUCKET_ACCESS_KEY_ID"]:
        values[key] = secrets.token_hex(10)
    private = p256_private_key()
    public = subprocess.run(["openssl", "pkey", "-pubout"], input=private, capture_output=True, check=True).stdout
    values["BACKEND_KEY"] = base64.b64encode(private).decode()
    values["BACKEND_PUB"] = base64.b64encode(public).decode()
    values.update(audit_key_values(audit_kms))
    values["AUDIT_ENTRY_KEY"] = base64.b64encode(secrets.token_bytes(32)).decode()
    values["MIGRATION_DATABASE_URL"] = f"postgresql://flowlike:{quote(values['POSTGRES_PASSWORD'], safe='')}@postgres:5432/flowlike"
    values["DATABASE_URL"] = f"postgresql://flowlike_api:{secrets.token_hex(32)}@postgres:5432/flowlike"
    values["AUDIT_DATABASE_URL"] = f"postgresql://flowlike_audit:{secrets.token_hex(32)}@postgres:5432/flowlike"
    header = encoded(json.dumps({"alg": "HS256", "typ": "JWT"}, separators=(",", ":")).encode())
    payload = encoded(json.dumps({"sub": "sink-trigger", "iss": "flow-like", "sink_types": ["cron", "discord", "telegram"],
                                 "iat": int(time.time())}, separators=(",", ":")).encode())
    message = f"{header}.{payload}"
    signature = encoded(hmac.new(values["SINK_SECRET"].encode(), message.encode(), hashlib.sha256).digest())
    values["SINK_TRIGGER_JWT"] = f"{message}.{signature}"
    values.update({"NEXT_PUBLIC_API_URL": api_url, "PUBLIC_API_URL": api_url,
                   "FRONTEND_BASE_URL": web_origin.rstrip("/"),
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
    runtime_config = {key: os.environ[key] for key in ("FLOW_LIKE_RUNTIME_CONFIG_FILE", "FLOW_LIKE_CONFIG_FILE", "FLOW_LIKE_CONFIG_JSON", "FLOW_LIKE_CONFIG_SECRET_REF") if key in os.environ}
    audit_kms = {key: os.environ[key] for key in AUDIT_KMS_SETTINGS if key in os.environ}
    stripe = {key: os.environ[key] for key in STRIPE_SETTINGS if key in os.environ}
    try:
        data = generate((ROOT / ".env.example").read_text(), args.mode, args.web_origin.rstrip("/"), args.api_url.rstrip("/"), args.s3_endpoint.rstrip("/"), runtime_config, audit_kms, stripe)
    except ValueError as error:
        parser.error(str(error))
    fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as target:
        target.write(data)
    print(f"Created {args.output} with mode 0600. Review URLs, then run scripts/preflight.py.")


if __name__ == "__main__":
    main()
