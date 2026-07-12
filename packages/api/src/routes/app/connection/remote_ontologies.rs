use crate::{
    ensure_permission,
    entity::{app_connection, role, sea_orm_active_enums::AppConnectionStatus},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::{RolePermissions, has_role_permission},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

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

    let connection = app_connection::Entity::find()
        .filter(
            app_connection::Column::SourceAppId
                .eq(&app_id)
                .and(app_connection::Column::TargetAppId.eq(&target_app_id))
                .and(app_connection::Column::Status.eq(AppConnectionStatus::Active)),
        )
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("No active connection to the target app"))?;
    let role_id = connection.role_id.ok_or(ApiError::FORBIDDEN)?;
    let role_model = role::Entity::find_by_id(role_id)
        .filter(role::Column::AppId.eq(&target_app_id))
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

    let credentials = state.master_credentials().await?;
    let builder = credentials.to_db(&target_app_id).await?;
    let database = builder.execute().await?;
    let ontologies = flow_like_storage::databases::graph::lancegraph::list_overlays(&database)
        .await?
        .into_iter()
        .filter(|ontology| ontology.exposed)
        .map(crate::routes::app::graph::list_overlays::def_to_overlay)
        .map(|mut ontology| {
            // A remote contract exposes action semantics, not the target
            // project's private implementation coordinates. Future remote
            // execution resolves the opaque ontology/action IDs server-side.
            for action in &mut ontology.actions {
                action.board_id.clear();
                action.board_version = None;
                action.start_node_id = None;
                action.event_id = None;
            }
            ontology
        })
        .collect();

    Ok(Json(ontologies))
}
