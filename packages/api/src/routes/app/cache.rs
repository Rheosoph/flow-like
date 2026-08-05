//! Key/value cache endpoints for running flows.
//!
//! These are runtime callbacks: the caller is normally an executor JWT minted for a run,
//! so authorization goes through [`AppUser::execution_app_permission`] rather than the
//! `ensure_permission!` macro, which rejects executor, API-key and app-connection
//! principals outright.

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_types::cache::CacheScope;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use crate::{
    cache::{CacheKey, CacheLimits, CacheStore, CacheStoreError, SetCacheEntry},
    error::ApiError,
    middleware::jwt::{AppPermissionResponse, AppUser},
    permission::role_permission::RolePermissions,
    state::AppState,
};

/// Synthetic sub used when a run has no resolvable human initiator. Partitioning a
/// user-scoped cache by it would silently merge unrelated callers into one bucket.
const ANONYMOUS_SUB: &str = "local";

#[derive(Debug, Deserialize)]
pub struct CacheEntryQuery {
    pub key: String,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReadCacheResponse {
    /// Whether a live (non-expired) entry existed.
    pub found: bool,
    pub key: String,
    /// `app` or `user`.
    #[schema(value_type = String)]
    pub scope: CacheScope,
    /// The stored value, or `null` on a miss.
    pub value: Option<serde_json::Value>,
    /// Unix timestamp in milliseconds; `null` when the entry never expires.
    pub expires_at: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExistsCacheResponse {
    /// Whether a live (non-expired) entry exists.
    pub found: bool,
    pub key: String,
    /// `app` or `user`.
    #[schema(value_type = String)]
    pub scope: CacheScope,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WriteCacheRequest {
    pub key: String,
    pub value: serde_json::Value,
    /// `app` (shared, default) or `user` (private to the caller).
    #[serde(default)]
    #[schema(value_type = String)]
    pub scope: CacheScope,
    /// Seconds until the entry expires. Omit to use the deployment default; pass `0` to
    /// keep the entry until it is explicitly deleted.
    #[serde(default)]
    pub ttl_seconds: Option<u64>,
    /// When true, keep any value that is already there and return it instead of
    /// overwriting. The check and the write happen as one atomic operation, so exactly
    /// one of several concurrent callers sees `stored: true`.
    #[serde(default)]
    pub if_absent: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WriteCacheResponse {
    pub key: String,
    /// `app` or `user`.
    #[schema(value_type = String)]
    pub scope: CacheScope,
    /// Whether this request is the one that wrote. Always true unless `ifAbsent` was set
    /// and a live entry already existed.
    pub stored: bool,
    /// The value now held under the key: the one supplied when `stored` is true, or the
    /// pre-existing one when it is false.
    pub value: serde_json::Value,
    /// Unix timestamp in milliseconds; `null` when the entry never expires.
    pub expires_at: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeleteCacheResponse {
    /// Whether an entry was actually removed.
    pub deleted: bool,
    pub key: String,
    /// `app` or `user`.
    #[schema(value_type = String)]
    pub scope: CacheScope,
}

/// Read a cached value.
#[utoipa::path(
    get,
    path = "/apps/{app_id}/cache",
    tag = "app",
    description = "Read a value the app previously stored in its cache. Returns found=false when the key is missing or its lifetime has elapsed.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("key" = String, Query, description = "Cache key to read"),
        ("scope" = Option<String>, Query, description = "'app' (shared, default) or 'user' (private to the caller)")
    ),
    responses(
        (status = 200, description = "Cache lookup result", body = ReadCacheResponse),
        (status = 400, description = "Invalid key or scope"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — execution permission required"),
        (status = 503, description = "Cache backend is not configured")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("executor_jwt" = []))
)]
#[tracing::instrument(name = "GET /apps/{app_id}/cache", skip(state, user))]
pub async fn read_cache_entry(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<CacheEntryQuery>,
) -> Result<Json<ReadCacheResponse>, ApiError> {
    let permission = authorize(&user, &app_id, &state).await?;
    let limits = CacheLimits::from_env();
    let store = cache_store(&state)?;

    let scope = parse_scope(query.scope.as_deref())?;
    let key = limits.validate_key(&query.key).map_err(to_api_error)?;
    let cache_key = build_key(&app_id, scope, &key, &permission)?;

    let entry = store.get(&cache_key).await.map_err(to_api_error)?;

    Ok(Json(match entry {
        Some(entry) => ReadCacheResponse {
            found: true,
            key,
            scope,
            value: Some(entry.value),
            expires_at: entry.expires_at,
        },
        None => ReadCacheResponse {
            found: false,
            key,
            scope,
            value: None,
            expires_at: None,
        },
    }))
}

/// Check whether a cached value exists.
#[utoipa::path(
    get,
    path = "/apps/{app_id}/cache/exists",
    tag = "app",
    description = "Check whether the app has a live value under a key, without downloading the value itself.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("key" = String, Query, description = "Cache key to check"),
        ("scope" = Option<String>, Query, description = "'app' (shared, default) or 'user' (private to the caller)")
    ),
    responses(
        (status = 200, description = "Existence check result", body = ExistsCacheResponse),
        (status = 400, description = "Invalid key or scope"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — execution permission required"),
        (status = 503, description = "Cache backend is not configured")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("executor_jwt" = []))
)]
#[tracing::instrument(name = "GET /apps/{app_id}/cache/exists", skip(state, user))]
pub async fn cache_entry_exists(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<CacheEntryQuery>,
) -> Result<Json<ExistsCacheResponse>, ApiError> {
    let permission = authorize(&user, &app_id, &state).await?;
    let limits = CacheLimits::from_env();
    let store = cache_store(&state)?;

    let scope = parse_scope(query.scope.as_deref())?;
    let key = limits.validate_key(&query.key).map_err(to_api_error)?;
    let cache_key = build_key(&app_id, scope, &key, &permission)?;

    let found = store.exists(&cache_key).await.map_err(to_api_error)?;

    Ok(Json(ExistsCacheResponse { found, key, scope }))
}

/// Write a cached value.
#[utoipa::path(
    put,
    path = "/apps/{app_id}/cache",
    tag = "app",
    description = "Store a value in the app's cache, optionally with a lifetime after which it disappears on its own. Set ifAbsent to keep an existing value and get it back instead of overwriting it.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = WriteCacheRequest,
    responses(
        (status = 200, description = "Value stored, or the existing value when ifAbsent prevented the write", body = WriteCacheResponse),
        (status = 400, description = "Key, value or lifetime exceeds the configured limits"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — execution permission required"),
        (status = 409, description = "The key was written and removed repeatedly during the operation"),
        (status = 503, description = "Cache backend is not configured")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("executor_jwt" = []))
)]
#[tracing::instrument(name = "PUT /apps/{app_id}/cache", skip(state, user, body))]
pub async fn write_cache_entry(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<WriteCacheRequest>,
) -> Result<Json<WriteCacheResponse>, ApiError> {
    let permission = authorize(&user, &app_id, &state).await?;
    let limits = CacheLimits::from_env();
    let store = cache_store(&state)?;

    let key = limits.validate_key(&body.key).map_err(to_api_error)?;
    limits.validate_value(&body.value).map_err(to_api_error)?;
    let expires_at = limits
        .resolve_expiry(body.ttl_seconds, chrono::Utc::now().timestamp_millis())
        .map_err(to_api_error)?;

    let entry = SetCacheEntry {
        key: build_key(&app_id, body.scope, &key, &permission)?,
        value: body.value,
        expires_at,
    };

    let (winner, stored) = if body.if_absent {
        store.get_or_set(entry).await.map_err(to_api_error)?
    } else {
        (store.set(entry).await.map_err(to_api_error)?, true)
    };

    Ok(Json(WriteCacheResponse {
        key,
        scope: body.scope,
        stored,
        value: winner.value,
        expires_at: winner.expires_at,
    }))
}

