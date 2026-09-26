//! The audit bucket: daily heads, monthly archives and batches of legacy entries.
//!
//! Only the audit worker writes here and it never deletes. Immutability and the storage
//! tier are properties of the bucket, set by whoever creates it. Retries accept an
//! existing object only when its bytes match; different content fails without
//! replacing the evidence. Database receipts record the committed object digest.

use std::io::Write;
use std::sync::Arc;

use chrono::{Datelike, NaiveDate};
use flow_like_storage::Path;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::aws::{AmazonS3Builder, Checksum};
use flow_like_storage::object_store::{
    Error as StoreError, ObjectStoreExt, PutPayload, WriteMultipart,
};
use flow_like_types::Context;
use futures::TryStreamExt;
use sha2::{Digest, Sha256};

use crate::storage_config::{AzureConfig, GcpConfig, S3Config, non_empty_env, secret_env};

const ZSTD_LEVEL: i32 = 9;
/// Compressed bytes buffered before they are handed to the multipart upload.
const FLUSH_BYTES: usize = 8 * 1024 * 1024;
const MAX_PARTS_IN_FLIGHT: usize = 4;

/// `heads/YYYY/MM/DD.json`
pub fn head_key(date: NaiveDate) -> Path {
    Path::from(format!(
        "heads/{:04}/{:02}/{:02}.json",
        date.year(),
        date.month(),
        date.day()
    ))
}

/// `YYYY-MM` of a month, the `period` of its archive rows.
pub fn period_of(date: NaiveDate) -> String {
    format!("{:04}-{:02}", date.year(), date.month())
}

/// `archive/YYYY/MM/NNNN.jsonl.zst` for `period` `YYYY-MM`; parts count from 1.
pub fn archive_part_key(period: &str, part: i32) -> Path {
    Path::from(format!(
        "archive/{}/{part:04}.jsonl.zst",
        period.replacen('-', "/", 1)
    ))
}

/// `archive/YYYY/MM/manifest.json`, written after every part of the month.
pub fn archive_manifest_key(period: &str) -> Path {
    Path::from(format!(
        "archive/{}/manifest.json",
        period.replacen('-', "/", 1)
    ))
}

/// The raw export of the entries written before the sealed format.
pub fn legacy_key() -> Path {
    Path::from("legacy/audit-entry.jsonl.zst")
}

#[derive(Clone, Copy, Debug)]
pub struct WrittenObject {
    pub sha256: [u8; 32],
    pub byte_size: u64,
}

/// Write a small object in one request.
pub async fn put_bytes(
    bucket: &FlowLikeStore,
    key: &Path,
    bytes: Vec<u8>,
) -> flow_like_types::Result<WrittenObject> {
    let written = WrittenObject {
        sha256: Sha256::digest(&bytes).into(),
        byte_size: bytes.len() as u64,
    };
    // Locked Azure/GCS objects cannot be overwritten. A retry may find a write
    // that succeeded before the database commit or before the response arrived.
    match bucket.as_generic().get(key).await {
        Ok(existing) => {
            let existing = existing.bytes().await?;
            if existing.as_ref() == bytes.as_slice() {
                return Ok(written);
            }
            return Err(flow_like_types::anyhow!(
                "audit object {key} already exists with different content; refusing to replace evidence"
            ));
        }
        Err(StoreError::NotFound { .. }) => {}
        Err(error) => return Err(error.into()),
    }
    bucket
        .as_generic()
        .put(key, PutPayload::from(bytes))
        .await
        .map_err(|error| flow_like_types::anyhow!("writing audit object {key} failed: {error}"))?;
    Ok(written)
}

/// Streams lines through zstd into a multipart upload, hashing the compressed bytes.
pub struct ArchiveWriter {
    key: Path,
    upload: Option<WriteMultipart>,
    existing: Option<WrittenObject>,
    encoder: zstd::stream::write::Encoder<'static, Vec<u8>>,
    sha256: Sha256,
    byte_size: u64,
    uncompressed: u64,
}

