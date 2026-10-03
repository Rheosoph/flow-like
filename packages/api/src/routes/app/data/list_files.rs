use crate::{
    cache::{CacheBackendHandle, best_effort},
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::credentials::SharedCredentials;
use flow_like_storage::{Path as ObjectPath, files::store::StorageItem};
use flow_like_types::anyhow;
use std::{future::Future, time::Duration};
use utoipa::{IntoParams, ToSchema};

const CACHE_NAMESPACE: &str = "file-listings-v1";
const CACHE_TTL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct ListFilesPayload {
    pub prefix: String,
}

#[derive(Debug, Clone, serde::Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListFilesQuery {
    /// Read storage directly after a mutation or an explicit refresh.
    #[serde(default)]
    pub refresh: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CachedListing {
    observed_at_ms: i64,
    items: Vec<StorageItem>,
}

impl CachedListing {
    fn is_fresh(&self, now_ms: i64) -> bool {
        now_ms
            .checked_sub(self.observed_at_ms)
            .is_some_and(|age| (0..CACHE_TTL.as_millis() as i64).contains(&age))
    }
}

/// Use the concrete content configuration that builds the scoped store. Never
/// serialize a credential or cache a token, SAS, signed URL or access key.
fn content_identity(credentials: &SharedCredentials) -> Option<serde_json::Value> {
    Some(match credentials {
        SharedCredentials::Aws(credentials) => serde_json::json!([
            "s3",
            credentials.content_bucket,
            credentials.region,
            credentials
                .content_config
                .as_ref()
                .map(|config| &config.endpoint),
            credentials
                .content_config
                .as_ref()
                .map(|config| config.express),
            credentials.content_path_prefix,
            credentials.user_content_path_prefix,
        ]),
        SharedCredentials::Azure(credentials) => serde_json::json!([
            "azure",
            credentials.account_name,
            credentials.content_container,
            credentials.content_path_prefix,
            credentials.user_content_path_prefix,
        ]),
        SharedCredentials::Gcp(credentials) => serde_json::json!([
            "gcp",
            credentials.content_bucket,
            credentials.allowed_prefixes,
            credentials.content_path_prefix,
            credentials.user_content_path_prefix,
        ]),
        SharedCredentials::Mixed(credentials) => return content_identity(&credentials.content),
        SharedCredentials::Renewable(_) => return None,
    })
}

fn listing_key(
    storage: &serde_json::Value,
    app: &str,
    subject: &str,
    scope: &str,
    path: &ObjectPath,
) -> String {
    blake3::hash(
        serde_json::json!([storage, app, subject, scope, path.as_ref()])
            .to_string()
            .as_bytes(),
    )
    .to_hex()
    .to_string()
}

async fn cached_listing(
    cache: &CacheBackendHandle,
    key: Option<&str>,
    refresh: bool,
    read: impl Future<Output = flow_like_types::Result<Vec<StorageItem>>>,
) -> flow_like_types::Result<Vec<StorageItem>> {
    if !refresh
        && let Some(key) = key
        && let Some(listing) = best_effort::get::<CachedListing>(cache, CACHE_NAMESPACE, key).await
        && listing.is_fresh(chrono::Utc::now().timestamp_millis())
    {
        return Ok(listing.items);
    }

    // A listing that started before an upload may finish after its refresh.
    // Age from the start of the storage read, so that late writer cannot extend
    // stale data's lifetime. Direct and workflow writes also expire within 10s.
    let observed_at_ms = chrono::Utc::now().timestamp_millis();
    let listing = CachedListing {
        observed_at_ms,
        items: read.await?,
    };
    if let Some(key) = key
        && listing.is_fresh(chrono::Utc::now().timestamp_millis())
    {
        best_effort::set(cache, CACHE_NAMESPACE, key, &listing, CACHE_TTL).await;
    }
    Ok(listing.items)
}

async fn read_listing(
    store: std::sync::Arc<dyn flow_like_storage::object_store::ObjectStore>,
    path: &ObjectPath,
) -> flow_like_types::Result<Vec<StorageItem>> {
    let listing = store
        .list_with_delimiter(Some(path))
        .await
        .map_err(|error| anyhow!("Failed to list items: {error}"))?;
    let mut items: Vec<StorageItem> = listing.objects.into_iter().map(StorageItem::from).collect();
    items.extend(listing.common_prefixes.into_iter().map(StorageItem::from));
    Ok(items)
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/data/list",
    tag = "data",
    description = "List files under a prefix.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ListFilesQuery
    ),
    request_body = ListFilesPayload,
    responses(
        (status = 200, description = "File list", body = String, content_type = "application/json"),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "POST /apps/{app_id}/data/list", skip(state, user, payload))]
