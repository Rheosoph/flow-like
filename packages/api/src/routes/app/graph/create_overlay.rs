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
use flow_like_catalog_core::DEFAULT_GRAPH_OVERLAY_LIMIT;
use flow_like_storage::databases::graph::lancegraph;
use utoipa::ToSchema;

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct CreateOverlayPayload {
    pub name: String,
    pub description: Option<String>,
    #[schema(value_type = Vec<Object>)]
    pub nodes: Vec<flow_like_catalog_core::NodeLabelMapping>,
    #[schema(value_type = Vec<Object>)]
    pub edges: Vec<flow_like_catalog_core::EdgeLabelMapping>,
    #[serde(default)]
    #[schema(value_type = Vec<Object>)]
    pub object_views: Vec<flow_like_catalog_core::ObjectViewDefinition>,
    #[serde(default)]
    #[schema(value_type = Vec<Object>)]
    pub actions: Vec<flow_like_catalog_core::OntologyActionDefinition>,
    #[serde(default)]
    pub exposed: bool,
    #[serde(default)]
    pub bindings_enabled: bool,
    #[serde(default = "default_limit")]
    pub default_limit: usize,
}

fn default_limit() -> usize {
    DEFAULT_GRAPH_OVERLAY_LIMIT
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/graph",
    tag = "graph",
    description = "Create a new graph overlay.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = CreateOverlayPayload,
    responses(
        (status = 200, description = "Created overlay", body = Object),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "POST /apps/{app_id}/graph", skip(state, user, payload))]
pub async fn create_overlay(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<CreateOverlayPayload>,
) -> Result<Json<flow_like_catalog_core::GraphOverlay>, ApiError> {
    let permission = ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );

    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;

    let now = chrono::Utc::now().to_rfc3339();
    let overlay_id = uuid::Uuid::new_v4().to_string();
    let mut action_owner_sub = None;
    let nodes = payload
        .nodes
        .iter()
        .map(|n| lancegraph::NodeMappingDef {
            id: n.id.clone(),
            api_name: n.api_name.clone(),
            label: n.label.clone(),
            table: n.table.clone(),
            id_column: n.id_column.clone(),
            display_column: n.display_column.clone(),
            property_columns: n
                .property_columns
                .iter()
                .map(|p| lancegraph::PropertyColumnDef {
                    name: p.name.clone(),
                    data_type: p.data_type.clone(),
                    nullable: p.nullable,
                })
                .collect(),
            style: serde_json::to_value(&n.style).unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    let mut actions = payload
        .actions
        .iter()
        .map(|action| lancegraph::OntologyActionDef {
            id: action.id.clone(),
            name: action.name.clone(),
            description: action.description.clone(),
            object_type: action.object_type.clone(),
            board_id: action.board_id.clone(),
            board_version: action.board_version,
            start_node_id: action.start_node_id.clone(),
            // Event IDs are server-managed implementation capabilities.
            event_id: None,
            enabled: action.enabled,
            allow_bulk: action.allow_bulk,
            parameter_schema: action.parameter_schema.clone(),
            exposed: action.exposed,
        })
        .collect::<Vec<_>>();
    super::actions::validate_action_object_types(&actions, |object_type| {
        nodes.iter().any(|object| {
            object.id.as_deref() == Some(object_type)
                || object.api_name.as_deref() == Some(object_type)
                || object.label == object_type
        })
    })?;
    if !actions.is_empty() {
        if scope.is_user_scoped() {
            return Err(ApiError::bad_request(
                "Executable ontology actions must use project scope",
            ));
        }
        if !permission.has_permission(RolePermissions::WriteEvents) {
            return Err(ApiError::FORBIDDEN);
        }
        let sub = permission.sub()?;
        action_owner_sub = Some(sub.clone());
        if let Err(error) = super::actions::materialize_action_events(
            &state,
            &sub,
            &app_id,
            &overlay_id,
            payload.exposed,
            &nodes,
            &mut actions,
        )
        .await
        {
            if let Err(cleanup_error) =
                super::actions::remove_action_events(&state, &sub, &app_id, &overlay_id, &actions)
                    .await
            {
                tracing::error!(%cleanup_error, "Failed to roll back ontology action bindings");
            }
            return Err(error);
        }
    }

    let def = lancegraph::GraphOverlayDef {
        id: overlay_id.clone(),
        name: payload.name.clone(),
        description: payload.description.clone(),
        nodes,
        edges: payload
            .edges
            .iter()
            .map(|e| lancegraph::EdgeMappingDef {
                id: e.id.clone(),
                api_name: e.api_name.clone(),
                label: e.label.clone(),
                table: e.table.clone(),
                src_column: e.src_column.clone(),
                dst_column: e.dst_column.clone(),
                src_label: e.src_label.clone(),
                dst_label: e.dst_label.clone(),
                src_node_column: e.src_node_column.clone(),
                dst_node_column: e.dst_node_column.clone(),
                containment: e.containment,
                dst_ontology: e.dst_ontology.clone(),
                dst_binding_id: e.dst_binding_id.clone(),
                property_columns: e
                    .property_columns
                    .iter()
                    .map(|p| lancegraph::PropertyColumnDef {
                        name: p.name.clone(),
                        data_type: p.data_type.clone(),
                        nullable: p.nullable,
                    })
                    .collect(),
                style: serde_json::to_value(&e.style).unwrap_or_default(),
            })
            .collect(),
        object_views: payload
            .object_views
            .iter()
            .map(|view| lancegraph::ObjectViewDef {
                object_type: view.object_type.clone(),
                title_property: view.title_property.clone(),
                prominent_properties: view.prominent_properties.clone(),
            })
            .collect(),
        actions,
        exposed: payload.exposed,
        bindings_enabled: payload.bindings_enabled,
        default_limit: payload.default_limit,
        created_at: now.clone(),
        updated_at: now.clone(),
    };

    let report = lancegraph::validate_overlay_definition(&connection, &def)
        .await
        .map_err(|error| ApiError::internal(format!("Overlay validation failed: {error}")))?;
    if !report.ok {
        let mut issues = report.issues;
        for mapping in &report.mappings {
            for issue in &mapping.issues {
                issues.push(format!("{} '{}': {}", mapping.kind, mapping.label, issue));
            }
        }
        if let Some(sub) = action_owner_sub.as_ref()
            && let Err(cleanup_error) = super::actions::remove_action_events(
                &state,
                sub,
                &app_id,
                &overlay_id,
                &def.actions,
            )
            .await
        {
            tracing::error!(%cleanup_error, "Failed to roll back ontology action bindings");
        }
        return Err(ApiError::bad_request(format!(
            "The overlay definition is invalid: {}",
            issues.join("; ")
        )));
    }

    if let Err(error) = lancegraph::save_overlay(&connection, &def).await {
        if let Some(sub) = action_owner_sub
            && let Err(cleanup_error) = super::actions::remove_action_events(
                &state,
                &sub,
                &app_id,
                &overlay_id,
                &def.actions,
            )
            .await
        {
            tracing::error!(%cleanup_error, "Failed to roll back ontology action bindings");
        }
        return Err(error.into());
    }
    Ok(Json(super::list_overlays::def_to_overlay(def)))
}
