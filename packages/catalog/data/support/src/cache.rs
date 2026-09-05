//! Key/value cache for flows.
//!
//! Cloud runs talk to `/apps/{app_id}/cache`, where the deployment's `CACHE_BACKEND`
//! decides whether entries land in Postgres, Redis or DynamoDB. Offline apps have no
//! backend to talk to, so their entries are stored as small JSON files in the app's
//! durable local storage instead. Both paths honour the same TTL semantics, so a flow
//! behaves the same either way.
//!
//! This is deliberately separate from `ExecutionContext::{get,set}_cache`, which is an
//! in-memory, per-run map of arbitrary Rust objects that is cleared when a run forks.

use flow_like::flow::execution::context::ExecutionContext;
use flow_like_storage::{Path, files::store::FlowLikeStore, object_store::ObjectStore};
use flow_like_types::{
    JsonSchema, Value,
    json::{Deserialize, Serialize},
};

use crate::remote_util::{api_base_url, control_plane_http_client};

/// Entries live under this prefix inside the app's storage, alongside `storage/`.
const LOCAL_CACHE_DIR: &str = "cache";
const LOCAL_APP_SCOPE_DIR: &str = "global";
const LOCAL_USER_SCOPE_DIR: &str = "user";

/// Offline entries are capped like the server-side cache (`CacheLimits` defaults), so a
/// flow built against local storage does not silently depend on values a cloud
/// deployment will reject.
const LOCAL_MAX_VALUE_BYTES: usize = 1024 * 1024;
const LOCAL_MAX_KEY_BYTES: usize = 512;

/// Who a cache entry belongs to. Mirrors `flow_like_types::cache::CacheScope` on the
/// wire; kept as its own type so the pin dropdown can carry friendly labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CacheScope {
    /// Shared by everyone who can execute in the app.
    #[default]
    App,
    /// Private to the user who triggered the run.
    User,
}

impl CacheScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::User => "user",
        }
    }

    pub fn from_label(label: &str) -> Self {
        match label.trim().to_ascii_lowercase().as_str() {
            "user" => Self::User,
            _ => Self::App,
        }
    }
}

/// Handle produced by the Open Cache node and consumed by the read/write nodes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct FlowCache {
    pub scope: CacheScope,
    /// Optional grouping travelling alongside every key. Entries sharing a namespace
    /// can be invalidated together; unrelated flows also use it to keep short keys
    /// from colliding.
    #[serde(default)]
    pub namespace: String,
}

impl FlowCache {
    /// The namespace sent to the backend: trimmed, empty meaning unnamespaced. Bounded
    /// like keys so offline flows match the server-side limits.
    pub fn validated_namespace(&self) -> flow_like_types::Result<&str> {
        let namespace = self.namespace.trim();
        if namespace.len() > LOCAL_MAX_KEY_BYTES {
            return Err(flow_like_types::anyhow!(
                "Cache namespace is {} bytes, exceeding the {} byte limit",
                namespace.len(),
                LOCAL_MAX_KEY_BYTES
            ));
        }
        Ok(namespace)
    }

    /// The key sent to the backend. Namespace and key travel as separate fields — the
    /// backends key their storage on both — so neither can impersonate the other.
    pub fn validated_key(&self, key: &str) -> flow_like_types::Result<String> {
        let key = key.trim();
        if key.is_empty() {
            return Err(flow_like_types::anyhow!("Cache key must not be empty"));
        }
        if key.len() > LOCAL_MAX_KEY_BYTES {
            return Err(flow_like_types::anyhow!(
                "Cache key is {} bytes, exceeding the {} byte limit",
                key.len(),
                LOCAL_MAX_KEY_BYTES
            ));
        }
        Ok(key.to_string())
    }
}

/// A stored entry, as returned by either transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheHit {
    pub value: Value,
    /// Unix timestamp in milliseconds; `None` when the entry never expires.
    pub expires_at: Option<i64>,
}

/// Where a run's cache operations are served from.
enum CacheTransport {
    Remote {
        base_url: String,
        app_id: String,
        token: String,
    },
    Local {
        store: FlowLikeStore,
        root: Path,
    },
}

