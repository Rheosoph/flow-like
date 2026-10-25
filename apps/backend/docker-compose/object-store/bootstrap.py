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
BUCKET_NAME = re.compile(r"[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]")
# The object gateway and the execution manager match bucket names without periods.
GATEWAY_BUCKET_NAME = re.compile(r"[a-z0-9][a-z0-9-]{1,61}[a-z0-9]")
LOCK_MODES = ("GOVERNANCE", "COMPLIANCE")
LOCKED_ENFORCEMENTS = ("object", "bucket")
LOCKED_POLICY = "flow-like-locked-v1"
# Headers the API signs into presigned locked-storage PUTs.
LOCKED_CORS_HEADERS = ["content-type", "x-amz-checksum-sha256", "x-amz-sdk-checksum-algorithm",
                       "x-amz-object-lock-mode", "x-amz-object-lock-retain-until-date",
                       "x-amz-server-side-encryption", "x-amz-server-side-encryption-aws-kms-key-id"]
ABORT_INCOMPLETE_UPLOADS = {"ID": "abort-incomplete-uploads", "Status": "Enabled", "Filter": {"Prefix": ""},
                            "AbortIncompleteMultipartUpload": {"DaysAfterInitiation": 1}}
# Every key is written once, so a noncurrent version is a repeated upload of a
# pending key or a plain delete. Retained versions are skipped while locked.
EXPIRE_NONCURRENT_VERSIONS = {"ID": "expire-noncurrent-versions", "Status": "Enabled", "Filter": {"Prefix": ""},
                              "NoncurrentVersionExpiration": {"NoncurrentDays": 1},
                              "Expiration": {"ExpiredObjectDeleteMarker": True}}
# RustFS 1.0.0-rc.5 rejects a bare "*" resource; for sts and admin actions this ARN matches everything.
ANY_RESOURCE = ["arn:aws:s3:::*"]


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
        {"Effect": "Allow" if issuer else "Deny", "Action": ["sts:AssumeRole"], "Resource": ANY_RESOURCE},
        # Deny self-service admin operations as well as broad administrative access.
        {"Effect": "Deny", "Action": ["admin:*"], "Resource": ANY_RESOURCE},
    ]}


def audit_policy(bucket):
    """Write and read only: no delete, no retention change and no governance bypass."""
    resource = f"arn:aws:s3:::{bucket}"
    return {"Version": "2012-10-17", "Statement": [
        {"Effect": "Allow", "Action": ["s3:ListBucket", "s3:GetBucketLocation"], "Resource": [resource]},
        {"Effect": "Allow", "Action": ["s3:GetObject", "s3:PutObject", "s3:AbortMultipartUpload",
         "s3:ListMultipartUploadParts"], "Resource": [resource + "/*"]},
        {"Effect": "Deny", "Action": ["s3:DeleteObject", "s3:DeleteObjectVersion", "s3:BypassGovernanceRetention",
         "s3:PutObjectRetention", "s3:PutObjectLegalHold", "s3:PutBucketObjectLockConfiguration"],
         "Resource": [resource, resource + "/*"]},
        {"Effect": "Deny", "Action": ["sts:AssumeRole"], "Resource": ANY_RESOURCE},
        {"Effect": "Deny", "Action": ["admin:*"], "Resource": ANY_RESOURCE},
    ]}


