use flow_like_storage::object_store::{
    Error as StoreError, ObjectStore, ObjectStoreExt, PutPayload, path::Path,
};
use flow_like_types::{Error, Result};
use serde::{Deserialize, Serialize};

const MAX_RECORD_BYTES: u64 = 16 * 1024;
const CACHE_PREFIX: &str = "bit-mirrors/v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MirroredArtifact {
    pub object_key: String,
    pub hash: String,
    pub size: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MirrorRecord {
    etag: String,
    artifact: MirroredArtifact,
    cdn_etag: Option<String>,
    cdn_version: Option<String>,
}

pub(super) fn strong_etag(value: Option<&str>) -> Option<String> {
    let value = value?.trim_matches([' ', '\t']);
    let opaque = value.strip_prefix('"')?.strip_suffix('"')?;
    if opaque.is_empty()
        || !opaque
            .bytes()
            .all(|byte| byte == 0x21 || (0x23..=0x7e).contains(&byte) || byte >= 0x80)
    {
        return None;
    }
    Some(value.to_string())
}

fn identity(domain: &[u8], fields: &[&str]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    for field in fields {
        hasher.update(&(field.len() as u64).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn record_path(source_url: &str) -> Path {
    let key = identity(b"flow-like-bit-mirror-source-v1", &[source_url]);
    Path::from(format!("{CACHE_PREFIX}/{key}.json"))
}

pub(super) fn object_key(source_url: &str, etag: &str) -> String {
    let key = identity(b"flow-like-bit-mirror-object-v1", &[source_url, etag]);
    format!("bits/{key}")
}

fn validate_artifact(source_url: &str, etag: &str, artifact: &MirroredArtifact) -> Result<()> {
    if strong_etag(Some(etag)).as_deref() != Some(etag)
        || artifact.object_key != object_key(source_url, etag)
        || artifact.size > i64::MAX as u64
        || artifact.hash.len() != 64
        || !artifact.hash.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(Error::msg("Invalid Bit mirror cache identity"));
    }
    Ok(())
}

pub(super) async fn lookup(
    cache: &dyn ObjectStore,
    cdn: &dyn ObjectStore,
    source_url: &str,
    etag: &str,
    content_length: Option<u64>,
) -> Result<Option<MirroredArtifact>> {
    if strong_etag(Some(etag)).as_deref() != Some(etag) {
        return Ok(None);
    }
    let response = match cache.get(&record_path(source_url)).await {
        Ok(response) => response,
        Err(StoreError::NotFound { .. }) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if response.meta.size > MAX_RECORD_BYTES {
        return Err(Error::msg("Bit mirror cache record exceeds its size limit"));
    }
    let bytes = response.bytes().await?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(Error::msg("Bit mirror cache record exceeds its size limit"));
    }
    let record: MirrorRecord = serde_json::from_slice(&bytes)?;
    if record.etag != etag {
        return Ok(None);
    }
    validate_artifact(source_url, etag, &record.artifact)?;
    if content_length.is_some_and(|size| size != record.artifact.size)
        || (record.cdn_etag.is_none() && record.cdn_version.is_none())
    {
        return Ok(None);
    }
    let metadata = match cdn
        .head(&Path::from(record.artifact.object_key.clone()))
        .await
    {
        Ok(metadata) => metadata,
        Err(StoreError::NotFound { .. }) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.size != record.artifact.size
        || metadata.e_tag != record.cdn_etag
        || metadata.version != record.cdn_version
    {
        return Ok(None);
    }
    Ok(Some(record.artifact))
}

pub(super) async fn record(
    cache: &dyn ObjectStore,
    cdn: &dyn ObjectStore,
    source_url: &str,
    etag: &str,
    artifact: MirroredArtifact,
) -> Result<()> {
    validate_artifact(source_url, etag, &artifact)?;
    let metadata = cdn.head(&Path::from(artifact.object_key.clone())).await?;
    if metadata.size != artifact.size {
        return Err(Error::msg(
            "Mirrored Bit size differs from the completed download",
        ));
    }
    if metadata.e_tag.is_none() && metadata.version.is_none() {
        return Err(Error::msg("Mirrored Bit has no storage validator"));
    }
    let record = MirrorRecord {
        etag: etag.to_string(),
        artifact,
        cdn_etag: metadata.e_tag,
        cdn_version: metadata.version,
    };
    let bytes = serde_json::to_vec(&record)?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(Error::msg("Bit mirror cache record exceeds its size limit"));
    }
    cache
        .put(&record_path(source_url), PutPayload::from(bytes))
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::object_store::memory::InMemory;

    const SOURCE: &str = "https://models.example.test/model?token=private-value";
    const ETAG: &str = "\"version-1\"";
    const CONTENT: &[u8] = b"model bytes";

    fn artifact(source: &str, etag: &str) -> MirroredArtifact {
        MirroredArtifact {
            object_key: object_key(source, etag),
            hash: blake3::hash(CONTENT).to_hex().to_string(),
            size: CONTENT.len() as u64,
        }
    }

    async fn completed_mirror(cache: &InMemory, cdn: &InMemory) -> MirroredArtifact {
        let artifact = artifact(SOURCE, ETAG);
        cdn.put(
            &Path::from(artifact.object_key.clone()),
            PutPayload::from(CONTENT),
        )
        .await
        .unwrap();
        record(cache, cdn, SOURCE, ETAG, artifact.clone())
            .await
            .unwrap();
        artifact
    }

    #[test]
    fn accepts_only_strong_quoted_validators() {
        assert_eq!(
            strong_etag(Some(" \t\"version/1\" \t")),
            Some("\"version/1\"".into())
        );
        for invalid in [
            None,
            Some(""),
            Some("\"\""),
            Some("version-1"),
            Some("W/\"version-1\""),
            Some("\"two words\""),
            Some("\"embedded\"quote\""),
            Some("\"newline\n\""),
            Some("\"delete\u{7f}\""),
            Some("\"one\", \"two\""),
        ] {
            assert_eq!(strong_etag(invalid), None, "{invalid:?}");
        }
    }

    #[test]
    fn source_and_validator_both_scope_object_identity() {
        assert_eq!(object_key(SOURCE, ETAG), object_key(SOURCE, ETAG));
        assert_ne!(
            object_key(SOURCE, ETAG),
            object_key("https://other.example.test/model", ETAG)
        );
        assert_ne!(
            object_key(SOURCE, ETAG),
            object_key(SOURCE, "\"version-2\"")
        );
        assert_ne!(
            identity(b"test", &["ab", "c"]),
            identity(b"test", &["a", "bc"])
        );
    }

    #[tokio::test]
    async fn completed_mirror_is_reusable_without_a_bit_id_or_source_url_disclosure() {
        let cache = InMemory::new();
        let cdn = InMemory::new();
        let artifact = completed_mirror(&cache, &cdn).await;
        for content_length in [Some(CONTENT.len() as u64), None] {
            assert_eq!(
                lookup(&cache, &cdn, SOURCE, ETAG, content_length)
                    .await
                    .unwrap(),
                Some(artifact.clone())
            );
        }
        let bytes = cache
            .get(&record_path(SOURCE))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let json = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(!json.contains(SOURCE));
        assert!(!json.contains("private-value"));
        assert!(!record_path(SOURCE).as_ref().contains("private-value"));
    }

    #[tokio::test]
    async fn changed_sources_validators_and_lengths_require_a_new_transfer() {
        let cache = InMemory::new();
        let cdn = InMemory::new();
        completed_mirror(&cache, &cdn).await;
        for (source, etag, length) in [
            ("https://other.example.test/model", ETAG, None),
            (SOURCE, "\"version-2\"", None),
            (SOURCE, ETAG, Some(999)),
            (SOURCE, "W/\"version-1\"", None),
            (SOURCE, "", None),
        ] {
            assert_eq!(
                lookup(&cache, &cdn, source, etag, length).await.unwrap(),
                None
            );
        }
    }

    #[tokio::test]
    async fn missing_or_replaced_cdn_objects_are_never_reused() {
        let cache = InMemory::new();
        let cdn = InMemory::new();
        let artifact = completed_mirror(&cache, &cdn).await;
        let path = Path::from(artifact.object_key);
        cdn.put(&path, PutPayload::from_static(b"other bytes"))
            .await
            .unwrap();
        assert_eq!(
            lookup(&cache, &cdn, SOURCE, ETAG, None).await.unwrap(),
            None
        );
        completed_mirror(&cache, &cdn).await;
        cdn.put(&path, PutPayload::from_static(b"short"))
            .await
            .unwrap();
        assert_eq!(
            lookup(&cache, &cdn, SOURCE, ETAG, None).await.unwrap(),
            None
        );
        completed_mirror(&cache, &cdn).await;
        cdn.delete(&path).await.unwrap();
        assert_eq!(
            lookup(&cache, &cdn, SOURCE, ETAG, None).await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn an_unfinished_or_inconsistent_upload_cannot_publish_a_cache_record() {
        let cache = InMemory::new();
        let cdn = InMemory::new();
        let artifact = artifact(SOURCE, ETAG);
        assert!(
            record(&cache, &cdn, SOURCE, ETAG, artifact.clone())
                .await
                .is_err()
        );
        assert_eq!(
            lookup(&cache, &cdn, SOURCE, ETAG, None).await.unwrap(),
            None
        );

        cdn.put(
            &Path::from(artifact.object_key.clone()),
            PutPayload::from_static(b"partial"),
        )
        .await
        .unwrap();
        assert!(
            record(&cache, &cdn, SOURCE, ETAG, artifact.clone())
                .await
                .is_err()
        );
        assert_eq!(
            lookup(&cache, &cdn, SOURCE, ETAG, None).await.unwrap(),
            None
        );

        cdn.put(
            &Path::from(artifact.object_key.clone()),
            PutPayload::from(CONTENT),
        )
        .await
        .unwrap();
        let mut invalid = artifact.clone();
        invalid.hash = "client-supplied-id".into();
        assert!(record(&cache, &cdn, SOURCE, ETAG, invalid).await.is_err());
        assert!(
            record(
                &cache,
                &cdn,
                "https://other.example.test/model",
                ETAG,
                artifact
            )
            .await
            .is_err()
        );
        assert_eq!(
            lookup(&cache, &cdn, SOURCE, ETAG, None).await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn malformed_or_oversized_records_fail_without_authorizing_reuse() {
        let cache = InMemory::new();
        let cdn = InMemory::new();
        for bytes in [
            b"not json".to_vec(),
            vec![b' '; MAX_RECORD_BYTES as usize + 1],
        ] {
            cache
                .put(&record_path(SOURCE), PutPayload::from(bytes))
                .await
                .unwrap();
            assert!(lookup(&cache, &cdn, SOURCE, ETAG, None).await.is_err());
        }
    }

    #[tokio::test]
    async fn a_record_copied_from_another_source_cannot_authorize_reuse() {
        let cache = InMemory::new();
        let cdn = InMemory::new();
        completed_mirror(&cache, &cdn).await;
        let other_source = "https://other.example.test/model";
        cache
            .copy(&record_path(SOURCE), &record_path(other_source))
            .await
            .unwrap();
        assert!(
            lookup(&cache, &cdn, other_source, ETAG, None)
                .await
                .is_err()
        );
    }
}
