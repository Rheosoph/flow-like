//! Authenticated proxy onto an event's REST / MCP surface.
//!
//! The public inbound routers (`/r`, `/m`) authenticate per registration
//! (api keys, OAuth, …). These endpoints instead authenticate through the
//! regular API auth — including app-connection tokens — so a connected app
//! can call another app's REST routes and MCP server with the permissions of
//! its connection role. Configured per-registration auth is still required.
//! Because the app-connection token occupies `Authorization`, callers forward
//! registration-level Basic/Bearer/OAuth credentials through
//! `x-flow-like-event-authorization`. The caller identity is injected into
//! `_client.proxy`, separately from registration auth in `_client.auth`.

use crate::{
    ensure_permission,
    entity::event,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::inbound::{ProxyCallerContext, dispatch_mcp_for_event, dispatch_rest_for_event},
    state::AppState,
};
use axum::{
    Extension,
    body::Bytes,
    extract::{Path, RawQuery, State},
    http::{HeaderMap, Method},
    response::Response,
};
use flow_like_types::json::json;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

/// Process-mining caller identity for proxied calls: ties the dispatched run
/// into the calling run's case (parent run, app chain, correlation).
fn proxy_caller(user: &AppUser) -> ProxyCallerContext {
    match user {
        AppUser::ConnectedApp(app) => ProxyCallerContext {
            app_chain: Some(app.app_chain.clone()),
            parent_run_id: app.run_id.clone(),
            correlation: app.correlation.clone(),
        },
        _ => ProxyCallerContext::default(),
    }
}

fn caller_auth(user: &AppUser) -> flow_like_types::Value {
    match user {
        AppUser::ConnectedApp(app) => json!({
            "via": "app_connection",
            "sub": app.sub,
            "origin_app_id": app.origin_app_id,
            "app_chain": app.app_chain,
        }),
        _ => json!({
            "via": "api",
            "sub": user.effective_user_id().ok(),
        }),
    }
}

fn ensure_connected_app_proxy(user: &AppUser) -> Result<(), ApiError> {
    if user.is_connected_app() {
        Ok(())
    } else {
        Err(ApiError::forbidden(
            "Internal event proxies may only be called by connected apps",
        ))
    }
}

async fn load_event(
    state: &AppState,
    app_id: &str,
    event_id: &str,
) -> Result<event::Model, ApiError> {
    event::Entity::find_by_id(event_id)
        .filter(event::Column::AppId.eq(app_id))
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("Event not found"))
}

#[allow(clippy::too_many_arguments)]
async fn proxy_rest_inner(
    state: AppState,
    user: AppUser,
    app_id: String,
    event_id: String,
    path: String,
    raw_query: Option<String>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    ensure_connected_app_proxy(&user)?;
    ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);
    let event_row = load_event(&state, &app_id, &event_id).await?;
    let auth = caller_auth(&user);
    let caller = proxy_caller(&user);

    dispatch_rest_for_event(
        &state,
        &event_row,
        &event_id,
        &path,
        raw_query.as_deref().unwrap_or(""),
        &headers,
        &method,
        &body,
        false,
        Some(auth),
        &caller,
    )
    .await
}

/// ANY /apps/{app_id}/events/{event_id}/rest — root of the event's REST surface
#[tracing::instrument(
    name = "ANY /apps/{app_id}/events/{event_id}/rest",
    skip(state, user, headers, body)
)]
pub async fn proxy_rest_root(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    RawQuery(raw_query): RawQuery,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    proxy_rest_inner(
        state,
        user,
        app_id,
        event_id,
        "/".to_string(),
        raw_query,
        method,
        headers,
        body,
    )
    .await
}

/// ANY /apps/{app_id}/events/{event_id}/rest/{*path}
#[tracing::instrument(
    name = "ANY /apps/{app_id}/events/{event_id}/rest/{*path}",
    skip(state, user, headers, body)
)]
pub async fn proxy_rest(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id, path)): Path<(String, String, String)>,
    RawQuery(raw_query): RawQuery,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    proxy_rest_inner(
        state, user, app_id, event_id, path, raw_query, method, headers, body,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn proxy_mcp_inner(
    state: AppState,
    user: AppUser,
    app_id: String,
    event_id: String,
    path: String,
    raw_query: Option<String>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    ensure_connected_app_proxy(&user)?;
    ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);
    let event_row = load_event(&state, &app_id, &event_id).await?;
    let auth = caller_auth(&user);
    let caller = proxy_caller(&user);

    dispatch_mcp_for_event(
        &state,
        &event_row,
        &event_id,
        &path,
        raw_query.as_deref().unwrap_or(""),
        &headers,
        &method,
        &body,
        false,
        Some(auth),
        &caller,
    )
    .await
}

/// ANY /apps/{app_id}/events/{event_id}/mcp — the event's MCP server,
/// reachable by connected apps with an ExecuteEvents connection role.
#[tracing::instrument(
    name = "ANY /apps/{app_id}/events/{event_id}/mcp",
    skip(state, user, headers, body)
)]
pub async fn proxy_mcp(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    RawQuery(raw_query): RawQuery,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    proxy_mcp_inner(
        state,
        user,
        app_id,
        event_id,
        "/".to_string(),
        raw_query,
        method,
        headers,
        body,
    )
    .await
}

/// ANY /apps/{app_id}/events/{event_id}/mcp/{*path} — auxiliary MCP paths,
/// including OAuth protected-resource metadata discovery.
#[tracing::instrument(
    name = "ANY /apps/{app_id}/events/{event_id}/mcp/{*path}",
    skip(state, user, headers, body)
)]
pub async fn proxy_mcp_path(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id, path)): Path<(String, String, String)>,
    RawQuery(raw_query): RawQuery,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    proxy_mcp_inner(
        state, user, app_id, event_id, path, raw_query, method, headers, body,
    )
    .await
}
