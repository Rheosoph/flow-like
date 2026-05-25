//! `GET /aliases/{slug}` — public alias lookup.
//!
//! Returns the resolved `(app_id, event_id)` for an alias slug without
//! requiring any app-level permission. This is intentionally read-only
//! and exists to let SDKs / inbound routers cheaply pre-check whether a
//! slug exists before constructing a request URL.

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{error::ApiError, state::AppState, utils::event_alias as alias_util};

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct AliasLookupResponse {
    pub slug: String,
    pub app_id: String,
    pub event_id: String,
}

#[utoipa::path(
    get,
    path = "/aliases/{slug}",
    tag = "aliases",
    description = "Resolve an alias slug to its (app_id, event_id).",
    params(("slug" = String, Path, description = "Alias slug")),
    responses(
        (status = 200, description = "Resolved", body = AliasLookupResponse),
        (status = 404, description = "Not found"),
    )
)]
pub async fn lookup_alias(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<AliasLookupResponse>, ApiError> {
    alias_util::validate_slug(&slug)?;
    let resolved = alias_util::resolve(&state.db, &slug, None).await?;
    Ok(Json(AliasLookupResponse {
        slug,
        app_id: resolved.app_id,
        event_id: resolved.event_id,
    }))
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/{slug}", get(lookup_alias))
}
