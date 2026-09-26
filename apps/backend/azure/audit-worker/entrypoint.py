#!/usr/bin/env python3
"""Add a short-lived Entra PostgreSQL token, then run exactly one audit tick.

DATABASE_URL is a password-free Key Vault template. Tokens never enter logs,
command-line arguments, Terraform state or files; only the child's environment.
"""
import json
import os
import signal
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

RESOURCE = "https://ossrdbms-aad.database.windows.net"


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        # The identity header must never follow a response to another server.
        return None


def token_request(environ):
    endpoint = environ.get("IDENTITY_ENDPOINT", "")
    header = environ.get("IDENTITY_HEADER", "")
    client_id = environ.get("AZURE_CLIENT_ID", "")
    parsed = urllib.parse.urlsplit(endpoint)
    if not header or not client_id or parsed.scheme != "http" or parsed.hostname not in {"localhost", "127.0.0.1", "::1"}:
        raise ValueError("Container Apps managed identity endpoint and client ID are required")
    if parsed.username or parsed.password or parsed.fragment:
        raise ValueError("Invalid managed identity endpoint")
    query = urllib.parse.urlencode({
        "api-version": "2019-08-01", "resource": RESOURCE, "client_id": client_id,
    })
    return urllib.request.Request(endpoint.split("?", 1)[0] + "?" + query,
                                  headers={"X-IDENTITY-HEADER": header})


def fetch_token(environ, opener=None, now=None, sleeper=time.sleep):
    # Do not honor proxies or redirects for the loopback credential endpoint.
    if opener is None:
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect()).open
    for attempt in range(4):
        try:
            with opener(token_request(environ), timeout=10) as response:
                payload = json.load(response)
            token = payload["access_token"]
            lifetime = int(payload["expires_on"]) - int(time.time() if now is None else now)
            if not isinstance(token, str) or not token or any(c.isspace() for c in token) or lifetime < 120:
                raise ValueError("Invalid or nearly expired token")
            return token, lifetime
        except urllib.error.HTTPError as error:
            retry = error.code == 429 or error.code >= 500
            error.close()
        except (urllib.error.URLError, TimeoutError, OSError):
            retry = True
        except Exception:
            retry = False
        if not retry or attempt == 3:
            break
        # The Container Apps identity endpoint can become ready shortly after
        # the job process starts. Bound retries to less than one schedule tick.
        sleeper(2 ** attempt)
    # Never include HTTP response bodies, Request URLs or token JSON.
    raise RuntimeError("Unable to obtain a usable PostgreSQL managed identity token") from None


def database_url(template, token, expected_user):
    parsed = urllib.parse.urlsplit(template)
    query = urllib.parse.parse_qs(parsed.query, keep_blank_values=True)
    if (parsed.scheme not in {"postgres", "postgresql"} or not parsed.hostname
            or not parsed.path.strip("/") or parsed.password is not None or parsed.fragment
            or not expected_user or urllib.parse.unquote(parsed.username or "") != expected_user
            or query.get("sslmode") != ["verify-full"]
            or any(key.lower() in {"user", "username", "password", "host", "hostaddr", "port", "dbname"} for key in query)):
        raise ValueError("DATABASE_URL must name the audit worker role, contain no password, and require sslmode=verify-full")
    # Re-encode the username and token independently; punctuation must not be
    # interpreted as another URL field or a connection parameter.
    host = parsed.hostname
    if ":" in host:
        host = "[" + host + "]"
    if parsed.port:
        host += ":" + str(parsed.port)
    authority = urllib.parse.quote(expected_user, safe="") + ":" + urllib.parse.quote(token, safe="") + "@" + host
    return urllib.parse.urlunsplit((parsed.scheme, authority, parsed.path, parsed.query, ""))


def main():
    if sys.argv[1:] != ["--once"]:
        raise ValueError("This managed identity launcher requires --once")
    environ = dict(os.environ)
    # Validate the password-free URL before asking the identity endpoint.
    template = environ.get("DATABASE_URL", "")
    expected_user = environ.get("AZURE_POSTGRES_USER", "")
    database_url(template, "validation", expected_user)
    token, lifetime = fetch_token(environ)
    environ["DATABASE_URL"] = database_url(template, token, expected_user)
    # The shared binary cannot renew its pool's password. A one-shot process
    # must end before its token expires, even if an unusually long tick hangs.
    signal.alarm(min(3300, lifetime - 60))
    os.execve("/app/flow-like-audit-worker", ["flow-like-audit-worker", "--once"], environ)


if __name__ == "__main__":
    try:
        main()
    except Exception:
        print("audit-worker: managed identity database setup failed; check the role, URL template and identity endpoint", file=sys.stderr)
        sys.exit(1)
