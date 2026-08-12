//! Types for the key/value cache store abstraction.

use async_trait::async_trait;
use flow_like_types::cache::CacheScope;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Debug;

/// Sentinel stored in the `user_id` column for app-scoped entries.
///
/// Postgres cannot put a nullable column in a composite primary key, and the Redis and
/// DynamoDB key builders must agree with it, so the sentinel is shared rather than
/// re-derived per backend.
pub const APP_SCOPE_USER_ID: &str = "";

/// Fixed 32-hex fingerprint of one storage-key segment.
///
/// Hashing is what makes the layout delimiter-proof: a value containing `#` cannot
/// impersonate another entry, and every segment has a known width, so aggregates
/// (namespace, user) are contiguous string ranges.
pub fn segment_hash(value: &str) -> String {
    blake3::hash(value.as_bytes()).to_hex()[..32].to_string()
}

/// Fully qualified identity of one cache entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheKey {
    pub app_id: String,
    pub scope: CacheScope,
    /// The owning user for `CacheScope::User`, [`APP_SCOPE_USER_ID`] otherwise.
    pub user_id: String,
    /// Optional grouping for bulk invalidation; empty means unnamespaced.
    pub namespace: String,
    pub key: String,
}

impl CacheKey {
    pub fn app(
        app_id: impl Into<String>,
        namespace: impl Into<String>,
        key: impl Into<String>,
    ) -> Self {
        Self {
            app_id: app_id.into(),
            scope: CacheScope::App,
            user_id: APP_SCOPE_USER_ID.to_string(),
            namespace: namespace.into(),
            key: key.into(),
        }
    }

    pub fn user(
        app_id: impl Into<String>,
        user_id: impl Into<String>,
        namespace: impl Into<String>,
        key: impl Into<String>,
    ) -> Self {
        Self {
            app_id: app_id.into(),
            scope: CacheScope::User,
            user_id: user_id.into(),
            namespace: namespace.into(),
            key: key.into(),
        }
    }

    /// Storage key used by the key/value backends, scoped under the app partition:
    /// `e#<scope>#h(user)#h(namespace)#h(key)`.
    ///
    /// The shape is what makes bulk invalidation cheap everywhere: every entry of one
    /// namespace — and every chunk item DynamoDB appends *below* an entry's key — shares
    /// the literal prefix [`CacheKey::namespace_sort_prefix`], so "delete a namespace"
    /// is a contiguous range query (DynamoDB `begins_with`) or a plain string prefix
    /// match (Redis), never a parse or an escape.
    pub fn sort_key(&self) -> String {
        format!(
            "{}{}",
            Self::namespace_sort_prefix(self.scope, &self.user_id, &self.namespace),
            segment_hash(&self.key)
        )
    }

    /// Prefix shared by every entry in one `(scope, user, namespace)` partition slice.
    pub fn namespace_sort_prefix(scope: CacheScope, user_id: &str, namespace: &str) -> String {
        format!(
            "e#{}#{}#{}#",
            scope.as_str(),
            segment_hash(user_id),
            segment_hash(namespace)
        )
    }
}

/// A stored entry as read back from a backend.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheEntry {
    pub key: String,
    pub value: Value,
    /// Unix timestamp in milliseconds; `None` means the entry never expires.
    pub expires_at: Option<i64>,
    /// Unix timestamp in milliseconds.
    pub updated_at: i64,
}

impl CacheEntry {
    pub fn is_expired_at(&self, now_ms: i64) -> bool {
        self.expires_at.is_some_and(|expires| expires <= now_ms)
    }
}

