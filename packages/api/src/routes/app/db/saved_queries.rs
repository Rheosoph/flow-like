//! Data Studio saved queries & views: CRUD plus an ad-hoc read-only SQL runner
//! over native tables or an ontology overlay. Persistence lives in
//! `flow_like_storage::databases::workbench`, statements run on the query
//! service; this module is the HTTP surface.

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_query::{Operation, QueryResponse, QueryShape, SurfaceSpec, ViewSpec};
use flow_like_storage::databases::{
    graph::lancegraph,
    workbench::{
        self, WorkbenchView,
        saved_query::{
            self, SavedQueryDef, SavedQueryKind, SavedQuerySaveResult, SavedQuerySurface,
        },
    },
};
use flow_like_types::Value;
use utoipa::ToSchema;

use crate::{
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    query::{self, LanceRead, QueryRows},
    routes::app::{
        data_sources::query_source::external_sources_named,
        db::{ScopeParams, resolve_connection, resolve_write_connection, validate_table_name},
        graph::load_scoped_overlay,
    },
    state::AppState,
};
use std::collections::BTreeSet;

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

fn deserialize_present_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    <Option<T> as serde::Deserialize>::deserialize(deserializer).map(Some)
}

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct UpdateSavedQueryPayload {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    pub description: Option<Option<String>>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    pub overlay_id: Option<Option<String>>,
    #[serde(default)]
    pub sql: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    pub param_schema: Option<Option<Value>>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    pub viz_config: Option<Option<Value>>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    pub default_limit: Option<Option<usize>>,
    /// Required optimistic-concurrency token: the `updated_at` the client last loaded.
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
    match def.surface {
        SavedQuerySurface::Overlay => {
            let overlay_id = def
                .overlay_id
                .as_deref()
                .filter(|overlay_id| !overlay_id.trim().is_empty())
                .ok_or_else(|| {
                    ApiError::bad_request("A non-empty overlay id is required for overlay queries")
                })?;
            lancegraph::load_overlay(connection, overlay_id)
                .await
                .map_err(|_| ApiError::bad_request("The selected graph overlay does not exist"))?;
        }
        SavedQuerySurface::Native if def.overlay_id.is_some() => {
            return Err(ApiError::bad_request(
                "overlay_id must be omitted for native queries",
            ));
        }
        SavedQuerySurface::Native => {}
    }
    workbench::validate_workbench_sql(&def.sql)
        .map_err(|error| ApiError::bad_request(format!("Invalid saved-query SQL: {error}")))?;

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
        if existing
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&def.name))
        {
            return Err(ApiError::bad_request(format!(
                "View name '{}' collides with an existing table",
                def.name
            )));
        }
    }
    Ok(())
}

pub(super) async fn collect_views(
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

/// Where a workbench statement runs: the app, which of its databases, and the surface.
pub(super) struct SurfaceTarget<'a> {
    pub app_id: &'a str,
    pub scope: &'a ScopeParams,
    /// `native` or `overlay`.
    pub surface: &'a str,
    pub overlay_id: Option<&'a str>,
}

/// The surface with the saved views registered on it, as the query service receives it.
async fn workbench_shape(
    state: &AppState,
    user: &AppUser,
    target: &SurfaceTarget<'_>,
) -> Result<QueryShape, ApiError> {
    let (surface, views) = match parse_surface(target.surface)? {
        SavedQuerySurface::Native => {
            let connection = resolve_connection(state, user, target.app_id, target.scope).await?;
            let views = collect_views(&connection, SavedQuerySurface::Native, None).await?;
            (SurfaceSpec::Native, views)
        }
        SavedQuerySurface::Overlay => {
            let overlay_id = target.overlay_id.ok_or_else(|| {
                ApiError::bad_request("An overlay id is required for the overlay surface")
            })?;
            let (connection, overlay) =
                load_scoped_overlay(state, user, target.app_id, overlay_id, target.scope).await?;
            let views =
                collect_views(&connection, SavedQuerySurface::Overlay, Some(overlay_id)).await?;
            (
                SurfaceSpec::Overlay {
                    overlay: Box::new(overlay),
                },
                views,
            )
        }
    };
    Ok(QueryShape::Workbench {
        surface,
        views: views
            .into_iter()
            .map(|view| ViewSpec {
                name: view.name,
                sql: view.sql,
            })
            .collect(),
    })
}

