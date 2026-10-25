//! S3 Object Lock: AWS, MinIO, RustFS, Ceph. R2 shares the client in bucket mode,
//! where its bucket lock rules do the retaining.

use super::backend::{
    ChecksumCheck, SignedUpload, StoredObject, UploadSpec, attachment_disposition,
};
use super::config::LockedProvider;
use crate::entity::sea_orm_active_enums::LockedRetentionMode;
use crate::storage_config::{non_empty_env, secret_env};
use aws_sdk_s3::config::{Credentials, RequestChecksumCalculation, ResponseChecksumValidation};
use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::types::{
    ChecksumMode, ObjectLockLegalHold, ObjectLockLegalHoldStatus, ObjectLockRetention,
    ObjectLockRetentionMode, ServerSideEncryption,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use flow_like_types::{Result, anyhow};
use sha2::{Digest, Sha256};
use std::time::Duration;

pub struct S3Locked {
    client: aws_sdk_s3::Client,
    bucket: String,
    /// R2 keeps no full-object SHA-256, so the API hashes on commit instead.
    checksums: bool,
    kms_key: Option<String>,
}

impl S3Locked {
    pub async fn from_env(bucket: &str, provider: LockedProvider) -> Result<Self> {
        let r2 = provider == LockedProvider::R2;
        let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest()).region(
            aws_config::Region::new(
                non_empty_env("LOCKED_BUCKET_REGION")
                    .or_else(|| non_empty_env("AWS_REGION"))
                    .unwrap_or_else(|| if r2 { "auto" } else { "us-east-1" }.to_owned()),
            ),
        );
        if let Some(endpoint) = endpoint(r2) {
            loader = loader.endpoint_url(endpoint);
        }
        if let Some(credentials) = static_credentials(r2)? {
            loader = loader.credentials_provider(credentials);
        }
        let path_style = non_empty_env("LOCKED_BUCKET_PATH_STYLE")
            .or_else(|| non_empty_env("AWS_USE_PATH_STYLE"))
            .is_some_and(|value| value == "true" || value == "1");
        let sdk = loader.load().await;
        let config = aws_sdk_s3::config::Builder::from(&sdk)
            .force_path_style(path_style)
            .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
            .response_checksum_validation(ResponseChecksumValidation::WhenRequired)
            .build();
        Ok(Self {
            client: aws_sdk_s3::Client::from_conf(config),
            bucket: bucket.to_owned(),
            checksums: !r2,
            kms_key: non_empty_env("LOCKED_BUCKET_KMS_KEY_ARN"),
        })
    }

    pub(super) async fn sign_upload(
        &self,
        spec: &UploadSpec<'_>,
        ttl: Duration,
    ) -> Result<SignedUpload> {
        let mut request = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(spec.key)
            .content_type(spec.content_type);
        if self.checksums {
            request = request.checksum_sha256(STANDARD.encode(spec.sha256));
        }
        if let Some(key) = &self.kms_key {
            request = request
                .server_side_encryption(ServerSideEncryption::AwsKms)
                .ssekms_key_id(key);
        }
        let presigned = request
            .presigned(PresigningConfig::expires_in(ttl)?)
            .await
            .map_err(|error| sdk_error("signing the locked upload", error))?;
        Ok(SignedUpload {
            url: presigned.uri().to_owned(),
            headers: client_headers(presigned.headers()),
        })
    }

    pub(super) async fn sign_download(
        &self,
        key: &str,
        version_id: Option<&str>,
        file_name: &str,
        content_type: &str,
        ttl: Duration,
    ) -> Result<String> {
        let presigned = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .set_version_id(version_id.map(str::to_owned))
            .response_content_disposition(attachment_disposition(file_name))
            .response_content_type(content_type)
            .presigned(PresigningConfig::expires_in(ttl)?)
            .await
            .map_err(|error| sdk_error("signing the locked download", error))?;
        Ok(presigned.uri().to_owned())
    }

    pub(super) async fn inspect(
        &self,
        key: &str,
        sha256: &[u8; 32],
    ) -> Result<Option<StoredObject>> {
        let mut request = self.client.head_object().bucket(&self.bucket).key(key);
        if self.checksums {
            request = request.checksum_mode(ChecksumMode::Enabled);
        }
        let head = match request.send().await {
            Ok(head) => head,
            Err(error)
                if error
                    .raw_response()
                    .is_some_and(|raw| raw.status().as_u16() == 404) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(sdk_error("inspecting the locked object", error)),
        };
        Ok(Some(StoredObject {
            size: head.content_length().unwrap_or_default().max(0) as u64,
            version_id: head.version_id().map(str::to_owned),
            checksum: match head.checksum_sha256() {
                // A multipart composite (`…-N`) is not a hash of the bytes.
                Some(value) if self.checksums && !value.contains('-') => {
                    if STANDARD.decode(value).ok().as_deref() == Some(sha256.as_slice()) {
                        ChecksumCheck::Matches
                    } else {
                        ChecksumCheck::Differs
                    }
                }
                _ => ChecksumCheck::Unknown,
            },
            retain_until: head
                .object_lock_retain_until_date()
                .and_then(|date| DateTime::from_timestamp(date.secs(), 0)),
        }))
    }

    pub(super) async fn sha256(
        &self,
        key: &str,
        version_id: Option<&str>,
    ) -> Result<([u8; 32], u64)> {
        let mut body = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .set_version_id(version_id.map(str::to_owned))
            .send()
            .await
            .map_err(|error| sdk_error("reading the locked object", error))?
            .body;
        let mut digest = Sha256::new();
        let mut size = 0u64;
        while let Some(chunk) = body
            .try_next()
            .await
            .map_err(|error| anyhow!("reading the locked object {key} failed: {error}"))?
        {
            digest.update(&chunk);
            size += chunk.len() as u64;
        }
        Ok((digest.finalize().into(), size))
    }

    pub(super) async fn lock(
        &self,
        key: &str,
        version_id: Option<&str>,
        mode: LockedRetentionMode,
        until: DateTime<Utc>,
    ) -> Result<()> {
        let retention = ObjectLockRetention::builder()
            .mode(match mode {
                LockedRetentionMode::Compliance => ObjectLockRetentionMode::Compliance,
                LockedRetentionMode::Governance => ObjectLockRetentionMode::Governance,
            })
            .retain_until_date(aws_sdk_s3::primitives::DateTime::from_secs(
                until.timestamp(),
            ))
            .build();
        self.client
            .put_object_retention()
            .bucket(&self.bucket)
            .key(key)
            .set_version_id(version_id.map(str::to_owned))
            .retention(retention)
            .send()
            .await
            .map_err(|error| sdk_error("locking the object", error))?;
        Ok(())
    }

    pub(super) async fn legal_hold(
        &self,
        key: &str,
        version_id: Option<&str>,
        enabled: bool,
    ) -> Result<()> {
        let status = if enabled {
            ObjectLockLegalHoldStatus::On
        } else {
            ObjectLockLegalHoldStatus::Off
        };
        self.client
            .put_object_legal_hold()
            .bucket(&self.bucket)
            .key(key)
            .set_version_id(version_id.map(str::to_owned))
            .legal_hold(ObjectLockLegalHold::builder().status(status).build())
            .send()
            .await
            .map_err(|error| sdk_error("setting the legal hold", error))?;
        Ok(())
    }

    /// Deleting what is already gone succeeds, so a retried purge converges.
    pub(super) async fn delete(&self, key: &str, version_id: Option<&str>) -> Result<()> {
        match self
            .client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .set_version_id(version_id.map(str::to_owned))
            .send()
            .await
        {
            Ok(_) => Ok(()),
            Err(error)
                if error
                    .raw_response()
                    .is_some_and(|raw| raw.status().as_u16() == 404) =>
            {
                Ok(())
            }
            Err(error) => Err(sdk_error("deleting the locked object", error)),
        }
    }
}