pub async fn list_files(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<ListFilesQuery>,
    Json(payload): Json<ListFilesPayload>,
) -> Result<Json<Vec<StorageItem>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadFiles);

    let sub = user.sub()?;

    let project_dir = state
        .scoped_credentials(
            &sub,
            &app_id,
            crate::credentials::CredentialsAccess::ReadApp,
        )
        .await?;
    let credentials = project_dir.into_shared_credentials();
    let project_dir = credentials.to_store(false).await?;
    let path = project_dir
        .construct_upload(&app_id, &payload.prefix)
        .await?;

    let key = content_identity(&credentials)
        .map(|storage| listing_key(&storage, &app_id, &sub, "app", &path));
    let items = cached_listing(
        &state.cache,
        key.as_deref(),
        query.refresh,
        read_listing(project_dir.as_generic(), &path),
    )
    .await?;

    Ok(Json(items))
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/data/user/list",
    tag = "data",
    description = "List your private app files under a prefix.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ListFilesQuery
    ),
    request_body = ListFilesPayload,
    responses(
        (status = 200, description = "File list", body = String, content_type = "application/json"),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/data/user/list",
    skip(state, user, payload)
)]
pub async fn list_user_files(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<ListFilesQuery>,
    Json(payload): Json<ListFilesPayload>,
) -> Result<Json<Vec<StorageItem>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadFiles);

    let sub = user.sub()?;

    let project_dir = state
        .scoped_credentials(
            &sub,
            &app_id,
            crate::credentials::CredentialsAccess::ReadUser,
        )
        .await?;
    let credentials = project_dir.into_shared_credentials();
    let project_dir = credentials.to_store(false).await?;
    let path = project_dir
        .construct_user_upload(&sub, &app_id, &payload.prefix)
        .await?;

    let key = content_identity(&credentials)
        .map(|storage| listing_key(&storage, &app_id, &sub, "user", &path));
    let items = cached_listing(
        &state.cache,
        key.as_deref(),
        query.refresh,
        read_listing(project_dir.as_generic(), &path),
    )
    .await?;

    Ok(Json(items))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::join_object_path;

    #[test]
    fn refresh_is_an_optional_query_parameter() {
        let payload: ListFilesPayload =
            serde_json::from_value(serde_json::json!({"prefix": "docs"})).unwrap();
        assert_eq!(payload.prefix, "docs");

        for route in ["/apps/app/data/list", "/apps/app/data/user/list"] {
            for (query, expected) in [
                ("", false),
                ("?refresh=false", false),
                ("?refresh=true", true),
            ] {
                let uri = format!("{route}{query}").parse().unwrap();
                let Query(query) = Query::<ListFilesQuery>::try_from_uri(&uri).unwrap();
                assert_eq!(query.refresh, expected);
            }
            let uri = format!("{route}?refresh=invalid").parse().unwrap();
            assert!(Query::<ListFilesQuery>::try_from_uri(&uri).is_err());
        }
    }

    #[test]
    fn keys_follow_normalized_storage_paths_and_every_scope() {
        let storage = serde_json::json!(["s3", "bucket", "region", "endpoint"]);
        let base = ObjectPath::from("apps/app/upload");
        let raw = join_object_path(&base, "/Q1 draft//");
        let encoded = join_object_path(&base, "Q1%20draft");
        let key = listing_key(&storage, "app", "alice", "app", &raw);
        assert_eq!(key, listing_key(&storage, "app", "alice", "app", &encoded));
        for changed in [
            listing_key(
                &serde_json::json!(["s3", "other", "region", "endpoint"]),
                "app",
                "alice",
                "app",
                &raw,
            ),
            listing_key(
                &serde_json::json!(["s3", "bucket", "region", "other"]),
                "app",
                "alice",
                "app",
                &raw,
            ),
            listing_key(&storage, "other", "alice", "app", &raw),
            listing_key(&storage, "app", "bob", "app", &raw),
            listing_key(&storage, "app", "alice", "user", &raw),
            listing_key(
                &storage,
                "app",
                "alice",
                "app",
                &join_object_path(&base, "other"),
            ),
        ] {
            assert_ne!(key, changed);
        }
        assert_ne!(
            listing_key(
                &storage,
                "app",
                "alice",
                "app",
                &join_object_path(&base, "a%2Fb")
            ),
            listing_key(
                &storage,
                "app",
                "alice",
                "app",
                &join_object_path(&base, "a/b")
            ),
        );
    }

    #[test]
    fn late_listing_writes_cannot_extend_staleness() {
        let listing = CachedListing {
            observed_at_ms: 1000,
            items: vec![],
        };
        assert!(listing.is_fresh(10_999));
        assert!(!listing.is_fresh(11_000));
        assert!(!listing.is_fresh(999));
    }

    fn items(name: &str) -> Vec<StorageItem> {
        vec![StorageItem::from(ObjectPath::from(name))]
    }

    #[flow_like_types::tokio::test]
    async fn shares_listing_and_explicit_refresh_reads_and_replaces_it() {
        let cache = CacheBackendHandle::memory_for_test();
        cached_listing(&cache, Some("folder"), false, async { Ok(items("before")) })
            .await
            .unwrap();
        let hit = cached_listing(&cache, Some("folder"), false, async {
            panic!("cache hit must not list storage")
        })
        .await
        .unwrap();
        assert_eq!(hit[0].location, "before");
        let fresh = cached_listing(&cache, Some("folder"), true, async { Ok(items("after")) })
            .await
            .unwrap();
        assert_eq!(fresh[0].location, "after");
        let hit = cached_listing(&cache, Some("folder"), false, async {
            panic!("refresh should replace cached result")
        })
        .await
        .unwrap();
        assert_eq!(hit[0].location, "after");
    }

    #[flow_like_types::tokio::test]
    async fn failures_propagate_without_becoming_empty_cached_listings() {
        let cache = CacheBackendHandle::memory_for_test();
        assert!(
            cached_listing(&cache, Some("folder"), false, async {
                Err(anyhow!("storage unavailable"))
            })
            .await
            .is_err()
        );
        assert!(
            best_effort::get::<CachedListing>(&cache, CACHE_NAMESPACE, "folder")
                .await
                .is_none()
        );
        let result = cached_listing(&cache, Some("folder"), false, async { Ok(vec![]) })
            .await
            .unwrap();
        assert!(result.is_empty());
        let hit = cached_listing(&cache, Some("folder"), false, async {
            panic!("successful empty listing should be cached")
        })
        .await
        .unwrap();
        assert!(hit.is_empty());
        assert!(
            cached_listing(&cache, Some("folder"), true, async {
                Err(anyhow!("storage unavailable"))
            })
            .await
            .is_err()
        );
    }

    #[flow_like_types::tokio::test]
    async fn rejects_expired_observation_even_when_cache_backend_retains_it() {
        let cache = CacheBackendHandle::memory_for_test();
        let old = CachedListing {
            observed_at_ms: chrono::Utc::now().timestamp_millis() - 10_001,
            items: items("old"),
        };
        best_effort::set(&cache, CACHE_NAMESPACE, "folder", &old, CACHE_TTL).await;
        let fresh = cached_listing(&cache, Some("folder"), false, async { Ok(items("fresh")) })
            .await
            .unwrap();
        assert_eq!(fresh[0].location, "fresh");
    }
}