/// Input for writing an entry.
#[derive(Clone, Debug)]
pub struct SetCacheEntry {
    pub key: CacheKey,
    pub value: Value,
    /// Unix timestamp in milliseconds; `None` persists until explicitly deleted.
    pub expires_at: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum CacheStoreError {
    #[error("Database error: {0}")]
    Database(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Connection error: {0}")]
    Connection(String),
    #[error("Configuration error: {0}")]
    Configuration(String),
    #[error("{0}")]
    InvalidInput(String),
    #[error("Cache key changed too rapidly to resolve: {0}")]
    Contention(String),
}

/// How many read/insert rounds `get_or_set` will make before giving up.
///
/// Each extra round is caused by another writer winning the insert and then the entry
/// vanishing again before we could read it. Two consecutive occurrences already imply a
/// pathological write/delete loop on one key.
const GET_OR_SET_ATTEMPTS: usize = 3;

/// Storage backend for the flow key/value cache.
///
/// Implementations must treat a lapsed `expires_at` as a miss even when the entry is
/// still physically present, because only Redis and DynamoDB evict on their own.
#[async_trait]
pub trait CacheStore: Send + Sync + Debug {
    /// Backend name for logging.
    fn backend_name(&self) -> &'static str;

    /// Read an entry. Returns `Ok(None)` for both "absent" and "expired".
    async fn get(&self, key: &CacheKey) -> Result<Option<CacheEntry>, CacheStoreError>;

    /// Whether a live entry exists, without transferring its value.
    ///
    /// Kept separate from `get` so an existence check on a large value costs one
    /// round trip and no payload.
    async fn exists(&self, key: &CacheKey) -> Result<bool, CacheStoreError>;

    /// Write an entry, replacing any existing value under the same key.
    async fn set(&self, entry: SetCacheEntry) -> Result<CacheEntry, CacheStoreError>;

    /// Atomically write the entry **only if** no live entry exists for its key.
    ///
    /// Returns the entry as stored when this call created it, or `None` when a live
    /// entry was already present. An entry whose TTL has lapsed but which has not been
    /// physically reclaimed yet does not count as present — otherwise a key with an
    /// expired value could never be refilled on the backends that sweep lazily.
    ///
    /// This is the primitive that makes [`CacheStore::get_or_set`] race-free; a caller
    /// doing `get` then `set` by hand would let two runs both decide to compute.
    async fn try_insert(&self, entry: SetCacheEntry)
    -> Result<Option<CacheEntry>, CacheStoreError>;

    /// Read the entry, or store `entry` and return it if there is nothing live.
    ///
    /// The returned flag is `true` only when this call was the one that wrote — which is
    /// what makes this usable as a "compute once" guard, not merely a convenience.
    ///
    /// Reads first because a cache is read-mostly: on a hit this costs a single round
    /// trip, and the insert is only attempted when the key is genuinely empty.
    async fn get_or_set(
        &self,
        entry: SetCacheEntry,
    ) -> Result<(CacheEntry, bool), CacheStoreError> {
        for _ in 0..GET_OR_SET_ATTEMPTS {
            if let Some(existing) = self.get(&entry.key).await? {
                return Ok((existing, false));
            }

            if let Some(created) = self.try_insert(entry.clone()).await? {
                return Ok((created, true));
            }

            // Another writer inserted between our read and our insert. Loop round to
            // read their value rather than reporting a miss.
        }

        Err(CacheStoreError::Contention(format!(
            "key '{}' was written and removed repeatedly during get-or-set",
            entry.key.key
        )))
    }

    /// Delete an entry. Returns whether something was actually removed.
    async fn delete(&self, key: &CacheKey) -> Result<bool, CacheStoreError>;

    /// Delete every entry in one `(app, scope, user, namespace)` partition slice,
    /// regardless of TTL — including entries that never expire. Returns how many
    /// entries were removed.
    ///
    /// `namespace` must not be empty: unnamespaced entries are deliberately not bulk
    /// deletable, and wiping a whole scope is `delete_app`'s job.
    async fn delete_namespace(
        &self,
        app_id: &str,
        scope: CacheScope,
        user_id: &str,
        namespace: &str,
    ) -> Result<i64, CacheStoreError>;

    /// Delete every entry belonging to an app (used when an app is torn down).
    async fn delete_app(&self, app_id: &str) -> Result<i64, CacheStoreError>;

    /// Drop entries whose TTL has elapsed.
    ///
    /// Backends with native TTL may return `Ok(0)` — the sweeper treats this as "nothing
    /// to do", not as a failure.
    async fn delete_expired(&self) -> Result<i64, CacheStoreError>;
}

/// Ceilings applied before an entry reaches a backend.
///
/// These are enforced in the API layer rather than per backend so that a flow behaves
/// identically no matter which `CACHE_BACKEND` a deployment runs — a value that fits in
/// Postgres but not in a DynamoDB item would otherwise fail only in production.
#[derive(Clone, Copy, Debug)]
pub struct CacheLimits {
    pub max_key_bytes: usize,
    pub max_value_bytes: usize,
    pub max_ttl_seconds: u64,
    /// Applied when a write specifies no TTL. `0` means "never expires".
    pub default_ttl_seconds: u64,
}

impl Default for CacheLimits {
    fn default() -> Self {
        Self {
            max_key_bytes: 512,
            // Deliberately small: the cache is for hot metadata and memoized lookups,
            // not payload storage. Every backend can hold a value this size — Postgres
            // and Redis natively, DynamoDB by splitting it across chunk items (its item
            // ceiling is 400 KB). Larger data belongs in the app's object storage.
            max_value_bytes: 1024 * 1024,
            max_ttl_seconds: 30 * 24 * 60 * 60,
            default_ttl_seconds: 0,
        }
    }
}

impl CacheLimits {
    pub fn from_env() -> Self {
        let defaults = Self::default();
        Self {
            max_key_bytes: env_parsed("CACHE_MAX_KEY_BYTES", defaults.max_key_bytes),
            max_value_bytes: env_parsed("CACHE_MAX_VALUE_BYTES", defaults.max_value_bytes),
            max_ttl_seconds: env_parsed("CACHE_MAX_TTL_SECONDS", defaults.max_ttl_seconds),
            default_ttl_seconds: env_parsed(
                "CACHE_DEFAULT_TTL_SECONDS",
                defaults.default_ttl_seconds,
            ),
        }
    }

    /// Validate a key and return the trimmed form to store.
    pub fn validate_key(&self, key: &str) -> Result<String, CacheStoreError> {
        let key = key.trim();
        if key.is_empty() {
            return Err(CacheStoreError::InvalidInput(
                "Cache key must not be empty".to_string(),
            ));
        }
        if key.len() > self.max_key_bytes {
            return Err(CacheStoreError::InvalidInput(format!(
                "Cache key is {} bytes, exceeding the {} byte limit",
                key.len(),
                self.max_key_bytes
            )));
        }
        Ok(key.to_string())
    }

    pub fn validate_value(&self, value: &Value) -> Result<(), CacheStoreError> {
        let encoded =
            serde_json::to_vec(value).map_err(|e| CacheStoreError::Serialization(e.to_string()))?;
        if encoded.len() > self.max_value_bytes {
            return Err(CacheStoreError::InvalidInput(format!(
                "Cache value is {} bytes, exceeding the {} byte cache limit. The cache is \
                 for small, hot values — write large payloads to the app's storage \
                 (S3-backed) instead and cache the path or a summary.",
                encoded.len(),
                self.max_value_bytes
            )));
        }
        Ok(())
    }

    /// Resolve a requested TTL into an absolute expiry in epoch milliseconds.
    ///
    /// An explicit `0` means "no expiry" and overrides `default_ttl_seconds`, so a flow
    /// can always opt out of a deployment-wide default.
    pub fn resolve_expiry(
        &self,
        requested_ttl_seconds: Option<u64>,
        now_ms: i64,
    ) -> Result<Option<i64>, CacheStoreError> {
        let ttl = match requested_ttl_seconds {
            Some(ttl) => ttl,
            None => self.default_ttl_seconds,
        };

        if ttl == 0 {
            return Ok(None);
        }
        if ttl > self.max_ttl_seconds {
            return Err(CacheStoreError::InvalidInput(format!(
                "TTL of {}s exceeds the maximum of {}s",
                ttl, self.max_ttl_seconds
            )));
        }

        Ok(Some(now_ms + (ttl as i64) * 1_000))
    }
}

fn env_parsed<T: std::str::FromStr>(name: &str, fallback: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<T>().ok())
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_keys_cannot_be_forged_across_segments() {
        // Hashing each segment means a key containing the delimiter — or text that
        // mimics another entry's whole layout — still lands in its own slot.
        assert_ne!(
            CacheKey::app("a", "ns", "b#c").sort_key(),
            CacheKey::app("a", "ns#b", "c").sort_key()
        );
        let mimic = CacheKey::app("a", "ns", "e#app#deadbeef#deadbeef#deadbeef").sort_key();
        assert_ne!(mimic, CacheKey::app("a", "ns", "x").sort_key());
    }

    #[test]
    fn scope_user_and_namespace_participate_in_identity() {
        assert_ne!(
            CacheKey::app("app", "", "k").sort_key(),
            CacheKey::user("app", "someone", "", "k").sort_key()
        );
        assert_ne!(
            CacheKey::user("app", "alice", "", "k").sort_key(),
            CacheKey::user("app", "bob", "", "k").sort_key()
        );
        assert_ne!(
            CacheKey::app("app", "ns-a", "k").sort_key(),
            CacheKey::app("app", "ns-b", "k").sort_key()
        );
    }

    #[test]
    fn namespace_prefix_ranges_exactly_one_namespace() {
        use flow_like_types::cache::CacheScope;

        let prefix = CacheKey::namespace_sort_prefix(CacheScope::App, "", "reports");

        let inside = CacheKey::app("app", "reports", "daily").sort_key();
        assert!(inside.starts_with(&prefix));

        // A different namespace, the same key text, or a namespace that is a textual
        // prefix of ours must all fall outside the range.
        assert!(
            !CacheKey::app("app", "billing", "daily")
                .sort_key()
                .starts_with(&prefix)
        );
        assert!(
            !CacheKey::app("app", "report", "daily")
                .sort_key()
                .starts_with(&prefix)
        );
        assert!(
            !CacheKey::app("app", "", "reports/daily")
                .sort_key()
                .starts_with(&prefix)
        );
        assert!(
            !CacheKey::user("app", "alice", "reports", "daily")
                .sort_key()
                .starts_with(&prefix),
            "user-scoped entries must not fall into the app-scope range"
        );
    }

    #[test]
    fn expiry_is_inclusive_of_the_boundary() {
        let entry = CacheEntry {
            key: "k".to_string(),
            value: Value::Null,
            expires_at: Some(1_000),
            updated_at: 0,
        };
        assert!(!entry.is_expired_at(999));
        assert!(entry.is_expired_at(1_000));

        let eternal = CacheEntry {
            expires_at: None,
            ..entry
        };
        assert!(!eternal.is_expired_at(i64::MAX));
    }

    #[test]
    fn explicit_zero_ttl_beats_a_deployment_default() {
        let limits = CacheLimits {
            default_ttl_seconds: 3_600,
            ..CacheLimits::default()
        };

        assert_eq!(limits.resolve_expiry(Some(0), 0).unwrap(), None);
        assert_eq!(
            limits.resolve_expiry(None, 0).unwrap(),
            Some(3_600 * 1_000),
            "omitting a TTL should adopt the deployment default"
        );
        assert_eq!(
            limits.resolve_expiry(Some(60), 1_000).unwrap(),
            Some(61_000)
        );
    }

    #[test]
    fn oversized_ttl_and_key_are_rejected() {
        let limits = CacheLimits {
            max_ttl_seconds: 100,
            max_key_bytes: 4,
            ..CacheLimits::default()
        };

        assert!(limits.resolve_expiry(Some(101), 0).is_err());
        assert!(limits.validate_key("abcde").is_err());
        assert_eq!(limits.validate_key("  ab  ").unwrap(), "ab");
        assert!(limits.validate_key("   ").is_err());
    }
}
