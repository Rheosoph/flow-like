use super::update_object::{UpdateOverlayRowResponse, identity_label, overlay_row_update_error};
use crate::{
    audit_branch, ensure_any_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::app::db::ScopeParams, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::databases::graph::lancegraph;
use flow_like_types::{Value, json::Map};
use utoipa::ToSchema;

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct UpdateRelationshipPayload {
    /// Relationship type to edit, by id, API name or label.
    pub relationship_type: String,
    /// Value of the relationship's source column.
    #[schema(value_type = Object)]
    pub source: Value,
    /// Value of the relationship's target column.
    #[schema(value_type = Object)]
    pub target: Value,
    /// Property values to save, keyed by column. Only changed properties belong here.
    #[schema(value_type = Object)]
    pub updates: Map<String, Value>,
    /// The value each updated property had when it was read. The edit is not
    /// applied when one of them no longer matches.
    #[schema(value_type = Object)]
    pub expected: Map<String, Value>,
}

impl From<UpdateRelationshipPayload> for lancegraph::RelationshipPropertyUpdate {
    fn from(payload: UpdateRelationshipPayload) -> Self {
        Self {
            relationship_type: payload.relationship_type,
            source: payload.source,
            target: payload.target,
            updates: payload.updates,
            expected: payload.expected,
        }
    }
}

#[utoipa::path(
    patch,
    path = "/apps/{app_id}/graph/{overlay_id}/relationships",
    tag = "graph",
    description = "Update property values of one relationship stored in its own table. The ontology resolves the table and the source and target columns; those columns cannot be changed and nothing is inserted. Relationships stored on an object's row are edited through that object. Returns the saved relationship, or the current relationship when it changed since it was read.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("overlay_id" = String, Path, description = "Overlay ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = UpdateRelationshipPayload,
    responses(
        (status = 200, description = "The saved relationship, or the current relationship when it changed since it was read", body = UpdateOverlayRowResponse),
        (status = 400, description = "A property cannot be edited or a value does not fit its column"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Ontology or relationship not found"),
        (status = 409, description = "Several relationships share this source and target, or the relationship changed while saving. The message says when a concurrent change made the edit reach several rows")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "PATCH /apps/{app_id}/graph/{overlay_id}/relationships",
    skip(state, user, scope, payload)
)]
pub async fn update_relationship(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<UpdateRelationshipPayload>,
) -> Result<Json<UpdateOverlayRowResponse>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );
    if user.is_connected_app() {
        return Err(ApiError::forbidden(
            "Connected projects cannot edit ontology relationships",
        ));
    }

    let (connection, overlay) =
        super::load_scoped_overlay_for_write(&state, &user, &app_id, &overlay_id, &scope).await?;
    let relationship_type = payload.relationship_type.clone();
    let source = identity_label(&payload.source);
    let target = identity_label(&payload.target);
    let columns = payload.updates.keys().cloned().collect::<Vec<_>>();

    let result =
        lancegraph::update_overlay_relationship(&connection, &overlay, payload.into()).await;
    let rows = lancegraph::overlay_rows_written(&result);
    if rows > 0 {
        audit_branch!(
            state,
            user,
            app_id,
            "graph.relationships.update",
            "GraphOverlay",
            overlay_id,
            serde_json::json!({
                "relationship_type": relationship_type,
                "source": source,
                "target": target,
                "columns": columns,
                "rows": rows,
                "user_scoped": scope.is_user_scoped(),
            })
        );
    }

    Ok(Json(result.map_err(overlay_row_update_error)?.into()))
}