fn endpoint(r2: bool) -> Option<String> {
    non_empty_env("LOCKED_BUCKET_ENDPOINT")
        .or_else(|| {
            r2.then(|| {
                non_empty_env("R2_ENDPOINT").or_else(|| {
                    non_empty_env("R2_ACCOUNT_ID")
                        .map(|account| format!("https://{account}.r2.cloudflarestorage.com"))
                })
            })
            .flatten()
        })
        .or_else(|| non_empty_env("S3_PUBLIC_ENDPOINT"))
        .or_else(|| non_empty_env("S3_INTERNAL_ENDPOINT"))
        .or_else(|| non_empty_env("AWS_ENDPOINT"))
}

/// A dedicated identity first, then the deployment's static keys; neither means
/// the SDK's own chain (instance role, web identity).
fn static_credentials(r2: bool) -> Result<Option<Credentials>> {
    let mut pairs = vec![(
        "LOCKED_BUCKET_ACCESS_KEY_ID",
        "LOCKED_BUCKET_SECRET_ACCESS_KEY",
    )];
    if r2 {
        pairs.push(("R2_ACCESS_KEY_ID", "R2_SECRET_ACCESS_KEY"));
    }
    pairs.push(("AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"));
    for (key_name, secret_name) in pairs {
        match (secret_env(key_name)?, secret_env(secret_name)?) {
            (Some(key), Some(secret)) => {
                let token = (key_name == "AWS_ACCESS_KEY_ID")
                    .then(|| secret_env("AWS_SESSION_TOKEN"))
                    .transpose()?
                    .flatten();
                return Ok(Some(Credentials::new(
                    key,
                    secret,
                    token,
                    None,
                    "locked-bucket",
                )));
            }
            (None, None) => {}
            _ => {
                return Err(anyhow!("set both {key_name} and {secret_name}, or neither"));
            }
        }
    }
    Ok(None)
}

