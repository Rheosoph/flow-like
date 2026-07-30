use crate::{
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{ScopeParams, resolve_connection},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_catalog_core::DEFAULT_GRAPH_SAMPLE_SIZE;
use flow_like_storage::databases::graph::lancegraph::{self, RemoteOntologyImportDef};
use flow_like_storage::databases::workbench::{self, WorkbenchSurface};
use flow_like_storage::lancedb::Connection;
use flow_like_types::Value;
use utoipa::ToSchema;

/// Resolves an installed remote ontology and opens a read connection to the
/// producing project.
///
/// The consumer never receives raw producer coordinates; access is authorized
/// through the pinned import record. Both the active connection and the source's
/// live `exposed` decision are re-checked here so uninstalling, disconnecting, or
/// revoking exposure at the source stops reads even though the installed snapshot
/// stays stable.
async fn open_import_target(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
    import_id: &str,
) -> Result<(RemoteOntologyImportDef, Connection), ApiError> {
    let local = resolve_connection(state, user, app_id, &ScopeParams::default()).await?;
    let import = lancegraph::find_ontology_import(&local, import_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Installed ontology not found"))?;
    if !import.bindings_enabled {
        return Err(ApiError::forbidden(
            "The installed ontology bindings are disabled",
        ));
    }

    crate::routes::app::connection::remote_ontologies::ensure_remote_ontology_access(
        state,
        app_id,
        &import.target_app_id,
    )
    .await?;

    let credentials = state.master_credentials().await?;
    let target = credentials
        .to_db(&import.target_app_id)
        .await?
        .execute()
        .await?;

    let live = lancegraph::load_overlay(&target, &import.remote_ontology_id)
        .await
        .map_err(|_| ApiError::forbidden("The remote ontology is no longer available"))?;
    if !live.exposed {
        return Err(ApiError::forbidden(
            "The remote ontology is no longer exposed to connected projects",
        ));
    }
    if live.updated_at != import.source_updated_at {
        return Err(ApiError::conflict(
            "The remote ontology changed after this contract was installed. Refresh the installed ontology before reading it.",
        ));
    }

    Ok((import, target))
}

#[derive(Debug, serde::Deserialize)]
pub struct ImportSampleParams {
    pub label: String,
    #[serde(default = "default_n")]
    pub n: usize,
}

fn default_n() -> usize {
    DEFAULT_GRAPH_SAMPLE_SIZE
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/graph/imports/{import_id}/sample",
    tag = "graph",
    description = "Preview rows for an object of an installed remote ontology. Rows are read live from the connected project through the pinned contract.",
    params(
        ("app_id" = String, Path, description = "Consuming application ID"),
        ("import_id" = String, Path, description = "Installed remote ontology identifier"),
        ("label" = String, Query, description = "Object type or edge label to sample from"),
        ("n" = Option<usize>, Query, description = "Number of samples (default 10)")
    ),
    responses(
        (status = 200, description = "Sample rows from the remote object", body = Vec<flow_like_types::Value>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Bindings disabled or exposure revoked"),
        (status = 404, description = "Installed ontology or label not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/graph/imports/{import_id}/sample",
    skip(state, user, params)
)]
pub async fn sample_import(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, import_id)): Path<(String, String)>,
    Query(params): Query<ImportSampleParams>,
) -> Result<Json<Vec<Value>>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );

    let (import, target) = open_import_target(&state, &user, &app_id, &import_id).await?;
    let results =
        lancegraph::sample_overlay(&target, &import.contract, &params.label, params.n.min(500))
            .await?;

    Ok(Json(results))
}

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct ImportQueryPayload {
    pub sql: String,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/graph/imports/{import_id}/query",
    tag = "graph",
    description = "Run a read-only SQL query against an installed remote ontology. Only the object and edge tables of the pinned contract are queryable; the query runs against the connected project's data.",
    params(
        ("app_id" = String, Path, description = "Consuming application ID"),
        ("import_id" = String, Path, description = "Installed remote ontology identifier")
    ),
    request_body = ImportQueryPayload,
    responses(
        (status = 200, description = "Query results"),
        (status = 400, description = "Invalid or non read-only query"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Bindings disabled or exposure revoked"),
        (status = 404, description = "Installed ontology not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/graph/imports/{import_id}/query",
    skip(state, user, payload)
)]
pub async fn query_import(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, import_id)): Path<(String, String)>,
    Json(payload): Json<ImportQueryPayload>,
) -> Result<Json<workbench::SqlQueryResult>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );

    let (import, target) = open_import_target(&state, &user, &app_id, &import_id).await?;
    let params = payload.params.unwrap_or(Value::Null);
    let result = workbench::execute_readonly_sql(
        &target,
        WorkbenchSurface::RemoteOverlay(import.contract),
        Vec::new(),
        &payload.sql,
        &params,
        payload.limit,
    )
    .await?;

    Ok(Json(result))
}
