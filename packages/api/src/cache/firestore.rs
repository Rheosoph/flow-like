//! Google Firestore (Native mode) cache backend.
//!
//! The Terraform collection must carry a TTL policy on the `expires_at_ts` field and a
//! single-field index exemption on `value`; both are collection properties, so nothing
//! here can create them. Authentication is delegated to
//! [`flow_like_gcp_data::firestore::FirestoreClient`] and is metadata-server only.
//!
//! Forgetting the index exemption is the failure mode to watch for. Firestore indexes
//! every field it is not told to skip, one index entry stops at 7.5 KiB, and a write
//! that would exceed that is rejected — so without the exemption every cached value
//! larger than a few kilobytes fails at write time rather than degrading.
//!
//! Three things differ from the Cosmos backend this mirrors, all forced by Firestore:
//!
//! * **No partition key.** Cosmos scopes an entry by writing `/app_id` as the partition
//!   key and hashing only the sort key into the document ID. Firestore has one flat
//!   collection, so the app fingerprint moves into the ID itself — see `document_id`.
//! * **The payload is one opaque JSON string.** Firestore's field encoder refuses a map
//!   key containing `.`, `/` or a backtick and refuses an array nested directly inside
//!   an array. Cache values are arbitrary user JSON, so storing them as native fields
//!   would turn values that every other backend accepts into hard errors here.
//! * **TTL is a typed timestamp.** The TTL policy only collects a `timestampValue`,
//!   which no `serde` shape can express, so the client writes that field itself from the
//!   expiry handed to each write. The millisecond copy stored beside it is what reads
//!   compare and what [`CacheEntry`] returns, so the read path never has to parse a wire
//!   timestamp back.

use super::types::*;
use async_trait::async_trait;
use chrono::Utc;
use flow_like_gcp_data::firestore::{
    Document, FilterOperator, FirestoreClient, FirestoreError, MutationOutcome, QueryFilter,
    validate_collection_id,
};
use flow_like_types::cache::CacheScope;
use futures::{StreamExt, stream};
use serde::{Deserialize, Serialize, de::IgnoredAny};

const DEFAULT_COLLECTION: &str = "cache";

/// Documents per bulk-invalidation page.
///
/// Cosmos can ask for 1,000 because its delete scan projects two columns. This client's
/// `runQuery` carries no projection, so every scanned page arrives as whole documents and
/// the page size is really a memory bound: 100 keeps a page near 100 MB even if every
/// entry sits at the value ceiling.
const DELETE_PAGE_SIZE: i32 = 100;
const DELETE_CONCURRENCY: usize = 16;
const MAX_DELETE_PAGES: usize = 10_000;
const CONDITIONAL_INSERT_ATTEMPTS: usize = 3;

/// Firestore's hard document ceiling, counted over field names, field values and the
/// document's own resource name.
const MAX_DOCUMENT_BYTES: usize = 1_048_576;

/// Room reserved inside that ceiling for the digests, the timestamps, the resource name
/// and Firestore's per-document accounting overhead.
const DOCUMENT_OVERHEAD_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug)]
pub struct FirestoreCacheStore {
    client: FirestoreClient,
    collection: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CacheDocument {
    app_id: String,
    sort_key: String,
    key: String,
    scope: String,
    user_id: String,
    namespace: String,
    /// The serialized payload. See the module header for why it is not stored as native
    /// Firestore fields.
    value: String,
    /// Millisecond mirror of the `expires_at_ts` Timestamp the client writes alongside
    /// this document (see `flow_like_gcp_data::firestore::EXPIRES_AT_FIELD`). Absent for
    /// entries that never expire, which is also what keeps them out of the TTL sweep.
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at_ms: Option<i64>,
    updated_at: i64,
}

#[derive(Clone, Debug, Deserialize)]
struct ExpiryProjection {
    #[serde(default)]
    expires_at_ms: Option<i64>,
}

impl FirestoreCacheStore {
    pub fn from_env() -> Result<Self, CacheStoreError> {
        let client = FirestoreClient::from_env().map_err(map_error)?;
        let collection = std::env::var("FIRESTORE_CACHE_COLLECTION")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_COLLECTION.to_string());
        // The client re-validates the prefixed name on every call; checking the bare one
        // here turns a mistyped collection into a startup failure instead of a failure on
        // the first cache write.
        validate_collection_id(&collection).map_err(map_error)?;
        Ok(Self { client, collection })
    }

