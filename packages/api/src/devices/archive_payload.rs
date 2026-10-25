use super::archives::sql;
use crate::error::ApiError;
use flow_like_device_protocol::{ArchiveManifest, EncryptedArchive, compact_digest};
use flow_like_storage::{
    files::store::FlowLikeStore,
    object_store::{ObjectStoreExt, path::Path},
};
use futures::StreamExt;
use sea_orm::{ConnectionTrait, QueryResult};

pub(super) const MAX_ARCHIVE_BYTES: usize = 90 * 1024;
pub(super) const PAYLOAD_PREFIX: &str = "device-archives";
const OFFLOAD_UPLOADS: usize = 8;

#[derive(Clone)]
pub(super) struct StoredArchive {
    pub bundle: Option<String>,
    pub payload_ref: Option<String>,
    pub manifest_jws: Option<String>,
    pub roster_jws: Option<String>,
    pub size_bytes: i64,
}

impl StoredArchive {
    #[cfg(test)]
    pub(super) fn inline(bundle: String) -> Self {
        Self {
            size_bytes: bundle.len() as i64,
            bundle: Some(bundle),
            payload_ref: None,
            manifest_jws: None,
            roster_jws: None,
        }
    }
}

fn archive_prefix(device_id: &str, archive_id: &str) -> String {
    format!(
        "{PAYLOAD_PREFIX}/{}/{}/",
        blake3::hash(device_id.as_bytes()).to_hex(),
        blake3::hash(archive_id.as_bytes()).to_hex(),
    )
}

fn payload_path(device_id: &str, archive_id: &str, body: &[u8]) -> Path {
    Path::from(format!(
        "{}{}.json",
        archive_prefix(device_id, archive_id),
        blake3::hash(body).to_hex(),
    ))
}

/// Writes the encoded bundle under its content-addressed key. Retries share that key, so a
/// failed or ambiguous DB commit must not delete it: another request may already reference it.
async fn put(
    store: &FlowLikeStore,
    device_id: &str,
    archive_id: &str,
    encoded: String,
) -> Result<Path, ApiError> {
    let path = payload_path(device_id, archive_id, encoded.as_bytes());
    store
        .as_generic()
        .put(&path, encoded.into_bytes().into())
        .await
        .map_err(|error| {
            tracing::warn!(%error, %path, "Device archive upload to content storage failed");
            ApiError::service_unavailable("Archive content storage is unavailable")
        })?;
    Ok(path)
}

pub(super) async fn stage(
    store: &FlowLikeStore,
    manifest: &ArchiveManifest,
    bundle: &EncryptedArchive,
    encoded: String,
) -> Result<StoredArchive, ApiError> {
    if encoded.len() > MAX_ARCHIVE_BYTES {
        return Err(ApiError::bad_request("Archive exceeds the transfer limit"));
    }
    let size_bytes = encoded.len() as i64;
    let path = put(store, &manifest.device_id, &manifest.archive_id, encoded).await?;
    Ok(StoredArchive {
        bundle: None,
        payload_ref: Some(path.to_string()),
        manifest_jws: Some(bundle.manifest_jws.clone()),
        roster_jws: Some(bundle.roster_jws.clone()),
        size_bytes,
    })
}

pub(super) async fn load(
    store: &FlowLikeStore,
    manifest: &ArchiveManifest,
    reference: &str,
    size_bytes: i64,
) -> Result<EncryptedArchive, ApiError> {
    let prefix = archive_prefix(&manifest.device_id, &manifest.archive_id);
    let hash = reference
        .strip_prefix(&prefix)
        .and_then(|suffix| suffix.strip_suffix(".json"))
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()));
    if hash.is_none() || !(1..=MAX_ARCHIVE_BYTES as i64).contains(&size_bytes) {
        return Err(ApiError::internal("Retained archive reference is invalid"));
    }
    let result = store
        .as_generic()
        .get(&Path::from(reference))
        .await
        .map_err(|error| {
            tracing::warn!(%error, "Retained device archive could not be read");
            ApiError::service_unavailable("Retained archive content is unavailable")
        })?;
    if result.meta.size != size_bytes as u64 {
        return Err(ApiError::internal(
            "Retained archive size differs from its metadata",
        ));
    }
    let body = result.bytes().await.map_err(|error| {
        tracing::warn!(%error, "Retained device archive download failed");
        ApiError::service_unavailable("Retained archive content is unavailable")
    })?;
    if body.len() != size_bytes as usize
        || payload_path(&manifest.device_id, &manifest.archive_id, &body).as_ref() != reference
    {
        return Err(ApiError::internal("Retained archive content digest failed"));
    }
    serde_json::from_slice(&body)
        .map_err(|_| ApiError::internal("Retained archive content is invalid"))
}

