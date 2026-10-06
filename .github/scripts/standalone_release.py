import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import time
import tempfile
from urllib.request import Request, HTTPRedirectHandler, build_opener
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit


TARGETS = {
    "x86_64-unknown-linux-gnu": "amd64",
    "aarch64-unknown-linux-gnu": "arm64",
    "x86_64-apple-darwin": None,
    "aarch64-apple-darwin": None,
}
MAX_RELEASE_BINARY_BYTES = 2 * 1024**3
CDN_NOT_FOUND_ATTEMPTS, CDN_NOT_FOUND_RETRY_SECONDS = 9, 30
DAY_SECONDS = 86400
DEFAULT_RELEASE_LIFETIME_DAYS, MAX_RELEASE_LIFETIME_DAYS = 365, 1825
# Agents of release 0.1.0 refuse a list dated ahead of their own clock, so every list is dated back.
ISSUED_BACKDATE_SECONDS = 300
# At setup an agent checks the list of its own release with the cap it was built with.
AGENT_LIFETIME_CAP_DAYS = {"0.1.0": 30}
RELEASE_DATES = ("issued_at", "expires_at")
RELEASE_JWS_TYPE, RUNTIME_MANIFEST_JWS_TYPE = "flow-like-standalone-release+jws", "flow-like-runtime-manifest"
# The release keys sign both documents; the JWS type keeps them apart. Each names its signature and its document.
SIGNED_DOCUMENTS = {RELEASE_JWS_TYPE: ("release", "release manifest"),
                    RUNTIME_MANIFEST_JWS_TYPE: ("runtime manifest", "runtime manifest")}


def usable_package_modes(records, container):
    # Browser packages embed small binaries and download larger signed binaries on first start.
    for record in records:
        if type(record.get("size")) is not int or not 0 < record["size"] <= MAX_RELEASE_BINARY_BYTES:
            raise ValueError(f"{record['target']} binary exceeds the release size limit of 2 GiB")


def binary_info(binary, target):
    result = subprocess.run([str(binary.resolve()), "release-info"], check=True,
                            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=15)
    if len(result.stdout) > 4096:
        raise ValueError("Standalone release-info exceeded its output limit")
    value = json.loads(result.stdout)
    if set(value) != {"version", "target", "state_schema_version", "runtime"}:
        raise ValueError("Unexpected standalone release-info fields")
    if value["target"] != target or value["runtime"] is not True:
        raise ValueError("Standalone binary target or runtime capability mismatch")
    if not isinstance(value["state_schema_version"], int) or value["state_schema_version"] <= 0:
        raise ValueError("Invalid management state schema version")
    return value


def describe(binary, target, output):
    if target not in TARGETS or binary.is_symlink() or not binary.is_file():
        raise ValueError("Expected a regular standalone binary for a supported target")
    usable_package_modes([{"target": target, "size": binary.stat().st_size}], None)
    info = binary_info(binary, target)
    output.mkdir(parents=True, exist_ok=True)
    name = f"flow-like-standalone-{target}"
    destination = output / name
    shutil.copyfile(binary, destination)
    destination.chmod(0o755)
    (output / f"{target}.json").write_text(json.dumps(info, sort_keys=True) + "\n")


def pinned_image(image):
    return bool(re.fullmatch(r"[a-z0-9][a-z0-9/._:-]{1,254}@sha256:[a-f0-9]{64}", image)
                and "/" in image.split("@")[0])


def validity(validity_days, issued_at=None):
    whole = isinstance(validity_days, int) and not isinstance(validity_days, bool)
    if not whole or not 1 <= validity_days <= MAX_RELEASE_LIFETIME_DAYS:
        raise ValueError(f"Release validity must be between 1 and {MAX_RELEASE_LIFETIME_DAYS} days")
    if issued_at is None:
        issued_at = int(time.time()) - ISSUED_BACKDATE_SECONDS
    return {"issued_at": issued_at, "expires_at": issued_at + validity_days * DAY_SECONDS}