/// On-disk shape for offline entries. Namespace and key are stored alongside the value
/// because the filename is a hash — this makes an unexpected collision detectable
/// rather than silent, and lets namespace invalidation identify entries by reading
/// them.
#[derive(Debug, Serialize, Deserialize)]
struct LocalCacheRecord {
    #[serde(default)]
    namespace: String,
    key: String,
    value: Value,
    expires_at: Option<i64>,
    updated_at: i64,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn resolve_transport(context: &ExecutionContext) -> flow_like_types::Result<CacheTransport> {
    let execution_cache = context
        .execution_cache
        .as_ref()
        .ok_or_else(|| flow_like_types::anyhow!("Cache nodes require an execution context"))?;

    let token = context
        .token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty());
    let base_url = api_base_url(&context.profile.hub, context.profile.secure);

    // `model_usage_app_id` is cleared for offline apps, whose ids exist only on this
    // machine and would be rejected by the API. Missing credentials mean the same thing
    // in practice: there is no backend to reach.
    let is_offline = execution_cache.model_usage_app_id.is_none();

    if let (false, Some(token), Some(base_url)) = (is_offline, token, base_url) {
        return Ok(CacheTransport::Remote {
            base_url,
            app_id: execution_cache.app_id.clone(),
            token: token.to_string(),
        });
    }

    let store = execution_cache
        .stores
        .app_storage_store
        .clone()
        .ok_or_else(|| {
            flow_like_types::anyhow!(
                "Offline cache requires app storage, but no app storage store is configured"
            )
        })?;

    Ok(CacheTransport::Local {
        store,
        root: Path::from("apps")
            .child(execution_cache.app_id.clone())
            .child(LOCAL_CACHE_DIR),
    })
}

fn local_scope_dir(root: &Path, scope: CacheScope, sub: &str) -> flow_like_types::Result<Path> {
    match scope {
        CacheScope::App => Ok(root.child(LOCAL_APP_SCOPE_DIR)),
        CacheScope::User => {
            let sub = sub.trim();
            if sub.is_empty() {
                return Err(flow_like_types::anyhow!(
                    "User-scoped cache requires an identifiable user"
                ));
            }
            Ok(root.child(LOCAL_USER_SCOPE_DIR).child(sub.to_string()))
        }
    }
}

fn local_entry_path(
    root: &Path,
    scope: CacheScope,
    sub: &str,
    namespace: &str,
    key: &str,
) -> flow_like_types::Result<Path> {
    let scoped = local_scope_dir(root, scope, sub)?;

    // Namespace and key are arbitrary user input; hashing keeps the filename safe and
    // bounded, and the length prefix keeps ("ab", "c") distinct from ("a", "bc").
    let mut hasher = blake3::Hasher::new();
    hasher.update(&(namespace.len() as u64).to_le_bytes());
    hasher.update(namespace.as_bytes());
    hasher.update(key.as_bytes());
    let file = hasher.finalize().to_hex().to_string();
    Ok(scoped.child(format!("{file}.json")))
}

/// Read an entry. Returns `None` for both "absent" and "expired".
pub async fn cache_get(
    context: &ExecutionContext,
    cache: &FlowCache,
    key: &str,
) -> flow_like_types::Result<Option<CacheHit>> {
    let key = cache.validated_key(key)?;
    let namespace = cache.validated_namespace()?;

    match resolve_transport(context)? {
        CacheTransport::Remote {
            base_url,
            app_id,
            token,
        } => {
            let response = control_plane_http_client()
                .get(format!("{base_url}/apps/{app_id}/cache"))
                .query(&[
                    ("key", key.as_str()),
                    ("scope", cache.scope.as_str()),
                    ("namespace", namespace),
                ])
                .bearer_auth(&token)
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "Cache read failed with status {status}: {body}"
                ));
            }

            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct ReadResponse {
                found: bool,
                value: Option<Value>,
                expires_at: Option<i64>,
            }

            let parsed: ReadResponse = response.json().await?;
            if !parsed.found {
                return Ok(None);
            }

            Ok(Some(CacheHit {
                value: parsed.value.unwrap_or(Value::Null),
                expires_at: parsed.expires_at,
            }))
        }

        CacheTransport::Local { store, root } => {
            let path = local_entry_path(&root, cache.scope, local_sub(context), namespace, &key)?;

            Ok(read_local_record(&store, &path, namespace, &key)
                .await?
                .map(|record| CacheHit {
                    value: record.value,
                    expires_at: record.expires_at,
                }))
        }
    }
}