    /// Document ID: `<app fingerprint>.<entry digest>`.
    ///
    /// Cosmos gets app scoping for free — `/app_id` is the partition key, so two apps may
    /// share a document ID. Firestore has no partition key, so the app has to be part of
    /// the ID or one app's write lands on another app's entry. Both halves are BLAKE3
    /// digests: fixed width, free of the `#` the cross-backend sort key uses, and free of
    /// the `/` Firestore reads as a path separator. The separator is a `.` rather than
    /// the more obvious `:` because a colon is what Google's REST routing reads as the
    /// start of a custom method, and because `.` survives URL encoding unchanged.
    fn document_id(key: &CacheKey) -> String {
        format!(
            "{}.{}",
            segment_hash(&key.app_id),
            blake3::hash(key.sort_key().as_bytes()).to_hex()
        )
    }

    fn document(entry: &SetCacheEntry, now_ms: i64) -> Result<CacheDocument, CacheStoreError> {
        let value = serde_json::to_string(&entry.value)
            .map_err(|error| CacheStoreError::Serialization(error.to_string()))?;

        // An entry is one document, and `CACHE_MAX_VALUE_BYTES` defaults to exactly the
        // Firestore document ceiling — so a value the shared limits accept can still be
        // one Firestore refuses. Refusing it here names the limit and the way out;
        // letting it through produces an INVALID_ARGUMENT that names neither.
        let user_bytes = value.len()
            + entry.key.key.len()
            + entry.key.namespace.len()
            + entry.key.user_id.len()
            + entry.key.app_id.len();
        if user_bytes > MAX_DOCUMENT_BYTES - DOCUMENT_OVERHEAD_BYTES {
            return Err(CacheStoreError::InvalidInput(format!(
                "Cache entry is {user_bytes} bytes; Firestore stores it as a single \
                 document and rejects anything above {MAX_DOCUMENT_BYTES} bytes including \
                 its metadata. Lower CACHE_MAX_VALUE_BYTES for this deployment, or write \
                 large payloads to the app's storage and cache the path instead."
            )));
        }

        Ok(CacheDocument {
            app_id: entry.key.app_id.clone(),
            sort_key: entry.key.sort_key(),
            key: entry.key.key.clone(),
            scope: entry.key.scope.as_str().to_string(),
            user_id: entry.key.user_id.clone(),
            namespace: entry.key.namespace.clone(),
            value,
            expires_at_ms: entry.expires_at,
            updated_at: now_ms,
        })
    }

    /// The entry as it was written. Writes do not parse the payload back: the value they
    /// would parse is the one the caller just handed us.
    fn written_entry(entry: SetCacheEntry, now_ms: i64) -> CacheEntry {
        CacheEntry {
            key: entry.key.key,
            value: entry.value,
            expires_at: entry.expires_at,
            updated_at: now_ms,
        }
    }

    /// The entry as read back. The payload is opaque in Firestore, so this is where it
    /// becomes JSON again.
    fn read_entry(document: CacheDocument) -> Result<CacheEntry, CacheStoreError> {
        Ok(CacheEntry {
            key: document.key,
            value: serde_json::from_str(&document.value)
                .map_err(|error| CacheStoreError::Serialization(error.to_string()))?,
            expires_at: document.expires_at_ms,
            updated_at: document.updated_at,
        })
    }

    async fn read_document(
        &self,
        key: &CacheKey,
    ) -> Result<Option<Document<CacheDocument>>, CacheStoreError> {
        self.client
            .read_document(&self.collection, &Self::document_id(key))
            .await
            .map_err(map_error)
    }

    async fn delete_matching(&self, filters: &[QueryFilter]) -> Result<i64, CacheStoreError> {
        let mut deleted = 0_i64;

        // Query from the start after every page. Removing the selected page advances the
        // next request without a cursor, and unlike Cosmos an empty page here really is
        // the end of the result set — the client only emits a continuation for a page it
        // filled.
        for _ in 0..MAX_DELETE_PAGES {
            // Nothing from the document body is needed: the ID and the `updateTime` that
            // makes each delete a compare-and-swap both live in the envelope around it.
            let page = self
                .client
                .query_page::<IgnoredAny>(&self.collection, filters, &[], DELETE_PAGE_SIZE, None)
                .await
                .map_err(map_error)?;
            if page.documents.is_empty() {
                return Ok(deleted);
            }

            let client = &self.client;
            let collection = &self.collection;
            let outcomes = stream::iter(page.documents.into_iter().map(|document| async move {
                client
                    .delete_document(collection, &document.id, document.update_time.as_deref())
                    .await
            }))
            .buffer_unordered(DELETE_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;

            for outcome in outcomes {
                if outcome.map_err(map_error)? == MutationOutcome::Applied {
                    deleted += 1;
                }
            }
        }

        Err(CacheStoreError::Database(format!(
            "Firestore bulk invalidation exceeded {MAX_DELETE_PAGES} pages; writes may be \
             continuously recreating the selected cache entries"
        )))
    }
}

fn map_error(error: FirestoreError) -> CacheStoreError {
    match error {
        FirestoreError::Configuration(message) => CacheStoreError::Configuration(message),
        FirestoreError::Authentication(message) | FirestoreError::Transport(message) => {
            CacheStoreError::Connection(message)
        }
        FirestoreError::Serialization(message) => CacheStoreError::Serialization(message),
        error @ FirestoreError::Service { .. } => CacheStoreError::Database(error.to_string()),
    }
}

#[async_trait]
impl CacheStore for FirestoreCacheStore {
    fn backend_name(&self) -> &'static str {
        "firestore"
    }