def manifest(artifacts, base_url, sequence, image, output, issued_at=None,
             validity_days=DEFAULT_RELEASE_LIFETIME_DAYS):
    url = urlsplit(base_url)
    if (url.scheme != "https" or not url.netloc or url.username or url.password
            or url.query or url.fragment or base_url.endswith("/") or len(base_url) > 800):
        raise ValueError("Artifact base must be a direct HTTPS prefix without credentials or query")
    if not 0 < sequence <= 9007199254740991 or not pinned_image(image):
        raise ValueError("Expected a positive release sequence and digest-pinned container")
    dates = validity(validity_days, issued_at)
    records = []
    version = schema = None
    for target in TARGETS:
        binary = artifacts / f"flow-like-standalone-{target}"
        info = json.loads((artifacts / f"{target}.json").read_text())
        if info["target"] != target or info["runtime"] is not True:
            raise ValueError("Artifact metadata does not match its target")
        if version is None:
            version, schema = info["version"], info["state_schema_version"]
        if info["version"] != version or info["state_schema_version"] != schema:
            raise ValueError("Release binaries disagree on version or database schema")
        if binary.is_symlink() or not binary.is_file() or not 0 < binary.stat().st_size <= MAX_RELEASE_BINARY_BYTES:
            raise ValueError("Release binary exceeds its size or filesystem bounds")
        with binary.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        records.append({"target": target, "url": f"{base_url}/{binary.name}",
                        "size": binary.stat().st_size, "sha256": digest})
    value = {"version": 1, "state_schema_version": schema, "sequence": sequence,
             "release_version": version, **dates, "artifacts": records,
             "container": {"image": image, "platforms": ["linux/amd64", "linux/arm64"]}}
    usable_package_modes(records, value["container"])
    output.write_text(json.dumps(value, separators=(",", ":")))
    return value


def secure_prefix(value):
    url = urlsplit(value)
    if (url.scheme != "https" or not url.netloc or url.username or url.password
            or url.query or url.fragment or value.endswith("/") or len(value) > 800
            or any(c.isspace() for c in value) or "\\" in value
            or any(part in {".", ".."} for part in url.path.split("/"))):
        raise ValueError("Expected a direct HTTPS prefix without credentials, redirects or query")
    return value


def decode64(value):
    if not isinstance(value, str) or not re.fullmatch(r"[A-Za-z0-9_-]+", value):
        raise ValueError("Invalid release encoding")
    result = base64.urlsafe_b64decode(value + "=" * (-len(value) % 4))
    if base64.urlsafe_b64encode(result).decode().rstrip("=") != value:
        raise ValueError("Noncanonical release encoding")
    return result


def verified_release(compact, public_keys, current=True, jws_type=RELEASE_JWS_TYPE):
    subject, document = SIGNED_DOCUMENTS[jws_type]
    if not isinstance(compact, bytes) or len(compact) > 16384:
        raise ValueError(f"{subject.capitalize()} signature exceeds its limit")
    parts = compact.decode("ascii").split(".")
    if len(parts) != 3:
        raise ValueError(f"Expected a signed {document}")
    header = json.loads(decode64(parts[0]))
    if set(header) != {"alg", "typ", "kid"} or header["alg"] != "EdDSA" or header["typ"] != jws_type:
        raise ValueError(f"Unexpected {subject} signature type")
    signature = decode64(parts[2])
    if len(signature) != 64:
        raise ValueError(f"Invalid {subject} signature")
    for encoded in public_keys:
        public = decode64(encoded)
        if len(public) != 32:
            raise ValueError("Release public keys must contain 32 bytes")
        jwk = json.dumps({"crv": "Ed25519", "kty": "OKP", "x": encoded}, separators=(",", ":")).encode()
        kid = base64.urlsafe_b64encode(hashlib.sha256(jwk).digest()).decode().rstrip("=")
        if header["kid"] != kid:
            continue
        with tempfile.TemporaryDirectory(prefix="standalone-release-verify-") as folder:
            directory = Path(folder)
            (directory / "public.der").write_bytes(bytes.fromhex("302a300506032b6570032100") + public)
            (directory / "signature").write_bytes(signature)
            (directory / "message").write_bytes(".".join(parts[:2]).encode())
            result = subprocess.run(["openssl", "pkeyutl", "-verify", "-pubin", "-keyform", "DER", "-inkey", str(directory / "public.der"), "-rawin", "-in", str(directory / "message"), "-sigfile", str(directory / "signature")], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15)
            if result.returncode:
                raise ValueError(f"{subject.capitalize()} signature verification failed")
        value = json.loads(decode64(parts[1]))
        if (type(value.get("sequence")) is not int or not 0 < value["sequence"] <= 9007199254740991
                or value.get("version") != 1 or type(value.get("issued_at")) is not int
                or type(value.get("expires_at")) is not int or value["issued_at"] < 0
                or not 0 < value["expires_at"] - value["issued_at"] <= MAX_RELEASE_LIFETIME_DAYS * DAY_SECONDS):
            raise ValueError(f"Invalid signed {subject} version, sequence or lifetime")
        if current and not value["issued_at"] <= int(time.time()) < value["expires_at"]:
            raise ValueError(f"{document.capitalize()} is expired or not yet valid")
        return value
    raise ValueError(f"{subject.capitalize()} signature is not trusted by STANDALONE_RELEASE_PUBLIC_KEYS")


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, newurl):
        raise ValueError("Release CDN redirected; direct HTTPS objects are required")


