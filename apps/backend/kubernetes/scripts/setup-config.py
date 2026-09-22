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

BASE = Path(__file__).resolve().parents[1]
STRIPE_SETTINGS = ("STRIPE_SECRET_KEY", "STRIPE_WEBHOOK_SECRET", "STRIPE_CONNECT_WEBHOOK_SECRET",
                   "STRIPE_CONNECT_WEBHOOK_SECRET_PREVIOUS", "STRIPE_MARKETPLACE_WEBHOOK_SECRET",
                   "STRIPE_MARKETPLACE_WEBHOOK_SECRET_PREVIOUS")


def unique_json_keys(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("Duplicate JSON key")
        result[key] = value
    return result


def runtime_config():
    """Read deployment config without logging its contents or provider details."""
    selected = {key: os.environ.get(key, "") for key in ("FLOW_LIKE_CONFIG_JSON", "FLOW_LIKE_CONFIG_FILE", "FLOW_LIKE_CONFIG_SECRET_REF")}
    inputs = {**selected, **{key: os.environ.get(key, "") for key in ("FLOW_LIKE_RUNTIME_CONFIG_FILE", "FLOW_LIKE_CONFIG")}}
    for key, value in inputs.items():
        if value and not value.strip():
            raise ValueError(f"{key} must not contain only whitespace")
        if key != "FLOW_LIKE_CONFIG_JSON" and value != value.strip():
            raise ValueError(f"{key} must not have surrounding whitespace")
    sources = [key for key, value in selected.items() if value]
    if len(sources) > 1:
        raise ValueError("Select only one nonempty API runtime config source")
    if sources == ["FLOW_LIKE_CONFIG_SECRET_REF"]:
        return None, selected[sources[0]]
    if sources == ["FLOW_LIKE_CONFIG_JSON"]:
        text = selected[sources[0]]
    else:
        if sources:
            path = Path(selected["FLOW_LIKE_CONFIG_FILE"])
        elif os.environ.get("FLOW_LIKE_RUNTIME_CONFIG_FILE"):
            path = Path(os.environ["FLOW_LIKE_RUNTIME_CONFIG_FILE"])
        elif os.environ.get("FLOW_LIKE_CONFIG"):
            # Compatibility with the previous repository-relative setup input.
            path = BASE.parents[2] / os.environ["FLOW_LIKE_CONFIG"]
        else:
            path = BASE / "flow-like.config.example.json"
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeError):
            raise ValueError("API runtime config file cannot be read as UTF-8") from None
    try:
        parsed = json.loads(text, object_pairs_hook=unique_json_keys)
        if not isinstance(parsed, dict):
            raise ValueError()
    except (ValueError, TypeError):
        raise ValueError("API runtime config must contain a JSON object") from None
    return json.dumps(parsed, separators=(",", ":")), None


def required(name):
    value = os.environ.get(name, "")
    if not value:
        raise ValueError(f"{name} is required")
    return value


def p256_private_key():
    """PKCS#8 PEM, the form both BACKEND_KEY and AUDIT_SIGNING_KEY expect."""
    return subprocess.run(["openssl", "genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256"], check=True, capture_output=True).stdout


def keypair():
    private = os.environ.get("BACKEND_KEY")
    public = os.environ.get("BACKEND_PUB")
    if bool(private) != bool(public):
        raise ValueError("BACKEND_KEY and BACKEND_PUB must be supplied together")
    if private:
        return private, public
    private = p256_private_key()
    public = subprocess.run(["openssl", "pkey", "-pubout"], input=private, check=True, capture_output=True).stdout
    return base64.b64encode(private).decode(), base64.b64encode(public).decode()


def audit_kms_provider(key_id, explicit, vault_address):
    """Mirrors audit/kms.rs: the explicit provider, else the shape of the key id."""
    if explicit:
        provider = {"aws": "aws", "gcp": "gcp", "google": "gcp", "azure": "azure",
                    "vault": "vault", "openbao": "vault", "transit": "vault"}.get(explicit.lower())
        if not provider:
            raise ValueError("AUDIT_KMS_PROVIDER must be aws, gcp, azure or vault")
        return provider
    if key_id.startswith("projects/"):
        return "gcp"
    if key_id.startswith("https://"):
        return "azure"
    return "vault" if vault_address else "aws"