    async fn get(&self, key: &CacheKey) -> Result<Option<CacheEntry>, CacheStoreError> {
        let Some(document) = self.read_document(key).await? else {
            return Ok(None);
        };
        if document
            .value
            .expires_at_ms
            .is_some_and(|expires_at| expires_at <= Utc::now().timestamp_millis())
        {
            return Ok(None);
        }
        Self::read_entry(document.value).map(Some)
    }

    async fn exists(&self, key: &CacheKey) -> Result<bool, CacheStoreError> {
        // Cosmos answers this with a projection query that returns one column. Firestore
        // has no field mask on this client and its queries carry whole documents anyway,
        // so what is saved here is decoding the payload, not transferring it.
        let document: Option<Document<ExpiryProjection>> = self
            .client
            .read_document(&self.collection, &Self::document_id(key))
            .await
            .map_err(map_error)?;
        let now = Utc::now().timestamp_millis();
        let live =
            document.is_some_and(|document| document.value.expires_at_ms.is_none_or(|ms| ms > now));
        Ok(live)
    }

    async fn set(&self, entry: SetCacheEntry) -> Result<CacheEntry, CacheStoreError> {
        let now = Utc::now().timestamp_millis();
        let id = Self::document_id(&entry.key);
        let document = Self::document(&entry, now)?;
        self.client
            .upsert_document(&self.collection, &id, &document, entry.expires_at)
            .await
            .map_err(map_error)?;
        Ok(Self::written_entry(entry, now))
    }

    async fn try_insert(
        &self,
        entry: SetCacheEntry,
    ) -> Result<Option<CacheEntry>, CacheStoreError> {
        let now = Utc::now().timestamp_millis();
        let id = Self::document_id(&entry.key);
        let document = Self::document(&entry, now)?;

        for _ in 0..CONDITIONAL_INSERT_ATTEMPTS {
            let outcome = self
                .client
                .create_document(&self.collection, &id, &document, entry.expires_at)
                .await
                .map_err(map_error)?;
            if outcome == MutationOutcome::Applied {
                return Ok(Some(Self::written_entry(entry, now)));
            }
            if outcome != MutationOutcome::Conflict {
                return Err(CacheStoreError::Database(format!(
                    "unexpected Firestore conditional create result: {outcome:?}"
                )));
            }

            // A physically present but logically expired document does not count as a
            // conflict. Replace it under its `updateTime` so only one contender can win.
            // Firestore's TTL sweep may trail the expiry by up to 24 hours, so this is
            // the ordinary path for a lapsed key rather than a rare one.
            let Some(existing) = self.read_document(&entry.key).await? else {
                // The conflicting document was collected between the create and the read.
                // Loop back and create it rather than reporting a false conflict.
                continue;
            };
            if existing
                .value
                .expires_at_ms
                .is_none_or(|expires_at| expires_at > Utc::now().timestamp_millis())
            {
                return Ok(None);
            }
            let update_time = existing.update_time.as_deref().ok_or_else(|| {
                CacheStoreError::Serialization(
                    "Firestore returned an expired cache document without an updateTime"
                        .to_string(),
                )
            })?;

            match self
                .client
                .replace_document(
                    &self.collection,
                    &id,
                    &document,
                    entry.expires_at,
                    Some(update_time),
                )
                .await
                .map_err(map_error)?
            {
                MutationOutcome::Applied => return Ok(Some(Self::written_entry(entry, now))),
                // Gone, or someone else replaced it first. Both mean re-read and retry.
                MutationOutcome::NotFound
                | MutationOutcome::Conflict
                | MutationOutcome::PreconditionFailed => continue,
            }
        }

        Err(CacheStoreError::Contention(format!(
            "Firestore cache key '{}' changed during {CONDITIONAL_INSERT_ATTEMPTS} \
             conditional inserts",
            entry.key.key
        )))
    }

