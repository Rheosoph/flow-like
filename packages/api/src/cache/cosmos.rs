//! Azure Cosmos DB for NoSQL cache backend.
//!
//! The Terraform container must use `/app_id` as its partition key and enable TTL with
//! a default of `-1`. Each expiring document gets a relative `ttl`; eternal entries omit
//! it. Authentication is delegated to [`crate::cosmos::CosmosClient`] and is Entra-only.

use super::types::*;
use crate::cosmos::{
    CosmosClient, CosmosError, MutationOutcome, QueryParameter, ttl_seconds, validate_container_id,
};
use async_trait::async_trait;
use chrono::Utc;
use flow_like_types::cache::CacheScope;
use futures::{StreamExt, stream};
use serde::{Deserialize, Serialize};

const DEFAULT_CONTAINER: &str = "cache";
const DELETE_PAGE_SIZE: i32 = 1_000;
const DELETE_CONCURRENCY: usize = 16;
const MAX_DELETE_PAGES: usize = 10_000;
const CONDITIONAL_INSERT_ATTEMPTS: usize = 3;

#[derive(Clone, Debug)]
pub struct CosmosCacheStore {
    client: CosmosClient,
    container: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CacheDocument {
    id: String,
    app_id: String,
    sort_key: String,
    key: String,
    scope: String,
    user_id: String,
    namespace: String,
    value: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<i64>,
    updated_at: i64,
    #[serde(rename = "ttl", skip_serializing_if = "Option::is_none")]
    ttl: Option<i64>,
    #[serde(rename = "_etag", default, skip_serializing)]
    etag: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ExpiryProjection {
    #[serde(default)]
    expires_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
struct DeleteProjection {
    id: String,
    #[serde(rename = "_etag")]
    etag: String,
}

impl CosmosCacheStore {
    pub fn from_env() -> Result<Self, CacheStoreError> {
        let client = CosmosClient::from_env().map_err(map_error)?;
        let container = std::env::var("COSMOS_CACHE_CONTAINER")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_CONTAINER.to_string());
        validate_container_id(&container).map_err(map_error)?;
        Ok(Self { client, container })
    }

    fn document_id(key: &CacheKey) -> String {
        // Cosmos IDs cannot contain '#', while the cross-backend sort key intentionally
        // does. A full BLAKE3 hex digest is path-safe and keeps IDs fixed-width.
        blake3::hash(key.sort_key().as_bytes()).to_hex().to_string()
    }

    fn document(entry: &SetCacheEntry, now_ms: i64) -> CacheDocument {
        CacheDocument {
            id: Self::document_id(&entry.key),
            app_id: entry.key.app_id.clone(),
            sort_key: entry.key.sort_key(),
            key: entry.key.key.clone(),
            scope: entry.key.scope.as_str().to_string(),
            user_id: entry.key.user_id.clone(),
            namespace: entry.key.namespace.clone(),
            value: entry.value.clone(),
            expires_at: entry.expires_at,
            updated_at: now_ms,
            ttl: ttl_seconds(entry.expires_at, now_ms),
            etag: None,
        }
    }

    fn to_entry(document: CacheDocument) -> CacheEntry {
        CacheEntry {
            key: document.key,
            value: document.value,
            expires_at: document.expires_at,
            updated_at: document.updated_at,
        }
    }

    async fn read_document(
        &self,
        key: &CacheKey,
    ) -> Result<Option<CacheDocument>, CacheStoreError> {
        self.client
            .read_document(&self.container, &Self::document_id(key), &key.app_id)
            .await
            .map_err(map_error)
    }

    /// Read forward until a page actually carries documents. Cosmos can answer with an
    /// empty page that only holds a continuation token, so an empty first page must never
    /// be treated as an exhausted result set.
    async fn next_delete_page(
        &self,
        query: &str,
        parameters: &[QueryParameter],
        app_id: &str,
    ) -> Result<Vec<DeleteProjection>, CacheStoreError> {
        let mut continuation: Option<String> = None;
        loop {
            let page = self
                .client
                .query_page::<DeleteProjection>(
                    &self.container,
                    query,
                    parameters,
                    Some(app_id),
                    DELETE_PAGE_SIZE,
                    continuation.as_deref(),
                )
                .await
                .map_err(map_error)?;
            if !page.documents.is_empty() {
                return Ok(page.documents);
            }
            match page.continuation {
                Some(token) => continuation = Some(token),
                None => return Ok(Vec::new()),
            }
        }
    }

    async fn delete_matching(
        &self,
        query: &str,
        parameters: &[QueryParameter],
        app_id: &str,
    ) -> Result<i64, CacheStoreError> {
        let mut deleted = 0_i64;

        // Query from the start after every page. Removing the selected page advances
        // the next request without relying on a continuation token invalidated by those
        // deletes, and bounds memory to 1,000 IDs.
        for _ in 0..MAX_DELETE_PAGES {
            let documents = self.next_delete_page(query, parameters, app_id).await?;
            if documents.is_empty() {
                return Ok(deleted);
            }

            let client = &self.client;
            let container = &self.container;
            let outcomes = stream::iter(documents.into_iter().map(|document| async move {
                client
                    .delete_document(container, &document.id, app_id, Some(&document.etag))
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
            "Cosmos bulk invalidation exceeded {MAX_DELETE_PAGES} pages; writes may be \
             continuously recreating the selected cache entries"
        )))
    }
}

fn map_error(error: CosmosError) -> CacheStoreError {
    match error {
        CosmosError::Configuration(message) => CacheStoreError::Configuration(message),
        CosmosError::Authentication(message) | CosmosError::Transport(message) => {
            CacheStoreError::Connection(message)
        }
        CosmosError::Serialization(message) => CacheStoreError::Serialization(message),
        error @ CosmosError::Service { .. } => CacheStoreError::Database(error.to_string()),
    }
}

#[async_trait]
impl CacheStore for CosmosCacheStore {
    fn backend_name(&self) -> &'static str {
        "cosmos"
    }

    async fn get(&self, key: &CacheKey) -> Result<Option<CacheEntry>, CacheStoreError> {
        let Some(document) = self.read_document(key).await? else {
            return Ok(None);
        };
        if document
            .expires_at
            .is_some_and(|expires_at| expires_at <= Utc::now().timestamp_millis())
        {
            return Ok(None);
        }
        Ok(Some(Self::to_entry(document)))
    }

    async fn exists(&self, key: &CacheKey) -> Result<bool, CacheStoreError> {
        let page = self
            .client
            .query_page::<ExpiryProjection>(
                &self.container,
                "SELECT c.expires_at FROM c WHERE c.id = @id",
                &[QueryParameter::new("@id", Self::document_id(key))],
                Some(&key.app_id),
                1,
                None,
            )
            .await
            .map_err(map_error)?;
        let now = Utc::now().timestamp_millis();
        Ok(page
            .documents
            .first()
            .is_some_and(|entry| entry.expires_at.is_none_or(|expiry| expiry > now)))
    }

    async fn set(&self, entry: SetCacheEntry) -> Result<CacheEntry, CacheStoreError> {
        let now = Utc::now().timestamp_millis();
        let document = Self::document(&entry, now);
        self.client
            .upsert_document(&self.container, &entry.key.app_id, &document)
            .await
            .map_err(map_error)?;
        Ok(Self::to_entry(document))
    }

    async fn try_insert(
        &self,
        entry: SetCacheEntry,
    ) -> Result<Option<CacheEntry>, CacheStoreError> {
        let now = Utc::now().timestamp_millis();
        let document = Self::document(&entry, now);
        for _ in 0..CONDITIONAL_INSERT_ATTEMPTS {
            let outcome = self
                .client
                .create_document(&self.container, &entry.key.app_id, &document)
                .await
                .map_err(map_error)?;
            if outcome == MutationOutcome::Applied {
                return Ok(Some(Self::to_entry(document)));
            }
            if outcome != MutationOutcome::Conflict {
                return Err(CacheStoreError::Database(format!(
                    "unexpected Cosmos conditional create result: {outcome:?}"
                )));
            }

            // A physically present but logically expired row does not count as a
            // conflict. Replace it under its ETag so only one contender can win.
            let Some(existing) = self.read_document(&entry.key).await? else {
                // Cosmos TTL deleted the conflicting row between the create and read.
                // Loop back and create it rather than reporting a false conflict.
                continue;
            };
            if existing
                .expires_at
                .is_none_or(|expires_at| expires_at > Utc::now().timestamp_millis())
            {
                return Ok(None);
            }
            let etag = existing.etag.as_deref().ok_or_else(|| {
                CacheStoreError::Serialization(
                    "Cosmos returned an expired cache document without an ETag".to_string(),
                )
            })?;

            match self
                .client
                .replace_document(
                    &self.container,
                    &document.id,
                    &entry.key.app_id,
                    &document,
                    Some(etag),
                )
                .await
                .map_err(map_error)?
            {
                MutationOutcome::Applied => return Ok(Some(Self::to_entry(document))),
                MutationOutcome::NotFound
                | MutationOutcome::Conflict
                | MutationOutcome::PreconditionFailed => continue,
            }
        }

        Err(CacheStoreError::Contention(format!(
            "Cosmos cache key '{}' changed during {CONDITIONAL_INSERT_ATTEMPTS} conditional inserts",
            entry.key.key
        )))
    }

    async fn delete(&self, key: &CacheKey) -> Result<bool, CacheStoreError> {
        let outcome = self
            .client
            .delete_document(&self.container, &Self::document_id(key), &key.app_id, None)
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
        self.delete_matching(
            "SELECT c.id, c._etag FROM c WHERE c.scope = @scope AND c.user_id = @user_id \
             AND c.namespace = @namespace",
            &[
                QueryParameter::new("@scope", scope.as_str()),
                QueryParameter::new("@user_id", user_id),
                QueryParameter::new("@namespace", namespace),
            ],
            app_id,
        )
        .await
    }

    async fn delete_app(&self, app_id: &str) -> Result<i64, CacheStoreError> {
        self.delete_matching("SELECT c.id, c._etag FROM c", &[], app_id)
            .await
    }

    async fn delete_expired(&self) -> Result<i64, CacheStoreError> {
        // Native Cosmos TTL reclaims these documents. Reads still enforce the absolute
        // expiry timestamp because TTL deletion is intentionally asynchronous.
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn document_ids_are_path_safe_and_include_the_complete_cache_identity() {
        let first = CacheKey::app("app", "namespace", "key");
        let second = CacheKey::user("app", "user", "namespace", "key");
        let first_id = CosmosCacheStore::document_id(&first);
        assert_eq!(first_id.len(), 64);
        assert!(
            first_id
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        );
        assert_ne!(first_id, CosmosCacheStore::document_id(&second));
    }

    #[test]
    fn cache_document_uses_cosmos_native_ttl_seconds() {
        let entry = SetCacheEntry {
            key: CacheKey::app("app", "", "key"),
            value: json!({"ok": true}),
            expires_at: Some(2_001),
        };
        let document = CosmosCacheStore::document(&entry, 1_000);
        assert_eq!(document.ttl, Some(2));
        assert_eq!(document.app_id, "app");
    }
}