def read_public(request, size, digest):
    with build_opener(NoRedirect).open(request, timeout=60) as response:
        if (response.status != 200 or response.headers.get("Access-Control-Allow-Origin") != "*"
                or response.headers.get("Content-Encoding", "identity") != "identity"):
            raise ValueError("Release CDN must allow anonymous browser CORS reads without transformation")
        received, checksum = 0, hashlib.sha256()
        while chunk := response.read(1024 * 1024):
            received += len(chunk)
            if received > size:
                raise ValueError("Release CDN returned an oversized object")
            checksum.update(chunk)
        if received != size or checksum.hexdigest() != digest:
            raise ValueError("Release CDN readback differs from the signed bytes")


def cdn_request(url, method="GET"):
    # Cloudflare's browser integrity check rejects urllib's default User-Agent (error 1010).
    return Request(secure_prefix(url), method=method,
                   headers={"User-Agent": "flow-like-standalone-release-verifier/1",
                            "Origin": "https://release-verification.flow-like.invalid",
                            "Accept-Encoding": "identity", "Cache-Control": "no-cache"})


def cdn_failure(url, error):
    if isinstance(error, HTTPError):
        error.close()
        return ValueError(f"Release CDN answered HTTP {error.code} for {url}; it must serve release objects to anonymous clients")
    return ValueError(f"Release CDN is unreachable for {url}: {getattr(error, 'reason', error)}")


def public_readback(url, size, digest):
    request = cdn_request(url)
    for attempt in range(CDN_NOT_FOUND_ATTEMPTS):
        try:
            return read_public(request, size, digest)
        except (URLError, TimeoutError) as error:
            # The CDN edge keeps a 404 served before the upload for up to three minutes.
            if getattr(error, "code", None) != 404 or attempt + 1 == CDN_NOT_FOUND_ATTEMPTS:
                raise cdn_failure(url, error) from error
            error.close()
        time.sleep(CDN_NOT_FOUND_RETRY_SECONDS)


def published_object(url, method="GET", limit=16384):
    try:
        with build_opener(NoRedirect).open(cdn_request(url, method), timeout=60) as response:
            body = response.read(limit + 1)
    except (URLError, TimeoutError) as error:
        if getattr(error, "code", None) == 404:
            error.close()
            return None
        raise cdn_failure(url, error) from error
    if len(body) > limit:
        raise ValueError(f"Release CDN returned an oversized object for {url}")
    return body


def preflight(base_url, sequence, public_keys, release_version=None):
    base_url = secure_prefix(base_url)
    stable = published_object(f"{base_url}/release.jws")
    if stable is not None:
        published = verified_release(stable, json.loads(public_keys), current=False)
        if sequence <= published["sequence"]:
            raise ValueError(f"Release sequence {sequence} must be greater than the published sequence {published['sequence']}")
        if release_version is not None and release_version == published.get("release_version"):
            raise ValueError(f"Release {published['sequence']} is already published as agent version {release_version}; "
                             "bump the agent version: the update button compares versions")
    # Immutable objects of a failed run keep their bytes; a rebuild under the same sequence cannot replace them.
    for name in [f"flow-like-standalone-{target}" for target in TARGETS] + ["release.jws"]:
        if published_object(f"{base_url}/releases/{sequence}/{name}", "HEAD") is not None:
            raise ValueError(f"An earlier run already uploaded {name} for sequence {sequence}; dispatch with a higher sequence")


