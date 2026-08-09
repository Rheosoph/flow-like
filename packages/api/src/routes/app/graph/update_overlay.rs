use crate::{
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{ScopeParams, resolve_write_connection},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::databases::graph::lancegraph;
use utoipa::ToSchema;

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct UpdateOverlayPayload {
    pub expected_updated_at: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    #[schema(value_type = Option<Vec<Object>>)]
    pub nodes: Option<Vec<flow_like_catalog_core::NodeLabelMapping>>,
    #[schema(value_type = Option<Vec<Object>>)]
    pub edges: Option<Vec<flow_like_catalog_core::EdgeLabelMapping>>,
    #[schema(value_type = Option<Vec<Object>>)]
    pub object_views: Option<Vec<flow_like_catalog_core::ObjectViewDefinition>>,
    #[schema(value_type = Option<Vec<Object>>)]
    pub actions: Option<Vec<flow_like_catalog_core::OntologyActionDefinition>>,
    pub exposed: Option<bool>,
    pub bindings_enabled: Option<bool>,
    pub default_limit: Option<usize>,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/graph/{overlay_id}",
    tag = "graph",
    description = "Update a graph overlay.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("overlay_id" = String, Path, description = "Overlay ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = UpdateOverlayPayload,
    responses(
        (status = 200, description = "Updated overlay", body = Object),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Overlay not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "PUT /apps/{app_id}/graph/{overlay_id}",
    skip(state, user, scope, payload)
)]
pub async fn update_overlay(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<UpdateOverlayPayload>,
) -> Result<Json<flow_like_catalog_core::GraphOverlay>, ApiError> {
    let permission = ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );

    let connection = resolve_write_connection(&state, &user, &app_id, &scope).await?;
    let mut def = lancegraph::load_overlay(&connection, &overlay_id).await?;
    let previous_def = def.clone();
    let actions_supplied = payload.actions.is_some();
    let mut removed_action_events = None;
    let mut action_rollback = None;
    let mut prepared_action_snapshots = None;

    if let Some(name) = payload.name {
        def.name = name;
    }
    if let Some(description) = payload.description {
        def.description = Some(description);
    }
    if let Some(nodes) = payload.nodes {
        def.nodes = nodes
            .into_iter()
            .map(|n| lancegraph::NodeMappingDef {
                id: n.id,
                api_name: n.api_name,
                label: n.label,
                table: n.table,
                id_column: n.id_column,
                display_column: n.display_column,
                property_columns: n
                    .property_columns
                    .into_iter()
                    .map(|p| lancegraph::PropertyColumnDef {
                        name: p.name,
                        data_type: p.data_type,
                        nullable: p.nullable,
                    })
                    .collect(),
                style: serde_json::to_value(&n.style).unwrap_or_default(),
            })
            .collect();
    }
    if let Some(edges) = payload.edges {
        def.edges = edges
            .into_iter()
            .map(|e| lancegraph::EdgeMappingDef {
                id: e.id,
                api_name: e.api_name,
                label: e.label,
                table: e.table,
                src_column: e.src_column,
                dst_column: e.dst_column,
                src_label: e.src_label,
                dst_label: e.dst_label,
                src_node_column: e.src_node_column,
                dst_node_column: e.dst_node_column,
                containment: e.containment,
                dst_ontology: e.dst_ontology,
                dst_binding_id: e.dst_binding_id,
                property_columns: e
                    .property_columns
                    .into_iter()
                    .map(|p| lancegraph::PropertyColumnDef {
                        name: p.name,
                        data_type: p.data_type,
                        nullable: p.nullable,
                    })
                    .collect(),
                style: serde_json::to_value(&e.style).unwrap_or_default(),
            })
            .collect();
    }
    if let Some(object_views) = payload.object_views {
        def.object_views = object_views
            .into_iter()
            .map(|view| lancegraph::ObjectViewDef {
                object_type: view.object_type,
                title_property: view.title_property,
                prominent_properties: view.prominent_properties,
            })
            .collect();
    }
    if let Some(actions) = payload.actions {
        def.actions = actions
            .into_iter()
            .map(|action| {
                // Event IDs are implementation capabilities. Preserve one only
                // from the same saved ontology/action binding; never trust an
                // ID supplied by the update client.
                let event_id = previous_def
                    .actions
                    .iter()
                    .find(|previous| previous.id == action.id)
                    .and_then(|previous| previous.event_id.clone());
                lancegraph::OntologyActionDef {
                    id: action.id,
                    name: action.name,
                    description: action.description,
                    object_type: action.object_type,
                    board_id: action.board_id,
                    board_version: action.board_version,
                    start_node_id: action.start_node_id,
                    event_id,
                    enabled: action.enabled,
                    allow_bulk: action.allow_bulk,
                    parameter_schema: action.parameter_schema,
                    exposed: action.exposed,
                }
            })
            .collect::<Vec<_>>();
    }
    if let Some(exposed) = payload.exposed {
        def.exposed = exposed;
    }
    if let Some(bindings_enabled) = payload.bindings_enabled {
        def.bindings_enabled = bindings_enabled;
    }
    if let Some(default_limit) = payload.default_limit {
        def.default_limit = default_limit;
    }

    let governed_contract_changed = actions_supplied
        || !lancegraph::ontology_action_contracts_equal(&previous_def, &def).unwrap_or(false);
    if let Some(expected_updated_at) = payload.expected_updated_at.as_deref()
        && expected_updated_at != previous_def.updated_at
    {
        return Err(ApiError::conflict(
            "The ontology has changed. Refresh Data Studio before saving your edits.",
        ));
    }
    if governed_contract_changed && payload.expected_updated_at.is_none() {
        return Err(ApiError::conflict(
            "A current ontology revision is required for governed action changes. Refresh Data Studio and try again.",
        ));
    }
    if scope.is_user_scoped() && !def.actions.is_empty() {
        return Err(ApiError::bad_request(
            "Executable ontology actions must use project scope",
        ));
    }
    if governed_contract_changed && !permission.has_permission(RolePermissions::WriteEvents) {
        return Err(ApiError::FORBIDDEN);
    }
    // Object mapping edits must not silently orphan existing governed actions,
    // even when the client only sends the `nodes` field in this update.
    super::actions::validate_action_object_types(&def.actions, |object_type| {
        def.nodes.iter().any(|object| {
            object.id.as_deref() == Some(object_type)
                || object.api_name.as_deref() == Some(object_type)
                || object.label == object_type
        })
    })?;

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
        return Err(ApiError::bad_request(format!(
            "The overlay definition is invalid: {}",
            issues.join("; ")
        )));
    }

    if governed_contract_changed && !def.actions.is_empty() {
        let sub = permission.sub()?;
        let mut reconciled_actions = def.actions.clone();
        let prepared = match super::actions::materialize_action_events(
            &state,
            &sub,
            &app_id,
            &overlay_id,
            def.exposed,
            &def.nodes,
            &def.edges,
            def.property_projection_mode,
            &mut reconciled_actions,
        )
        .await
        {
            Ok(prepared) => prepared,
            Err(error) => {
                let persisted = lancegraph::load_overlay(&connection, &overlay_id)
                    .await
                    .unwrap_or_else(|_| previous_def.clone());
                if let Err(rollback_error) = super::actions::rollback_action_event_changes(
                    &state,
                    &sub,
                    &app_id,
                    &overlay_id,
                    persisted.exposed,
                    &persisted.nodes,
                    &persisted.edges,
                    persisted.property_projection_mode,
                    &persisted.actions,
                    &reconciled_actions,
                )
                .await
                {
                    tracing::error!(%rollback_error, "Failed to restore ontology action bindings");
                }
                return Err(error);
            }
        };
        def.actions = reconciled_actions.clone();
        prepared_action_snapshots = Some((sub.clone(), prepared));
        action_rollback = Some((sub, reconciled_actions));
    }
    if actions_supplied && !previous_def.actions.is_empty() {
        let removed = previous_def
            .actions
            .iter()
            .filter(|previous| !def.actions.iter().any(|action| action.id == previous.id))
            .cloned()
            .collect::<Vec<_>>();
        removed_action_events = Some((permission.sub()?, removed));
    }
    def.updated_at = chrono::Utc::now().to_rfc3339();

    let save_result =
        lancegraph::save_overlay_if_unchanged(&connection, &def, &previous_def.updated_at).await;
    if !matches!(&save_result, Ok(true)) {
        let persisted = lancegraph::load_overlay(&connection, &overlay_id)
            .await
            .unwrap_or_else(|_| previous_def.clone());
        // Some object stores can report an error after a successful commit.
        // Treat the exact attempted revision as committed; otherwise reconcile
        // the managed event back to the actual winning ontology revision.
        if persisted.updated_at != def.updated_at {
            if let Some((sub, attempted)) = action_rollback
                && let Err(rollback_error) = super::actions::rollback_action_event_changes(
                    &state,
                    &sub,
                    &app_id,
                    &overlay_id,
                    persisted.exposed,
                    &persisted.nodes,
                    &persisted.edges,
                    persisted.property_projection_mode,
                    &persisted.actions,
                    &attempted,
                )
                .await
            {
                tracing::error!(%rollback_error, "Failed to restore ontology action bindings");
            }
            return match save_result {
                Ok(false) => Err(ApiError::conflict(
                    "The ontology changed while it was being saved. Refresh Data Studio and try again.",
                )),
                Err(error) => Err(error.into()),
                Ok(true) => unreachable!(),
            };
        }
    }
    if let Some((sub, prepared)) = prepared_action_snapshots
        && let Err(error) =
            super::actions::commit_action_board_snapshots(&state, &sub, &app_id, &prepared).await
    {
        tracing::warn!(%error, "Could not advance a prepared action board draft");
    }
    if let Some((sub, removed)) = removed_action_events
        && let Err(error) =
            super::actions::remove_action_events(&state, &sub, &app_id, &overlay_id, &removed).await
    {
        tracing::error!(%error, "Failed to clean up removed ontology action bindings");
    }

    Ok(Json(super::list_overlays::def_to_overlay(def)))
}