def audit_key(bundled):
    """Exactly one audit key: AUDIT_KMS_KEY_ID when given, else a dedicated AUDIT_SIGNING_KEY.
    Returns the Helm values and the secret entries."""
    key_id = os.environ.get("AUDIT_KMS_KEY_ID", "")
    if not key_id:
        return {}, {"AUDIT_SIGNING_KEY": os.environ.get("AUDIT_SIGNING_KEY") or base64.b64encode(p256_private_key()).decode()}
    if os.environ.get("AUDIT_SIGNING_KEY"):
        raise ValueError("Set AUDIT_SIGNING_KEY or AUDIT_KMS_KEY_ID, not both; the audit worker refuses to start with both")
    address = os.environ.get("AUDIT_VAULT_ADDR", "")
    provider = audit_kms_provider(key_id, os.environ.get("AUDIT_KMS_PROVIDER", ""), address)
    values = {"kmsKeyId": key_id}
    for key, name in [("AUDIT_KMS_PROVIDER", "kmsProvider"), ("AUDIT_KMS_REGION", "kmsRegion")]:
        if os.environ.get(key):
            values[name] = os.environ[key]
    if provider == "vault":
        return audit_vault_key(values, address)
    credentials = {key: os.environ.get(key, "") for key in ["AUDIT_KMS_AWS_ACCESS_KEY_ID", "AUDIT_KMS_AWS_SECRET_ACCESS_KEY"]}
    if any(credentials.values()) and not all(credentials.values()):
        raise ValueError("Set both AUDIT_KMS_AWS_ACCESS_KEY_ID and AUDIT_KMS_AWS_SECRET_ACCESS_KEY or neither")
    if provider == "aws" and bundled and not all(credentials.values()):
        raise ValueError("AWS KMS with bundled RustFS needs AUDIT_KMS_AWS_ACCESS_KEY_ID and AUDIT_KMS_AWS_SECRET_ACCESS_KEY; the default AWS credentials are the object store's")
    return values, {key: value for key, value in credentials.items() if value}


def audit_vault_key(values, address):
    """Vault or OpenBao transit: the address is a value, the token a secret entry, unless
    a Vault Agent sidecar renews it in a file the pod mounts."""
    if not address.startswith(("http://", "https://")):
        raise ValueError("A Vault audit key needs AUDIT_VAULT_ADDR, for example https://vault:8200")
    values["vaultAddress"] = origin(address, "AUDIT_VAULT_ADDR")
    token_file = os.environ.get("AUDIT_VAULT_TOKEN_FILE", "")
    if token_file:
        values["vaultTokenFile"] = token_file
        return values, {}
    token = os.environ.get("AUDIT_VAULT_TOKEN", "")
    if not token:
        raise ValueError("A Vault audit key needs AUDIT_VAULT_TOKEN or AUDIT_VAULT_TOKEN_FILE")
    return values, {"AUDIT_VAULT_TOKEN": token}