impl ArchiveWriter {
    pub async fn create(bucket: &FlowLikeStore, key: Path) -> flow_like_types::Result<Self> {
        let (upload, existing) = match bucket.as_generic().get(&key).await {
            Ok(object) => {
                // Compare a replay to the committed bytes without replacing a locked
                // object. Hash as a stream so archive size does not bound memory use.
                let mut stream = object.into_stream();
                let mut digest = Sha256::new();
                let mut byte_size = 0;
                while let Some(chunk) = stream.try_next().await? {
                    digest.update(&chunk);
                    byte_size += chunk.len() as u64;
                }
                (
                    None,
                    Some(WrittenObject {
                        sha256: digest.finalize().into(),
                        byte_size,
                    }),
                )
            }
            Err(StoreError::NotFound { .. }) => {
                let upload = bucket
                    .as_generic()
                    .put_multipart(&key)
                    .await
                    .map_err(|error| {
                        flow_like_types::anyhow!("starting audit upload {key} failed: {error}")
                    })?;
                (Some(WriteMultipart::new(upload)), None)
            }
            Err(error) => return Err(error.into()),
        };
        let encoder = zstd::stream::write::Encoder::new(Vec::new(), ZSTD_LEVEL)
            .map_err(|error| flow_like_types::anyhow!("zstd encoder failed: {error}"))?;
        Ok(Self {
            key,
            upload,
            existing,
            encoder,
            sha256: Sha256::new(),
            byte_size: 0,
            uncompressed: 0,
        })
    }

    /// Uncompressed bytes written so far, for splitting large months into parts.
    pub fn uncompressed_bytes(&self) -> u64 {
        self.uncompressed
    }

    /// Append one encoded line (see [`crate::audit::wire::encode`]).
    pub async fn write_line(&mut self, line: &[u8]) -> flow_like_types::Result<()> {
        self.encoder
            .write_all(line)
            .map_err(|error| flow_like_types::anyhow!("zstd write failed: {error}"))?;
        self.uncompressed += line.len() as u64;
        if self.encoder.get_ref().len() >= FLUSH_BYTES {
            let chunk = std::mem::take(self.encoder.get_mut());
            self.push(chunk).await?;
        }
        Ok(())
    }

    async fn push(&mut self, chunk: Vec<u8>) -> flow_like_types::Result<()> {
        if chunk.is_empty() {
            return Ok(());
        }
        self.sha256.update(&chunk);
        self.byte_size += chunk.len() as u64;
        if let Some(upload) = self.upload.as_mut() {
            upload
                .wait_for_capacity(MAX_PARTS_IN_FLIGHT)
                .await
                .map_err(|error| {
                    flow_like_types::anyhow!("uploading audit object {} failed: {error}", self.key)
                })?;
            upload.put(chunk.into());
        }
        Ok(())
    }

    pub async fn finish(mut self) -> flow_like_types::Result<WrittenObject> {
        let encoder = std::mem::replace(
            &mut self.encoder,
            zstd::stream::write::Encoder::new(Vec::new(), ZSTD_LEVEL)
                .map_err(|error| flow_like_types::anyhow!("zstd encoder failed: {error}"))?,
        );
        let rest = encoder
            .finish()
            .map_err(|error| flow_like_types::anyhow!("zstd finish failed: {error}"))?;
        self.push(rest).await?;
        let key = self.key.clone();
        let written = WrittenObject {
            sha256: self.sha256.finalize().into(),
            byte_size: self.byte_size,
        };
        if let Some(existing) = self.existing {
            if written.sha256 != existing.sha256 || written.byte_size != existing.byte_size {
                return Err(flow_like_types::anyhow!(
                    "audit archive {key} already exists with different content; refusing to replace evidence"
                ));
            }
        } else if let Some(upload) = self.upload {
            upload.finish().await.map_err(|error| {
                flow_like_types::anyhow!("completing audit object {key} failed: {error}")
            })?;
        }
        Ok(written)
    }

    /// Abandon the upload. Parts already sent are cleaned up where the store allows.
    pub async fn abort(self) {
        let key = self.key.clone();
        if let Some(upload) = self.upload
            && let Err(error) = upload.abort().await
        {
            tracing::warn!(%key, %error, "aborting audit upload failed");
        }
    }
}

/// The audit bucket from the environment, first match wins: `AUDIT_BUCKET` or
/// `AWS_AUDIT_BUCKET` (S3 or S3-compatible), `GCP_AUDIT_BUCKET`, `AZURE_AUDIT_CONTAINER`
/// (in `AZURE_STORAGE_ACCOUNT_NAME`). Credentials and region come from the same variables
/// as the other buckets. `None` means nothing is archived and no evidence is pruned.
pub fn from_env() -> flow_like_types::Result<Option<Arc<FlowLikeStore>>> {
    let store = if let Some(bucket) =
        non_empty_env("AUDIT_BUCKET").or_else(|| non_empty_env("AWS_AUDIT_BUCKET"))
    {
        s3_store(&bucket).with_context(|| format!("configuring audit bucket {bucket}"))?
    } else if let Some(bucket) = non_empty_env("GCP_AUDIT_BUCKET") {
        GcpConfig::from_env()
            .and_then(|config| config.build_store(&bucket))
            .with_context(|| format!("configuring GCP_AUDIT_BUCKET {bucket}"))?
    } else if let Some(container) = non_empty_env("AZURE_AUDIT_CONTAINER") {
        AzureConfig::from_env()
            .and_then(|config| config.build_store(&container))
            .with_context(|| format!("configuring AZURE_AUDIT_CONTAINER {container}"))?
    } else {
        return Ok(None);
    };
    Ok(Some(Arc::new(store)))
}

