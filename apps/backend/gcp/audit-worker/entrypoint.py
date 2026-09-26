#!/usr/bin/env python3
"""Run one shared audit tick using a short-lived Cloud SQL IAM login token."""

import ipaddress
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import urllib.parse
import urllib.request

METADATA = "http://169.254.169.254/computeMetadata/v1/instance/service-accounts/default/"
SQL_SCOPE = "https://www.googleapis.com/auth/sqlservice.login"


def metadata(path):
    # Ignore ambient proxies and reject redirects; tokens never leave metadata.
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, *args, **kwargs):
            raise ValueError("metadata redirect rejected")

    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    request = urllib.request.Request(METADATA + path, headers={"Metadata-Flavor": "Google"})
    with opener.open(request, timeout=10) as response:
        if response.headers.get("Metadata-Flavor") != "Google":
            raise ValueError("metadata identity response missing Google marker")
        return response.read(65536).decode("utf-8")


def database_url(environ, token, ca_path):
    required = ("GCP_POSTGRES_HOST", "GCP_POSTGRES_DATABASE", "GCP_POSTGRES_USER")
    if any(not environ.get(name, "").strip() for name in required):
        raise ValueError("Cloud SQL host, database, and worker IAM username are required")
    host = environ["GCP_POSTGRES_HOST"]
    try:
        address = ipaddress.ip_address(host)
        authority = f"[{host}]" if address.version == 6 else host
        sslmode = "verify-ca"
    except ValueError:
        if not re.fullmatch(r"[A-Za-z0-9](?:[A-Za-z0-9.-]*[A-Za-z0-9])?", host):
            raise ValueError("invalid Cloud SQL host")
        authority, sslmode = host, "verify-full"
    quote = lambda value: urllib.parse.quote(value, safe="")
    query = urllib.parse.urlencode({"sslmode": sslmode, "sslrootcert": str(ca_path)})
    return (f"postgresql://{quote(environ['GCP_POSTGRES_USER'])}:{quote(token)}@"
            f"{authority}:5432/{quote(environ['GCP_POSTGRES_DATABASE'])}?{query}")


def token_timeout(payload):
    token = payload.get("access_token")
    expires = payload.get("expires_in")
    if not isinstance(token, str) or not token or not isinstance(expires, (int, float)):
        raise ValueError("invalid metadata login token")
    if expires < 180:
        raise ValueError("login token expires too soon; next scheduled tick will retry")
    # Stop two minutes before expiry. Shared SeaORM pools cannot refresh their
    # passwords in place; the next scheduled process obtains a fresh token.
    return token, min(3480, int(expires) - 120)


def validate_environment(environ):
    forbidden = {
        "DATABASE_URL", "DATABASE_URL_FILE", "PGPASSWORD", "PGPASSFILE",
        "GOOGLE_APPLICATION_CREDENTIALS", "GOOGLE_APPLICATION_CREDENTIALS_JSON",
        "GOOGLE_CREDENTIALS", "GOOGLE_OAUTH_ACCESS_TOKEN", "CLOUDSDK_AUTH_ACCESS_TOKEN",
        "GCE_METADATA_HOST", "GCE_METADATA_IP", "GCE_METADATA_ROOT",
        "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY",
    }
    if any(value and key.upper() in forbidden for key, value in environ.items()):
        raise ValueError("database credentials, metadata and proxy overrides are forbidden")
    ca = environ.get("GCP_POSTGRES_SERVER_CA", "")
    if "-----BEGIN CERTIFICATE-----" not in ca or "-----END CERTIFICATE-----" not in ca:
        raise ValueError("Cloud SQL server CA is required")


def run():
    if sys.argv[1:] != ["--once"]:
        raise ValueError("the scheduled IAM wrapper requires exactly --once")
    validate_environment(os.environ)
    account = metadata("email").strip()
    expected_user = account.removesuffix(".gserviceaccount.com")
    if os.environ.get("GCP_POSTGRES_USER") != expected_user:
        raise ValueError("database username does not match the attached worker identity")
    payload = json.loads(metadata("token?" + urllib.parse.urlencode({"scopes": SQL_SCOPE})))
    token, timeout = token_timeout(payload)
    with tempfile.TemporaryDirectory(prefix="audit-db-") as temporary:
        ca_path = Path(temporary) / "server-ca.pem"
        ca_path.write_text(os.environ["GCP_POSTGRES_SERVER_CA"], encoding="utf-8")
        ca_path.chmod(0o600)
        child_env = dict(os.environ)
        child_env["DATABASE_URL"] = database_url(child_env, token, ca_path)
        child = subprocess.Popen(["/app/flow-like-audit-worker", "--once"], env=child_env)
        def terminate(signum, _frame):
            child.send_signal(signum)
        signal.signal(signal.SIGTERM, terminate)
        signal.signal(signal.SIGINT, terminate)
        try:
            return child.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            child.terminate()
            try:
                child.wait(timeout=20)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
            print("audit worker stopped before Cloud SQL login token expiry", file=sys.stderr)
            return 1


if __name__ == "__main__":
    try:
        sys.exit(run())
    except Exception:
        # SDK/HTTP diagnostics may contain credentials. Never echo exceptions.
        print("audit worker IAM initialization failed; verify identity, CA and metadata access", file=sys.stderr)
        sys.exit(1)
