"""Initialize the pinned RustFS 1.0.0-rc.5 native IAM API.

Root credentials stay in this one-shot initializer and the store. Existing IAM
policies and user memberships must match, or initialization stops for review.
"""
import hashlib
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

import boto3
from botocore.auth import SigV4Auth
from botocore.awsrequest import AWSRequest
from botocore.config import Config
from botocore.credentials import Credentials
from botocore.exceptions import ClientError

ADMIN_PREFIX = "/rustfs/admin/v3"


def setting(name, default=None):
    value = os.environ.get(name, "").strip()
    return value or default


def secret(name):
    path = setting(name + "_FILE")
    if path:
        with open(path, encoding="utf-8") as handle:
            value = handle.read().rstrip("\r\n")
    else:
        value = os.environ.get(name, "")
    if not value:
        raise ValueError(f"{name} or {name}_FILE must be configured")
    return value


def canonical(value):
    """IAM arrays are sets; RustFS may reorder them and add empty defaults."""
    if isinstance(value, dict):
        return {key: canonical(item) for key, item in sorted(value.items())
                if item not in (None, "", [], {})}
    if isinstance(value, list):
        return sorted((canonical(item) for item in value), key=lambda item: json.dumps(item, sort_keys=True))
    return value


def base_policy(buckets, issuer):
    resources = [f"arn:aws:s3:::{bucket}" for bucket in buckets]
    return {"Version": "2012-10-17", "Statement": [
        {"Effect": "Allow", "Action": ["s3:ListBucket", "s3:GetBucketLocation"], "Resource": resources},
        {"Effect": "Allow", "Action": ["s3:GetObject", "s3:PutObject", "s3:DeleteObject",
         "s3:AbortMultipartUpload", "s3:ListMultipartUploadParts"],
         "Resource": [resource + "/*" for resource in resources]},
        {"Effect": "Allow" if issuer else "Deny", "Action": ["sts:AssumeRole"], "Resource": ["*"]},
        # Deny self-service admin operations as well as broad administrative access.
        {"Effect": "Deny", "Action": ["admin:*"], "Resource": ["*"]},
    ]}


class Admin:
    def __init__(self, endpoint, region, access_key, secret_key, session_token=None):
        self.endpoint = endpoint.rstrip("/")
        self.region = region
        self.credentials = Credentials(access_key, secret_key, session_token)

    def request(self, method, path, body=None):
        data = json.dumps(body, separators=(",", ":")).encode() if body is not None else b""
        url = self.endpoint + ADMIN_PREFIX + path
        request = AWSRequest(method=method, url=url, data=data, headers={
            "Content-Type": "application/json",
            "X-Amz-Content-SHA256": hashlib.sha256(data).hexdigest(),
        })
        SigV4Auth(self.credentials, "s3", self.region).add_auth(request)
        prepared = urllib.request.Request(url, data=data if body is not None else None,
                                          headers=dict(request.headers), method=method)
        # No redirects: a signed admin request must stay on its configured origin.
        class NoRedirect(urllib.request.HTTPRedirectHandler):
            def redirect_request(self, *args, **kwargs):
                return None
        with urllib.request.build_opener(NoRedirect).open(prepared, timeout=30) as response:
            payload = response.read()
            return json.loads(payload) if payload else None


def s3_client(endpoint, region, key, password, token=None):
    return boto3.client("s3", endpoint_url=endpoint, region_name=region,
                        aws_access_key_id=key, aws_secret_access_key=password,
                        aws_session_token=token,
                        config=Config(signature_version="s3v4", s3={"addressing_style": "path"},
                                      retries={"max_attempts": 3},
                                      request_checksum_calculation="when_required",
                                      response_checksum_validation="when_required"))


def ensure_policy(admin, existing, name, policy):
    if name in existing:
        if canonical(existing[name]) != canonical(policy):
            raise ValueError(f"IAM policy drift in {name}; review and migrate the policy before restarting")
    else:
        admin.request("PUT", "/add-canned-policy?" + urllib.parse.urlencode({"name": name}), policy)


def ensure_user(admin, users, key, password, policy_name):
    if key in users:
        info = users[key]
        names = {name for name in info.get("policyName", "").split(",") if name}
        if names not in ({policy_name}, set()) or info.get("memberOf"):
            raise ValueError("IAM user policy or group drift; review the configured application user")
        if info.get("status") != "enabled":
            raise ValueError("Configured IAM user is disabled")
    else:
        admin.request("PUT", "/add-user?" + urllib.parse.urlencode({"accessKey": key}),
                      {"secretKey": password, "status": "enabled"})
    if key not in users or not users[key].get("policyName"):
        admin.request("POST", "/idp/builtin/policy/attach", {"policies": [policy_name], "user": key})


