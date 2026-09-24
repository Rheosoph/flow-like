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
MAX_BROWSER_BINARY_BYTES = 256 * 1024 * 1024


def usable_package_modes(records, container):
    platforms = container.get("platforms", []) if isinstance(container, dict) else []
    for record in records:
        if record["size"] > MAX_BROWSER_BINARY_BYTES:
            architecture = TARGETS.get(record["target"])
            if not architecture or f"linux/{architecture}" not in platforms:
                raise ValueError(f"{record['target']} binary exceeds the 256 MiB browser limit and has no signed Docker alternative; reduce its release size before publishing")


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


def manifest(artifacts, base_url, sequence, image, output, issued_at=None):
    url = urlsplit(base_url)
    if (url.scheme != "https" or not url.netloc or url.username or url.password
            or url.query or url.fragment or base_url.endswith("/") or len(base_url) > 800):
        raise ValueError("Artifact base must be a direct HTTPS prefix without credentials or query")
    if not 0 < sequence <= 9007199254740991 or not pinned_image(image):
        raise ValueError("Expected a positive release sequence and digest-pinned container")
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
        if binary.is_symlink() or not binary.is_file() or not 0 < binary.stat().st_size <= 2 * 1024**3:
            raise ValueError("Release binary exceeds its size or filesystem bounds")
        with binary.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        records.append({"target": target, "url": f"{base_url}/{binary.name}",
                        "size": binary.stat().st_size, "sha256": digest})
    issued_at = int(time.time()) if issued_at is None else issued_at
    value = {"version": 1, "state_schema_version": schema, "sequence": sequence,
             "release_version": version, "issued_at": issued_at,
             "expires_at": issued_at + 30 * 86400, "artifacts": records,
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


def verified_release(compact, public_keys, current=True):
    if not isinstance(compact, bytes) or len(compact) > 16384:
        raise ValueError("Release signature exceeds its limit")
    parts = compact.decode("ascii").split(".")
    if len(parts) != 3:
        raise ValueError("Expected a signed release manifest")
    header = json.loads(decode64(parts[0]))
    if (set(header) != {"alg", "typ", "kid"} or header["alg"] != "EdDSA"
            or header["typ"] != "flow-like-standalone-release+jws"):
        raise ValueError("Unexpected release signature type")
    signature = decode64(parts[2])
    if len(signature) != 64:
        raise ValueError("Invalid release signature")
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
                raise ValueError("Release signature verification failed")
        value = json.loads(decode64(parts[1]))
        if (type(value.get("sequence")) is not int or not 0 < value["sequence"] <= 9007199254740991
                or value.get("version") != 1 or type(value.get("issued_at")) is not int
                or type(value.get("expires_at")) is not int or value["issued_at"] < 0
                or not 0 < value["expires_at"] - value["issued_at"] <= 30 * 86400):
            raise ValueError("Invalid signed release version, sequence or lifetime")
        if current and not value["issued_at"] <= int(time.time()) < value["expires_at"]:
            raise ValueError("Release manifest is expired or not yet valid")
        return value
    raise ValueError("Release signature is not trusted by STANDALONE_RELEASE_PUBLIC_KEYS")


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, newurl):
        raise ValueError("Release CDN redirected; direct HTTPS objects are required")


def public_readback(url, size, digest):
    secure_prefix(url)
    request = Request(url, headers={"Origin": "https://release-verification.flow-like.invalid", "Accept-Encoding": "identity", "Cache-Control": "no-cache"})
    try:
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
    except (HTTPError, URLError, TimeoutError) as error:
        raise ValueError("Release CDN is unavailable or does not permit direct public reads") from error


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
        for platform in container["platforms"]:
            try:
                result = subprocess.run([docker, "--config", folder, "--host", "unix:///var/run/docker.sock",
                                         "image", "pull", "--quiet", "--platform", platform, container["image"]],
                                        env=environment, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                        stderr=subprocess.DEVNULL, timeout=1800)
            except (OSError, subprocess.TimeoutExpired):
                raise ValueError(f"Anonymous image verification failed for {platform}; check the Docker daemon and public registry access") from None
            if result.returncode:
                raise ValueError(f"Release image is not anonymously pullable for {platform}; make the digest-pinned GHCR package public before publishing")


def publish_bundle(artifacts, base_url, prefix, public_keys, store, verify_public=public_readback,
                   verify_container=anonymous_container_readback):
    base_url = secure_prefix(base_url)
    if not re.fullmatch(r"[A-Za-z0-9_-]+(?:/[A-Za-z0-9_-]+)*", prefix):
        raise ValueError("Release prefix must contain safe, nonempty path segments")
    if not isinstance(public_keys, list) or not 1 <= len(public_keys) <= 8:
        raise ValueError("Configure one to eight trusted release public keys")
    signed_path = artifacts / "release.jws"
    if signed_path.is_symlink() or not signed_path.is_file() or signed_path.stat().st_size > 16384:
        raise ValueError("Expected a bounded signed release manifest")
    signed = signed_path.read_bytes()
    value = verified_release(signed, public_keys)
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
                or not 0 < record["size"] <= 2 * 1024**3 or path.stat().st_size != record["size"]
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
    # Keep the old stable ETag throughout uploads. A competing publisher wins once.
    verified_release(signed, public_keys)
    if not previous or previous["body"] != signed:
        try:
            store.put(stable_key, signed_path, etag=previous["etag"] if previous else None, immutable=False)
        except FileExistsError as error:
            current = store.read(stable_key)
            if not current or current["body"] != signed:
                raise ValueError("Stable release changed concurrently; refusing to overwrite it") from error
    current = store.read(stable_key)
    if not current or current["body"] != signed:
        raise ValueError("Stable release acknowledgement could not be verified")
    verify_public(f"{base_url}/release.jws", len(signed), hashlib.sha256(signed).hexdigest())
    return {"sequence": sequence, "manifest_url": f"{base_url}/release.jws"}


def publish(artifacts, base_url, bucket, prefix, public_keys, endpoint=None):
    keys = json.loads(public_keys)
    result = publish_bundle(artifacts, base_url, prefix, keys, S3Store(bucket, endpoint))
    print(json.dumps(result, separators=(",", ":")))


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


if __name__ == "__main__":
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
    publish_parser = commands.add_parser("publish")
    publish_parser.add_argument("--artifacts", type=Path, required=True)
    publish_parser.add_argument("--base-url", required=True)
    publish_parser.add_argument("--bucket", required=True)
    publish_parser.add_argument("--prefix", required=True)
    publish_parser.add_argument("--public-keys", required=True)
    publish_parser.add_argument("--endpoint")
    args = vars(parser.parse_args())
    command = args.pop("command")
    globals()[command](**args)