def renewable(value, sequence, validity_days):
    # The CDN could serve an older genuine list; re-signing it would give it a new life.
    if sequence is not None and value["sequence"] != sequence:
        raise ValueError(f"The published release is number {value['sequence']}, not {sequence}; nothing was renewed")
    version = value.get("release_version")
    cap = AGENT_LIFETIME_CAP_DAYS.get(version, MAX_RELEASE_LIFETIME_DAYS)
    if validity_days > cap:
        raise ValueError(f"Agents of release {version} accept at most {cap} days, so no device could be set up; "
                         f"renew for {cap} days or less, or publish a newer release")


def published_runtime_manifests(base_url, public_keys, sequence):
    """Each target's published runtime manifest. A renewal keeps its number: devices refuse only lower ones."""
    manifests = {}
    for target in TARGETS:
        compact = published_object(f"{base_url}/runtimes/{target}.jws")
        if compact is None:
            continue
        value = verified_release(compact, public_keys, False, RUNTIME_MANIFEST_JWS_TYPE)
        if value["sequence"] > sequence:
            raise ValueError(f"The {target} runtime manifest is number {value['sequence']}, newer than release {sequence}, "
                             "which no release run publishes; nothing was renewed")
        if value["sequence"] < sequence:
            print(f"::warning::The {target} runtime manifest is number {value['sequence']}, older than release {sequence}; "
                  "it is renewed under its own number")
        manifests[target] = value
    return manifests


def renewal_due(value, runtimes, only_if_days_left):
    remaining = min(listed["expires_at"] for listed in [value, *runtimes.values()]) - int(time.time())
    if only_if_days_left is None or remaining <= only_if_days_left * DAY_SECONDS:
        return True
    lists = " and its runtime pack lists are" if runtimes else " is"
    print(f"::notice::Release {value['sequence']}{lists} valid for {remaining // DAY_SECONDS} more days, "
          f"more than {only_if_days_left}; nothing was renewed")
    return False


def renew_manifest(base_url, public_keys, output, validity_days=DEFAULT_RELEASE_LIFETIME_DAYS,
                   sequence=None, only_if_days_left=None, runtime_output=None):
    dates = validity(validity_days)
    if only_if_days_left is not None and only_if_days_left < 0:
        raise ValueError("Expected a number of remaining days of zero or more")
    base_url, keys = secure_prefix(base_url), json.loads(public_keys)
    stable = published_object(f"{base_url}/release.jws")
    if stable is None:
        raise ValueError("No published release to renew")
    # The published list may have run out already: a renewal is how it becomes usable again.
    value = verified_release(stable, keys, current=False)
    renewable(value, sequence, validity_days)
    runtimes = published_runtime_manifests(base_url, keys, value["sequence"]) if runtime_output else {}
    if not renewal_due(value, runtimes, only_if_days_left):
        return None
    # Every list takes the release's new dates. The release file is written last: it marks a complete renewal.
    for target, runtime in runtimes.items():
        runtime.update(dates)
        runtime_output.mkdir(parents=True, exist_ok=True)
        (runtime_output / f"{target}.json").write_text(json.dumps(runtime, separators=(",", ":")))
    value.update(dates)
    output.write_text(json.dumps(value, separators=(",", ":")))
    return value


class S3Store:
    def __init__(self, bucket, endpoint=None):
        if not re.fullmatch(r"[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]", bucket):
            raise ValueError("Invalid release bucket")
        self.bucket = bucket
        self.endpoint = secure_prefix(endpoint) if endpoint else None

    def command(self, operation, arguments):
        command = ["aws", "--no-cli-pager", "--output", "json"]
        if self.endpoint:
            command += ["--endpoint-url", self.endpoint]
        result = subprocess.run(command + ["s3api", operation, "--bucket", self.bucket] + arguments,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=1800)
        if result.returncode:
            if b"(NoSuchKey)" in result.stderr or b"(404)" in result.stderr:
                return None
            if b"(PreconditionFailed)" in result.stderr or b"(412)" in result.stderr or b"(ConditionalRequestConflict)" in result.stderr:
                raise FileExistsError("Release object condition changed")
            # Provider responses can contain endpoint credentials. Do not echo them.
            raise ValueError(f"S3 {operation} failed; check publisher permissions and conditional-write support")
        return json.loads(result.stdout or b"{}")

    def read(self, key, maximum=16384, retain=True):
        with tempfile.TemporaryDirectory(prefix="standalone-release-read-") as folder:
            path = Path(folder) / "object"
            result = self.command("get-object", ["--key", key, "--range", f"bytes=0-{maximum}", str(path)])
            if result is None:
                return None
            size = path.stat().st_size
            if size > maximum:
                raise ValueError("Stored release object exceeds its expected size")
            with path.open("rb") as stream:
                digest = hashlib.file_digest(stream, "sha256").hexdigest()
            etag = result.get("ETag")
            if not isinstance(etag, str) or not etag:
                raise ValueError("S3 did not return an object revision")
            return {"etag": etag, "size": size, "sha256": digest, "body": path.read_bytes() if retain else None}

    def put(self, key, path, *, etag=None, immutable=True):
        arguments = ["--key", key, "--body", str(path), "--content-type", "application/jose" if key.endswith(".jws") else "application/octet-stream", "--cache-control", "public,max-age=31536000,immutable" if immutable else "no-cache,max-age=0,must-revalidate"]
        arguments += ["--if-match", etag] if etag else ["--if-none-match", "*"]
        result = self.command("put-object", arguments)
        if result is None:
            raise ValueError("S3 refused the conditional release upload")