def audit_values(bundled, secret):
    """Audit bucket with a write-only identity, and the audit key. Bundled RustFS gets a
    bucket by default; external storage only when AUDIT_BUCKET is set."""
    bucket = os.environ.get("AUDIT_BUCKET", "flow-like-audit" if bundled else "")
    values, data = audit_key(bundled)
    if os.environ.get("AUDIT_KID"):
        values["kid"] = os.environ["AUDIT_KID"]
    if os.environ.get("AUDIT_VERIFYING_KEYS"):
        values["verifyingKeys"] = os.environ["AUDIT_VERIFYING_KEYS"]
    if "AUDIT_SIGNING_KEY" in data:
        values.setdefault("kid", "audit-es256-" + secrets.token_hex(12))
        try:
            pem = base64.b64decode(data["AUDIT_SIGNING_KEY"], validate=True)
            public = subprocess.run(["openssl", "pkey", "-pubout"], input=pem, check=True, capture_output=True).stdout.decode()
        except (ValueError, subprocess.CalledProcessError):
            raise ValueError("AUDIT_SIGNING_KEY must contain a base64 PKCS#8 PEM key") from None
        public_keys = json.loads(values.get("verifyingKeys", "{}"))
        if values["kid"] in public_keys and public_keys[values["kid"]] != public:
            raise ValueError("AUDIT_KID conflicts with AUDIT_VERIFYING_KEYS")
        public_keys[values["kid"]] = public
        values["verifyingKeys"] = json.dumps(public_keys)
    bucket_data = {}
    if bucket:
        values["bucket"] = bucket
        for key in ["AUDIT_BUCKET_ACCESS_KEY_ID", "AUDIT_BUCKET_SECRET_ACCESS_KEY"]:
            value = os.environ.get(key) or (secrets.token_hex(16 if key.endswith("KEY_ID") else 32) if bundled else "")
            if value:
                bucket_data[key] = value
        if not bundled and os.environ.get("AUDIT_BUCKET_ENDPOINT"):
            values["endpoint"] = origin(os.environ["AUDIT_BUCKET_ENDPOINT"], "AUDIT_BUCKET_ENDPOINT")
    if data:
        values["existingSecret"] = secret("audit", data)
    if bucket_data:
        if len(bucket_data) != 2:
            raise ValueError("Set both AUDIT_BUCKET_ACCESS_KEY_ID and AUDIT_BUCKET_SECRET_ACCESS_KEY")
        values["bucketSecret"] = secret("audit-bucket", bucket_data)
    values.update({"lockMode": "COMPLIANCE", "retentionYears": 4})
    if os.environ.get("AUDIT_BUCKET_KMS_KEY_ARN"):
        values["kmsKeyArn"] = os.environ["AUDIT_BUCKET_KMS_KEY_ARN"]
    entry = os.environ.get("AUDIT_ENTRY_KEY") or base64.b64encode(secrets.token_bytes(32)).decode()
    try:
        if len(base64.b64decode(entry, validate=True)) != 32:
            raise ValueError()
    except ValueError:
        raise ValueError("AUDIT_ENTRY_KEY must be base64 of 32 random bytes") from None
    entry_data = {"AUDIT_ENTRY_KEY": entry}
    if os.environ.get("AUDIT_ENTRY_KEY_PREVIOUS"):
        entry_data["AUDIT_ENTRY_KEY_PREVIOUS"] = os.environ["AUDIT_ENTRY_KEY_PREVIOUS"]
    values["entrySecret"] = secret("audit-entry", entry_data)
    return values


def database_certificates(namespace, release):
    """Create a private CA, a server certificate and an init-only root client."""
    with tempfile.TemporaryDirectory(prefix="flow-like-db-certs-") as directory:
        root = Path(directory)
        def openssl(*args):
            subprocess.run(["openssl", *args], cwd=root, check=True, capture_output=True)
        openssl("genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out", "ca.key")
        openssl("req", "-x509", "-new", "-key", "ca.key", "-days", "3650", "-sha256", "-subj", "/CN=Flow-Like database CA", "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign,cRLSign", "-out", "ca.crt")
        service = f"{release}-cockroachdb-public"
        peer = f"{release}-cockroachdb"
        hosts = ["localhost", service, f"{service}.{namespace}", f"{service}.{namespace}.svc", f"{service}.{namespace}.svc.cluster.local", f"{peer}-0.{peer}.{namespace}.svc.cluster.local"]
        for name, common_name, extensions in [
            ("node", "node", "extendedKeyUsage=serverAuth,clientAuth\nsubjectAltName=" + ",".join([*("DNS:" + host for host in hosts), "IP:127.0.0.1"]) + "\n"),
            ("client.root", "root", "extendedKeyUsage=clientAuth\n"),
        ]:
            openssl("genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out", name + ".key")
            openssl("req", "-new", "-key", name + ".key", "-subj", "/CN=" + common_name, "-out", name + ".csr")
            (root / (name + ".ext")).write_text("basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\n" + extensions)
            openssl("x509", "-req", "-in", name + ".csr", "-CA", "ca.crt", "-CAkey", "ca.key", "-CAcreateserial", "-days", "365", "-sha256", "-extfile", name + ".ext", "-out", name + ".crt")
        return {name: (root / name).read_text() for name in ["ca.crt", "ca.key", "node.crt", "node.key", "client.root.crt", "client.root.key"]}