/// Load a local entry, treating a missing, unreadable, mismatched or expired file as a
/// miss. Expired and corrupt files are removed so they cannot accumulate.
async fn read_local_record(
    store: &FlowLikeStore,
    path: &Path,
    namespace: &str,
    key: &str,
) -> flow_like_types::Result<Option<LocalCacheRecord>> {
    let generic = store.as_generic();

    let bytes = match generic.get(path).await {
        Ok(result) => result.bytes().await?,
        // Any read failure here — missing file, missing directory — is a miss.
        Err(_) => return Ok(None),
    };

    let record: LocalCacheRecord = match flow_like_types::json::from_slice(&bytes) {
        Ok(record) => record,
        Err(_) => {
            // A truncated or hand-edited file must not poison the flow.
            let _ = generic.delete(path).await;
            return Ok(None);
        }
    };

    // The filename is a hash of namespace and key, so a mismatch means a hash
    // collision rather than the entry we asked for.
    if record.namespace != namespace || record.key != key {
        return Ok(None);
    }

    if record.expires_at.is_some_and(|expires| expires <= now_ms()) {
        let _ = generic.delete(path).await;
        return Ok(None);
    }

    Ok(Some(record))
}

/// Persist a local entry. Returns the resolved expiry, if any.
async fn write_local_record(
    store: &FlowLikeStore,
    path: &Path,
    namespace: &str,
    key: &str,
    value: Value,
    ttl_seconds: Option<u64>,
) -> flow_like_types::Result<Option<i64>> {
    let encoded_value = flow_like_types::json::to_vec(&value)?;
    if encoded_value.len() > LOCAL_MAX_VALUE_BYTES {
        return Err(flow_like_types::anyhow!(
            "Cache value is {} bytes, exceeding the {} byte cache limit. The cache is \
             for small, hot values — write large payloads to the app's storage instead \
             and cache the path or a summary.",
            encoded_value.len(),
            LOCAL_MAX_VALUE_BYTES
        ));
    }

    let updated_at = now_ms();
    let expires_at = ttl_seconds
        .filter(|ttl| *ttl > 0)
        .map(|ttl| updated_at + (ttl as i64) * 1_000);

    let record = LocalCacheRecord {
        namespace: namespace.to_string(),
        key: key.to_string(),
        value,
        expires_at,
        updated_at,
    };

    let bytes = flow_like_types::json::to_vec(&record)?;
    store.as_generic().put(path, bytes.into()).await?;

    Ok(expires_at)
}

/// Whether a live entry exists, without downloading its value.
pub async fn cache_has(
    context: &ExecutionContext,
    cache: &FlowCache,
    key: &str,
) -> flow_like_types::Result<bool> {
    let key = cache.validated_key(key)?;
    let namespace = cache.validated_namespace()?;

    match resolve_transport(context)? {
        CacheTransport::Remote {
            base_url,
            app_id,
            token,
        } => {
            let response = control_plane_http_client()
                .get(format!("{base_url}/apps/{app_id}/cache/exists"))
                .query(&[
                    ("key", key.as_str()),
                    ("scope", cache.scope.as_str()),
                    ("namespace", namespace),
                ])
                .bearer_auth(&token)
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "Cache existence check failed with status {status}: {body}"
                ));
            }

            #[derive(Deserialize)]
            struct ExistsResponse {
                found: bool,
            }

            let parsed: ExistsResponse = response.json().await?;
            Ok(parsed.found)
        }

        CacheTransport::Local { store, root } => {
            let path = local_entry_path(&root, cache.scope, local_sub(context), namespace, &key)?;
            // A HEAD tells us the file is there but not whether its lifetime has
            // elapsed, and the expiry lives inside the file — so this reads it.
            Ok(read_local_record(&store, &path, namespace, &key)
                .await?
                .is_some())
        }
    }
}

