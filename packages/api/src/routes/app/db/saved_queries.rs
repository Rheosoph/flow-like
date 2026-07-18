//! Data Studio saved queries & views: CRUD plus an ad-hoc read-only SQL runner
//! over native tables or an ontology overlay. Persistence and execution live in
//! `flow_like_storage::databases::workbench`; this module is the HTTP surface.

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::databases::workbench::{
    self, WorkbenchSurface, WorkbenchView,
    saved_query::{self, SavedQueryDef, SavedQueryKind, SavedQuerySurface},
};
use flow_like_types::Value;
use utoipa::ToSchema;

use crate::{
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::{
        db::{ScopeParams, resolve_connection, validate_table_name},
        graph::load_scoped_overlay,
    },
    state::AppState,
};

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct CreateSavedQueryPayload {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// `query` (runnable, optionally parametrized) or `view` (composable virtual table).
    pub kind: String,
    /// `native` (raw tables) or `overlay` (an ontology).
    pub surface: String,
    #[serde(default)]
    pub overlay_id: Option<String>,
    pub sql: String,
    #[serde(default)]
    pub param_schema: Option<Value>,
    #[serde(default)]
    pub viz_config: Option<Value>,
    #[serde(default)]
    pub default_limit: Option<usize>,
}

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct UpdateSavedQueryPayload {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub overlay_id: Option<String>,
    #[serde(default)]
    pub sql: Option<String>,
    #[serde(default)]
    pub param_schema: Option<Value>,
    #[serde(default)]
    pub viz_config: Option<Value>,
    #[serde(default)]
    pub default_limit: Option<usize>,
    /// Optimistic-concurrency token: the `updated_at` the client last loaded.
    #[serde(default)]
    pub expected_updated_at: Option<String>,
}

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct ExecuteQueryPayload {
    pub sql: String,
    #[serde(default)]
    pub params: Option<Value>,
    /// `native` or `overlay`.
    pub surface: String,
    #[serde(default)]
    pub overlay_id: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

fn parse_kind(value: &str) -> Result<SavedQueryKind, ApiError> {
    match value {
        "query" => Ok(SavedQueryKind::Query),
        "view" => Ok(SavedQueryKind::View),
        other => Err(ApiError::bad_request(format!(
            "Unknown saved query kind '{other}'"
        ))),
    }
}

fn parse_surface(value: &str) -> Result<SavedQuerySurface, ApiError> {
    match value {
        "native" => Ok(SavedQuerySurface::Native),
        "overlay" => Ok(SavedQuerySurface::Overlay),
        other => Err(ApiError::bad_request(format!(
            "Unknown query surface '{other}'"
        ))),
    }
}

/// Enforces internal invariants shared by create and update: overlay surface
/// needs an overlay id, the SQL must be read-only, and views must be param-less
/// with a table-safe, non-colliding name.
async fn validate_saved_query(
    connection: &flow_like_storage::lancedb::Connection,
    def: &SavedQueryDef,
) -> Result<(), ApiError> {
    if def.surface == SavedQuerySurface::Overlay && def.overlay_id.is_none() {
        return Err(ApiError::bad_request(
            "An overlay id is required for overlay-surface queries",
        ));
    }
    workbench::validate_workbench_sql(&def.sql).map_err(ApiError::internal_error)?;

    if def.kind == SavedQueryKind::View {
        if def.sql.contains('$') {
            return Err(ApiError::bad_request("Views must not declare parameters"));
        }
        validate_table_name(&def.name)?;
        let existing = connection
            .table_names()
            .execute()
            .await
            .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!("{}", e)))?;
        if existing.iter().any(|name| name == &def.name) {
            return Err(ApiError::bad_request(format!(
                "View name '{}' collides with an existing table",
                def.name
            )));
        }
    }
    Ok(())
}

