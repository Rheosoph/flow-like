"""Provision the optional Keycloak realm and its deployment-specific TLS files."""
import ipaddress
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
from urllib.parse import urlsplit

REALM = "flow-like"
CLIENT = "flow-like"


def origin(value):
    parsed = urlsplit(value)
    hostname = parsed.hostname or ""
    if (any(char.isspace() for char in value) or parsed.scheme != "https" or parsed.username or parsed.password
            or parsed.path not in {"", "/"} or parsed.query or parsed.fragment
            or not re.fullmatch(r"[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?", hostname)
            or len(hostname) > 253 or any(not label or len(label) > 63 or label.startswith("-") or label.endswith("-") for label in hostname.split("."))):
        raise ValueError("--oidc-url must be an HTTPS origin with a DNS hostname")
    try:
        ipaddress.ip_address(hostname)
    except ValueError:
        pass
    else:
        raise ValueError("--oidc-url needs a DNS hostname shared by browsers and containers")
    if hostname == "localhost" or "." not in hostname:
        raise ValueError("Use auth.localhost or a public DNS name for --oidc-url; localhost refers to each container itself")
    port = parsed.port if parsed.port is not None else 443
    if not 1 <= port <= 65535:
        raise ValueError("--oidc-url has an invalid port")
    if port == 9000:
        raise ValueError("OIDC port 9000 is reserved for the internal health listener; use 8443 or another port")
    return value.rstrip("/"), hostname, port


def redirect_uri(value):
    parsed = urlsplit(value)
    if (any(char.isspace() for char in value) or "*" in value or "$" in value
            or parsed.username or parsed.password or parsed.fragment):
        raise ValueError("OIDC redirect URIs must be exact URLs without wildcards or credentials")
    if value == "flow-like://auth":
        return value
    if parsed.scheme == "https" and parsed.hostname:
        return value
    if parsed.scheme == "http" and parsed.hostname in {"localhost", "127.0.0.1", "::1"}:
        return value
    raise ValueError("OIDC redirect URIs require HTTPS, loopback HTTP, or flow-like://auth")


def realm_config(web_origin, extra_redirects=()):
    redirects = list(dict.fromkeys([redirect_uri(web_origin + "/callback"), "flow-like://auth", *map(redirect_uri, extra_redirects)]))
    return {
        "realm": REALM, "enabled": True, "sslRequired": "all",
        "registrationAllowed": False, "resetPasswordAllowed": False,
        "loginWithEmailAllowed": True, "duplicateEmailsAllowed": False,
        "bruteForceProtected": True,
        "clients": [{
            "clientId": CLIENT, "name": "Flow-Like", "enabled": True,
            "protocol": "openid-connect", "publicClient": True,
            "standardFlowEnabled": True, "implicitFlowEnabled": False,
            "directAccessGrantsEnabled": False, "serviceAccountsEnabled": False,
            "redirectUris": redirects,
            "webOrigins": [web_origin, "tauri://localhost", "http://tauri.localhost", "https://tauri.localhost"],
            "attributes": {"pkce.code.challenge.method": "S256",
                           "post.logout.redirect.uris": web_origin + "/"},
            "defaultClientScopes": ["profile", "email", "roles", "web-origins"],
            "optionalClientScopes": ["offline_access"],
            "protocolMappers": [{
                "name": "flow-like-api-audience", "protocol": "openid-connect",
                "protocolMapper": "oidc-audience-mapper",
                "config": {"included.client.audience": CLIENT,
                           "access.token.claim": "true", "id.token.claim": "false"},
            }],
        }],
        "users": [{
            "username": "${OIDC_BOOTSTRAP_USERNAME}", "enabled": True,
            "email": "${OIDC_BOOTSTRAP_EMAIL}", "emailVerified": False,
            "firstName": "Flow-Like", "lastName": "Owner",
            "credentials": [{"type": "password", "value": "${OIDC_BOOTSTRAP_PASSWORD}", "temporary": True}],
        }],
    }


def hub_config(template, public_url, web_origin, api_url):
    hub = json.loads(template)
    issuer = public_url + "/realms/" + REALM
    hub["authentication"] = {"variant": "openid", "openid": {
        "authority": issuer, "issuer": issuer, "client_id": CLIENT, "audience": CLIENT,
        # Web uses its runtime callback setting; native clients use this deep link.
        "redirect_uri": "flow-like://auth", "post_logout_redirect_uri": web_origin + "/",
        "response_type": "code", "scope": "openid email profile",
        "discovery_url": issuer + "/.well-known/openid-configuration",
        "jwks_url": issuer + "/protocol/openid-connect/certs",
        "user_info_url": issuer + "/protocol/openid-connect/userinfo",
    }}
    api = urlsplit(api_url)
    hub.update({"domain": api.netloc, "secure": api.scheme == "https",
                "app": urlsplit(web_origin).netloc, "web": urlsplit(web_origin).netloc})
    if api.hostname in {"localhost", "127.0.0.1", "::1"}:
        hub["signaling"] = ["ws://localhost:4444"]
    return hub


def openssl(*args, input=None):
    try:
        return subprocess.run(["openssl", *map(str, args)], input=input or b"", capture_output=True, check=True, timeout=30).stdout
    except (OSError, subprocess.SubprocessError):
        raise ValueError("OIDC TLS provisioning failed; check OpenSSL and the certificate, key, CA and hostname") from None