def docker_failure(stderr):
    # Pulls run with an empty config and no credential helpers, so Docker errors hold no secrets.
    detail = re.sub(r"[^\x20-\x7e]", "?", " ".join((stderr or b"").decode("utf-8", "replace").split()))[:400]
    if not detail:
        return "Docker reported no error message"
    if re.search(r"unauthorized|denied|forbidden", detail, re.IGNORECASE):
        return f"{detail}; make the digest-pinned GHCR package public before publishing"
    return detail


def anonymous_container_readback(container):
    if (not isinstance(container, dict) or not pinned_image(container.get("image", ""))
            or not isinstance(container.get("platforms"), list) or not container["platforms"]
            or len(container["platforms"]) > 2 or len(set(container["platforms"])) != len(container["platforms"])
            or any(platform not in {"linux/amd64", "linux/arm64"} for platform in container["platforms"])):
        raise ValueError("Release needs a digest-pinned container with supported platforms")
    docker = shutil.which("docker")
    if not docker:
        raise ValueError("Install Docker to verify anonymous release image downloads")
    with tempfile.TemporaryDirectory(prefix="standalone-anonymous-image-") as folder:
        (Path(folder) / "config.json").write_text('{"auths":{}}')
        # An empty PATH also prevents Docker from discovering native credential
        # helpers. Only this fresh config and the local CI daemon are permitted.
        environment = {"PATH": folder, "DOCKER_CONFIG": folder}
        def image(*arguments):
            return subprocess.run([docker, "--config", folder, "--host", "unix:///var/run/docker.sock", "image", *arguments],
                                  env=environment, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                  stderr=subprocess.PIPE, timeout=1800)
        for platform in container["platforms"]:
            try:
                # The classic image store binds a digest reference to one platform's image and
                # refuses to repoint it ("cannot overwrite digest"), so every pull starts without it.
                image("rm", container["image"])
                result = image("pull", "--quiet", "--platform", platform, container["image"])
            except (OSError, subprocess.TimeoutExpired):
                raise ValueError(f"Anonymous image verification failed for {platform}; check the Docker daemon and public registry access") from None
            if result.returncode:
                raise ValueError(f"Release image is not anonymously pullable for {platform}: {docker_failure(result.stderr)}")


def signed_manifest(artifacts, base_url, prefix, public_keys):
    base_url = secure_prefix(base_url)
    if not re.fullmatch(r"[A-Za-z0-9_-]+(?:/[A-Za-z0-9_-]+)*", prefix):
        raise ValueError("Release prefix must contain safe, nonempty path segments")
    if not isinstance(public_keys, list) or not 1 <= len(public_keys) <= 8:
        raise ValueError("Configure one to eight trusted release public keys")
    signed_path = artifacts / "release.jws"
    if signed_path.is_symlink() or not signed_path.is_file() or signed_path.stat().st_size > 16384:
        raise ValueError("Expected a bounded signed release manifest")
    signed = signed_path.read_bytes()
    return base_url, signed_path, signed, verified_release(signed, public_keys)


def holds(stored, signed):
    return bool(stored) and stored["body"] == signed


