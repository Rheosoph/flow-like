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
    extract::{Path, State},
};
use flow_like_storage::databases::graph::lancegraph;

#[utoipa::path(
    get,
    path = "/apps/{app_id}/graph/imports",
    tag = "graph",
    description = "List remote ontology contracts installed into this project.",
    params(("app_id" = String, Path, description = "Consuming application ID")),
    responses(
        (status = 200, description = "Installed remote ontology contracts", body = Vec<Object>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/graph/imports", skip(state, user))]
pub async fn list_imports(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Vec<flow_like_catalog_core::RemoteOntologyImport>>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );

    let connection = resolve_connection(&state, &user, &app_id, &ScopeParams::default()).await?;
    let imports = lancegraph::list_ontology_imports(&connection)
        .await?
        .into_iter()
        .map(def_to_import)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(imports))
}

pub(crate) fn def_to_import(
    definition: lancegraph::RemoteOntologyImportDef,
) -> Result<flow_like_catalog_core::RemoteOntologyImport, ApiError> {
    let value = serde_json::to_value(definition).map_err(|error| {
        ApiError::internal(format!("Could not encode ontology import: {error}"))
    })?;
    serde_json::from_value(value)
        .map_err(|error| ApiError::internal(format!("Could not decode ontology import: {error}")))
}