/// Read the entry, or store `value` when nothing live is there.
///
/// Returns the value now held under the key and whether this call is the one that wrote
/// it. Use this instead of a Has followed by a Write when only one caller should do the
/// expensive work — those two calls have a gap between them, this does not.
pub async fn cache_get_or_set(
    context: &ExecutionContext,
    cache: &FlowCache,
    key: &str,
    value: Value,
    ttl_seconds: Option<u64>,
) -> flow_like_types::Result<(Value, bool)> {
    let key = cache.validated_key(key)?;
    let namespace = cache.validated_namespace()?;

    match resolve_transport(context)? {
        CacheTransport::Remote {
            base_url,
            app_id,
            token,
        } => {
            let response = control_plane_http_client()
                .put(format!("{base_url}/apps/{app_id}/cache"))
                .bearer_auth(&token)
                .json(&flow_like_types::json::json!({
                    "key": key,
                    "namespace": namespace,
                    "value": value,
                    "scope": cache.scope.as_str(),
                    "ttlSeconds": ttl_seconds,
                    "ifAbsent": true,
                }))
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "Cache get-or-write failed with status {status}: {body}"
                ));
            }

            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct WriteResponse {
                stored: bool,
                value: Value,
            }

            let parsed: WriteResponse = response.json().await?;
            Ok((parsed.value, parsed.stored))
        }

        CacheTransport::Local { store, root } => {
            let path = local_entry_path(&root, cache.scope, local_sub(context), namespace, &key)?;

            if let Some(record) = read_local_record(&store, &path, namespace, &key).await? {
                return Ok((record.value, false));
            }

            // Object stores offer no compare-and-set here, so this is a read followed by
            // a write. Offline apps run in a single local runtime, where the only racers
            // are two flows in the same process hitting the same key in the same
            // instant; the loser's value simply wins. Cloud runs go through the atomic
            // backend path above.
            write_local_record(&store, &path, namespace, &key, value.clone(), ttl_seconds).await?;
            Ok((value, true))
        }
    }
}

/// Write an entry. `ttl_seconds` of `0` (or `None`) keeps it until it is deleted.
pub async fn cache_set(
    context: &ExecutionContext,
    cache: &FlowCache,
    key: &str,
    value: Value,
    ttl_seconds: Option<u64>,
) -> flow_like_types::Result<Option<i64>> {
    let key = cache.validated_key(key)?;
    let namespace = cache.validated_namespace()?;

    match resolve_transport(context)? {
        CacheTransport::Remote {
            base_url,
            app_id,
            token,
        } => {
            let response = control_plane_http_client()
                .put(format!("{base_url}/apps/{app_id}/cache"))
                .bearer_auth(&token)
                .json(&flow_like_types::json::json!({
                    "key": key,
                    "namespace": namespace,
                    "value": value,
                    "scope": cache.scope.as_str(),
                    "ttlSeconds": ttl_seconds,
                }))
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "Cache write failed with status {status}: {body}"
                ));
            }

            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct WriteResponse {
                expires_at: Option<i64>,
            }

            let parsed: WriteResponse = response.json().await?;
            Ok(parsed.expires_at)
        }

        CacheTransport::Local { store, root } => {
            let path = local_entry_path(&root, cache.scope, local_sub(context), namespace, &key)?;
            write_local_record(&store, &path, namespace, &key, value, ttl_seconds).await
        }
    }
}

/// Delete an entry. Returns whether something was removed.
pub async fn cache_delete(
    context: &ExecutionContext,
    cache: &FlowCache,
    key: &str,
) -> flow_like_types::Result<bool> {
    let key = cache.validated_key(key)?;
    let namespace = cache.validated_namespace()?;

    match resolve_transport(context)? {
        CacheTransport::Remote {
            base_url,
            app_id,
            token,
        } => {
            let response = control_plane_http_client()
                .delete(format!("{base_url}/apps/{app_id}/cache"))
                .query(&[
                    ("key", key.as_str()),
                    ("scope", cache.scope.as_str()),
                    ("namespace", namespace),
                ])
                .bearer_auth(&token)
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "Cache delete failed with status {status}: {body}"
                ));
            }

            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct DeleteResponse {
                deleted: bool,
            }

            let parsed: DeleteResponse = response.json().await?;
            Ok(parsed.deleted)
        }

        CacheTransport::Local { store, root } => {
            let path = local_entry_path(&root, cache.scope, local_sub(context), namespace, &key)?;
            let generic = store.as_generic();
            let existed = generic.head(&path).await.is_ok();
            if existed {
                generic.delete(&path).await?;
            }
            Ok(existed)
        }
    }
}