def promote(store, stable_key, signed_path, signed, previous, public_keys, jws_type=RELEASE_JWS_TYPE):
    # The ETag read before any upload still guards this write. A competing publisher wins once.
    verified_release(signed, public_keys, jws_type=jws_type)
    subject = SIGNED_DOCUMENTS[jws_type][0]
    # One release list, but one runtime list per target: those name their object.
    stable = subject if jws_type == RELEASE_JWS_TYPE else f"{subject} {stable_key}"
    if not holds(previous, signed):
        try:
            store.put(stable_key, signed_path, etag=previous["etag"] if previous else None, immutable=False)
        except FileExistsError as error:
            if not holds(store.read(stable_key), signed):
                raise ValueError(f"Stable {stable} changed concurrently; refusing to overwrite it") from error
    if not holds(store.read(stable_key), signed):
        raise ValueError(f"Stable {stable} acknowledgement could not be verified")


def publish_bundle(artifacts, base_url, prefix, public_keys, store, verify_public=public_readback,
                   verify_container=anonymous_container_readback):
    base_url, signed_path, signed, value = signed_manifest(artifacts, base_url, prefix, public_keys)
    sequence = value["sequence"]
    stable_key = f"{prefix}/release.jws"
    previous = store.read(stable_key)
    if previous:
        old = verified_release(previous["body"], public_keys, current=False)
        if old["sequence"] > sequence or (old["sequence"] == sequence and previous["body"] != signed):
            raise ValueError("Release sequence would roll back or replace an existing signed release")
    records = value.get("artifacts")
    if not isinstance(records, list) or {entry.get("target") for entry in records} != set(TARGETS) or len(records) != len(TARGETS):
        raise ValueError("Signed release must include each supported target exactly once")
    uploads = []
    for record in records:
        name = f"flow-like-standalone-{record['target']}"
        path = artifacts / name
        if (path.is_symlink() or not path.is_file() or type(record.get("size")) is not int
                or not 0 < record["size"] <= MAX_RELEASE_BINARY_BYTES or path.stat().st_size != record["size"]
                or record.get("url") != f"{base_url}/releases/{sequence}/{name}"):
            raise ValueError("Signed artifact path, size or immutable URL mismatch")
        with path.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        if digest != record.get("sha256"):
            raise ValueError("Release binary differs from its signed digest")
        uploads.append((f"{prefix}/releases/{sequence}/{name}", path, record["url"], record["size"], digest))
    usable_package_modes(records, value.get("container"))
    verify_container(value.get("container"))
    uploads.append((f"{prefix}/releases/{sequence}/release.jws", signed_path, f"{base_url}/releases/{sequence}/release.jws", len(signed), hashlib.sha256(signed).hexdigest()))
    for key, path, url, size, digest in uploads:
        try:
            store.put(key, path)
        except FileExistsError:
            pass  # A retry may find the exact immutable bytes already committed.
        stored = store.read(key, size, retain=False)
        if not stored or stored["size"] != size or stored["sha256"] != digest:
            raise ValueError("Immutable release object differs from the signed bundle")
        verify_public(url, size, digest)
    promote(store, stable_key, signed_path, signed, previous, public_keys)
    verify_public(f"{base_url}/release.jws", len(signed), hashlib.sha256(signed).hexdigest())
    return {"sequence": sequence, "manifest_url": f"{base_url}/release.jws"}


def publish(artifacts, base_url, bucket, prefix, public_keys, endpoint=None):
    keys = json.loads(public_keys)
    result = publish_bundle(artifacts, base_url, prefix, keys, S3Store(bucket, endpoint))
    print(json.dumps(result, separators=(",", ":")))


def undated(value):
    return {name: field for name, field in value.items() if name not in RELEASE_DATES}


def redated(path, previous, key, public_keys):
    """The re-dated copy of a stable runtime manifest, if nothing but its dates changed."""
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 16384:
        raise ValueError(f"Expected a bounded signed runtime manifest at {path}")
    signed = path.read_bytes()
    published = verified_release(previous["body"], public_keys, False, RUNTIME_MANIFEST_JWS_TYPE)
    if undated(published) != undated(verified_release(signed, public_keys, True, RUNTIME_MANIFEST_JWS_TYPE)):
        raise ValueError(f"A renewal may change only the dates of {key}; it differs in other fields")
    return signed