/// Runs the operation `build` makes for the target's surface on the query service, after the
/// read check every workbench read makes.
pub(super) async fn run_on_surface(
    state: &AppState,
    user: &AppUser,
    target: SurfaceTarget<'_>,
    rows: usize,
    build: impl FnOnce(QueryShape) -> Operation,
) -> Result<QueryResponse, ApiError> {
    let permission = ensure_any_permission!(
        user,
        target.app_id,
        state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );
    let shape = workbench_shape(state, user, &target).await?;
    let read = LanceRead::scoped(user, &permission, target.app_id, target.scope)?;
    let operation = build(shape);
    let mut sources = vec![state.query.lance_source(state, &read, None).await?];
    if permission.has_permission(RolePermissions::ReadDatabase) {
        let named =
            external_sources_named(state, target.app_id, &named_schemas(&operation)).await?;
        sources.extend(
            named
                .into_iter()
                .map(|(alias, source)| query::external_source(source, Some(alias))),
        );
    }
    let request = query::request(sources, rows, operation);
    state.query.run(&query::subject(user), &request).await
}

/// The schemas an operation's statement and the saved views it can reach name, which is where
/// an app's external databases appear (`<alias>.<table>`). Only those are sent with credentials.
fn named_schemas(operation: &Operation) -> BTreeSet<String> {
    let (sql, shape) = match operation {
        Operation::Sql { sql, shape, .. } | Operation::Describe { sql, shape } => (sql, shape),
        Operation::ParameterOptions { table, .. } => {
            return table
                .split_once('.')
                .map(|(schema, _)| schema.to_lowercase())
                .into_iter()
                .collect();
        }
        _ => return BTreeSet::new(),
    };
    let mut statements = vec![sql.as_str()];
    if let QueryShape::Workbench { views, .. } = shape {
        statements.extend(views.iter().map(|view| view.sql.as_str()));
    }
    workbench::referenced_schemas(&statements)
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
    let connection = resolve_write_connection(&state, &user, &app_id, &scope).await?;

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
    match saved_query::save_saved_query(&connection, &def).await? {
        SavedQuerySaveResult::Saved => {}
        SavedQuerySaveResult::ViewNameConflict { .. } => {
            return Err(ApiError::bad_request(format!(
                "View name '{}' is already used by another saved view on this surface",
                def.name
            )));
        }
        SavedQuerySaveResult::RevisionConflict => {
            return Err(ApiError::conflict(
                "A saved query with this ID was created concurrently. Retry the request.",
            ));
        }
        SavedQuerySaveResult::ViewLimitExceeded { limit } => {
            return Err(ApiError::bad_request(format!(
                "A workbench surface can contain at most {limit} saved views"
            )));
        }
    }
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
    let connection = resolve_write_connection(&state, &user, &app_id, &scope).await?;
    let previous = saved_query::load_saved_query(&connection, &query_id)
        .await
        .map_err(|_| ApiError::not_found("Saved query not found"))?;

    let expected_updated_at = payload.expected_updated_at.clone().ok_or_else(|| {
        ApiError::bad_request("expected_updated_at is required when updating a saved query")
    })?;
    if expected_updated_at != previous.updated_at {
        return Err(ApiError::conflict(
            "This saved query was modified elsewhere. Reload and try again.",
        ));
    }

    let mut def = previous.clone();
    if let Some(name) = payload.name {
        def.name = name;
    }
    if let Some(description) = payload.description {
        def.description = description;
    }
    if let Some(kind) = payload.kind {
        def.kind = parse_kind(&kind)?;
    }
    if let Some(surface) = payload.surface {
        def.surface = parse_surface(&surface)?;
    }
    if let Some(overlay_id) = payload.overlay_id {
        def.overlay_id = overlay_id;
    }
    if let Some(sql) = payload.sql {
        def.sql = sql;
    }
    if let Some(param_schema) = payload.param_schema {
        def.param_schema = param_schema;
    }
    if let Some(viz_config) = payload.viz_config {
        def.viz_config = viz_config;
    }
    if let Some(default_limit) = payload.default_limit {
        def.default_limit = default_limit;
    }
    def.updated_at = saved_query::next_updated_at(&previous.updated_at);

    validate_saved_query(&connection, &def).await?;
    match saved_query::save_saved_query_if_unchanged(&connection, &def, &expected_updated_at)
        .await?
    {
        SavedQuerySaveResult::Saved => {}
        SavedQuerySaveResult::RevisionConflict => {
            return Err(ApiError::conflict(
                "This saved query was modified elsewhere. Reload and try again.",
            ));
        }
        SavedQuerySaveResult::ViewNameConflict { .. } => {
            return Err(ApiError::bad_request(format!(
                "View name '{}' is already used by another saved view on this surface",
                def.name
            )));
        }
        SavedQuerySaveResult::ViewLimitExceeded { limit } => {
            return Err(ApiError::bad_request(format!(
                "A workbench surface can contain at most {limit} saved views"
            )));
        }
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
    let connection = resolve_write_connection(&state, &user, &app_id, &scope).await?;
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
        (status = 200, description = "Query results, with `warnings` when the query service lowered a limit"),
        (status = 400, description = "Bad request, or the query failed"),
        (status = 403, description = "Forbidden"),
        (status = 413, description = "The query needs more memory than the query service allows"),
        (status = 503, description = "The query service is unavailable"),
        (status = 504, description = "The query ran out of time")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(name = "POST /apps/{app_id}/db/queries/execute", skip_all)]
pub async fn execute_query(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<ExecuteQueryPayload>,
) -> Result<Json<QueryRows>, ApiError> {
    let params = payload.params.unwrap_or(Value::Null);
    let target = SurfaceTarget {
        app_id: &app_id,
        scope: &scope,
        surface: &payload.surface,
        overlay_id: payload.overlay_id.as_deref(),
    };
    let response = run_on_surface(
        &state,
        &user,
        target,
        query::workbench_rows(payload.limit),
        |shape| Operation::Sql {
            sql: payload.sql,
            params,
            shape,
            reuse_if_unchanged: None,
        },
    )
    .await?;
    Ok(Json(QueryRows::from_response(&response)?))
}

#[cfg(test)]
mod tests {
    use super::{UpdateSavedQueryPayload, named_schemas};
    use flow_like_query::{Operation, QueryShape, SurfaceSpec, ViewSpec};
    use serde_json::json;
    use std::collections::BTreeSet;

    fn native(views: Vec<ViewSpec>) -> QueryShape {
        QueryShape::Workbench {
            surface: SurfaceSpec::Native,
            views,
        }
    }

    fn options(table: &str) -> Operation {
        Operation::ParameterOptions {
            table: table.to_string(),
            column: "status".to_string(),
            search: None,
            range: false,
            shape: native(Vec::new()),
        }
    }

    #[test]
    fn external_databases_are_sent_only_when_a_statement_or_view_names_them() {
        let late = ViewSpec {
            name: "late".to_string(),
            sql: "SELECT * FROM billing.invoices".to_string(),
        };
        let run = Operation::Sql {
            sql: "SELECT * FROM warehouse.orders JOIN customers USING (id)".to_string(),
            params: json!(null),
            shape: native(vec![late]),
            reuse_if_unchanged: None,
        };
        assert_eq!(
            named_schemas(&run),
            BTreeSet::from(["billing".to_string(), "warehouse".to_string()])
        );
        let describe = Operation::Describe {
            sql: "SELECT * FROM customers".to_string(),
            shape: native(Vec::new()),
        };
        assert!(named_schemas(&describe).is_empty());
        assert_eq!(
            named_schemas(&options("Warehouse.orders")),
            BTreeSet::from(["warehouse".to_string()])
        );
        assert!(named_schemas(&options("orders")).is_empty());
    }

    #[test]
    fn nullable_update_fields_distinguish_omission_clear_and_value() {
        let omitted: UpdateSavedQueryPayload = serde_json::from_value(json!({})).unwrap();
        assert_eq!(omitted.description, None);
        assert_eq!(omitted.default_limit, None);

        let cleared: UpdateSavedQueryPayload = serde_json::from_value(json!({
            "description": null,
            "overlay_id": null,
            "param_schema": null,
            "viz_config": null,
            "default_limit": null
        }))
        .unwrap();
        assert_eq!(cleared.description, Some(None));
        assert_eq!(cleared.overlay_id, Some(None));
        assert_eq!(cleared.param_schema, Some(None));
        assert_eq!(cleared.viz_config, Some(None));
        assert_eq!(cleared.default_limit, Some(None));

        let populated: UpdateSavedQueryPayload = serde_json::from_value(json!({
            "description": "description",
            "default_limit": 25
        }))
        .unwrap();
        assert_eq!(populated.description, Some(Some("description".to_string())));
        assert_eq!(populated.default_limit, Some(Some(25)));
    }
}
