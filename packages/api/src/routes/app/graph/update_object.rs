use crate::{
    audit_branch, ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{ScopeParams, table_input_error},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::databases::graph::lancegraph;
use flow_like_types::{Value, json::Map};
use utoipa::ToSchema;

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct UpdateObjectPayload {
    /// Object type to edit, by id, API name or label.
    pub object_type: String,
    /// Identity value of the object, as the ontology's identity column stores it.
    #[schema(value_type = Object)]
    pub id: Value,
    /// Property values to save, keyed by column. Only changed properties belong here.
    #[schema(value_type = Object)]
    pub updates: Map<String, Value>,
    /// The value each updated property had when it was read. The edit is not
    /// applied when one of them no longer matches.
    #[schema(value_type = Object)]
    pub expected: Map<String, Value>,
}

impl From<UpdateObjectPayload> for lancegraph::ObjectPropertyUpdate {
    fn from(payload: UpdateObjectPayload) -> Self {
        Self {
            object_type: payload.object_type,
            id: payload.id,
            updates: payload.updates,
            expected: payload.expected,
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UpdateOverlayRowOutcome {
    Updated,
    Stale,
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct UpdateOverlayRowResponse {
    /// `updated` when the values were saved, `stale` when nothing was written
    /// because the row changed since it was read.
    pub outcome: UpdateOverlayRowOutcome,
    /// The saved row, or the current row when the outcome is `stale`.
    #[schema(value_type = Object)]
    pub row: Value,
}

impl From<lancegraph::OverlayRowUpdateResult> for UpdateOverlayRowResponse {
    fn from(result: lancegraph::OverlayRowUpdateResult) -> Self {
        let outcome = match result.outcome {
            lancegraph::OverlayRowUpdateOutcome::Updated => UpdateOverlayRowOutcome::Updated,
            lancegraph::OverlayRowUpdateOutcome::Stale => UpdateOverlayRowOutcome::Stale,
        };
        Self {
            outcome,
            row: result.row,
        }
    }
}

/// Maps a rejected ontology row edit to its status: a missing row is 404, an
/// ambiguous or concurrently changed row is 409, and invalid input is 400.
pub(crate) fn overlay_row_update_error(error: flow_like_types::Error) -> ApiError {
    let rejection = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<lancegraph::OverlayRowUpdateRejected>())
        .map(|rejection| match rejection {
            lancegraph::OverlayRowUpdateRejected::NotFound(message) => {
                ApiError::not_found(message.clone())
            }
            lancegraph::OverlayRowUpdateRejected::NotUnique(message)
            | lancegraph::OverlayRowUpdateRejected::NotApplied(message)
            | lancegraph::OverlayRowUpdateRejected::AppliedToSeveral { message, .. } => {
                ApiError::conflict(message.clone())
            }
        });
    rejection.unwrap_or_else(|| table_input_error(error))
}

pub(crate) fn identity_label(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[utoipa::path(
    patch,
    path = "/apps/{app_id}/graph/{overlay_id}/objects",
    tag = "graph",
    description = "Update property values of one ontology object. The ontology resolves the table and identity; identity and relationship columns cannot be changed and nothing is inserted. Returns the saved object, or the current object when it changed since it was read.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("overlay_id" = String, Path, description = "Overlay ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = UpdateObjectPayload,
    responses(
        (status = 200, description = "The saved object, or the current object when it changed since it was read", body = UpdateOverlayRowResponse),
        (status = 400, description = "A property cannot be edited or a value does not fit its column"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Ontology or object not found"),
        (status = 409, description = "Several objects share this identity, or the object changed while saving. The message says when a concurrent change made the edit reach several rows")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "PATCH /apps/{app_id}/graph/{overlay_id}/objects",
    skip(state, user, scope, payload)
)]
pub async fn update_object(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<UpdateObjectPayload>,
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
            "Connected projects cannot edit ontology objects",
        ));
    }

    let (connection, overlay) =
        super::load_scoped_overlay_for_write(&state, &user, &app_id, &overlay_id, &scope).await?;
    let object_type = payload.object_type.clone();
    let object_id = identity_label(&payload.id);
    let columns = payload.updates.keys().cloned().collect::<Vec<_>>();

    let result = lancegraph::update_overlay_object(&connection, &overlay, payload.into()).await;
    let rows = lancegraph::overlay_rows_written(&result);
    if rows > 0 {
        audit_branch!(
            state,
            user,
            app_id,
            "graph.objects.update",
            "GraphOverlay",
            overlay_id,
            serde_json::json!({
                "object_type": object_type,
                "object_id": object_id,
                "columns": columns,
                "rows": rows,
                "user_scoped": scope.is_user_scoped(),
            })
        );
    }

    Ok(Json(result.map_err(overlay_row_update_error)?.into()))
}