/// Delete a cached value.
#[utoipa::path(
    delete,
    path = "/apps/{app_id}/cache",
    tag = "app",
    description = "Remove a value from the app's cache.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("key" = String, Query, description = "Cache key to remove"),
        ("scope" = Option<String>, Query, description = "'app' (shared, default) or 'user' (private to the caller)")
    ),
    responses(
        (status = 200, description = "Deletion result", body = DeleteCacheResponse),
        (status = 400, description = "Invalid key or scope"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — execution permission required"),
        (status = 503, description = "Cache backend is not configured")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("executor_jwt" = []))
)]
#[tracing::instrument(name = "DELETE /apps/{app_id}/cache", skip(state, user))]
pub async fn delete_cache_entry(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<CacheEntryQuery>,
) -> Result<Json<DeleteCacheResponse>, ApiError> {
    let permission = authorize(&user, &app_id, &state).await?;
    let limits = CacheLimits::from_env();
    let store = cache_store(&state)?;

    let scope = parse_scope(query.scope.as_deref())?;
    let key = limits.validate_key(&query.key).map_err(to_api_error)?;
    let cache_key = build_key(&app_id, scope, &key, &permission)?;

    let deleted = store.delete(&cache_key).await.map_err(to_api_error)?;

    Ok(Json(DeleteCacheResponse {
        deleted,
        key,
        scope,
    }))
}