/// One retained-history chain: a device's archives of one scope and kind.
struct Stream {
    device_id: String,
    scope: String,
    kind: String,
    head: i64,
}

impl Stream {
    fn from_head(row: &QueryResult) -> Result<Self, ApiError> {
        Ok(Self {
            device_id: row.try_get("", "deviceId")?,
            scope: row.try_get("", "scope")?,
            kind: row.try_get("", "kind")?,
            head: row.try_get("", "sequence")?,
        })
    }

    fn values(&self) -> [sea_orm::Value; 3] {
        [
            self.device_id.clone().into(),
            self.scope.clone().into(),
            self.kind.clone().into(),
        ]
    }

    /// The stream's first row at or after `from`, and whether it holds its bundle inline.
    async fn first_from<C: ConnectionTrait>(
        &self,
        db: &C,
        from: i64,
    ) -> Result<Option<(i64, bool)>, ApiError> {
        db.query_one_raw(sql(
            r#"SELECT sequence,"payloadRef" IS NULL AS inline FROM "DeviceArchive" WHERE "deviceId"=$1 AND scope=$2 AND kind=$3 AND sequence>=$4 ORDER BY sequence LIMIT 1"#,
            self.values().into_iter().chain([from.into()]),
        ))
        .await?
        .map(|row| Ok((row.try_get("", "sequence")?, row.try_get("", "inline")?)))
        .transpose()
    }

    /// Inline rows predate the content bucket, so they are a prefix of the stream, and the
    /// offload converts that prefix from its top down to keep it one. Returns the sequence
    /// just past the prefix, or `None` when the stream keeps no inline row.
    async fn inline_end<C: ConnectionTrait>(&self, db: &C) -> Result<Option<i64>, ApiError> {
        let Some((mut low, true)) = self.first_from(db, 0).await? else {
            return Ok(None);
        };
        let mut high = self.head.saturating_add(1);
        while high - low > 1 {
            let middle = low + (high - low) / 2;
            match self.first_from(db, middle).await? {
                Some((sequence, true)) => low = sequence,
                _ => high = middle,
            }
        }
        Ok(Some(high))
    }
}

/// A legacy row whose bundle is ready to move.
struct Inline {
    archive_id: String,
    encoded: String,
    manifest_jws: String,
    roster_jws: String,
}

impl Inline {
    /// `None` once the row is already offloaded or expired: rows below it are older still.
    fn parse(row: &QueryResult, now: i64) -> Result<Option<Self>, ApiError> {
        let expires: i64 = row.try_get("", "expiresAt")?;
        let encoded = row.try_get::<Option<String>>("", "bundle")?;
        let Some(encoded) = encoded.filter(|_| expires > now) else {
            return Ok(None);
        };
        let archive_id: String = row.try_get("", "archiveId")?;
        let digest: String = row.try_get("", "digest")?;
        let bundle = Self::bound(
            &archive_id,
            &encoded,
            row.try_get("", "sizeBytes")?,
            &digest,
        )?;
        Ok(Some(Self {
            archive_id,
            encoded,
            manifest_jws: bundle.manifest_jws,
            roster_jws: bundle.roster_jws,
        }))
    }

