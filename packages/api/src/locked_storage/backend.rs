use super::config::{LockedProvider, LockedStorageConfig};
use crate::entity::sea_orm_active_enums::LockedRetentionMode;
use chrono::{DateTime, Utc};
use flow_like_types::{Result, anyhow};
#[cfg(any(feature = "gcp", feature = "azure", test))]
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::time::Duration;

pub struct UploadSpec<'a> {
    pub key: &'a str,
    pub content_type: &'a str,
    pub sha256: [u8; 32],
}

/// One signed PUT for one exact object, and the headers the client must send verbatim.
pub struct SignedUpload {
    pub url: String,
    pub headers: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksumCheck {
    Matches,
    Differs,
    /// The store keeps no SHA-256; the caller hashes the object itself.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredObject {
    pub size: u64,
    pub version_id: Option<String>,
    pub checksum: ChecksumCheck,
    /// Retention the store reports, when it reports one.
    pub retain_until: Option<DateTime<Utc>>,
}

pub enum LockedBackend {
    #[cfg(feature = "aws")]
    S3(super::s3::S3Locked),
    #[cfg(feature = "gcp")]
    Gcs(super::gcs::GcsLocked),
    #[cfg(feature = "azure")]
    Azure(super::azure::AzureLocked),
    #[cfg(test)]
    Memory(MemoryLocked),
}

macro_rules! dispatch {
    ($self:expr, $backend:ident => $body:expr) => {
        match $self {
            #[cfg(feature = "aws")]
            LockedBackend::S3($backend) => $body,
            #[cfg(feature = "gcp")]
            LockedBackend::Gcs($backend) => $body,
            #[cfg(feature = "azure")]
            LockedBackend::Azure($backend) => $body,
            #[cfg(test)]
            LockedBackend::Memory($backend) => $body,
            #[cfg(not(any(feature = "aws", feature = "gcp", feature = "azure", test)))]
            backend => match *backend {},
        }
    };
}

impl LockedBackend {
    pub async fn from_config(config: &LockedStorageConfig) -> Result<Self> {
        match config.provider {
            #[cfg(feature = "aws")]
            LockedProvider::S3 | LockedProvider::R2 => Ok(Self::S3(
                super::s3::S3Locked::from_env(&config.bucket, config.provider).await?,
            )),
            #[cfg(feature = "gcp")]
            LockedProvider::Gcs => Ok(Self::Gcs(super::gcs::GcsLocked::from_env(&config.bucket)?)),
            #[cfg(feature = "azure")]
            LockedProvider::Azure => Ok(Self::Azure(super::azure::AzureLocked::from_env(
                &config.bucket,
            )?)),
            #[allow(unreachable_patterns)]
            provider => Err(anyhow!(
                "this API build has no {provider:?} support for the locked bucket"
            )),
        }
    }

    pub async fn sign_upload(&self, spec: &UploadSpec<'_>, ttl: Duration) -> Result<SignedUpload> {
        dispatch!(self, backend => backend.sign_upload(spec, ttl).await)
    }

    pub async fn sign_download(
        &self,
        key: &str,
        version_id: Option<&str>,
        file_name: &str,
        content_type: &str,
        ttl: Duration,
    ) -> Result<String> {
        dispatch!(self, backend => backend.sign_download(key, version_id, file_name, content_type, ttl).await)
    }

    pub async fn inspect(&self, key: &str, sha256: &[u8; 32]) -> Result<Option<StoredObject>> {
        dispatch!(self, backend => backend.inspect(key, sha256).await)
    }

    /// SHA-256 and size of the stored bytes, streamed through this process.
    pub async fn sha256(&self, key: &str, version_id: Option<&str>) -> Result<([u8; 32], u64)> {
        dispatch!(self, backend => backend.sha256(key, version_id).await)
    }

    pub async fn lock(
        &self,
        key: &str,
        version_id: Option<&str>,
        mode: LockedRetentionMode,
        until: DateTime<Utc>,
    ) -> Result<()> {
        dispatch!(self, backend => backend.lock(key, version_id, mode, until).await)
    }

    pub async fn legal_hold(
        &self,
        key: &str,
        version_id: Option<&str>,
        enabled: bool,
    ) -> Result<()> {
        dispatch!(self, backend => backend.legal_hold(key, version_id, enabled).await)
    }

    /// The store refuses while the version is retained; that refusal is the
    /// guarantee, the database only mirrors it.
    pub async fn delete(&self, key: &str, version_id: Option<&str>) -> Result<()> {
        dispatch!(self, backend => backend.delete(key, version_id).await)
    }
}

#[cfg(test)]
pub(crate) fn sha256_of_chunks<'a>(chunks: impl IntoIterator<Item = &'a [u8]>) -> ([u8; 32], u64) {
    let mut digest = Sha256::new();
    let mut size = 0u64;
    for chunk in chunks {
        digest.update(chunk);
        size += chunk.len() as u64;
    }
    (digest.finalize().into(), size)
}

/// Streams any object_store read into a digest without buffering the object.
#[cfg(any(feature = "gcp", feature = "azure", test))]
pub(crate) async fn object_store_sha256(
    store: &dyn flow_like_storage::object_store::ObjectStore,
    key: &str,
    version_id: Option<&str>,
) -> Result<([u8; 32], u64)> {
    use flow_like_storage::object_store::GetOptions;
    use futures::TryStreamExt;
    let options = GetOptions {
        version: version_id.map(str::to_owned),
        ..Default::default()
    };
    let mut stream = store
        .get_opts(&flow_like_storage::Path::from(key), options)
        .await?
        .into_stream();
    let mut digest = Sha256::new();
    let mut size = 0u64;
    while let Some(chunk) = stream.try_next().await? {
        digest.update(&chunk);
        size += chunk.len() as u64;
    }
    Ok((digest.finalize().into(), size))
}