def database_values(namespace, release, secret):
    """Keep schema ownership and audit mutations out of the API database identity."""
    if os.environ.get("DATABASE_URL"):
        urls = {"migration": required("DATABASE_URL"), "api": required("API_DATABASE_URL"), "audit": required("AUDIT_DATABASE_URL")}
        users = [urllib.parse.urlsplit(url).username for url in urls.values()]
        if len(set(users)) != 3 or not all(users):
            raise ValueError("DATABASE_URL, API_DATABASE_URL and AUDIT_DATABASE_URL must use three distinct database roles")
        for url in urls.values():
            if urllib.parse.parse_qs(urllib.parse.urlsplit(url).query).get("sslmode") != ["verify-full"]:
                raise ValueError("All database URLs require sslmode=verify-full")
        values = {"type": "external", "external": {"existingSecret": secret("database", {"DATABASE_URL": urls["api"]}), "provider": os.environ.get("DATABASE_PROVIDER", "postgresql")}, "migration": {"existingSecret": secret("database-migration", {"DATABASE_URL": urls["migration"]})}}
        if os.environ.get("DATABASE_CA_FILE"):
            try:
                certificate = Path(os.environ["DATABASE_CA_FILE"]).read_text()
            except (OSError, UnicodeError):
                raise ValueError("DATABASE_CA_FILE cannot be read as UTF-8 PEM") from None
            values["caSecret"] = secret("database-ca", {"ca.crt": certificate})
        return values, secret("database-audit", {"DATABASE_URL": urls["audit"]}), urllib.parse.unquote(users[1])
    certs = database_certificates(namespace, release)
    ca = secret("database-ca", {"ca.crt": certs["ca.crt"]})
    # The signing CA is retained for operator-led renewal and never mounted into a pod.
    secret("database-ca-admin", {"ca.crt": certs["ca.crt"], "ca.key": certs["ca.key"]})
    node = secret("database-node", {key: certs[key] for key in ["ca.crt", "node.crt", "node.key"]})
    owner_password = secrets.token_hex(32)
    init = secret("database-init", {**{key: certs[key] for key in ["ca.crt", "client.root.crt", "client.root.key"]}, "MIGRATION_PASSWORD": owner_password})
    host = f"{release}-cockroachdb-public"
    def url(user, password):
        return f"postgresql://{user}:{password}@{host}:26257/flowlike?sslmode=verify-full&sslrootcert=/etc/database/ca.crt"
    api = secret("database", {"DATABASE_URL": url("flowlike_api", secrets.token_hex(32))})
    worker = secret("database-audit", {"DATABASE_URL": url("flowlike_audit", secrets.token_hex(32))})
    migration = secret("database-migration", {"DATABASE_URL": url("flowlike_migration", owner_password)})
    return {"type": "internal", "caSecret": ca, "apiExistingSecret": api, "migration": {"existingSecret": migration}, "internal": {"auth": {"username": "flowlike_migration", "database": "flowlike"}, "tls": {"nodeSecret": node, "initSecret": init}}}, worker, "flowlike_api"


def origin(value, name):
    url = urllib.parse.urlsplit(value)
    if url.scheme not in ("http", "https") or not url.hostname or url.path not in ("", "/") or url.username or url.query or url.fragment:
        raise ValueError(f"{name} must be an HTTP(S) origin")
    return value.rstrip("/")


def pull_secret_command(name, namespace):
    return f"kubectl create secret docker-registry {name} --namespace {namespace} --docker-server=ghcr.io --docker-username=<github-user> --docker-password=<read:packages token>"