    /// The bundle, when it is the one its index row was written for.
    fn bound(
        archive_id: &str,
        encoded: &str,
        size: i64,
        digest: &str,
    ) -> Result<EncryptedArchive, ApiError> {
        let bundle: EncryptedArchive = serde_json::from_str(encoded).map_err(|error| {
            ApiError::internal(format!(
                "Inline device archive {archive_id} is not a bundle: {error}"
            ))
        })?;
        if encoded.len() as i64 != size || compact_digest(&bundle.manifest_jws) != digest {
            return Err(ApiError::internal(format!(
                "Inline device archive {archive_id} differs from its index row ({} bytes, {size} recorded)",
                encoded.len()
            )));
        }
        Ok(bundle)
    }
}

/// Moves the newest inline rows of one stream below `end`, newest first, and stops at the
/// first row it cannot move so the inline rows stay a prefix.
async fn offload_stream<C: ConnectionTrait>(
    db: &C,
    store: &FlowLikeStore,
    stream: &Stream,
    end: i64,
    now: i64,
    limit: usize,
) -> Result<usize, ApiError> {
    let rows = db.query_all_raw(sql(
        &format!(r#"SELECT "archiveId",bundle,digest,"sizeBytes","expiresAt" FROM "DeviceArchive" WHERE "deviceId"=$1 AND scope=$2 AND kind=$3 AND sequence<$4 ORDER BY sequence DESC LIMIT {limit}"#),
        stream.values().into_iter().chain([end.into()]),
    )).await?;
    let mut ready = Vec::with_capacity(rows.len());
    for row in &rows {
        match Inline::parse(row, now) {
            Ok(Some(inline)) => ready.push(inline),
            Ok(None) => break,
            Err(error) => {
                tracing::warn!(%error, device_id = %stream.device_id, scope = %stream.scope, kind = %stream.kind, "Inline device archive stays in the database");
                break;
            }
        }
    }
    let mut uploads = futures::stream::iter(ready)
        .map(|mut inline| async move {
            let encoded = std::mem::take(&mut inline.encoded);
            let path = put(store, &stream.device_id, &inline.archive_id, encoded).await;
            (inline, path)
        })
        .buffered(OFFLOAD_UPLOADS);
    let mut moved = 0;
    while let Some((inline, path)) = uploads.next().await {
        let Ok(path) = path else { break };
        // Readers see the inline bundle or the reference, never neither.
        moved += db.execute_raw(sql(
            r#"UPDATE "DeviceArchive" SET bundle=NULL,"payloadRef"=$3,"manifestJws"=$4,"rosterJws"=$5 WHERE "deviceId"=$1 AND "archiveId"=$2 AND bundle IS NOT NULL"#,
            [stream.device_id.clone().into(), inline.archive_id.into(), path.to_string().into(), inline.manifest_jws.into(), inline.roster_jws.into()],
        )).await?.rows_affected() as usize;
    }
    Ok(moved)
}

/// Moves up to `budget` archive bundles stored inline before the content bucket existed into
/// it, under the key `stage` would have used. Idempotent and safe to run concurrently; once no
/// inline row remains a pass costs one head scan and one probe per stream.
pub(super) async fn offload_inline<C: ConnectionTrait>(
    db: &C,
    store: &FlowLikeStore,
    now: i64,
    budget: usize,
) -> Result<usize, ApiError> {
    let heads = db
        .query_all_raw(sql(
            r#"SELECT "deviceId",scope,kind,sequence FROM "DeviceArchiveHead""#,
            [],
        ))
        .await?;
    let mut moved = 0;
    for head in heads {
        if moved >= budget {
            break;
        }
        let stream = Stream::from_head(&head)?;
        if let Some(end) = stream.inline_end(db).await? {
            moved += offload_stream(db, store, &stream, end, now, budget - moved).await?;
        }
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use flow_like_device_protocol::ArchiveKind;
    use flow_like_storage::object_store::{ObjectStore, memory::InMemory};
    use futures::TryStreamExt;
    use std::sync::Arc;

    fn store() -> FlowLikeStore {
        FlowLikeStore::Memory(Arc::new(InMemory::new()))
    }

    fn fixture() -> (ArchiveManifest, EncryptedArchive) {
        let manifest = ArchiveManifest {
            version: 1,
            device_id: "device-one".into(),
            scope: "device".into(),
            kind: ArchiveKind::Logs,
            archive_id: "archive-one".into(),
            sequence: 1,
            previous_manifest_digest: None,
            roster_digest: "verified-by-caller".into(),
            created_at: 100,
            nonce: [7; 24],
            ciphertext_size: 32,
            ciphertext_digest: "verified-by-caller".into(),
            recipients: Vec::new(),
        };
        let bundle = EncryptedArchive {
            manifest_jws: "manifest-✓".into(),
            roster_jws: "roster".into(),
            ciphertext: "opaque-ciphertext".into(),
            recipient_keys: Vec::new(),
        };
        (manifest, bundle)
    }

    async fn object_count(store: &FlowLikeStore) -> usize {
        store
            .as_generic()
            .list(None)
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .len()
    }

    #[tokio::test]
    async fn staged_archive_round_trips_with_only_metadata_in_the_row() {
        let store = store();
        let (manifest, bundle) = fixture();
        let encoded = serde_json::to_string(&bundle).unwrap();
        let stored = stage(&store, &manifest, &bundle, encoded.clone())
            .await
            .unwrap();
        assert!(stored.bundle.is_none());
        assert_eq!(
            stored.manifest_jws.as_deref(),
            Some(bundle.manifest_jws.as_str())
        );
        assert_eq!(
            stored.roster_jws.as_deref(),
            Some(bundle.roster_jws.as_str())
        );
        assert_eq!(stored.size_bytes, encoded.len() as i64);
        assert!(encoded.len() > encoded.chars().count());
        let reference = stored.payload_ref.as_deref().unwrap();
        assert!(reference.starts_with("device-archives/"));
        let downloaded = load(&store, &manifest, reference, stored.size_bytes)
            .await
            .unwrap();
        assert_eq!(serde_json::to_string(&downloaded).unwrap(), encoded);
        assert_eq!(object_count(&store).await, 1);
    }

    #[tokio::test]
    async fn retried_bundle_reuses_one_object_and_changed_content_gets_another_key() {
        let store = store();
        let (manifest, mut bundle) = fixture();
        let encoded = serde_json::to_string(&bundle).unwrap();
        let first = stage(&store, &manifest, &bundle, encoded.clone())
            .await
            .unwrap();
        let second = stage(&store, &manifest, &bundle, encoded).await.unwrap();
        assert_eq!(first.payload_ref, second.payload_ref);
        assert_eq!(object_count(&store).await, 1);

        bundle.ciphertext.push('x');
        let changed = stage(
            &store,
            &manifest,
            &bundle,
            serde_json::to_string(&bundle).unwrap(),
        )
        .await
        .unwrap();
        assert_ne!(first.payload_ref, changed.payload_ref);
        assert_eq!(object_count(&store).await, 2);
    }

    #[tokio::test]
    async fn references_cannot_escape_the_prefix_or_name_another_archive() {
        let store = store();
        let (manifest, bundle) = fixture();
        let encoded = serde_json::to_string(&bundle).unwrap();
        let stored = stage(&store, &manifest, &bundle, encoded).await.unwrap();
        let reference = stored.payload_ref.as_deref().unwrap();
        let prefix = archive_prefix(&manifest.device_id, &manifest.archive_id);
        let hash = "a".repeat(64);
        for invalid in [
            reference.replacen("device-archives/", "apps/", 1),
            format!("store://{reference}"),
            format!("{prefix}../{hash}.json"),
            format!("{prefix}{hash}.json/extra"),
            format!("{prefix}{}.json", "g".repeat(64)),
            format!("{prefix}{}.json", "a".repeat(63)),
        ] {
            assert_eq!(
                load(&store, &manifest, &invalid, stored.size_bytes)
                    .await
                    .unwrap_err()
                    .status(),
                StatusCode::INTERNAL_SERVER_ERROR,
                "accepted invalid reference: {invalid}"
            );
        }
        let mut other_device = manifest.clone();
        other_device.device_id = "device-two".into();
        let mut other_archive = manifest.clone();
        other_archive.archive_id = "archive-two".into();
        for other in [other_device, other_archive] {
            assert_eq!(
                load(&store, &other, reference, stored.size_bytes)
                    .await
                    .unwrap_err()
                    .status(),
                StatusCode::INTERNAL_SERVER_ERROR
            );
        }
    }

    #[tokio::test]
    async fn missing_or_corrupted_objects_never_return_a_bundle() {
        let store = store();
        let (manifest, bundle) = fixture();
        let encoded = serde_json::to_string(&bundle).unwrap();
        let stored = stage(&store, &manifest, &bundle, encoded.clone())
            .await
            .unwrap();
        let reference = stored.payload_ref.as_deref().unwrap();
        let path = Path::from(reference);
        let mut changed = encoded.into_bytes();
        changed[0] = b'[';
        store.as_generic().put(&path, changed.into()).await.unwrap();
        assert_eq!(
            load(&store, &manifest, reference, stored.size_bytes)
                .await
                .unwrap_err()
                .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        store.as_generic().delete(&path).await.unwrap();
        assert_eq!(
            load(&store, &manifest, reference, stored.size_bytes)
                .await
                .unwrap_err()
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn matching_hash_does_not_allow_invalid_json_or_wrong_size_metadata() {
        let store = store();
        let (manifest, bundle) = fixture();
        let invalid_body = b"not an archive";
        let path = payload_path(&manifest.device_id, &manifest.archive_id, invalid_body);
        store
            .as_generic()
            .put(&path, invalid_body.to_vec().into())
            .await
            .unwrap();
        assert_eq!(
            load(&store, &manifest, path.as_ref(), invalid_body.len() as i64)
                .await
                .unwrap_err()
                .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );

        let stored = stage(
            &store,
            &manifest,
            &bundle,
            serde_json::to_string(&bundle).unwrap(),
        )
        .await
        .unwrap();
        for size in [
            -1,
            0,
            stored.size_bytes - 1,
            stored.size_bytes + 1,
            MAX_ARCHIVE_BYTES as i64 + 1,
        ] {
            assert_eq!(
                load(
                    &store,
                    &manifest,
                    stored.payload_ref.as_deref().unwrap(),
                    size
                )
                .await
                .unwrap_err()
                .status(),
                StatusCode::INTERNAL_SERVER_ERROR
            );
        }
    }

    #[tokio::test]
    async fn transfer_limit_allows_exact_size_and_rejects_excess_before_uploading() {
        let store = store();
        let (manifest, mut bundle) = fixture();
        let encoded_size = serde_json::to_string(&bundle).unwrap().len();
        bundle
            .ciphertext
            .push_str(&"x".repeat(MAX_ARCHIVE_BYTES - encoded_size));
        let encoded = serde_json::to_string(&bundle).unwrap();
        assert_eq!(encoded.len(), MAX_ARCHIVE_BYTES);
        let stored = stage(&store, &manifest, &bundle, encoded).await.unwrap();
        assert_eq!(stored.size_bytes, MAX_ARCHIVE_BYTES as i64);
        assert!(
            load(
                &store,
                &manifest,
                stored.payload_ref.as_deref().unwrap(),
                stored.size_bytes
            )
            .await
            .is_ok()
        );
        bundle.ciphertext.push('x');
        let failure = stage(
            &store,
            &manifest,
            &bundle,
            serde_json::to_string(&bundle).unwrap(),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(failure.status(), StatusCode::BAD_REQUEST);
        assert_eq!(object_count(&store).await, 1);
    }

    #[tokio::test]
    async fn failed_upload_does_not_return_a_database_reference() {
        let store = store().read_only();
        let (manifest, bundle) = fixture();
        let failure = stage(
            &store,
            &manifest,
            &bundle,
            serde_json::to_string(&bundle).unwrap(),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(failure.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(object_count(&store).await, 0);
    }
}