/// S3 with `AUDIT_BUCKET_ENDPOINT`, `AUDIT_BUCKET_KMS_KEY_ARN` (SSE-KMS) and optionally a
/// dedicated identity (`AUDIT_BUCKET_ACCESS_KEY_ID` / `AUDIT_BUCKET_SECRET_ACCESS_KEY`).
/// Every upload carries a SHA-256 checksum, which Object Lock buckets require.
fn s3_store(bucket: &str) -> flow_like_types::Result<FlowLikeStore> {
    let shared = S3Config::from_env()?;
    let builder = match (
        secret_env("AUDIT_BUCKET_ACCESS_KEY_ID")?,
        secret_env("AUDIT_BUCKET_SECRET_ACCESS_KEY")?,
    ) {
        (Some(key), Some(secret)) => AmazonS3Builder::new()
            .with_access_key_id(key)
            .with_secret_access_key(secret),
        (None, None) => {
            // `from_env` also resolves task-role, web-identity and pod-identity credentials.
            let mut builder = AmazonS3Builder::from_env();
            if let (Some(key), Some(secret)) = (shared.access_key_id, shared.secret_access_key) {
                builder = builder
                    .with_access_key_id(key)
                    .with_secret_access_key(secret);
                if let Some(token) = shared.session_token {
                    builder = builder.with_token(token);
                }
            }
            builder
        }
        _ => {
            return Err(flow_like_types::anyhow!(
                "set both AUDIT_BUCKET_ACCESS_KEY_ID and AUDIT_BUCKET_SECRET_ACCESS_KEY or neither"
            ));
        }
    };
    let mut builder = builder
        .with_bucket_name(bucket)
        .with_region(shared.region)
        .with_checksum_algorithm(Checksum::SHA256);
    if let Some(endpoint) = non_empty_env("AUDIT_BUCKET_ENDPOINT").or(shared.endpoint) {
        builder = builder
            .with_allow_http(endpoint.starts_with("http://"))
            .with_endpoint(endpoint);
    }
    if shared.use_path_style {
        builder = builder.with_virtual_hosted_style_request(false);
    }
    if let Some(key) = non_empty_env("AUDIT_BUCKET_KMS_KEY_ARN") {
        builder = builder.with_sse_kms_encryption(key);
    }
    let store = builder
        .build()
        .map_err(|error| flow_like_types::anyhow!("building the S3 client failed: {error}"))?;
    Ok(FlowLikeStore::AWS(Arc::new(store)))
}

/// Whether a long-lived server runs the audit worker in process. `AUDIT_WORKER=off`
/// (or `false`, `0`) leaves it to a separate function or container.
pub fn in_process_worker_enabled() -> bool {
    worker_switch(non_empty_env("AUDIT_WORKER").as_deref())
}

/// Public API binaries have no worker mode. Refuse an accidentally mounted audit
/// credential instead of starting with authority an API compromise could steal.
pub fn ensure_api_only() -> flow_like_types::Result<()> {
    api_settings_are_isolated(non_empty_env)
}

fn api_settings_are_isolated(
    lookup: impl Fn(&str) -> Option<String>,
) -> flow_like_types::Result<()> {
    if lookup("AUDIT_WORKER").is_some_and(|value| worker_switch(Some(&value))) {
        return Err(flow_like_types::anyhow!(
            "this API cannot run the audit worker; deploy the dedicated audit-worker service"
        ));
    }
    for name in [
        "AUDIT_SIGNING_KEY",
        "AUDIT_KMS_KEY_ID",
        "AUDIT_KMS_AWS_ACCESS_KEY_ID",
        "AUDIT_KMS_AWS_SECRET_ACCESS_KEY",
        "AUDIT_VAULT_TOKEN",
        "AUDIT_VAULT_TOKEN_FILE",
        "AUDIT_VAULT_ADDR",
        "AUDIT_BUCKET",
        "AWS_AUDIT_BUCKET",
        "GCP_AUDIT_BUCKET",
        "AZURE_AUDIT_CONTAINER",
        "AUDIT_BUCKET_ACCESS_KEY_ID",
        "AUDIT_BUCKET_SECRET_ACCESS_KEY",
        "AUDIT_DATABASE_URL",
    ] {
        if lookup(name).is_some() || lookup(&format!("{name}_FILE")).is_some() {
            return Err(flow_like_types::anyhow!(
                "{name} belongs only on the dedicated audit worker; remove it from the API"
            ));
        }
    }
    Ok(())
}

