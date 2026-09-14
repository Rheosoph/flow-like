//! S3 enforces both the destination key and byte count in this POST policy.

use std::{collections::BTreeMap, time::Duration};

use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::{credentials::RuntimeCredentials, error::ApiError};

#[derive(Clone, Debug, serde::Serialize)]
pub struct UploadForm {
    pub url: String,
    pub bucket: String,
    pub method: &'static str,
    pub fields: BTreeMap<String, String>,
}

fn hmac(key: &[u8], value: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(value);
    mac.finalize().into_bytes().to_vec()
}

fn signature(secret: &str, date: &str, region: &str, policy: &str) -> String {
    let date_key = hmac(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let region_key = hmac(&date_key, region.as_bytes());
    let service_key = hmac(&region_key, b"s3");
    let signing_key = hmac(&service_key, b"aws4_request");
    hex::encode(hmac(&signing_key, policy.as_bytes()))
}

pub(crate) fn content_bucket(credentials: &RuntimeCredentials) -> Option<&str> {
    let mut credentials = credentials;
    while let RuntimeCredentials::Mixed(mixed) = credentials {
        credentials = &mixed.content;
    }
    #[cfg(feature = "aws")]
    if let RuntimeCredentials::Aws(aws) = credentials {
        return Some(&aws.content_bucket);
    }
    let _ = credentials;
    None
}

/// Return `None` only for providers without the S3 POST protocol. The caller keeps
/// their existing eventual storage gate instead of claiming a byte-bound grant.
pub fn bounded_form(
    credentials: &RuntimeCredentials,
    key: &str,
    bytes: u64,
    ttl: Duration,
) -> Result<Option<UploadForm>, ApiError> {
    let mut credentials = credentials;
    while let RuntimeCredentials::Mixed(mixed) = credentials {
        credentials = &mixed.content;
    }
    #[cfg(feature = "aws")]
    if let RuntimeCredentials::Aws(aws) = credentials {
        let access = aws
            .access_key_id
            .as_deref()
            .ok_or_else(|| ApiError::internal("upload signing credentials missing"))?;
        let secret = aws
            .secret_access_key
            .as_deref()
            .ok_or_else(|| ApiError::internal("upload signing credentials missing"))?;
        if bytes > 5_000_000_000 {
            return Err(ApiError::bad_request(
                "Each upload can contain at most 5 GB. Split larger datasets into files before uploading.",
            ));
        }
        let now = chrono::Utc::now();
        let expiry = now
            + chrono::Duration::from_std(ttl)
                .map_err(|_| ApiError::internal("invalid upload lifetime"))?;
        let expiry = aws
            .expiration
            .map_or(expiry, |deadline| deadline.min(expiry));
        if expiry <= now {
            return Err(ApiError::internal("upload credentials have expired"));
        }
        let date = now.format("%Y%m%d").to_string();
        let mut fields = BTreeMap::from([
            ("key".into(), key.into()),
            ("x-amz-algorithm".into(), "AWS4-HMAC-SHA256".into()),
            (
                "x-amz-credential".into(),
                format!("{access}/{date}/{}/s3/aws4_request", aws.region),
            ),
            (
                "x-amz-date".into(),
                now.format("%Y%m%dT%H%M%SZ").to_string(),
            ),
            ("success_action_status".into(), "204".into()),
        ]);
        if let Some(token) = &aws.session_token {
            fields.insert("x-amz-security-token".into(), token.clone());
        }
        if let Some(key) = &crate::credentials::aws_credentials::KmsKeys::configured().content {
            fields.insert("x-amz-server-side-encryption".into(), "aws:kms".into());
            fields.insert(
                "x-amz-server-side-encryption-aws-kms-key-id".into(),
                key.clone(),
            );
        }
        let policy = encode_policy(
            &fields,
            &aws.content_bucket,
            bytes,
            &expiry.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        );
        fields.insert(
            "x-amz-signature".into(),
            signature(secret, &date, &aws.region, &policy),
        );
        fields.insert("policy".into(), policy);
        let endpoint = std::env::var("S3_PUBLIC_ENDPOINT")
            .or_else(|_| std::env::var("AWS_ENDPOINT"))
            .ok()
            .filter(|v| !v.trim().is_empty());
        let url = if let Some(endpoint) = endpoint {
            format!("{}/{}/", endpoint.trim_end_matches('/'), aws.content_bucket)
        } else {
            format!(
                "https://{}.s3.{}.amazonaws.com/",
                aws.content_bucket, aws.region
            )
        };
        return Ok(Some(UploadForm {
            url,
            bucket: aws.content_bucket.clone(),
            method: "POST",
            fields,
        }));
    }
    let _ = (credentials, key, bytes, ttl);
    Ok(None)
}

fn encode_policy(
    fields: &BTreeMap<String, String>,
    bucket: &str,
    bytes: u64,
    expiry: &str,
) -> String {
    let mut conditions = vec![
        serde_json::json!({"bucket": bucket}),
        serde_json::json!(["content-length-range", bytes, bytes]),
    ];
    conditions.extend(
        fields
            .iter()
            .map(|(name, value)| serde_json::json!({name: value})),
    );
    STANDARD.encode(
        serde_json::to_vec(&serde_json::json!({"expiration": expiry, "conditions": conditions}))
            .expect("JSON values serialize"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_binds_every_field_exactly_and_rejects_a_larger_file() {
        let fields = BTreeMap::from([
            ("key".into(), "apps/a/upload/dataset.parquet".into()),
            ("x-amz-security-token".into(), "test-session".into()),
        ]);
        let encoded = encode_policy(&fields, "test-bucket", 1234, "2026-09-13T12:00:00.000Z");
        let policy: serde_json::Value =
            serde_json::from_slice(&STANDARD.decode(encoded).unwrap()).unwrap();
        let conditions = policy["conditions"].as_array().unwrap();
        assert!(conditions.contains(&serde_json::json!(["content-length-range", 1234, 1234])));
        assert!(conditions.contains(&serde_json::json!({"key":"apps/a/upload/dataset.parquet"})));
        assert!(conditions.contains(&serde_json::json!({"x-amz-security-token":"test-session"})));
        assert!(
            !conditions
                .iter()
                .any(|value| value.is_array() && value[0] == "starts-with")
        );
    }

    #[test]
    fn signing_is_scoped_to_region_date_and_exact_policy() {
        let original = signature("example-secret", "20260913", "eu-central-1", "policy");
        assert_eq!(original.len(), 64);
        assert_ne!(
            original,
            signature("example-secret", "20260914", "eu-central-1", "policy")
        );
        assert_ne!(
            original,
            signature("example-secret", "20260913", "us-east-1", "policy")
        );
        assert_ne!(
            original,
            signature("example-secret", "20260913", "eu-central-1", "changed")
        );
    }

    #[test]
    fn signature_matches_aws_published_post_policy_vector() {
        // Fictitious credentials and test vector published by AWS:
        // https://docs.aws.amazon.com/AmazonS3/latest/developerguide/sigv4-post-example.html
        let policy = "eyAiZXhwaXJhdGlvbiI6ICIyMDE1LTEyLTMwVDEyOjAwOjAwLjAwMFoiLA0KICAiY29uZGl0aW9ucyI6IFsNCiAgICB7ImJ1Y2tldCI6ICJzaWd2NGV4YW1wbGVidWNrZXQifSwNCiAgICBbInN0YXJ0cy13aXRoIiwgIiRrZXkiLCAidXNlci91c2VyMS8iXSwNCiAgICB7ImFjbCI6ICJwdWJsaWMtcmVhZCJ9LA0KICAgIHsic3VjY2Vzc19hY3Rpb25fcmVkaXJlY3QiOiAiaHR0cDovL3NpZ3Y0ZXhhbXBsZWJ1Y2tldC5zMy5hbWF6b25hd3MuY29tL3N1Y2Nlc3NmdWxfdXBsb2FkLmh0bWwifSwNCiAgICBbInN0YXJ0cy13aXRoIiwgIiRDb250ZW50LVR5cGUiLCAiaW1hZ2UvIl0sDQogICAgeyJ4LWFtei1tZXRhLXV1aWQiOiAiMTQzNjUxMjM2NTEyNzQifSwNCiAgICB7IngtYW16LXNlcnZlci1zaWRlLWVuY3J5cHRpb24iOiAiQUVTMjU2In0sDQogICAgWyJzdGFydHMtd2l0aCIsICIkeC1hbXotbWV0YS10YWciLCAiIl0sDQoNCiAgICB7IngtYW16LWNyZWRlbnRpYWwiOiAiQUtJQUlPU0ZPRE5ON0VYQU1QTEUvMjAxNTEyMjkvdXMtZWFzdC0xL3MzL2F3czRfcmVxdWVzdCJ9LA0KICAgIHsieC1hbXotYWxnb3JpdGhtIjogIkFXUzQtSE1BQy1TSEEyNTYifSwNCiAgICB7IngtYW16LWRhdGUiOiAiMjAxNTEyMjlUMDAwMDAwWiIgfQ0KICBdDQp9";
        assert_eq!(
            signature(
                "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
                "20151229",
                "us-east-1",
                policy
            ),
            "8afdbf4008c03f22c2cd3cdb72e4afbb1f6a588f3255ac628749a66d7f09699e"
        );
    }
}
