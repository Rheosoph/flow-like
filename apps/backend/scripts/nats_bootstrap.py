"""Create a NATS operator and account using OpenSSL without external Python packages."""

import argparse
import base64
import binascii
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time
from urllib.parse import urlsplit


OPENSSL = shutil.which("openssl") or "openssl"


def encoded(data):
    return base64.urlsafe_b64encode(data).decode().rstrip("=")


def nkey(payload):
    checksum = binascii.crc_hqx(payload, 0).to_bytes(2, "little")
    return base64.b32encode(payload + checksum).decode().rstrip("=")


def keypair(prefix):
    try:
        private = subprocess.run(
            [OPENSSL, "genpkey", "-algorithm", "ED25519", "-outform", "DER"],
            check=True, capture_output=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        raise ValueError("NATS provisioning requires OpenSSL with Ed25519 support on PATH") from None
    public = subprocess.run(
        [OPENSSL, "pkey", "-inform", "DER", "-pubout", "-outform", "DER"],
        input=private, check=True, capture_output=True,
    ).stdout
    # Ed25519 PKCS#8 and SubjectPublicKeyInfo each end in their 32-byte key.
    seed = nkey(bytes([144 | (prefix >> 5), (prefix & 31) << 3]) + private[-32:])
    return private, nkey(bytes([prefix]) + public[-32:]), seed


def signed_jwt(private, claims):
    claims = dict(claims)
    claims["jti"] = base64.b32encode(hashlib.sha256(json.dumps(claims, separators=(",", ":")).encode()).digest()).decode().rstrip("=")
    header = encoded(b'{"typ":"JWT","alg":"ed25519-nkey"}')
    message = f"{header}.{encoded(json.dumps(claims, separators=(',', ':')).encode())}".encode()
    # OpenSSL's Ed25519 signing requires a seekable input on some platforms.
    with tempfile.TemporaryDirectory(prefix="flow-like-nats-key-") as directory:
        key = Path(directory) / "key.der"
        body = Path(directory) / "claims"
        key.write_bytes(private)
        key.chmod(0o600)
        body.write_bytes(message)
        signature = subprocess.run(
            [OPENSSL, "pkeyutl", "-sign", "-rawin", "-keyform", "DER", "-inkey", str(key), "-in", str(body)],
            check=True, capture_output=True,
        ).stdout
    return f"{message.decode()}.{encoded(signature)}"


def generate():
    operator_private, operator_public, _ = keypair(112)
    _, account_public, account_seed = keypair(0)
    issued = int(time.time())
    operator = signed_jwt(operator_private, {
        "iss": operator_public, "sub": operator_public, "iat": issued,
        "name": "flow-like-channels", "nats": {"type": "operator", "version": 2},
    })
    account = signed_jwt(operator_private, {
        "iss": operator_public, "sub": account_public, "iat": issued,
        "name": "flow-like-channels",
        "nats": {
            "type": "account", "version": 2, "disallow_bearer": False,
            "limits": {"subs": -1, "data": -1, "payload": 1048576,
                       "imports": -1, "exports": -1, "wildcards": True, "conn": -1, "leaf": -1},
        },
    })
    return {"operator_jwt": operator, "account_jwt": account,
            "account_public_key": account_public, "account_seed": account_seed}


def server_config(trust, origins):
    if not origins or any(not isinstance(origin, str) or not origin for origin in origins):
        raise ValueError("NATS requires an explicit browser origin list")
    for origin in origins:
        parsed = urlsplit(origin)
        if any(char.isspace() for char in origin) or parsed.scheme not in {"http", "https"} or not parsed.hostname or parsed.username or parsed.password or parsed.path not in {"", "/"} or parsed.query or parsed.fragment:
            raise ValueError("NATS browser origins must be HTTP(S) origins")
    return json.dumps({
        "port": 4222, "http_port": 8222, "max_payload": 1048576,
        "max_pending": 8388608, "write_deadline": "10s",
        "operator": trust["operator_jwt"], "resolver": "MEMORY",
        "resolver_preload": {trust["account_public_key"]: trust["account_jwt"]},
        "websocket": {"port": 8080, "no_tls": True, "compression": False,
                      "allowed_origins": list(origins)},
    }, separators=(",", ":"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--web-origin", required=True)
    parser.add_argument("--websocket-url", required=True)
    parser.add_argument("--format", choices=["env", "kubernetes"], default="env")
    parser.add_argument("--namespace", default="flow-like")
    parser.add_argument("--release", default="flow-like")
    args = parser.parse_args()
    for value in (args.namespace, args.release):
        if len(value) > 40 or not re.fullmatch(r"[a-z0-9](?:[a-z0-9-]*[a-z0-9])?", value):
            parser.error("--namespace and --release must be DNS labels of at most 40 characters")
    websocket = urlsplit(args.websocket_url)
    if any(char.isspace() for char in args.websocket_url) or websocket.scheme not in {"ws", "wss"} or not websocket.hostname or websocket.username or websocket.password or websocket.query or websocket.fragment:
        parser.error("--websocket-url must be a ws:// or wss:// URL without credentials")
    try:
        trust = generate()
        config = server_config(trust, [args.web_origin, "http://tauri.localhost", "https://tauri.localhost"])
    except ValueError as error:
        parser.error(str(error))
    if args.format == "env":
        output = "\n".join([
            "CHANNEL_TRANSPORT=nats", "CHANNEL_NATS_URL=nats://nats:4222",
            f"CHANNEL_NATS_WS_URL={args.websocket_url}",
            f"CHANNEL_NATS_ACCOUNT_SEED={trust['account_seed']}",
            "NATS_SERVER_CONFIG=" + base64.b64encode(config.encode()).decode(), "",
        ])
    else:
        output = json.dumps({"apiVersion": "v1", "kind": "List", "items": [
            {"apiVersion": "v1", "kind": "Secret", "type": "Opaque",
             "metadata": {"name": f"{args.release}-nats-{suffix}", "namespace": args.namespace},
             "stringData": values}
            for suffix, values in [
                ("issuer", {"CHANNEL_NATS_ACCOUNT_SEED": trust["account_seed"]}),
                ("server", {"nats.conf": config}),
            ]
        ]}, indent=2) + "\n"
    try:
        descriptor = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        parser.error("Output already exists; refusing to replace NATS credentials")
    with os.fdopen(descriptor, "w") as target:
        target.write(output)
    print(f"Created {args.output} with mode 0600.")


if __name__ == "__main__":
    main()