#[cfg(any(feature = "gcp", feature = "azure", test))]
pub(crate) async fn object_store_inspect(
    store: &dyn flow_like_storage::object_store::ObjectStore,
    key: &str,
) -> Result<Option<StoredObject>> {
    use flow_like_storage::object_store::{Error as StoreError, ObjectStoreExt};
    match store.head(&flow_like_storage::Path::from(key)).await {
        Ok(meta) => Ok(Some(StoredObject {
            size: meta.size,
            version_id: meta.version,
            checksum: ChecksumCheck::Unknown,
            retain_until: None,
        })),
        Err(StoreError::NotFound { .. }) => Ok(None),
        Err(error) => Err(anyhow!("inspecting locked object {key} failed: {error}")),
    }
}

/// `Content-Disposition` that keeps the original name for every browser.
pub(crate) fn attachment_disposition(file_name: &str) -> String {
    let fallback: String = file_name
        .chars()
        .map(|c| {
            if (c.is_ascii_graphic() && c != '"' && c != '\\') || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!(
        "attachment; filename=\"{fallback}\"; filename*=UTF-8''{}",
        urlencoding::encode(file_name)
    )
}

/// Test double with the provider semantics the service relies on: retained
/// versions refuse deletion and retention can only be extended.
#[cfg(test)]
#[derive(Default)]
pub struct MemoryLocked {
    pub objects: parking_lot::Mutex<
        std::collections::HashMap<String, (Vec<u8>, Option<DateTime<Utc>>, bool)>,
    >,
}

#[cfg(test)]
impl MemoryLocked {
    pub fn put(&self, key: &str, bytes: &[u8]) {
        self.objects
            .lock()
            .insert(key.to_owned(), (bytes.to_vec(), None, false));
    }

    async fn sign_upload(&self, spec: &UploadSpec<'_>, _ttl: Duration) -> Result<SignedUpload> {
        Ok(SignedUpload {
            url: format!("memory://{}", spec.key),
            headers: BTreeMap::from([("content-type".to_owned(), spec.content_type.to_owned())]),
        })
    }

    async fn sign_download(
        &self,
        key: &str,
        _version_id: Option<&str>,
        _file_name: &str,
        _content_type: &str,
        _ttl: Duration,
    ) -> Result<String> {
        Ok(format!("memory://{key}"))
    }

    async fn inspect(&self, key: &str, _sha256: &[u8; 32]) -> Result<Option<StoredObject>> {
        Ok(self
            .objects
            .lock()
            .get(key)
            .map(|(bytes, until, _)| StoredObject {
                size: bytes.len() as u64,
                version_id: Some("v1".to_owned()),
                checksum: ChecksumCheck::Unknown,
                retain_until: *until,
            }))
    }

    async fn sha256(&self, key: &str, _version_id: Option<&str>) -> Result<([u8; 32], u64)> {
        let objects = self.objects.lock();
        let (bytes, _, _) = objects.get(key).ok_or_else(|| anyhow!("missing {key}"))?;
        Ok(sha256_of_chunks([bytes.as_slice()]))
    }

    async fn lock(
        &self,
        key: &str,
        _version_id: Option<&str>,
        _mode: LockedRetentionMode,
        until: DateTime<Utc>,
    ) -> Result<()> {
        let mut objects = self.objects.lock();
        let (_, retained, _) = objects
            .get_mut(key)
            .ok_or_else(|| anyhow!("missing {key}"))?;
        if retained.is_some_and(|current| current > until) {
            return Err(anyhow!("retention can only be extended"));
        }
        *retained = Some(until);
        Ok(())
    }

    async fn legal_hold(&self, key: &str, _version_id: Option<&str>, enabled: bool) -> Result<()> {
        let mut objects = self.objects.lock();
        let (_, _, hold) = objects
            .get_mut(key)
            .ok_or_else(|| anyhow!("missing {key}"))?;
        *hold = enabled;
        Ok(())
    }

    async fn delete(&self, key: &str, _version_id: Option<&str>) -> Result<()> {
        let mut objects = self.objects.lock();
        if let Some((_, until, hold)) = objects.get(key)
            && (*hold || until.is_some_and(|until| until > Utc::now()))
        {
            return Err(anyhow!("object {key} is retained"));
        }
        objects.remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disposition_keeps_unicode_names_and_an_ascii_fallback() {
        assert_eq!(
            attachment_disposition("Übersicht (2)#1.pdf"),
            "attachment; filename=\"_bersicht (2)#1.pdf\"; filename*=UTF-8''%C3%9Cbersicht%20%282%29%231.pdf"
        );
        assert_eq!(
            attachment_disposition("a\"b.txt"),
            "attachment; filename=\"a_b.txt\"; filename*=UTF-8''a%22b.txt"
        );
    }

    #[flow_like_types::tokio::test]
    async fn object_store_helpers_hash_and_inspect() {
        use flow_like_storage::object_store::{ObjectStoreExt, PutPayload, memory::InMemory};
        let store = InMemory::new();
        let key = "apps/a/project/i/v";
        store
            .put(
                &flow_like_storage::Path::from(key),
                PutPayload::from_static(b"hello"),
            )
            .await
            .unwrap();
        let (digest, size) = object_store_sha256(&store, key, None).await.unwrap();
        assert_eq!(size, 5);
        assert_eq!(digest, sha256_of_chunks([b"hello".as_slice()]).0);
        assert_eq!(
            object_store_inspect(&store, key)
                .await
                .unwrap()
                .unwrap()
                .size,
            5
        );
        assert!(
            object_store_inspect(&store, "missing")
                .await
                .unwrap()
                .is_none()
        );
    }
}