def runtime_renewals(artifacts, prefix, public_keys, store):
    """Each stable runtime manifest with its re-dated copy: every published one is renewed, and only its dates change."""
    renewals = []
    for target in TARGETS:
        key, path = f"{prefix}/runtimes/{target}.jws", artifacts / "runtimes" / f"{target}.jws"
        previous, copied = store.read(key), path.exists() or path.is_symlink()
        if bool(previous) != copied:
            raise ValueError(f"{key} is published, but this renewal carries no re-dated copy of it; renew again" if previous
                             else f"No published runtime manifest {key} to renew")
        if previous:
            renewals.append((target, path, redated(path, previous, key, public_keys), previous))
    return renewals


def renew_bundle(artifacts, base_url, prefix, public_keys, store, verify_public=public_readback):
    base_url, signed_path, signed, value = signed_manifest(artifacts, base_url, prefix, public_keys)
    stable_key = f"{prefix}/release.jws"
    previous = store.read(stable_key)
    if not previous:
        raise ValueError("No published release to renew")
    # releases/<sequence>/release.jws keeps the first signature; only the stable list is re-signed.
    if undated(verified_release(previous["body"], public_keys, current=False)) != undated(value):
        raise ValueError("A renewal may change only the dates of the published release; it differs in other fields")
    # Runtime lists advance first, so a run that stops early leaves the release list due for the next renewal.
    for target, path, compact, stable in runtime_renewals(artifacts, prefix, public_keys, store):
        promote(store, f"{prefix}/runtimes/{target}.jws", path, compact, stable, public_keys, RUNTIME_MANIFEST_JWS_TYPE)
        verify_public(f"{base_url}/runtimes/{target}.jws", len(compact), hashlib.sha256(compact).hexdigest())
    promote(store, stable_key, signed_path, signed, previous, public_keys)
    verify_public(f"{base_url}/release.jws", len(signed), hashlib.sha256(signed).hexdigest())
    return {"sequence": value["sequence"], "manifest_url": f"{base_url}/release.jws"}


def renew_publish(artifacts, base_url, bucket, prefix, public_keys, endpoint=None):
    keys = json.loads(public_keys)
    result = renew_bundle(artifacts, base_url, prefix, keys, S3Store(bucket, endpoint))
    print(json.dumps(result, separators=(",", ":")))


def runtime_lists(artifacts, public_keys):
    numbers = {path.stem: verified_release(path.read_bytes(), public_keys, False, RUNTIME_MANIFEST_JWS_TYPE)["sequence"]
               for path in sorted((artifacts / "runtimes").glob("*.jws"))}
    return ", ".join(f"{target} (number {number})" for target, number in numbers.items()) or "none published"


def summary(artifacts, public_keys, renewed=False):
    keys = json.loads(public_keys)
    value = verified_release((artifacts / "release.jws").read_bytes(), keys, current=False)
    sequence = value["sequence"]
    until = time.strftime("%Y-%m-%d %H:%M:%S UTC", time.gmtime(value["expires_at"]))
    days = (value["expires_at"] - value["issued_at"]) / DAY_SECONDS
    minimum = (f"If the hub's minimum release number is below {sequence}, raise it to {sequence}" if renewed
               else f"Next: set the hub's minimum release number to {sequence}")
    next_step = (f"{minimum} (`standalone.release_trust.minimum_sequence` in the hub config) and deploy the hub. "
                 "While the minimum is lower, an older genuine release can still be served to new devices "
                 "for as long as that older release is valid.")
    rows = [f"| Release number | {sequence} |", f"| Agent version | {value.get('release_version')} |",
            f"| Valid until | {until} ({days:g} days) |"]
    if renewed:
        rows.append(f"| Runtime pack lists | {runtime_lists(artifacts, keys)} |")
    print("\n".join([
        f"### Standalone release {sequence} {'renewed' if renewed else 'published'}", "",
        "| | |", "|---|---|", *rows, "",
        next_step,
    ]))


