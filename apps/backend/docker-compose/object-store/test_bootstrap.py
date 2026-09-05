"""Offline tests for bootstrap invariants and real SDK request validation."""
import copy
import json
import os
import tempfile
import unittest
from unittest.mock import Mock, patch

from botocore.stub import Stubber

from bootstrap import (base_policy, configuration, ensure_bucket,
                       ensure_policy, ensure_user, s3_client, secret)


class BootstrapTests(unittest.TestCase):
    def setUp(self):
        self.env = {
            "META_BUCKET": "flow-like-meta", "CONTENT_BUCKET": "flow-like-content", "LOG_BUCKET": "flow-like-logs",
            "RUSTFS_ROOT_USER": "root-key", "RUSTFS_ROOT_PASSWORD": "root-password-for-test",
            "AWS_ACCESS_KEY_ID": "api-key", "AWS_SECRET_ACCESS_KEY": "api-password-for-test",
            "STS_ISSUER_ACCESS_KEY": "issuer-key", "STS_ISSUER_SECRET_KEY": "issuer-password-for-test",
            "S3_CORS_ALLOWED_ORIGINS": "http://localhost:3000,https://app.example.com",
        }

    def test_separate_identities_and_exact_origins_required(self):
        with patch.dict(os.environ, self.env, clear=True):
            self.assertEqual(configuration()[2], ["flow-like-meta", "flow-like-content", "flow-like-logs"])
            with patch.dict(os.environ, {"AWS_ACCESS_KEY_ID": "root-key"}):
                self.assertRaises(ValueError, configuration)
            for origin in ("*", "https://*.example.com", "https://app.example.com/path", "https://user@host"):
                with patch.dict(os.environ, {"S3_CORS_ALLOWED_ORIGINS": origin}):
                    self.assertRaises(ValueError, configuration)
            with patch.dict(os.environ, {"META_BUCKET": "flow-like-content"}):
                self.assertRaises(ValueError, configuration)

    def test_secret_files_preserve_special_characters_and_fail_closed(self):
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8") as handle:
            handle.write(" secret $&'\\\\value \n")
            handle.flush()
            with patch.dict(os.environ, {"TEST_SECRET_FILE": handle.name, "TEST_SECRET": "ignored"}, clear=True):
                self.assertEqual(secret("TEST_SECRET"), " secret $&'\\\\value ")
            with patch.dict(os.environ, {"TEST_SECRET_FILE": "/missing/secret", "TEST_SECRET": "fallback"}, clear=True):
                self.assertRaises(FileNotFoundError, secret, "TEST_SECRET")

    def test_policies_deny_admin_and_keep_buckets_explicit(self):
        for issuer in (False, True):
            policy = base_policy(["meta", "content", "logs"], issuer)
            s3 = [s for s in policy["Statement"] if s["Action"][0].startswith("s3:")]
            self.assertTrue(all("*" not in statement["Resource"] for statement in s3))
            self.assertIn({"Effect": "Deny", "Action": ["admin:*"], "Resource": ["*"]}, policy["Statement"])
            self.assertIn({"Effect": "Allow" if issuer else "Deny", "Action": ["sts:AssumeRole"], "Resource": ["*"]}, policy["Statement"])

    def test_policy_idempotency_and_drift(self):
        admin = Mock()
        policy = base_policy(["meta", "content", "logs"], True)
        reordered = copy.deepcopy(policy)
        reordered["Statement"].reverse()
        reordered["Statement"][0]["Sid"] = ""
        ensure_policy(admin, {"issuer": reordered}, "issuer", policy)
        admin.request.assert_not_called()
        drifted = copy.deepcopy(policy)
        drifted["Statement"][0]["Resource"] = ["*"]
        self.assertRaises(ValueError, ensure_policy, admin, {"issuer": drifted}, "issuer", policy)
        admin.request.assert_not_called()
        ensure_policy(admin, {}, "issuer", policy)
        admin.request.assert_called_once()

    def test_user_idempotency_and_membership_drift(self):
        admin = Mock()
        existing = {"api": {"status": "enabled", "policyName": "flow-like-api-v1", "memberOf": []}}
        ensure_user(admin, existing, "api", "unused", "flow-like-api-v1")
        admin.request.assert_not_called()
        for info in ({"status": "enabled", "policyName": "flow-like-api-v1,consoleAdmin"},
                     {"status": "enabled", "policyName": "flow-like-api-v1", "memberOf": ["admins"]},
                     {"status": "disabled", "policyName": "flow-like-api-v1"}):
            self.assertRaises(ValueError, ensure_user, admin, {"api": info}, "api", "unused", "flow-like-api-v1")
        admin.request.assert_not_called()

    def test_existing_bucket_policy_blocks_mutation(self):
        client = s3_client("http://127.0.0.1:1", "us-east-1", "test", "test-secret")
        with Stubber(client) as stub:
            stub.add_response("head_bucket", {}, {"Bucket": "flow-like-meta"})
            stub.add_response("get_bucket_policy", {"Policy": json.dumps({"Statement": []})}, {"Bucket": "flow-like-meta"})
            self.assertRaises(ValueError, ensure_bucket, client, "flow-like-meta", "us-east-1", [], False)
            stub.assert_no_pending_responses()

    def test_new_content_bucket_private_cors_and_lifecycle(self):
        client = s3_client("http://127.0.0.1:1", "us-east-1", "test", "test-secret")
        with Stubber(client) as stub:
            stub.add_client_error("head_bucket", service_error_code="404", http_status_code=404, expected_params={"Bucket": "flow-like-content"})
            stub.add_response("create_bucket", {}, {"Bucket": "flow-like-content"})
            stub.add_client_error("get_bucket_policy", service_error_code="NoSuchBucketPolicy", http_status_code=404, expected_params={"Bucket": "flow-like-content"})
            stub.add_response("delete_bucket_cors", {}, {"Bucket": "flow-like-content"})
            stub.add_response("put_bucket_lifecycle_configuration", {}, {"Bucket": "flow-like-content", "LifecycleConfiguration": {"Rules": [
                {"ID": "abort-incomplete-uploads", "Status": "Enabled", "Filter": {"Prefix": ""}, "AbortIncompleteMultipartUpload": {"DaysAfterInitiation": 1}},
                {"ID": "expire-temporary-content", "Status": "Enabled", "Filter": {"Prefix": "tmp/"}, "Expiration": {"Days": 2}},
            ]}})
            ensure_bucket(client, "flow-like-content", "us-east-1", [], True)
            stub.assert_no_pending_responses()


if __name__ == "__main__":
    unittest.main()