/// Browsers set `host` and `content-length` themselves and refuse to be told.
fn client_headers<'a>(
    headers: impl Iterator<Item = (&'a str, &'a str)>,
) -> std::collections::BTreeMap<String, String> {
    headers
        .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
        .filter(|(name, _)| name != "host" && name != "content-length")
        .collect()
}

fn sdk_error(operation: &str, error: impl std::error::Error) -> flow_like_types::Error {
    anyhow!("{operation} failed: {}", DisplayErrorContext(error))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn client() -> S3Locked {
        let config = aws_sdk_s3::config::Builder::new()
            .behavior_version(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new("eu-central-1"))
            .credentials_provider(Credentials::new(
                "AKIDEXAMPLE",
                "secret",
                None,
                None,
                "test",
            ))
            .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
            .build();
        S3Locked {
            client: aws_sdk_s3::Client::from_conf(config),
            bucket: "flow-like-locked".into(),
            checksums: true,
            kms_key: Some("arn:aws:kms:eu-central-1:1:key/k".into()),
        }
    }

    #[flow_like_types::tokio::test]
    async fn upload_signs_the_checksum_type_and_encryption_headers() {
        let sha256 = [7u8; 32];
        let signed = client()
            .await
            .sign_upload(
                &UploadSpec {
                    key: "apps/app/project/item/version",
                    content_type: "application/pdf",
                    sha256,
                },
                Duration::from_secs(600),
            )
            .await
            .unwrap();
        assert!(signed.url.contains("apps/app/project/item/version"));
        assert!(signed.url.contains("X-Amz-Signature="));
        assert_eq!(
            signed.headers.get("content-type").map(String::as_str),
            Some("application/pdf")
        );
        assert_eq!(
            signed
                .headers
                .get("x-amz-checksum-sha256")
                .map(String::as_str),
            Some(STANDARD.encode(sha256).as_str())
        );
        assert_eq!(
            signed
                .headers
                .get("x-amz-server-side-encryption")
                .map(String::as_str),
            Some("aws:kms")
        );
        assert!(!signed.headers.contains_key("host"));
        let signed_headers = signed
            .url
            .split('&')
            .find_map(|pair| pair.strip_prefix("X-Amz-SignedHeaders="))
            .unwrap()
            .replace("%3B", ";");
        for name in [
            "content-type",
            "x-amz-checksum-sha256",
            "x-amz-server-side-encryption",
        ] {
            assert!(
                signed_headers.split(';').any(|signed| signed == name),
                "{name} unsigned"
            );
        }
    }

    #[flow_like_types::tokio::test]
    async fn download_pins_the_version_and_the_file_name() {
        let url = client()
            .await
            .sign_download(
                "apps/app/project/item/version",
                Some("3HL4kqtJlcpXroDTDmJ+rmSpXd3dIbrHY"),
                "Übersicht.pdf",
                "application/pdf",
                Duration::from_secs(600),
            )
            .await
            .unwrap();
        assert!(url.contains("versionId=3HL4kqtJlcpXroDTDmJ%2BrmSpXd3dIbrHY"));
        assert!(url.contains("response-content-disposition="));
        assert!(url.contains("response-content-type=application%2Fpdf"));
    }
}