/// Anyone allowed to execute in the app may use its cache — the cache is runtime state,
/// so it follows execution rights rather than the file or config permissions.
async fn authorize(
    user: &AppUser,
    app_id: &str,
    state: &AppState,
) -> Result<AppPermissionResponse, ApiError> {
    let permission = user.execution_app_permission(app_id, state).await?;

    if !permission.has_permission(RolePermissions::ExecuteBoards)
        && !permission.has_permission(RolePermissions::ExecuteEvents)
    {
        return Err(ApiError::FORBIDDEN);
    }

    Ok(permission)
}

fn cache_store(state: &AppState) -> Result<Arc<dyn CacheStore>, ApiError> {
    state.cache_store.clone().ok_or_else(|| {
        ApiError::service_unavailable(
            "Cache backend is not configured or failed to initialize on this deployment",
        )
    })
}

fn parse_scope(raw: Option<&str>) -> Result<CacheScope, ApiError> {
    match raw {
        None => Ok(CacheScope::App),
        Some(value) if value.trim().is_empty() => Ok(CacheScope::App),
        Some(value) => CacheScope::parse(value).ok_or_else(|| {
            ApiError::bad_request(format!("Unknown cache scope '{value}'; expected app or user"))
        }),
    }
}

fn build_key(
    app_id: &str,
    scope: CacheScope,
    key: &str,
    permission: &AppPermissionResponse,
) -> Result<CacheKey, ApiError> {
    if !scope.is_user() {
        return Ok(CacheKey::app(app_id, key));
    }

    // App-connection principals carry no user identity of their own, and a run started
    // without a resolvable initiator would otherwise share one bucket with every other
    // such run. Both must fail loudly rather than read someone else's data.
    let user_id = permission.effective_user_id.as_deref().unwrap_or_default();
    let user_id = user_id.trim();

    if user_id.is_empty() || user_id == ANONYMOUS_SUB {
        return Err(ApiError::forbidden(
            "User-scoped cache requires an identifiable user; use the app scope instead",
        ));
    }

    Ok(CacheKey::user(app_id, user_id, key))
}

fn to_api_error(error: CacheStoreError) -> ApiError {
    match error {
        CacheStoreError::InvalidInput(message) => ApiError::bad_request(message),
        CacheStoreError::Configuration(message) => ApiError::service_unavailable(message),
        CacheStoreError::Contention(message) => ApiError::conflict(message),
        other => {
            tracing::error!(error = %other, "Cache backend operation failed");
            ApiError::internal_error(flow_like_types::anyhow!("Cache operation failed: {}", other))
        }
    }
}