def generate(namespace, release, image_pull_secrets=()):
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
    system_data = {key: os.environ.get(key) or secrets.token_hex(32) for key in ["SINK_TOKEN_ENCRYPTION_KEY", "SINK_SECRET", "MAINTENANCE_TOKEN"]}
    for key in STRIPE_SETTINGS:
        value = os.environ.get(key, "")
        if value and not re.fullmatch(r"[A-Za-z0-9_]+", value):
            raise ValueError(f"{key} must contain only letters, digits and underscores")
        if value:
            system_data[key] = value
    system = secret("api-config", system_data)
    execution = secret("execution", {"EXECUTION_MANAGER_TOKEN": os.environ.get("EXECUTION_MANAGER_TOKEN") or secrets.token_hex(32)})
    web = origin(os.environ.get("PUBLIC_WEB_URL", "http://localhost:3001"), "PUBLIC_WEB_URL")
    api = origin(os.environ.get("PUBLIC_API_URL", "http://localhost:8080"), "PUBLIC_API_URL")
    bundled = os.environ.get("RUSTFS_ENABLED", "true").lower() == "true"
    public_s3 = origin(os.environ.get("S3_PUBLIC_ENDPOINT", f"http://{release}-object-gateway.{namespace}.svc.cluster.local:9000") if bundled else required("S3_PUBLIC_ENDPOINT"), "S3_PUBLIC_ENDPOINT")
    values = {"fullnameOverride": release, "jwt": {"existingSecret": jwt}, "api": {"existingSecret": system, "publicUrl": api, "corsAllowedOrigins": [web, "tauri://localhost", "http://tauri.localhost", "https://tauri.localhost"]}, "execution": {"existingSecret": execution}, "storage": {"provider": "s3", "s3": {"publicEndpoint": public_s3}}}
    values["api"]["frontendBaseUrl"] = web
    hub_json, hub_ref = runtime_config()
    if hub_ref is not None:
        raise ValueError("The dedicated audit worker cannot resolve FLOW_LIKE_CONFIG_SECRET_REF; provide the hub config through FLOW_LIKE_CONFIG_FILE or FLOW_LIKE_CONFIG_JSON")
    # API and audit worker read the same document; the worker takes its audit section.
    values["api"]["runtimeConfig"] = {"existingSecret": secret("hub-config", {"flow-like.config.json": hub_json})}
    values["rustfs"] = {"enabled": bundled}
    storage = {}
    for key in ["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "STS_ISSUER_ACCESS_KEY", "STS_ISSUER_SECRET_KEY"]:
        storage[key] = (os.environ.get(key) or secrets.token_hex(16 if key.endswith("ACCESS_KEY") or key.endswith("KEY_ID") else 32)) if bundled else required(key)
    values["storage"]["s3"]["existingSecret"] = secret("storage", storage)
    if bundled:
        values["rustfs"]["existingSecret"] = secret("rustfs-root", {"RUSTFS_ROOT_USER": os.environ.get("RUSTFS_ROOT_USER") or secrets.token_hex(16), "RUSTFS_ROOT_PASSWORD": os.environ.get("RUSTFS_ROOT_PASSWORD") or secrets.token_hex(32)})
    else:
        values["storage"]["s3"].update({"internalEndpoint": origin(required("S3_INTERNAL_ENDPOINT"), "S3_INTERNAL_ENDPOINT"), "stsEndpoint": origin(required("STS_ENDPOINT_URL"), "STS_ENDPOINT_URL"), "runtimeCredentialsProvider": os.environ.get("S3_STS_PROVIDER", "rustfs")})
    audit = audit_values(bundled, secret)
    values["database"], audit_database, api_role = database_values(namespace, release, secret)
    audit["database"] = {"existingSecret": audit_database}
    audit["apiDatabaseRole"] = api_role
    audit["exportSecret"] = system
    values["audit"] = audit
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
    for name in image_pull_secrets:
        if len(name) > 253 or not re.fullmatch(r"[a-z0-9]([-a-z0-9.]*[a-z0-9])?", name):
            raise ValueError("image pull secret names must be DNS subdomain names")
    if image_pull_secrets:
        values["global"] = {"imagePullSecrets": [{"name": name} for name in image_pull_secrets]}
    return {"apiVersion": "v1", "kind": "List", "items": objects}, values


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--namespace", default=os.environ.get("K8S_NAMESPACE", "flow-like"))
    parser.add_argument("--release", default=os.environ.get("RELEASE", "flow-like"))
    parser.add_argument("--output-dir", type=Path, default=Path(__file__).resolve().parents[1] / ".generated")
    parser.add_argument("--image-pull-secret", action="append", default=[], metavar="NAME", help="reference an existing docker-registry Secret for private packages or mirrors (repeatable)")
    args = parser.parse_args()
    os.umask(0o077)
    paths = [args.output_dir / "secrets.yaml", args.output_dir / "values-generated.yaml"]
    if any(path.exists() for path in paths):
        raise ValueError("Generated files already exist. Reuse them; use a new output directory for a deliberate credential rotation.")
    objects, values = generate(args.namespace, args.release, args.image_pull_secret)
    args.output_dir.mkdir(parents=True, mode=0o700, exist_ok=True)
    for path, content in zip(paths, [objects, values]):
        with path.open("x", encoding="utf-8") as handle:
            json.dump(content, handle, indent=2)
            handle.write("\n")
    print(f"Wrote private configuration to {args.output_dir}. No cluster resources were changed.")
    print("Review values-generated.yaml, apply secrets.yaml, then deploy with that values file.")
    for name in args.image_pull_secret:
        print(f"Create the referenced pull secret yourself (no credentials are stored here): {pull_secret_command(name, args.namespace)}")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, subprocess.CalledProcessError) as error:
        raise SystemExit(str(error) if isinstance(error, ValueError) else "OpenSSL key generation failed") from None