    async fn delete(&self, key: &CacheKey) -> Result<bool, CacheStoreError> {
        let outcome = self
            .client
            .delete_document(&self.collection, &Self::document_id(key), None)
            .await
            .map_err(map_error)?;
        Ok(outcome == MutationOutcome::Applied)
    }

    async fn delete_namespace(
        &self,
        app_id: &str,
        scope: CacheScope,
        user_id: &str,
        namespace: &str,
    ) -> Result<i64, CacheStoreError> {
        if namespace.is_empty() {
            return Err(CacheStoreError::InvalidInput(
                "Namespace invalidation requires a non-empty namespace".to_string(),
            ));
        }
        // Every predicate is an equality, which is what keeps both bulk deletes servable
        // from the single-field indexes Firestore maintains on its own; a range or an
        // inequality would demand a composite index that no Terraform module creates.
        self.delete_matching(&[
            QueryFilter::new("app_id", FilterOperator::Equal, app_id),
            QueryFilter::new("scope", FilterOperator::Equal, scope.as_str()),
            QueryFilter::new("user_id", FilterOperator::Equal, user_id),
            QueryFilter::new("namespace", FilterOperator::Equal, namespace),
        ])
        .await
    }

    async fn delete_app(&self, app_id: &str) -> Result<i64, CacheStoreError> {
        self.delete_matching(&[QueryFilter::new("app_id", FilterOperator::Equal, app_id)])
            .await
    }

    async fn delete_expired(&self) -> Result<i64, CacheStoreError> {
        // The collection's TTL policy reclaims these documents. Reads still enforce the
        // absolute expiry because that sweep is asynchronous and only promises deletion
        // within 24 hours of the timestamp.
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_gcp_data::firestore::{from_fields, to_fields};
    use serde_json::json;

    #[test]
    fn document_ids_are_path_safe_and_scope_an_entry_to_its_app() {
        let key = CacheKey::app("app", "namespace", "key");
        let id = FirestoreCacheStore::document_id(&key);

        assert_eq!(id.len(), 97);
        assert!(
            id.chars()
                .all(|character| character.is_ascii_hexdigit() || character == '.')
        );
        assert_ne!(
            id,
            FirestoreCacheStore::document_id(&CacheKey::user("app", "user", "namespace", "key"))
        );
        // Without a partition key the ID is the only thing separating two apps.
        assert_ne!(
            id,
            FirestoreCacheStore::document_id(&CacheKey::app("other", "namespace", "key"))
        );
    }

    #[test]
    fn expiry_is_mirrored_in_millis_and_the_ttl_field_is_left_to_the_client() {
        let entry = SetCacheEntry {
            key: CacheKey::app("app", "", "key"),
            value: json!({"ok": true}),
            expires_at: Some(2_001),
        };
        let document = FirestoreCacheStore::document(&entry, 1_000).unwrap();
        assert_eq!(document.expires_at_ms, Some(2_001));
        assert_eq!(document.app_id, "app");

        // The TTL field is a typed timestamp written by the client, so the document must
        // not carry an `expires_at` of its own — a string there is never collected.
        let fields = to_fields(&document).unwrap();
        assert!(fields.get("expires_at").is_none());
    }

    #[test]
    fn values_firestore_cannot_hold_natively_still_round_trip() {
        // A '.' in a map key and an array directly inside an array are both hard errors
        // for Firestore's field encoder, and both are ordinary JSON everywhere else.
        // This is the test that pins the opaque-string payload.
        let value = json!({ "a.b": [[1, 2], [3]], "nested": { "c/d": null } });
        let entry = SetCacheEntry {
            key: CacheKey::app("app", "ns", "key"),
            value: value.clone(),
            expires_at: None,
        };

        let document = FirestoreCacheStore::document(&entry, 7).unwrap();
        let fields = to_fields(&document).expect("the document must survive field encoding");
        assert!(fields["value"]["stringValue"].is_string());

        let decoded: CacheDocument = from_fields(&fields).unwrap();
        let restored = FirestoreCacheStore::read_entry(decoded).unwrap();
        assert_eq!(restored.value, value);
        assert_eq!(restored.expires_at, None);
        assert_eq!(restored.updated_at, 7);
    }

    #[test]
    fn entries_too_large_for_a_document_are_refused_before_the_write() {
        let entry = SetCacheEntry {
            key: CacheKey::app("app", "", "key"),
            value: json!("x".repeat(MAX_DOCUMENT_BYTES)),
            expires_at: None,
        };
        assert!(matches!(
            FirestoreCacheStore::document(&entry, 0),
            Err(CacheStoreError::InvalidInput(_))
        ));
    }
}
