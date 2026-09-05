"""Opt-in destructive authorization checks in unique temporary test prefixes.

Run against a disposable or backed-up store after bootstrap. Uses only the API
and restricted issuer identities. Every created object is removed in finally.
This checks a single node's prefix boundaries; it does not qualify expiry,
failover, backups, multipart-copy, or the complete Flow-Like application.
"""
import datetime
import json
import sys
import urllib.error
import urllib.request
import uuid

import boto3
from botocore.config import Config
from botocore.exceptions import ClientError

from bootstrap import Admin, s3_client, secret, setting


def denied(call, description):
    try:
        call()
    except ClientError as error:
        if error.response["ResponseMetadata"]["HTTPStatusCode"] == 403:
            return
        raise AssertionError(f"{description}: expected authorization denial") from None
    except urllib.error.HTTPError as error:
        if error.code == 403:
            return
        raise AssertionError(f"{description}: expected HTTP 403, got {error.code}") from None
    raise AssertionError(f"{description}: request unexpectedly succeeded")


def main():
    endpoint = setting("S3_INTERNAL_ENDPOINT", "http://object-store:9000")
    public_endpoint = setting("S3_PUBLIC_ENDPOINT", endpoint)
    region = setting("AWS_REGION", "us-east-1")
    bucket = setting("CONTENT_BUCKET")
    api_key, api_secret = secret("AWS_ACCESS_KEY_ID"), secret("AWS_SECRET_ACCESS_KEY")
    issuer_key, issuer_secret = secret("STS_ISSUER_ACCESS_KEY"), secret("STS_ISSUER_SECRET_KEY")
    master = s3_client(endpoint, region, api_key, api_secret)
    sts = boto3.client("sts", endpoint_url=setting("STS_ENDPOINT_URL", endpoint), region_name=region,
                       aws_access_key_id=issuer_key, aws_secret_access_key=issuer_secret,
                       config=Config(retries={"max_attempts": 0}))
    prefix = "tmp/qualification/" + uuid.uuid4().hex
    allowed, sibling = prefix + "/app-a/", prefix + "/app-b/"
    keys = [allowed + "input", sibling + "input", allowed + "output", allowed + "copy", allowed + "presigned"]
    policy = {"Version": "2012-10-17", "Statement": [
        {"Effect": "Allow", "Action": ["s3:ListBucket"], "Resource": [f"arn:aws:s3:::{bucket}"],
         "Condition": {"StringLike": {"s3:prefix": [allowed + "*"]}}},
        {"Effect": "Allow", "Action": ["s3:GetObject", "s3:PutObject", "s3:DeleteObject",
         "s3:AbortMultipartUpload", "s3:ListMultipartUploadParts"],
         "Resource": [f"arn:aws:s3:::{bucket}/{allowed}*"]},
        {"Effect": "Deny", "Action": ["s3:GetObject"], "Resource": [f"arn:aws:s3:::{bucket}/{allowed}denied"]},
    ]}
    text = json.dumps(policy, separators=(",", ":"))
    assert len(text.encode()) <= 2048
    role_args = {"RoleArn": "arn:aws:iam::000000000000:role/flow-like-runtime", "RoleSessionName": "flow-like-qualification",
                 "DurationSeconds": 900, "Policy": text}
    credentials = sts.assume_role(**role_args)["Credentials"]
    assert credentials["Expiration"] > datetime.datetime.now(datetime.timezone.utc)
    key, password, token = (credentials[name] for name in ("AccessKeyId", "SecretAccessKey", "SessionToken"))
    scoped = s3_client(public_endpoint, region, key, password, token)
    internal = s3_client(endpoint, region, key, password, token)
    try:
        for name in keys[:2]:
            master.put_object(Bucket=bucket, Key=name, Body=b"qualification")
        assert scoped.get_object(Bucket=bucket, Key=keys[0])["Body"].read() == b"qualification"
        scoped.put_object(Bucket=bucket, Key=keys[2], Body=b"output")
        assert all(item["Key"].startswith(allowed) for item in scoped.list_objects_v2(Bucket=bucket, Prefix=allowed).get("Contents", []))
        denied(lambda: scoped.get_object(Bucket=bucket, Key=keys[1]), "sibling GET")
        denied(lambda: scoped.put_object(Bucket=bucket, Key=sibling + "output", Body=b"escape"), "sibling PUT")
        denied(lambda: scoped.put_object(Bucket=bucket, Key=allowed.rstrip("/") + "-confusable/input", Body=b"escape"), "prefix-confusable PUT")
        denied(lambda: scoped.list_objects_v2(Bucket=bucket), "empty-prefix LIST")
        denied(lambda: scoped.list_objects_v2(Bucket=bucket, Prefix=sibling), "sibling LIST")
        denied(lambda: scoped.get_object(Bucket=bucket, Key=allowed + "denied"), "explicit deny")
        for name in (setting("META_BUCKET"), setting("LOG_BUCKET")):
            denied(lambda name=name: scoped.put_object(Bucket=name, Key=allowed + "output", Body=b"escape"), "other bucket PUT")
        denied(lambda: scoped.copy_object(Bucket=bucket, Key=keys[3], CopySource={"Bucket": bucket, "Key": keys[1]}), "cross-prefix COPY source")
        scoped.copy_object(Bucket=bucket, Key=keys[3], CopySource={"Bucket": bucket, "Key": keys[0]})
        upload = scoped.create_multipart_upload(Bucket=bucket, Key=keys[2])["UploadId"]
        try:
            scoped.upload_part(Bucket=bucket, Key=keys[2], UploadId=upload, PartNumber=1, Body=b"multipart")
            scoped.list_parts(Bucket=bucket, Key=keys[2], UploadId=upload)
        finally:
            scoped.abort_multipart_upload(Bucket=bucket, Key=keys[2], UploadId=upload)
        for bad_token in (None, "invalid-token"):
            invalid = s3_client(endpoint, region, key, password, bad_token)
            denied(lambda: invalid.get_object(Bucket=bucket, Key=keys[0]), "invalid/missing session token")
        # Test denial directly on RustFS as well as through the network gateway.
        child_sts = boto3.client("sts", endpoint_url=endpoint, region_name=region,
                                 aws_access_key_id=key, aws_secret_access_key=password, aws_session_token=token)
        denied(lambda: child_sts.assume_role(**role_args), "session STS escalation")
        api_sts = boto3.client("sts", endpoint_url=endpoint, region_name=region,
                               aws_access_key_id=api_key, aws_secret_access_key=api_secret)
        denied(lambda: api_sts.assume_role(**role_args), "API user STS issuance")
        denied(lambda: Admin(endpoint, region, key, password, token).request("GET", "/list-users"), "session admin access")
        denied(lambda: Admin(endpoint, region, issuer_key, issuer_secret).request("GET", "/list-users"), "issuer admin access")
        denied(lambda: internal.list_buckets(), "session list all buckets")
        url = scoped.generate_presigned_url("get_object", Params={"Bucket": bucket, "Key": keys[0]}, ExpiresIn=60)
        with urllib.request.urlopen(url, timeout=30) as response:
            assert response.read() == b"qualification"
        url = scoped.generate_presigned_url("put_object", Params={"Bucket": bucket, "Key": keys[4]}, ExpiresIn=60)
        with urllib.request.urlopen(urllib.request.Request(url, data=b"presigned", method="PUT"), timeout=30):
            pass
        print("Single-node STS prefix, copy-source, token, admin and presigned URL checks passed.")
    finally:
        # Also clean unexpected writes if a denial regression was found.
        for cleanup_bucket in (bucket, setting("META_BUCKET"), setting("LOG_BUCKET")):
            for page in master.get_paginator("list_objects_v2").paginate(Bucket=cleanup_bucket, Prefix=prefix + "/"):
                for item in page.get("Contents", []):
                    master.delete_object(Bucket=cleanup_bucket, Key=item["Key"])



if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"Object-store conformance failed ({type(error).__name__}).", file=sys.stderr)
        if isinstance(error, AssertionError):
            print(str(error), file=sys.stderr)
        sys.exit(1)
