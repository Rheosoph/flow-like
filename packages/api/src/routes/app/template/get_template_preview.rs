use crate::{
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::{RolePermissions, has_role_permission},
    routes::app::ensure_app_publicly_visible,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::board::Board;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use utoipa::ToSchema;

/// How many distinct node types to report. Enough to characterise what a
/// template does without shipping its graph.
const MAX_NODE_TYPES: usize = 12;

/// A structural summary of a template — never the template itself.
///
/// This is the only template detail a non-member of the owning app can read. It
/// deliberately carries shape, not content: counts and node type names, no pin
/// values, no variable defaults, no comments, no connections. That is enough for
/// a caller to judge "is this a useful foundation?" and decide whether to fork
/// or acquire the app, without handing out the author's work.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct TemplatePreview {
    pub app_id: String,
    pub template_id: String,
    pub node_count: usize,
    pub layer_count: usize,
    pub variable_count: usize,
    /// Distinct node type names, capped at `MAX_NODE_TYPES` and sorted
    pub node_types: Vec<String>,
    /// True when `node_types` was truncated
    pub node_types_truncated: bool,
    /// Whether the template declares its own entry event node
    pub has_entry_event: bool,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/templates/{template_id}/preview",
    tag = "templates",
    description = "Get a structural summary of a template: node/layer/variable counts and the node types it uses. Readable for any publicly visible app, so a template can be evaluated before forking or joining. Returns shape only, never the template contents.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("template_id" = String, Path, description = "Template ID")
    ),
    responses(
        (status = 200, description = "Structural summary of the template", body = TemplatePreview),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "The app is neither publicly visible nor readable by the caller"),
        (status = 404, description = "Template not found")
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/templates/{template_id}/preview",
    skip(state, user)
)]
pub async fn get_template_preview(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, template_id)): Path<(String, String)>,
) -> Result<Json<TemplatePreview>, ApiError> {
    if !state.platform_config.features.unauthorized_read {
        user.sub()?;
    }

    ensure_preview_readable(&user, &app_id, &state).await?;

    // Master credentials: the caller was authorized by the app's visibility, not
    // by membership, so there is no scoped credential to issue for them.
    let template = state
        .master_template(&app_id, &template_id, &state, None)
        .await
        .map_err(|_| ApiError::NOT_FOUND)?;

    Ok(Json(summarize_template(&app_id, &template_id, &template)))
}

/// A preview is readable when the app is publicly visible, or when the caller is
/// a member who may read templates. Members are covered explicitly so the
/// endpoint keeps working for `Private` and `Prototype` apps, where the
/// visibility check alone would refuse the app's own team.
async fn ensure_preview_readable(
    user: &AppUser,
    app_id: &str,
    state: &AppState,
) -> Result<(), ApiError> {
    if ensure_app_publicly_visible(app_id, state).await.is_ok() {
        return Ok(());
    }

    let permission = user.app_permission(app_id, state).await?;
    if has_role_permission(&permission.permissions, RolePermissions::ReadTemplates) {
        return Ok(());
    }

    Err(ApiError::FORBIDDEN)
}

fn summarize_template(app_id: &str, template_id: &str, template: &Board) -> TemplatePreview {
    let mut node_types: BTreeSet<String> = BTreeSet::new();
    let mut has_entry_event = false;

    for node in template.nodes.values() {
        node_types.insert(node.name.clone());
        if node.start.unwrap_or(false) {
            has_entry_event = true;
        }
    }

    let node_types_truncated = node_types.len() > MAX_NODE_TYPES;

    TemplatePreview {
        app_id: app_id.to_string(),
        template_id: template_id.to_string(),
        node_count: template.nodes.len(),
        layer_count: template.layers.len(),
        variable_count: template.variables.len(),
        node_types: node_types.into_iter().take(MAX_NODE_TYPES).collect(),
        node_types_truncated,
        has_entry_event,
    }
}
