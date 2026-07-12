use crate::{
    ensure_any_permission, ensure_permission,
    entity::{app_connection, role, sea_orm_active_enums::AppConnectionStatus},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::{RolePermissions, has_role_permission},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use flow_like_storage::databases::graph::lancegraph;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

async fn ensure_remote_ontology_access(
    state: &AppState,
    app_id: &str,
    target_app_id: &str,
) -> Result<(), ApiError> {
    let connection = app_connection::Entity::find()
        .filter(
            app_connection::Column::SourceAppId
                .eq(app_id)
                .and(app_connection::Column::TargetAppId.eq(target_app_id))
                .and(app_connection::Column::Status.eq(AppConnectionStatus::Active)),
        )
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("No active connection to the target app"))?;
    let role_id = connection.role_id.ok_or(ApiError::FORBIDDEN)?;
    let role_model = role::Entity::find_by_id(role_id)
        .filter(role::Column::AppId.eq(target_app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::FORBIDDEN)?;
    let permissions = RolePermissions::from_bits(role_model.permissions)
        .ok_or_else(|| ApiError::internal("Invalid role permission bits"))?;
    if !has_role_permission(&permissions, RolePermissions::ReadFiles)
        && !has_role_permission(&permissions, RolePermissions::ReadDatabase)
    {
        return Err(ApiError::forbidden(
            "The connection role does not allow reading ontology contracts",
        ));
    }
    Ok(())
}

pub(crate) fn sanitize_remote_contract(
    mut ontology: lancegraph::GraphOverlayDef,
) -> lancegraph::GraphOverlayDef {
    // Remote consumers receive action semantics, never private implementation
    // coordinates. Governed action execution resolves these opaque IDs in the
    // target project.
    for action in &mut ontology.actions {
        action.board_id.clear();
        action.board_version = None;
        action.start_node_id = None;
        action.event_id = None;
    }
    ontology
}

fn ontology_import_id(target_app_id: &str, ontology_id: &str) -> String {
    format!("{target_app_id}::{ontology_id}")
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/connections/{target_app_id}/ontologies",
    tag = "team",
    description = "List ontology contracts exposed by a connected app. The connection role must allow reading files or databases.",
    params(
        ("app_id" = String, Path, description = "Application consuming the ontology"),
        ("target_app_id" = String, Path, description = "Connected application exposing the ontology")
    ),
    responses(
        (status = 200, description = "Exposed ontology contracts", body = Vec<Object>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Connection role cannot read project data"),
        (status = 404, description = "No active connection to the target app")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/connections/{target_app_id}/ontologies",
    skip(state, user)
)]
pub async fn get_remote_ontologies(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, target_app_id)): Path<(String, String)>,
) -> Result<Json<Vec<flow_like_catalog_core::GraphOverlay>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    ensure_remote_ontology_access(&state, &app_id, &target_app_id).await?;

    let credentials = state.master_credentials().await?;
    let builder = credentials.to_db(&target_app_id).await?;
    let database = builder.execute().await?;
    let ontologies = lancegraph::list_overlays(&database)
        .await?
        .into_iter()
        .filter(|ontology| ontology.exposed)
        .map(sanitize_remote_contract)
        .map(crate::routes::app::graph::list_overlays::def_to_overlay)
        .collect();

    Ok(Json(ontologies))
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/connections/{target_app_id}/ontologies/{ontology_id}/install",
    tag = "team",
    description = "Install or refresh an exposed ontology contract from a connected project.",
    params(
        ("app_id" = String, Path, description = "Application consuming the ontology"),
        ("target_app_id" = String, Path, description = "Connected application exposing the ontology"),
        ("ontology_id" = String, Path, description = "Remote ontology identifier")
    ),
    responses(
        (status = 200, description = "Installed ontology contract", body = Object),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Connection or local project permissions are insufficient"),
        (status = 404, description = "Connection or exposed ontology not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "PUT /apps/{app_id}/connections/{target_app_id}/ontologies/{ontology_id}/install",
    skip(state, user)
)]
pub async fn install_remote_ontology(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, target_app_id, ontology_id)): Path<(String, String, String)>,
) -> Result<Json<flow_like_catalog_core::RemoteOntologyImport>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );
    ensure_remote_ontology_access(&state, &app_id, &target_app_id).await?;

    let credentials = state.master_credentials().await?;
    let target_builder = credentials.to_db(&target_app_id).await?;
    let target_database = target_builder.execute().await?;
    let contract = lancegraph::load_overlay(&target_database, &ontology_id).await?;
    if !contract.exposed {
        return Err(ApiError::not_found("The remote ontology is not exposed"));
    }
    let contract = sanitize_remote_contract(contract);

    let local_builder = credentials.to_db(&app_id).await?;
    let local_database = local_builder.execute().await?;
    let import_id = ontology_import_id(&target_app_id, &contract.id);
    let previous = lancegraph::find_ontology_import(&local_database, &import_id).await?;
    let now = chrono::Utc::now().to_rfc3339();
    let ontology_import = lancegraph::RemoteOntologyImportDef {
        id: import_id,
        target_app_id,
        remote_ontology_id: contract.id.clone(),
        source_updated_at: contract.updated_at.clone(),
        bindings_enabled: previous
            .as_ref()
            .map(|installed| installed.bindings_enabled)
            .unwrap_or(true),
        installed_at: previous
            .map(|installed| installed.installed_at)
            .unwrap_or_else(|| now.clone()),
        updated_at: now,
        contract,
    };
    lancegraph::save_ontology_import(&local_database, &ontology_import).await?;

    Ok(Json(
        crate::routes::app::graph::list_imports::def_to_import(ontology_import)?,
    ))
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/connections/{target_app_id}/ontologies/{ontology_id}/install",
    tag = "team",
    description = "Remove a remote ontology contract and its generated bindings from this project.",
    params(
        ("app_id" = String, Path, description = "Application consuming the ontology"),
        ("target_app_id" = String, Path, description = "Connected application exposing the ontology"),
        ("ontology_id" = String, Path, description = "Remote ontology identifier")
    ),
    responses(
        (status = 204, description = "Ontology import removed"),
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
    name = "DELETE /apps/{app_id}/connections/{target_app_id}/ontologies/{ontology_id}/install",
    skip(state, user)
)]
pub async fn uninstall_remote_ontology(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, target_app_id, ontology_id)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );

    // Uninstall deliberately does not require the remote connection to remain
    // active. A project must always be able to clean up a stale import.
    let credentials = state.master_credentials().await?;
    let local_builder = credentials.to_db(&app_id).await?;
    let local_database = local_builder.execute().await?;
    let import_id = ontology_import_id(&target_app_id, &ontology_id);
    lancegraph::delete_ontology_import(&local_database, &import_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