def dependencies(executable):
    result = subprocess.run(["ldd", str(executable)], check=True, text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if "not found" in result.stdout:
        raise ValueError("Standalone container has an unresolved runtime library")
    files = []
    for line in result.stdout.splitlines():
        match = re.search(r"(?:=>\s+)?(/[^\s]+)", line)
        if match:
            files.append(Path(match.group(1)))
    return files


def rootfs(binary, output, epoch):
    if os.name != "posix" or not Path("/proc/self").is_dir():
        raise ValueError("Container rootfs assembly requires the Linux build runner")
    output.mkdir(parents=True, exist_ok=False)
    files = {binary.resolve(): Path("usr/local/bin/flow-like-standalone"),
             Path("/bin/sh").resolve(): Path("bin/sh"),
             Path("/etc/ssl/certs/ca-certificates.crt"): Path("etc/ssl/certs/ca-certificates.crt")}
    for executable in [binary, Path("/bin/sh")]:
        for dependency in dependencies(executable):
            files[dependency] = dependency.relative_to("/")
    for source, relative in files.items():
        destination = output / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
        destination.chmod(0o755 if source.stat().st_mode & 0o111 else 0o644)
    (output / "etc/nsswitch.conf").write_text("hosts: files dns\n")
    (output / "etc/passwd").write_text("agent:x:65532:65532:Flow-Like agent:/var/lib/flow-like:/bin/sh\n")
    (output / "etc/group").write_text("agent:x:65532:\n")
    (output / "var/lib/flow-like").mkdir(parents=True)
    (output / "tmp").mkdir()
    archive = output.with_suffix(".tar")
    def normalize(info):
        info.uid = info.gid = 0
        info.uname = info.gname = ""
        info.mtime = epoch
        if info.name == "var/lib/flow-like":
            info.uid = info.gid = 65532
            info.mode = 0o700
        elif info.name == "tmp":
            info.mode = 0o1777
        return info
    with tarfile.open(archive, "w", format=tarfile.PAX_FORMAT) as result:
        for path in sorted(output.rglob("*")):
            result.add(path, arcname=path.relative_to(output), recursive=False, filter=normalize)


def main(argv=None):
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    describe_parser = commands.add_parser("describe")
    describe_parser.add_argument("--binary", type=Path, required=True)
    describe_parser.add_argument("--target", choices=TARGETS, required=True)
    describe_parser.add_argument("--output", type=Path, required=True)
    root_parser = commands.add_parser("rootfs")
    root_parser.add_argument("--binary", type=Path, required=True)
    root_parser.add_argument("--output", type=Path, required=True)
    root_parser.add_argument("--epoch", type=int, required=True)
    manifest_parser = commands.add_parser("manifest")
    manifest_parser.add_argument("--artifacts", type=Path, required=True)
    manifest_parser.add_argument("--base-url", required=True)
    manifest_parser.add_argument("--sequence", type=int, required=True)
    manifest_parser.add_argument("--image", required=True)
    manifest_parser.add_argument("--output", type=Path, required=True)
    manifest_parser.add_argument("--validity-days", type=int, default=DEFAULT_RELEASE_LIFETIME_DAYS)
    for name in ["publish", "renew-publish"]:
        publish_parser = commands.add_parser(name)
        publish_parser.add_argument("--artifacts", type=Path, required=True)
        publish_parser.add_argument("--base-url", required=True)
        publish_parser.add_argument("--bucket", required=True)
        publish_parser.add_argument("--prefix", required=True)
        publish_parser.add_argument("--public-keys", required=True)
        publish_parser.add_argument("--endpoint")
    preflight_parser = commands.add_parser("preflight")
    preflight_parser.add_argument("--base-url", required=True)
    preflight_parser.add_argument("--sequence", type=int, required=True)
    preflight_parser.add_argument("--public-keys", required=True)
    preflight_parser.add_argument("--release-version")
    renew_parser = commands.add_parser("renew-manifest")
    renew_parser.add_argument("--base-url", required=True)
    renew_parser.add_argument("--public-keys", required=True)
    renew_parser.add_argument("--output", type=Path, required=True)
    renew_parser.add_argument("--validity-days", type=int, default=DEFAULT_RELEASE_LIFETIME_DAYS)
    renew_parser.add_argument("--sequence", type=int)
    renew_parser.add_argument("--only-if-days-left", type=int)
    renew_parser.add_argument("--runtime-output", type=Path)
    summary_parser = commands.add_parser("summary")
    summary_parser.add_argument("--artifacts", type=Path, required=True)
    summary_parser.add_argument("--public-keys", required=True)
    summary_parser.add_argument("--renewed", action="store_true")
    args = vars(parser.parse_args(argv))
    command = args.pop("command")
    globals()[command.replace("-", "_")](**args)


if __name__ == "__main__":
    main()