fn worker_switch(value: Option<&str>) -> bool {
    !value.is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "off" | "false" | "0"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::object_store::memory::InMemory;
    use std::io::Read;

    #[flow_like_types::tokio::test]
    async fn immutable_writes_accept_identical_replays_and_refuse_changed_evidence() {
        let bucket = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let key = Path::from("receipts/test.json");
        let first = put_bytes(&bucket, &key, b"original".to_vec())
            .await
            .unwrap();
        let replay = put_bytes(&bucket, &key, b"original".to_vec())
            .await
            .unwrap();
        assert_eq!(first.sha256, replay.sha256);
        assert!(put_bytes(&bucket, &key, b"changed".to_vec()).await.is_err());
        let key = Path::from("archive/test.zst");
        let mut first = ArchiveWriter::create(&bucket, key.clone()).await.unwrap();
        first.write_line(b"original\n").await.unwrap();
        let first = first.finish().await.unwrap();
        let mut replay = ArchiveWriter::create(&bucket, key.clone()).await.unwrap();
        replay.write_line(b"original\n").await.unwrap();
        assert_eq!(first.sha256, replay.finish().await.unwrap().sha256);
        let mut changed = ArchiveWriter::create(&bucket, key).await.unwrap();
        changed.write_line(b"changed\n").await.unwrap();
        assert!(changed.finish().await.is_err());
    }

    #[test]
    fn api_refuses_worker_authority_even_when_worker_is_off() {
        for name in [
            "AUDIT_SIGNING_KEY",
            "AUDIT_SIGNING_KEY_FILE",
            "AUDIT_VAULT_TOKEN_FILE",
            "GCP_AUDIT_BUCKET",
            "AZURE_AUDIT_CONTAINER",
            "AUDIT_DATABASE_URL",
        ] {
            assert!(
                api_settings_are_isolated(|key| match key {
                    "AUDIT_WORKER" => Some("off".into()),
                    key if key == name => Some("configured".into()),
                    _ => None,
                })
                .is_err(),
                "{name}"
            );
        }
        assert!(api_settings_are_isolated(|_| None).is_ok());
        assert!(
            api_settings_are_isolated(|key| (key == "AUDIT_WORKER").then(|| "on".into())).is_err()
        );
    }
    use std::sync::Arc;

    #[test]
    fn keys_follow_the_layout() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 5).unwrap();
        assert_eq!(head_key(date).as_ref(), "heads/2026/09/05.json");
        assert_eq!(period_of(date), "2026-09");
        assert_eq!(
            archive_part_key("2026-09", 1).as_ref(),
            "archive/2026/09/0001.jsonl.zst"
        );
        assert_eq!(
            archive_manifest_key("2026-09").as_ref(),
            "archive/2026/09/manifest.json"
        );
    }

    #[flow_like_types::tokio::test]
    async fn archive_writer_round_trips() {
        let bucket = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let key = archive_part_key("2026-09", 1);
        let mut writer = ArchiveWriter::create(&bucket, key.clone()).await.unwrap();
        for index in 0..10_000 {
            writer
                .write_line(format!("{{\"n\":{index}}}\n").as_bytes())
                .await
                .unwrap();
        }
        let written = writer.finish().await.unwrap();
        let stored = bucket
            .as_generic()
            .get(&key)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(stored.len() as u64, written.byte_size);
        assert_eq!(<[u8; 32]>::from(Sha256::digest(&stored)), written.sha256);
        let mut text = String::new();
        zstd::stream::read::Decoder::new(stored.as_ref())
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text.lines().count(), 10_000);
    }

    #[test]
    fn only_an_explicit_off_disables_the_in_process_worker() {
        for value in ["off", "OFF", "false", "False", "0"] {
            assert!(!worker_switch(Some(value)), "{value}");
        }
        for value in [None, Some("on"), Some("true"), Some("1")] {
            assert!(worker_switch(value), "{value:?}");
        }
    }
}
