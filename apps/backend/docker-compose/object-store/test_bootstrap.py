"""Offline tests for bootstrap invariants and real SDK request validation."""
import copy
import json
import os
import tempfile
import unittest
from unittest.mock import Mock, patch

from botocore.stub import Stubber

from bootstrap import (LOCKED_CORS_HEADERS, audit_policy, base_policy, configuration, ensure_audit_bucket,
                       ensure_bucket, ensure_locked_bucket, ensure_policy, ensure_user, locked_policy,
                       s3_client, secret)

LOCKED = {"Bucket": "flow-like-locked"}
NO_POLICY = {"service_error_code": "NoSuchBucketPolicy", "http_status_code": 404, "expected_params": LOCKED}
LOCKED_LIFECYCLE = {**LOCKED, "LifecycleConfiguration": {"Rules": [
    {"ID": "abort-incomplete-uploads", "Status": "Enabled", "Filter": {"Prefix": ""}, "AbortIncompleteMultipartUpload": {"DaysAfterInitiation": 1}},
    {"ID": "expire-noncurrent-versions", "Status": "Enabled", "Filter": {"Prefix": ""},
     "NoncurrentVersionExpiration": {"NoncurrentDays": 1}, "Expiration": {"ExpiredObjectDeleteMarker": True}},
]}}
LOCKED_CORS = {**LOCKED, "CORSConfiguration": {"CORSRules": [{
    "AllowedOrigins": ["https://app.example.com"], "AllowedMethods": ["GET", "PUT", "HEAD"],
    "AllowedHeaders": LOCKED_CORS_HEADERS, "ExposeHeaders": ["ETag", "x-amz-version-id"], "MaxAgeSeconds": 3600,
}]}}


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
            self.assertIn({"Effect": "Deny", "Action": ["admin:*"], "Resource": ["arn:aws:s3:::*"]}, policy["Statement"])
            self.assertIn({"Effect": "Allow" if issuer else "Deny", "Action": ["sts:AssumeRole"], "Resource": ["arn:aws:s3:::*"]}, policy["Statement"])

    def test_no_policy_uses_a_bare_star_resource(self):
        # RustFS 1.0.0-rc.5 rejects the whole canned policy with 400 InvalidRequest.
        for policy in (base_policy(["meta", "content", "logs"], False), base_policy(["meta", "content", "logs"], True),
                       audit_policy("flow-like-audit"), locked_policy("flow-like-locked")):
            self.assertFalse([statement for statement in policy["Statement"] if "*" in statement["Resource"]])

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

    def test_locked_policy_is_attached_to_existing_and_new_users_without_drift(self):
        admin = Mock()
        existing = {"api": {"status": "enabled", "policyName": "flow-like-api-v1", "memberOf": []}}
        ensure_user(admin, existing, "api", "unused", "flow-like-api-v1", "flow-like-locked-v1")
        admin.request.assert_called_once_with("POST", "/idp/builtin/policy/attach",
                                              {"policies": ["flow-like-locked-v1"], "user": "api"})
        admin = Mock()
        both = {"api": {"status": "enabled", "policyName": "flow-like-locked-v1,flow-like-api-v1"}}
        ensure_user(admin, both, "api", "unused", "flow-like-api-v1", "flow-like-locked-v1")
        admin.request.assert_not_called()
        self.assertRaises(ValueError, ensure_user, admin, both, "api", "unused", "flow-like-api-v1")
        admin.request.assert_not_called()
        ensure_user(admin, {}, "api", "api-password-for-test", "flow-like-api-v1", "flow-like-locked-v1")
        self.assertEqual(admin.request.call_args_list[-1].args,
                         ("POST", "/idp/builtin/policy/attach",
                          {"policies": ["flow-like-api-v1", "flow-like-locked-v1"], "user": "api"}))

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

    def test_audit_bucket_is_optional_separate_and_has_its_own_identity(self):
        audit = {"AUDIT_BUCKET": "flow-like-audit", "AUDIT_BUCKET_ACCESS_KEY_ID": "audit-key",
                 "AUDIT_BUCKET_SECRET_ACCESS_KEY": "audit-password-for-test"}
        with patch.dict(os.environ, self.env, clear=True):
            self.assertIsNone(configuration()[7])
            with patch.dict(os.environ, audit):
                self.assertEqual(configuration()[7], {"bucket": "flow-like-audit", "mode": "COMPLIANCE", "years": 4,
                                                      "identity": ("audit-key", "audit-password-for-test")})
                for change in ({"AUDIT_BUCKET": "flow-like-logs"}, {"AUDIT_BUCKET_ACCESS_KEY_ID": "api-key"},
                               {"AUDIT_BUCKET_SECRET_ACCESS_KEY": "api-password-for-test"},
                               {"AUDIT_BUCKET_LOCK_MODE": "legal-hold"}, {"AUDIT_BUCKET_RETENTION_YEARS": "0"}):
                    with patch.dict(os.environ, change):
                        self.assertRaises(ValueError, configuration)
                with patch.dict(os.environ, {"AUDIT_BUCKET_LOCK_MODE": "compliance", "AUDIT_BUCKET_RETENTION_YEARS": "6"}):
                    self.assertEqual(configuration()[7]["mode"], "COMPLIANCE")
                    self.assertEqual(configuration()[7]["years"], 6)

    def test_audit_policy_writes_but_never_deletes_or_bypasses_retention(self):
        policy = audit_policy("flow-like-audit")
        allowed = {action for statement in policy["Statement"] if statement["Effect"] == "Allow" for action in statement["Action"]}
        denied = {action for statement in policy["Statement"] if statement["Effect"] == "Deny" for action in statement["Action"]}
        self.assertIn("s3:PutObject", allowed)
        self.assertFalse({action for action in allowed if "Delete" in action or "Retention" in action or "Bypass" in action})
        self.assertTrue({"s3:DeleteObject", "s3:DeleteObjectVersion", "s3:BypassGovernanceRetention"} <= denied)
        self.assertTrue(all("*" not in statement["Resource"] for statement in policy["Statement"] if statement["Effect"] == "Allow"))

    def test_new_audit_bucket_is_created_with_object_lock_and_default_retention(self):
        client = s3_client("http://127.0.0.1:1", "us-east-1", "test", "test-secret")
        bucket = {"Bucket": "flow-like-audit"}
        with Stubber(client) as stub:
            stub.add_client_error("head_bucket", service_error_code="404", http_status_code=404, expected_params=bucket)
            stub.add_response("create_bucket", {}, {**bucket, "ObjectLockEnabledForBucket": True})
            stub.add_client_error("get_bucket_policy", service_error_code="NoSuchBucketPolicy", http_status_code=404, expected_params=bucket)
            stub.add_response("put_object_lock_configuration", {}, {**bucket, "ObjectLockConfiguration": {
                "ObjectLockEnabled": "Enabled", "Rule": {"DefaultRetention": {"Mode": "GOVERNANCE", "Years": 3}}}})
            stub.add_response("put_bucket_lifecycle_configuration", {}, {**bucket, "LifecycleConfiguration": {"Rules": [
                {"ID": "abort-incomplete-uploads", "Status": "Enabled", "Filter": {"Prefix": ""}, "AbortIncompleteMultipartUpload": {"DaysAfterInitiation": 1}},
            ]}})
            ensure_audit_bucket(client, "flow-like-audit", "us-east-1", "GOVERNANCE", 3)
            stub.assert_no_pending_responses()

    def test_existing_audit_bucket_retention_is_never_reconfigured(self):
        client = s3_client("http://127.0.0.1:1", "us-east-1", "test", "test-secret")
        with Stubber(client) as stub:
            stub.add_response("head_bucket", {}, {"Bucket": "flow-like-audit"})
            stub.add_response("get_object_lock_configuration", {"ObjectLockConfiguration": {
                "ObjectLockEnabled": "Enabled", "Rule": {"DefaultRetention": {"Mode": "COMPLIANCE", "Years": 5}}}},
                {"Bucket": "flow-like-audit"})
            stub.add_client_error("get_bucket_policy", service_error_code="NoSuchBucketPolicy", http_status_code=404,
                                  expected_params={"Bucket": "flow-like-audit"})
            ensure_audit_bucket(client, "flow-like-audit", "us-east-1", "COMPLIANCE", 4)
            stub.assert_no_pending_responses()

    def test_existing_audit_bucket_weaker_retention_is_rejected_without_mutation(self):
        for retention in ({"Mode": "GOVERNANCE", "Years": 4}, {"Mode": "COMPLIANCE", "Years": 3}, {}):
            client = s3_client("http://127.0.0.1:1", "us-east-1", "test", "test-secret")
            with Stubber(client) as stub:
                stub.add_response("head_bucket", {}, {"Bucket": "flow-like-audit"})
                stub.add_response("get_object_lock_configuration", {"ObjectLockConfiguration": {
                    "ObjectLockEnabled": "Enabled", "Rule": {"DefaultRetention": retention}}}, {"Bucket": "flow-like-audit"})
                self.assertRaises(ValueError, ensure_audit_bucket, client, "flow-like-audit", "us-east-1", "COMPLIANCE", 4)
                stub.assert_no_pending_responses()

    def test_existing_audit_bucket_without_object_lock_stops_initialization(self):
        client = s3_client("http://127.0.0.1:1", "us-east-1", "test", "test-secret")
        with Stubber(client) as stub:
            stub.add_response("head_bucket", {}, {"Bucket": "flow-like-audit"})
            stub.add_client_error("get_object_lock_configuration", service_error_code="ObjectLockConfigurationNotFoundError",
                                  http_status_code=404, expected_params={"Bucket": "flow-like-audit"})
            self.assertRaises(ValueError, ensure_audit_bucket, client, "flow-like-audit", "us-east-1", "GOVERNANCE", 3)
            stub.assert_no_pending_responses()

    def test_locked_bucket_is_optional_validated_and_separate(self):
        with patch.dict(os.environ, self.env, clear=True):
            self.assertIsNone(configuration()[8])
            with patch.dict(os.environ, {"LOCKED_BUCKET": "flow-like-locked"}):
                self.assertEqual(configuration()[8], {"bucket": "flow-like-locked", "enforcement": "object",
                                                      "mode": "COMPLIANCE", "days": 3650})
                with patch.dict(os.environ, {"LOCKED_BUCKET_ENFORCEMENT": "Bucket", "LOCKED_BUCKET_LOCK_MODE": "governance",
                                             "LOCKED_BUCKET_RETENTION_DAYS": "2190"}):
                    self.assertEqual(configuration()[8], {"bucket": "flow-like-locked", "enforcement": "bucket",
                                                          "mode": "GOVERNANCE", "days": 2190})
                for change in ({"LOCKED_BUCKET": "flow-like-logs"}, {"LOCKED_BUCKET": "flow.like.locked"},
                               {"LOCKED_BUCKET": "Flow-Like-Locked"}, {"LOCKED_BUCKET_ENFORCEMENT": "prefix"},
                               {"LOCKED_BUCKET_LOCK_MODE": "legal-hold"}, {"LOCKED_BUCKET_RETENTION_DAYS": "0"},
                               {"LOCKED_BUCKET_RETENTION_DAYS": "36501"}, {"LOCKED_BUCKET_RETENTION_DAYS": "10y"}):
                    with patch.dict(os.environ, change):
                        self.assertRaises(ValueError, configuration)
            audit = {"AUDIT_BUCKET": "flow-like-audit", "AUDIT_BUCKET_ACCESS_KEY_ID": "audit-key",
                     "AUDIT_BUCKET_SECRET_ACCESS_KEY": "audit-password-for-test", "LOCKED_BUCKET": "flow-like-audit"}
            with patch.dict(os.environ, audit):
                self.assertRaises(ValueError, configuration)

    def test_locked_policy_grants_single_objects_but_never_bypass_or_lock_changes(self):
        policy = locked_policy("flow-like-locked")
        allowed = {action for statement in policy["Statement"] if statement["Effect"] == "Allow" for action in statement["Action"]}
        denied = {action for statement in policy["Statement"] if statement["Effect"] == "Deny" for action in statement["Action"]}
        self.assertEqual(allowed, {"s3:PutObject", "s3:GetObject", "s3:GetObjectVersion", "s3:PutObjectRetention",
                                   "s3:GetObjectRetention", "s3:PutObjectLegalHold", "s3:GetObjectLegalHold",
                                   "s3:DeleteObject", "s3:DeleteObjectVersion", "s3:ListBucket",
                                   "s3:GetBucketObjectLockConfiguration"})
        self.assertEqual(denied, {"s3:BypassGovernanceRetention", "s3:PutBucketObjectLockConfiguration"})
        resources = {resource for statement in policy["Statement"] for resource in statement["Resource"]}
        self.assertEqual(resources, {"arn:aws:s3:::flow-like-locked", "arn:aws:s3:::flow-like-locked/*"})

    def test_new_object_mode_locked_bucket_has_object_lock_without_default_rule(self):
        client = s3_client("http://127.0.0.1:1", "us-east-1", "test", "test-secret")
        with Stubber(client) as stub:
            stub.add_client_error("head_bucket", service_error_code="404", http_status_code=404, expected_params=LOCKED)
            stub.add_response("create_bucket", {}, {**LOCKED, "ObjectLockEnabledForBucket": True})
            stub.add_client_error("get_bucket_policy", **NO_POLICY)
            stub.add_response("put_bucket_lifecycle_configuration", {}, LOCKED_LIFECYCLE)
            stub.add_response("put_bucket_cors", {}, LOCKED_CORS)
            ensure_locked_bucket(client, "flow-like-locked", "us-east-1", ["https://app.example.com"], "object", "COMPLIANCE", 3650)
            stub.assert_no_pending_responses()

    def test_new_bucket_mode_locked_bucket_gets_default_retention_in_days(self):
        client = s3_client("http://127.0.0.1:1", "us-east-1", "test", "test-secret")
        with Stubber(client) as stub:
            stub.add_client_error("head_bucket", service_error_code="404", http_status_code=404, expected_params=LOCKED)
            stub.add_response("create_bucket", {}, {**LOCKED, "ObjectLockEnabledForBucket": True})
            stub.add_client_error("get_bucket_policy", **NO_POLICY)
            stub.add_response("put_object_lock_configuration", {}, {**LOCKED, "ObjectLockConfiguration": {
                "ObjectLockEnabled": "Enabled", "Rule": {"DefaultRetention": {"Mode": "COMPLIANCE", "Days": 3650}}}})
            stub.add_response("put_bucket_lifecycle_configuration", {}, LOCKED_LIFECYCLE)
            stub.add_response("delete_bucket_cors", {}, LOCKED)
            ensure_locked_bucket(client, "flow-like-locked", "us-east-1", [], "bucket", "COMPLIANCE", 3650)
            stub.assert_no_pending_responses()

    def test_existing_locked_bucket_rule_is_validated_and_never_reconfigured(self):
        for enforcement, retention in (("object", {}), ("bucket", {"Mode": "COMPLIANCE", "Years": 10}),
                                       ("bucket", {"Mode": "COMPLIANCE", "Days": 4000})):
            client = s3_client("http://127.0.0.1:1", "us-east-1", "test", "test-secret")
            rule = {"Rule": {"DefaultRetention": retention}} if retention else {}
            with Stubber(client) as stub:
                stub.add_response("head_bucket", {}, LOCKED)
                stub.add_response("get_object_lock_configuration", {"ObjectLockConfiguration": {
                    "ObjectLockEnabled": "Enabled", **rule}}, LOCKED)
                stub.add_client_error("get_bucket_policy", **NO_POLICY)
                # The noncurrent-version rule reaches buckets created before it existed.
                stub.add_response("put_bucket_lifecycle_configuration", {}, LOCKED_LIFECYCLE)
                stub.add_response("put_bucket_cors", {}, LOCKED_CORS)
                ensure_locked_bucket(client, "flow-like-locked", "us-east-1", ["https://app.example.com"], enforcement, "COMPLIANCE", 3650)
                stub.assert_no_pending_responses()

    def test_existing_locked_bucket_with_wrong_lock_state_stops_without_mutation(self):
        for enforcement, lock in (("object", {"ObjectLockEnabled": "Enabled", "Rule": {"DefaultRetention": {"Mode": "GOVERNANCE", "Days": 1}}}),
                                  ("bucket", {"ObjectLockEnabled": "Enabled"}),
                                  ("bucket", {"ObjectLockEnabled": "Enabled", "Rule": {"DefaultRetention": {"Mode": "GOVERNANCE", "Days": 3650}}}),
                                  ("bucket", {"ObjectLockEnabled": "Enabled", "Rule": {"DefaultRetention": {"Mode": "COMPLIANCE", "Days": 365}}})):
            client = s3_client("http://127.0.0.1:1", "us-east-1", "test", "test-secret")
            with Stubber(client) as stub:
                stub.add_response("head_bucket", {}, LOCKED)
                stub.add_response("get_object_lock_configuration", {"ObjectLockConfiguration": lock}, LOCKED)
                self.assertRaises(ValueError, ensure_locked_bucket, client, "flow-like-locked", "us-east-1", [], enforcement, "COMPLIANCE", 3650)
                stub.assert_no_pending_responses()
        client = s3_client("http://127.0.0.1:1", "us-east-1", "test", "test-secret")
        with Stubber(client) as stub:
            stub.add_response("head_bucket", {}, LOCKED)
            stub.add_client_error("get_object_lock_configuration", service_error_code="ObjectLockConfigurationNotFoundError",
                                  http_status_code=404, expected_params=LOCKED)
            self.assertRaises(ValueError, ensure_locked_bucket, client, "flow-like-locked", "us-east-1", [], "object", "COMPLIANCE", 3650)
            stub.assert_no_pending_responses()


if __name__ == "__main__":
    unittest.main()