/// Remove every entry in the cache handle's namespace, regardless of lifetime.
/// Returns how many entries were removed.
pub async fn cache_invalidate_namespace(
    context: &ExecutionContext,
    cache: &FlowCache,
) -> flow_like_types::Result<i64> {
    let namespace = cache.validated_namespace()?;
    if namespace.is_empty() {
        return Err(flow_like_types::anyhow!(
            "Invalidation requires a cache handle with a namespace; refusing to delete every entry in the scope"
        ));
    }

    match resolve_transport(context)? {
        CacheTransport::Remote {
            base_url,
            app_id,
            token,
        } => {
            let response = control_plane_http_client()
                .delete(format!("{base_url}/apps/{app_id}/cache/namespace"))
                .query(&[("namespace", namespace), ("scope", cache.scope.as_str())])
                .bearer_auth(&token)
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "Cache namespace invalidation failed with status {status}: {body}"
                ));
            }

            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct InvalidateResponse {
                deleted: i64,
            }

            let parsed: InvalidateResponse = response.json().await?;
            Ok(parsed.deleted)
        }

        CacheTransport::Local { store, root } => {
            // Filenames are hashes, so each record is read to learn which namespace it
            // belongs to — the record stores it for exactly this purpose. Unreadable
            // files are skipped rather than deleted; the read path already quarantines
            // corrupt entries when they are actually accessed.
            let scoped = local_scope_dir(&root, cache.scope, local_sub(context))?;
            let generic = store.as_generic();

            use futures::StreamExt;
            let entries: Vec<_> = generic
                .list(Some(&scoped))
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?;

            let mut deleted = 0i64;
            for meta in entries {
                let Ok(result) = generic.get(&meta.location).await else {
                    continue;
                };
                let Ok(bytes) = result.bytes().await else {
                    continue;
                };
                let Ok(record) = flow_like_types::json::from_slice::<LocalCacheRecord>(&bytes)
                else {
                    continue;
                };

                if record.namespace == namespace {
                    // Files legitimately vanish between list and delete: reads reclaim
                    // expired entries, and two invalidations can run concurrently. One
                    // vanished file must not abort the rest of the sweep.
                    if generic.delete(&meta.location).await.is_ok() {
                        deleted += 1;
                    }
                }
            }

            Ok(deleted)
        }
    }
}

fn local_sub(context: &ExecutionContext) -> &str {
    context
        .execution_cache
        .as_ref()
        .map(|cache| cache.sub.as_str())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_trimmed_and_empty_keys_are_rejected() {
        let plain = FlowCache {
            scope: CacheScope::App,
            namespace: String::new(),
        };
        assert_eq!(plain.validated_key(" token ").unwrap(), "token");
        assert!(plain.validated_key("   ").is_err());

        let scoped = FlowCache {
            scope: CacheScope::User,
            namespace: " billing ".to_string(),
        };
        assert_eq!(scoped.validated_namespace().unwrap(), "billing");

        let oversized = FlowCache {
            scope: CacheScope::App,
            namespace: "n".repeat(LOCAL_MAX_KEY_BYTES + 1),
        };
        assert!(oversized.validated_namespace().is_err());
        assert!(
            plain
                .validated_key(&"k".repeat(LOCAL_MAX_KEY_BYTES + 1))
                .is_err()
        );
    }

    #[test]
    fn local_paths_separate_scopes_users_and_namespaces() {
        let root = Path::from("apps").child("app-1").child(LOCAL_CACHE_DIR);

        let app = local_entry_path(&root, CacheScope::App, "", "", "k").unwrap();
        let alice = local_entry_path(&root, CacheScope::User, "alice", "", "k").unwrap();
        let bob = local_entry_path(&root, CacheScope::User, "bob", "", "k").unwrap();

        assert_ne!(app.as_ref(), alice.as_ref());
        assert_ne!(alice.as_ref(), bob.as_ref());
        assert!(app.as_ref().contains("/global/"));
        assert!(alice.as_ref().contains("/user/alice/"));

        // Namespace participates in the filename, and the length prefix keeps
        // ("ab", "c") distinct from ("a", "bc").
        let ns = local_entry_path(&root, CacheScope::App, "", "reports", "k").unwrap();
        assert_ne!(app.as_ref(), ns.as_ref());
        let ab_c = local_entry_path(&root, CacheScope::App, "", "ab", "c").unwrap();
        let a_bc = local_entry_path(&root, CacheScope::App, "", "a", "bc").unwrap();
        assert_ne!(ab_c.as_ref(), a_bc.as_ref());

        // A user-scoped entry with no identity must fail rather than silently share the
        // app bucket.
        assert!(local_entry_path(&root, CacheScope::User, "  ", "", "k").is_err());
    }

    #[test]
    fn keys_with_path_separators_stay_inside_the_scope_directory() {
        let root = Path::from("apps").child("app-1").child(LOCAL_CACHE_DIR);
        let traversal =
            local_entry_path(&root, CacheScope::App, "", "../ns", "../../escape").unwrap();
        assert!(traversal.as_ref().starts_with("apps/app-1/cache/global/"));
        assert!(!traversal.as_ref().contains(".."));
    }
}