def write_file(path, data, mode=0o644):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, mode)
    with os.fdopen(descriptor, "w") as target:
        target.write(data)
    path.chmod(mode)


def certificates(directory, hostname, supplied):
    certificate, key, ca = (directory / name for name in ("tls.crt", "tls.key", "ca.crt"))
    if supplied:
        for source, target in zip(supplied, (certificate, key, ca)):
            write_file(target, Path(source).read_text())
    else:
        ca_key = directory / "ca.key"
        ca_key.write_bytes(openssl("genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256"))
        ca_key.chmod(0o600)
        openssl("req", "-new", "-x509", "-sha256", "-days", "3650", "-key", ca_key,
                "-out", ca, "-subj", "/CN=Flow-Like Local OIDC CA",
                "-addext", "basicConstraints=critical,CA:TRUE,pathlen:0",
                "-addext", "keyUsage=critical,keyCertSign,cRLSign")
        key.write_bytes(openssl("genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256"))
        request = directory / "tls.csr"
        extensions = directory / "tls.extensions"
        openssl("req", "-new", "-sha256", "-key", key, "-out", request, "-subj", f"/CN={hostname}")
        write_file(extensions, f"basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:{hostname}\n")
        openssl("x509", "-req", "-sha256", "-days", "365", "-in", request, "-CA", ca,
                "-CAkey", ca_key, "-set_serial", str(secrets.randbits(128) or 1), "-out", certificate,
                "-extfile", extensions)
        request.unlink()
        extensions.unlink()
    # A private parent directory protects these files on the host. Container users
    # need read access to the individual bind mounts regardless of the host UID.
    for path in (certificate, key, ca):
        path.chmod(0o644)
    verify_certificates(certificate, key, ca, hostname)


def verify_certificates(certificate, key, ca, hostname):
    if openssl("x509", "-in", certificate, "-pubkey", "-noout") != openssl("pkey", "-in", key, "-passin", "pass:", "-pubout"):
        raise ValueError("OIDC TLS certificate and private key do not match")
    openssl("verify", "-purpose", "sslserver", "-verify_hostname", hostname,
            "-CAfile", ca, "-untrusted", certificate, certificate)


def prepare(output, template, public_url, web_origin, api_url, username="owner",
            email="owner@localhost.invalid", extra_redirects=(), tls_files=()):
    public_url, hostname, port = origin(public_url)
    if not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_.-]{0,63}", username):
        raise ValueError("OIDC bootstrap username must contain letters, digits, '.', '_' or '-'")
    if not re.fullmatch(r"[a-zA-Z0-9_.+\-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}", email):
        raise ValueError("Provide a valid --oidc-bootstrap-email")
    if any(tls_files) and not all(tls_files):
        raise ValueError("Supply --oidc-tls-cert, --oidc-tls-key and --oidc-ca-cert together")
    supplied = tls_files if all(tls_files) and tls_files else ()
    if not supplied and not hostname.endswith(".localhost"):
        raise ValueError("Public OIDC hostnames require --oidc-tls-cert, --oidc-tls-key and --oidc-ca-cert; generated certificates are only for .localhost")
    realm = realm_config(web_origin, extra_redirects)
    hub = hub_config(template, public_url, web_origin, api_url)
    directory = Path(str(output.absolute()) + ".oidc")
    if any(char in str(directory) for char in "'\r\n"):
        raise ValueError("OIDC output paths must not contain quotes or newlines")
    directory.mkdir(mode=0o700)
    try:
        certificates(directory, hostname, supplied)
        write_file(directory / "flow-like-realm.json", json.dumps(realm, indent=2) + "\n")
        write_file(directory / "flow-like.config.json", json.dumps(hub, indent=2) + "\n")
        values = {
            "OIDC_MODE": "bundled", "COMPOSE_FILE": "docker-compose.yml:docker-compose.oidc.yml",
            "OIDC_PUBLIC_URL": public_url, "OIDC_HOSTNAME": hostname, "OIDC_PORT": str(port),
            "OIDC_DB_PASSWORD": secrets.token_hex(32), "OIDC_ADMIN_USERNAME": "admin",
            "OIDC_ADMIN_PASSWORD": secrets.token_hex(32), "OIDC_BOOTSTRAP_USERNAME": username,
            "OIDC_BOOTSTRAP_EMAIL": email, "OIDC_BOOTSTRAP_PASSWORD": secrets.token_hex(32),
            "FLOW_LIKE_CONFIG_FILE": "/app/flow-like.config.json",
            "FLOW_LIKE_CONFIG_JSON": "", "FLOW_LIKE_CONFIG_SECRET_REF": "",
        }
        for key, name in [("OIDC_REALM_FILE", "flow-like-realm.json"), ("OIDC_TLS_CERT_FILE", "tls.crt"),
                          ("OIDC_TLS_KEY_FILE", "tls.key"), ("OIDC_CA_CERT_FILE", "ca.crt"),
                          ("FLOW_LIKE_RUNTIME_CONFIG_FILE", "flow-like.config.json")]:
            values[key] = "'" + str(directory / name) + "'"
        return values, directory
    except BaseException:
        shutil.rmtree(directory)
        raise


def apply_values(text, values):
    return re.sub(r"^([A-Z][A-Z0-9_]*)=.*$", lambda m: f"{m[1]}={values[m[1]]}" if m[1] in values else m[0], text, flags=re.M)