async fn collect_views(
    connection: &flow_like_storage::lancedb::Connection,
    surface: SavedQuerySurface,
    overlay_id: Option<&str>,
) -> Result<Vec<WorkbenchView>, ApiError> {
    let all = saved_query::list_saved_queries(connection).await?;
    Ok(all
        .into_iter()
        .filter(|query| query.kind == SavedQueryKind::View && query.surface == surface)
        .filter(|query| {
            surface != SavedQuerySurface::Overlay || query.overlay_id.as_deref() == overlay_id
        })
        .map(|query| WorkbenchView {
            name: query.name,
            sql: query.sql,
        })
        .collect())
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/db/queries",
    tag = "data-studio",
    description = "List saved queries and views for this app.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    responses((status = 200, description = "Saved queries")),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn list_saved_queries(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(scope): Query<ScopeParams>,
) -> Result<Json<Vec<SavedQueryDef>>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );
    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
    let queries = saved_query::list_saved_queries(&connection).await?;
    Ok(Json(queries))
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/db/queries/{query_id}",
    tag = "data-studio",
    description = "Fetch a single saved query or view.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("query_id" = String, Path, description = "Saved query ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    responses((status = 200, description = "Saved query"), (status = 404, description = "Not found")),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn get_saved_query(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, query_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
) -> Result<Json<SavedQueryDef>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );
    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
    let query = saved_query::find_saved_query(&connection, &query_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Saved query not found"))?;
    Ok(Json(query))
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/db/queries",
    tag = "data-studio",
    description = "Create a saved query or view.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = CreateSavedQueryPayload,
    responses((status = 200, description = "Created saved query"), (status = 400, description = "Bad request")),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn create_saved_query(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<CreateSavedQueryPayload>,
) -> Result<Json<SavedQueryDef>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );
    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;

    let now = chrono::Utc::now().to_rfc3339();
    let def = SavedQueryDef {
        id: uuid::Uuid::new_v4().to_string(),
        app_id: app_id.clone(),
        name: payload.name,
        description: payload.description,
        kind: parse_kind(&payload.kind)?,
        surface: parse_surface(&payload.surface)?,
        overlay_id: payload.overlay_id,
        sql: payload.sql,
        param_schema: payload.param_schema,
        viz_config: payload.viz_config,
        default_limit: payload.default_limit,
        created_at: now.clone(),
        updated_at: now,
    };
    validate_saved_query(&connection, &def).await?;
    saved_query::save_saved_query(&connection, &def).await?;
    Ok(Json(def))
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/db/queries/{query_id}",
    tag = "data-studio",
    description = "Update a saved query or view.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("query_id" = String, Path, description = "Saved query ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = UpdateSavedQueryPayload,
    responses(
        (status = 200, description = "Updated saved query"),
        (status = 404, description = "Not found"),
        (status = 409, description = "Revision conflict")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn update_saved_query(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, query_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<UpdateSavedQueryPayload>,
) -> Result<Json<SavedQueryDef>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );
    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
    let previous = saved_query::load_saved_query(&connection, &query_id)
        .await
        .map_err(|_| ApiError::not_found("Saved query not found"))?;

    if let Some(expected) = payload.expected_updated_at.as_deref()
        && expected != previous.updated_at
    {
        return Err(ApiError::conflict(
            "This saved query was modified elsewhere. Reload and try again.",
        ));
    }

    let mut def = previous.clone();
    if let Some(name) = payload.name {
        def.name = name;
    }
    if payload.description.is_some() {
        def.description = payload.description;
    }
    if let Some(kind) = payload.kind {
        def.kind = parse_kind(&kind)?;
    }
    if let Some(surface) = payload.surface {
        def.surface = parse_surface(&surface)?;
    }
    if payload.overlay_id.is_some() {
        def.overlay_id = payload.overlay_id;
    }
    if let Some(sql) = payload.sql {
        def.sql = sql;
    }
    if payload.param_schema.is_some() {
        def.param_schema = payload.param_schema;
    }
    if payload.viz_config.is_some() {
        def.viz_config = payload.viz_config;
    }
    if payload.default_limit.is_some() {
        def.default_limit = payload.default_limit;
    }
    def.updated_at = chrono::Utc::now().to_rfc3339();

    validate_saved_query(&connection, &def).await?;
    let saved =
        saved_query::save_saved_query_if_unchanged(&connection, &def, &previous.updated_at).await?;
    if !saved {
        return Err(ApiError::conflict(
            "This saved query was modified elsewhere. Reload and try again.",
        ));
    }
    Ok(Json(def))
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/db/queries/{query_id}",
    tag = "data-studio",
    description = "Delete a saved query or view.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("query_id" = String, Path, description = "Saved query ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    responses((status = 200, description = "Deleted")),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn delete_saved_query(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, query_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );
    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
    saved_query::delete_saved_query(&connection, &query_id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/db/queries/execute",
    tag = "data-studio",
    description = "Run an ad-hoc read-only SQL query against native tables or an ontology overlay, with parameters and saved views registered.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = ExecuteQueryPayload,
    responses(
        (status = 200, description = "Query results"),
        (status = 400, description = "Bad request"),
        (status = 403, description = "Forbidden")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn execute_query(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<ExecuteQueryPayload>,
) -> Result<Json<workbench::SqlQueryResult>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );

    let params = payload.params.unwrap_or(Value::Null);
    let (connection, surface, views) = match parse_surface(&payload.surface)? {
        SavedQuerySurface::Native => {
            let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
            let views = collect_views(&connection, SavedQuerySurface::Native, None).await?;
            (connection, WorkbenchSurface::Native, views)
        }
        SavedQuerySurface::Overlay => {
            let overlay_id = payload.overlay_id.as_deref().ok_or_else(|| {
                ApiError::bad_request("An overlay id is required for the overlay surface")
            })?;
            let (connection, overlay) =
                load_scoped_overlay(&state, &user, &app_id, overlay_id, &scope).await?;
            let views =
                collect_views(&connection, SavedQuerySurface::Overlay, Some(overlay_id)).await?;
            (connection, WorkbenchSurface::Overlay(overlay), views)
        }
    };

    let result = workbench::execute_readonly_sql(
        &connection,
        surface,
        views,
        &payload.sql,
        &params,
        payload.limit,
    )
    .await?;
    Ok(Json(result))
}