def locked_policy(bucket):
    """API-only access to single objects. Object Lock refuses deleting retained versions; nothing may bypass or reconfigure it.
    ListBucket makes HEAD on a never-uploaded key answer 404 instead of 403."""
    resource = f"arn:aws:s3:::{bucket}"
    return {"Version": "2012-10-17", "Statement": [
        {"Effect": "Allow", "Action": ["s3:ListBucket", "s3:GetBucketObjectLockConfiguration"], "Resource": [resource]},
        {"Effect": "Allow", "Action": ["s3:PutObject", "s3:GetObject", "s3:GetObjectVersion",
         "s3:PutObjectRetention", "s3:GetObjectRetention", "s3:PutObjectLegalHold", "s3:GetObjectLegalHold",
         "s3:DeleteObject", "s3:DeleteObjectVersion"], "Resource": [resource + "/*"]},
        {"Effect": "Deny", "Action": ["s3:BypassGovernanceRetention", "s3:PutBucketObjectLockConfiguration"],
         "Resource": [resource, resource + "/*"]},
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


def ensure_user(admin, users, key, password, *policy_names):
    """Attach only missing policies; any policy outside the expected set is drift."""
    expected = set(policy_names)
    names = set()
    if key in users:
        info = users[key]
        names = {name for name in info.get("policyName", "").split(",") if name}
        if not names <= expected or info.get("memberOf"):
            unexpected = ", ".join(sorted(names - expected))
            raise ValueError(f"IAM user policy or group drift ({unexpected or 'group membership'}); review the configured application user")
        if info.get("status") != "enabled":
            raise ValueError("Configured IAM user is disabled")
    else:
        admin.request("PUT", "/add-user?" + urllib.parse.urlencode({"accessKey": key}),
                      {"secretKey": password, "status": "enabled"})
    missing = sorted(expected - names)
    if missing:
        admin.request("POST", "/idp/builtin/policy/attach", {"policies": missing, "user": key})


def create_missing_bucket(client, bucket, region, object_lock=False):
    """Create the bucket unless it exists; report whether it was created."""
    try:
        client.head_bucket(Bucket=bucket)
        return False
    except ClientError as error:
        if error.response["ResponseMetadata"]["HTTPStatusCode"] != 404:
            raise
    kwargs = {"Bucket": bucket}
    if object_lock:
        kwargs["ObjectLockEnabledForBucket"] = True
    if region != "us-east-1":
        kwargs["CreateBucketConfiguration"] = {"LocationConstraint": region}
    client.create_bucket(**kwargs)
    return True


def reject_bucket_policy(client, bucket):
    # Existing public policy is drift, not an instruction to silently unpublish data.
    try:
        client.get_bucket_policy(Bucket=bucket)
    except ClientError as error:
        if error.response["Error"]["Code"] not in ("NoSuchBucketPolicy", "NoSuchPolicy", "404"):
            raise
    else:
        raise ValueError(f"Bucket {bucket} has a policy; bundled buckets must be private")


def ensure_bucket(client, bucket, region, origins, temporary=False):
    create_missing_bucket(client, bucket, region)
    reject_bucket_policy(client, bucket)
    if origins:
        client.put_bucket_cors(Bucket=bucket, CORSConfiguration={"CORSRules": [{
            "AllowedOrigins": origins, "AllowedMethods": ["GET", "PUT", "POST", "DELETE", "HEAD"],
            "AllowedHeaders": ["*"], "ExposeHeaders": ["ETag", "Content-Length", "Content-Range"],
            "MaxAgeSeconds": 3600,
        }]})
    else:
        client.delete_bucket_cors(Bucket=bucket)
    rules = [ABORT_INCOMPLETE_UPLOADS]
    if temporary:
        rules.append({"ID": "expire-temporary-content", "Status": "Enabled", "Filter": {"Prefix": "tmp/"},
                      "Expiration": {"Days": 2}})
    client.put_bucket_lifecycle_configuration(Bucket=bucket, LifecycleConfiguration={"Rules": rules})


def object_lock_configuration(client, bucket):
    try:
        return client.get_object_lock_configuration(Bucket=bucket)["ObjectLockConfiguration"]
    except ClientError as error:
        if error.response["Error"]["Code"] != "ObjectLockConfigurationNotFoundError":
            raise
        return {}


def ensure_audit_bucket(client, bucket, region, mode, years):
    """Object Lock can only be switched on at creation, so an existing bucket without it stops here."""
    if not create_missing_bucket(client, bucket, region, object_lock=True):
        configuration = object_lock_configuration(client, bucket)
        if configuration.get("ObjectLockEnabled") != "Enabled":
            raise ValueError(f"Audit bucket {bucket} exists without Object Lock; recreate it with Object Lock enabled")
        retention = configuration.get("Rule", {}).get("DefaultRetention", {})
        actual_mode = retention.get("Mode")
        sufficient_mode = actual_mode == "COMPLIANCE" or (mode == "GOVERNANCE" and actual_mode == "GOVERNANCE")
        sufficient_duration = retention.get("Years", 0) >= years or retention.get("Days", 0) >= years * 366
        if not sufficient_mode or not sufficient_duration:
            raise ValueError(f"Audit bucket {bucket} retention does not meet {mode} for {years} years; a storage administrator must review it")
        # Existing retention belongs to the storage administrator. Changing deployment
        # defaults must never alter an existing bucket's policy.
        reject_bucket_policy(client, bucket)
        return
    reject_bucket_policy(client, bucket)
    client.put_object_lock_configuration(Bucket=bucket, ObjectLockConfiguration={
        "ObjectLockEnabled": "Enabled", "Rule": {"DefaultRetention": {"Mode": mode, "Years": years}}})
    client.put_bucket_lifecycle_configuration(Bucket=bucket, LifecycleConfiguration={"Rules": [ABORT_INCOMPLETE_UPLOADS]})


def check_locked_retention(bucket, retention, enforcement, mode, days):
    """Object mode locks each version at commit, so a default rule would lock never-committed uploads."""
    if enforcement == "object":
        if retention:
            raise ValueError(f"Locked bucket {bucket} has a default retention rule; object enforcement needs Object Lock without one, or set LOCKED_BUCKET_ENFORCEMENT=bucket")
        return
    actual_mode = retention.get("Mode")
    sufficient_mode = actual_mode == "COMPLIANCE" or (mode == "GOVERNANCE" and actual_mode == "GOVERNANCE")
    sufficient_duration = retention.get("Days", 0) >= days or retention.get("Years", 0) * 365 >= days
    if not sufficient_mode or not sufficient_duration:
        raise ValueError(f"Locked bucket {bucket} default retention does not meet {mode} for {days} days; a storage administrator must review it")


def ensure_locked_bucket(client, bucket, region, origins, enforcement, mode, days):
    """Object Lock can only be switched on at creation. An existing bucket's lock rule is validated, never changed."""
    created = create_missing_bucket(client, bucket, region, object_lock=True)
    if not created:
        configuration = object_lock_configuration(client, bucket)
        if configuration.get("ObjectLockEnabled") != "Enabled":
            raise ValueError(f"Locked bucket {bucket} exists without Object Lock; recreate it with Object Lock enabled")
        check_locked_retention(bucket, configuration.get("Rule", {}).get("DefaultRetention", {}), enforcement, mode, days)
    reject_bucket_policy(client, bucket)
    if created and enforcement == "bucket":
        client.put_object_lock_configuration(Bucket=bucket, ObjectLockConfiguration={
            "ObjectLockEnabled": "Enabled", "Rule": {"DefaultRetention": {"Mode": mode, "Days": days}}})
    client.put_bucket_lifecycle_configuration(Bucket=bucket, LifecycleConfiguration={
        "Rules": [ABORT_INCOMPLETE_UPLOADS, EXPIRE_NONCURRENT_VERSIONS]})
    if origins:
        client.put_bucket_cors(Bucket=bucket, CORSConfiguration={"CORSRules": [{
            "AllowedOrigins": origins, "AllowedMethods": ["GET", "PUT", "HEAD"],
            "AllowedHeaders": LOCKED_CORS_HEADERS, "ExposeHeaders": ["ETag", "x-amz-version-id"],
            "MaxAgeSeconds": 3600,
        }]})
    else:
        client.delete_bucket_cors(Bucket=bucket)


def ensure_locked_storage(admin, policies, client, region, origins, locked):
    """The extra policy names for the API user. Only the API signs locked-storage URLs;
    the issuer and its executor sessions never reach that bucket."""
    if not locked:
        return ()
    ensure_locked_bucket(client, locked["bucket"], region, origins, locked["enforcement"], locked["mode"], locked["days"])
    ensure_policy(admin, policies, LOCKED_POLICY, locked_policy(locked["bucket"]))
    return (LOCKED_POLICY,)


def option(name, default, options):
    """Case-insensitive choice, returned in the spelling `options` uses."""
    value = setting(name, default)
    for candidate in options:
        if value.lower() == candidate.lower():
            return candidate
    raise ValueError(f"{name} must be {' or '.join(options)}")


def whole_number(name, default, low, high):
    value = setting(name, default)
    if not value.isdigit() or not low <= int(value) <= high:
        raise ValueError(f"{name} must be a whole number from {low} to {high}")
    return int(value)


def audit_configuration(buckets, identities):
    """Optional audit bucket with its own write-only identity; None when AUDIT_BUCKET is unset."""
    bucket = setting("AUDIT_BUCKET")
    if not bucket:
        return None
    if not BUCKET_NAME.fullmatch(bucket) or bucket in buckets:
        raise ValueError("AUDIT_BUCKET must be a valid S3 bucket name separate from the application buckets")
    mode = option("AUDIT_BUCKET_LOCK_MODE", "COMPLIANCE", LOCK_MODES)
    years = whole_number("AUDIT_BUCKET_RETENTION_YEARS", "4", 1, 100)
    identity = (secret("AUDIT_BUCKET_ACCESS_KEY_ID"), secret("AUDIT_BUCKET_SECRET_ACCESS_KEY"))
    if identity[0] in {key for key, _ in identities} or identity[1] in {password for _, password in identities}:
        raise ValueError("The audit identity must have its own access key and secret")
    if len(identity[1]) < 16:
        raise ValueError("AUDIT_BUCKET_SECRET_ACCESS_KEY must have at least 16 characters")
    return {"bucket": bucket, "mode": mode, "years": years, "identity": identity}


def locked_configuration(buckets, audit):
    """Optional revision-safe bucket for the API identity; None when LOCKED_BUCKET is unset."""
    bucket = setting("LOCKED_BUCKET")
    if not bucket:
        return None
    if not GATEWAY_BUCKET_NAME.fullmatch(bucket) or bucket in buckets or (audit and bucket == audit["bucket"]):
        raise ValueError("LOCKED_BUCKET must be a lowercase DNS bucket name without periods, separate from the application and audit buckets")
    return {"bucket": bucket, "enforcement": option("LOCKED_BUCKET_ENFORCEMENT", "object", LOCKED_ENFORCEMENTS),
            "mode": option("LOCKED_BUCKET_LOCK_MODE", "COMPLIANCE", LOCK_MODES),
            "days": whole_number("LOCKED_BUCKET_RETENTION_DAYS", "3650", 1, 36500)}


def configuration():
    endpoint = setting("S3_INTERNAL_ENDPOINT", "http://object-store:9000")
    parsed = urllib.parse.urlsplit(endpoint)
    if parsed.scheme not in ("http", "https") or not parsed.hostname or parsed.path not in ("", "/") or parsed.query or parsed.fragment or parsed.username:
        raise ValueError("S3_INTERNAL_ENDPOINT must be an http(s) origin")
    buckets = [setting("META_BUCKET"), setting("CONTENT_BUCKET"), setting("LOG_BUCKET")]
    if any(not bucket or not BUCKET_NAME.fullmatch(bucket) for bucket in buckets):
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
    audit = audit_configuration(buckets, (root, api, issuer))
    locked = locked_configuration(buckets, audit)
    return endpoint, setting("AWS_REGION", "us-east-1"), buckets, origins, root, api, issuer, audit, locked


def main():
    endpoint, region, buckets, origins, root, api, issuer, audit, locked = configuration()
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
    api_names = ("flow-like-api-v1", *ensure_locked_storage(admin, policies, client, region, origins, locked))
    for identity, is_issuer, names in ((api, False, api_names), (issuer, True, ("flow-like-issuer-v1",))):
        ensure_policy(admin, policies, names[0], base_policy(buckets, is_issuer))
        ensure_user(admin, users, *identity, *names)
        # Existing user secrets are not overwritten. A mismatch requires explicit rotation.
        s3_client(endpoint, region, *identity).head_bucket(Bucket=buckets[1])
    if locked:
        s3_client(endpoint, region, *api).get_object_lock_configuration(Bucket=locked["bucket"])
    if audit:
        ensure_audit_bucket(client, audit["bucket"], region, audit["mode"], audit["years"])
        ensure_policy(admin, policies, "flow-like-audit-v1", audit_policy(audit["bucket"]))
        ensure_user(admin, users, *audit["identity"], "flow-like-audit-v1")
        s3_client(endpoint, region, *audit["identity"]).head_bucket(Bucket=audit["bucket"])
    print("Private buckets and restricted API/STS/audit users are ready.")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        # SDK/admin errors can contain request details; never print bodies or credentials.
        print(f"Object-store initialization failed ({type(error).__name__}). Review private initializer diagnostics and configuration.", file=sys.stderr)
        if isinstance(error, (ValueError, RuntimeError)):
            print(str(error), file=sys.stderr)
        sys.exit(1)