def ensure_bucket(client, bucket, region, origins, temporary=False):
    try:
        client.head_bucket(Bucket=bucket)
    except ClientError as error:
        if error.response["ResponseMetadata"]["HTTPStatusCode"] != 404:
            raise
        kwargs = {"Bucket": bucket}
        if region != "us-east-1":
            kwargs["CreateBucketConfiguration"] = {"LocationConstraint": region}
        client.create_bucket(**kwargs)
    # Existing public policy is drift, not an instruction to silently unpublish data.
    try:
        client.get_bucket_policy(Bucket=bucket)
    except ClientError as error:
        if error.response["Error"]["Code"] not in ("NoSuchBucketPolicy", "NoSuchPolicy", "404"):
            raise
    else:
        raise ValueError(f"Bucket {bucket} has a policy; bundled application buckets must be private")
    if origins:
        client.put_bucket_cors(Bucket=bucket, CORSConfiguration={"CORSRules": [{
            "AllowedOrigins": origins, "AllowedMethods": ["GET", "PUT", "POST", "DELETE", "HEAD"],
            "AllowedHeaders": ["*"], "ExposeHeaders": ["ETag", "Content-Length", "Content-Range"],
            "MaxAgeSeconds": 3600,
        }]})
    else:
        client.delete_bucket_cors(Bucket=bucket)
    rules = [{"ID": "abort-incomplete-uploads", "Status": "Enabled", "Filter": {"Prefix": ""},
              "AbortIncompleteMultipartUpload": {"DaysAfterInitiation": 1}}]
    if temporary:
        rules.append({"ID": "expire-temporary-content", "Status": "Enabled", "Filter": {"Prefix": "tmp/"},
                      "Expiration": {"Days": 2}})
    client.put_bucket_lifecycle_configuration(Bucket=bucket, LifecycleConfiguration={"Rules": rules})


def configuration():
    endpoint = setting("S3_INTERNAL_ENDPOINT", "http://object-store:9000")
    parsed = urllib.parse.urlsplit(endpoint)
    if parsed.scheme not in ("http", "https") or not parsed.hostname or parsed.path not in ("", "/") or parsed.query or parsed.fragment or parsed.username:
        raise ValueError("S3_INTERNAL_ENDPOINT must be an http(s) origin")
    buckets = [setting("META_BUCKET"), setting("CONTENT_BUCKET"), setting("LOG_BUCKET")]
    if any(not bucket or not re.fullmatch(r"[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]", bucket) for bucket in buckets):
        raise ValueError("META_BUCKET, CONTENT_BUCKET and LOG_BUCKET must be valid S3 bucket names")
    if len(set(buckets)) != 3:
        raise ValueError("Bundled storage requires separate metadata, content and log buckets")
    origins = [origin.strip() for origin in setting("S3_CORS_ALLOWED_ORIGINS", "").split(",") if origin.strip()]
    for origin in origins:
        parsed = urllib.parse.urlsplit(origin)
        if "*" in origin or parsed.scheme not in ("http", "https") or not parsed.netloc or parsed.path or parsed.query or parsed.fragment or parsed.username:
            raise ValueError("S3_CORS_ALLOWED_ORIGINS must contain exact http(s) origins")
    root = (secret("RUSTFS_ROOT_USER"), secret("RUSTFS_ROOT_PASSWORD"))
    api = (secret("AWS_ACCESS_KEY_ID"), secret("AWS_SECRET_ACCESS_KEY"))
    issuer = (secret("STS_ISSUER_ACCESS_KEY"), secret("STS_ISSUER_SECRET_KEY"))
    if len({root[0], api[0], issuer[0]}) != 3 or len({root[1], api[1], issuer[1]}) != 3:
        raise ValueError("Root, API and issuer must have distinct access keys and secrets")
    if any(len(password) < 16 for _, password in (root, api, issuer)):
        raise ValueError("Root, API and issuer secrets must each have at least 16 characters")
    return endpoint, setting("AWS_REGION", "us-east-1"), buckets, origins, root, api, issuer


def main():
    endpoint, region, buckets, origins, root, api, issuer = configuration()
    admin = Admin(endpoint, region, *root)
    # Readiness includes IAM. Bound startup so orchestration can report failure.
    for attempt in range(30):
        try:
            policies = admin.request("GET", "/list-canned-policies")
            users = admin.request("GET", "/list-users")
            break
        except (urllib.error.URLError, TimeoutError):
            if attempt == 29:
                raise RuntimeError("RustFS storage/IAM did not become ready") from None
            time.sleep(2)
    client = s3_client(endpoint, region, *root)
    for bucket in buckets:
        ensure_bucket(client, bucket, region, origins, temporary=bucket == buckets[1])
    for identity, is_issuer, name in ((api, False, "flow-like-api-v1"), (issuer, True, "flow-like-issuer-v1")):
        ensure_policy(admin, policies, name, base_policy(buckets, is_issuer))
        ensure_user(admin, users, *identity, name)
        # Existing user secrets are not overwritten. A mismatch requires explicit rotation.
        s3_client(endpoint, region, *identity).head_bucket(Bucket=buckets[1])
    print("Private buckets and restricted API/STS users are ready.")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        # SDK/admin errors can contain request details; never print bodies or credentials.
        print(f"Object-store initialization failed ({type(error).__name__}). Review private initializer diagnostics and configuration.", file=sys.stderr)
        if isinstance(error, (ValueError, RuntimeError)):
            print(str(error), file=sys.stderr)
        sys.exit(1)
