use flow_like::{
    app::App,
    flow::board::{Board, PreparedBoardSnapshot},
    flow::event::{Event, EventExecutionMode, EventExposure},
    flow_like_storage::{
        Path,
        databases::graph::lancegraph::{
            self, EdgeMappingDef, GraphOverlayDef, LanceGraphStore, NodeMappingDef,
        },
        databases::graph::{GraphStore, TraversalDirection},
        lancedb::Connection,
    },
};
use flow_like_catalog::{
    DEFAULT_GRAPH_NEIGHBORS_DIRECTION, DEFAULT_GRAPH_OVERLAY_LIMIT, DEFAULT_GRAPH_QUERY_LIMIT,
    DEFAULT_GRAPH_SAMPLE_SIZE,
};
use flow_like_types::create_id;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::{
    functions::{TauriFunctionError, flow::storage::current_user_sub},
    state::TauriFlowLikeState,
};

pub(crate) async fn graph_connection(
    app_handle: &AppHandle,
    app_id: &str,
    user_scoped: bool,
) -> flow_like_types::Result<Connection> {
    let flow_like_state = TauriFlowLikeState::construct(app_handle).await?;
    let project_db_dir = Path::from("apps")
        .child(app_id)
        .child("storage")
        .child("db");

    let builder = if user_scoped {
        let sub = current_user_sub(app_handle)
            .await
            .map_err(|e| flow_like_types::anyhow!(e.to_string()))?;
        let user_db_dir = Path::from("users")
            .child(sub)
            .child("apps")
            .child(app_id)
            .child("db");
        flow_like_state
            .config
            .read()
            .await
            .callbacks
            .build_user_database
            .clone()
            .ok_or(flow_like_types::anyhow!("No user database builder found"))?(user_db_dir)
    } else {
        flow_like_state
            .config
            .read()
            .await
            .callbacks
            .build_project_database
            .clone()
            .ok_or(flow_like_types::anyhow!("No database builder found"))?(project_db_dir)
    };

    builder
        .execute()
        .await
        .map_err(|e| flow_like_types::anyhow!("Failed to connect to database: {}", e))
}

pub(crate) fn graph_overlay_from_def(
    definition: GraphOverlayDef,
) -> flow_like_types::Result<flow_like_catalog::GraphOverlay> {
    let value = flow_like_types::json::to_value(definition)?;
    Ok(flow_like_types::json::from_value(value)?)
}

fn managed_event_matches(event: &Event, ontology_id: &str, action_id: &str) -> bool {
    if event.event_type != "ontology_action" {
        return false;
    }
    flow_like_types::json::from_slice::<flow_like_types::Value>(&event.config)
        .ok()
        .is_some_and(|config| {
            config
                .get("managed_by")
                .and_then(flow_like_types::Value::as_str)
                == Some("ontology_action")
                && config
                    .get("ontology_id")
                    .and_then(flow_like_types::Value::as_str)
                    == Some(ontology_id)
                && config
                    .get("action_id")
                    .and_then(flow_like_types::Value::as_str)
                    == Some(action_id)
        })
}

fn managed_event_binding_is_current(
    event: &Event,
    ontology_id: &str,
    ontology_exposed: bool,
    objects: &[NodeMappingDef],
    edges: &[EdgeMappingDef],
    projection_mode: lancegraph::PropertyProjectionMode,
    action: &lancegraph::OntologyActionDef,
) -> bool {
    let Ok(projection) = lancegraph::governed_object_projection_from_event_config(&event.config)
    else {
        return false;
    };
    let Ok(object) = lancegraph::validate_governed_object_projection_for_mappings(
        objects,
        edges,
        projection_mode,
        action,
        &projection,
    ) else {
        return false;
    };
    let Ok(contract_hash) = lancegraph::ontology_action_contract_hash(
        ontology_id,
        ontology_exposed,
        action,
        object,
        &projection,
    ) else {
        return false;
    };
    let saved_contract_hash =
        flow_like_types::json::from_slice::<flow_like_types::Value>(&event.config)
            .ok()
            .and_then(|config| {
                config
                    .get("contract_hash")
                    .and_then(flow_like_types::Value::as_str)
                    .map(str::to_owned)
            });
    managed_event_matches(event, ontology_id, &action.id)
        && saved_contract_hash.as_deref() == Some(contract_hash.as_str())
        && event.name == action.name
        && event.description == action.description.clone().unwrap_or_default()
        && event.board_id == action.board_id
        && event.board_version
            == action
                .board_version
                .map(|version| (version[0], version[1], version[2]))
        && action.start_node_id.as_deref() == Some(event.node_id.as_str())
        && event.active == action.enabled
        && event.exposure == EventExposure::Internal
        && event.route.is_none()
        && event.variables.is_empty()
        && event.canary.is_none()
        && event.priority == 0
        && event.default_page_id.is_none()
        && !event.is_default
        && event.correlation_mappings.is_none()
}

fn action_object<'a>(
    objects: &'a [NodeMappingDef],
    action: &lancegraph::OntologyActionDef,
) -> Option<&'a NodeMappingDef> {
    objects.iter().find(|object| {
        object.id.as_deref() == Some(action.object_type.as_str())
            || object.api_name.as_deref() == Some(action.object_type.as_str())
            || object.label == action.object_type
    })
}

fn validated_action_parameter_schema(
    board: &Board,
    start_node_id: &str,
) -> flow_like_types::Result<Option<flow_like_types::Value>> {
    let schema = board.action_parameter_schema(start_node_id)?;
    if let Some(schema) = schema.as_ref() {
        flow_like_catalog::ontology_action_parameter_validator(schema).map_err(|error| {
            flow_like_types::anyhow!(
                "The action's implementation has an invalid parameter schema: {error}"
            )
        })?;
    }
    Ok(schema)
}

/// Reads the authoritative parameter schema from the pinned board version's
/// start node. Loading the pinned snapshot (rather than the working board)
/// guarantees the schema matches the exact implementation that executes, even
/// when the action pins an older published version (mirror of the API path).
async fn derive_action_parameter_schema(
    app: &App,
    action: &lancegraph::OntologyActionDef,
    pinned: (u32, u32, u32),
) -> flow_like_types::Result<Option<flow_like_types::Value>> {
    let Some(start_node_id) = action
        .start_node_id
        .as_deref()
        .map(str::trim)
        .filter(|node_id| !node_id.is_empty())
    else {
        return Err(flow_like_types::anyhow!(
            "The ontology action has no start node"
        ));
    };
    let board = app
        .open_board_authoritative(action.board_id.clone(), Some(pinned))
        .await
        .map_err(|error| {
            flow_like_types::anyhow!(
                "Could not load the action's pinned board version to derive its parameter schema: {error}"
            )
        })?;
    let guard = board.lock().await;
    validated_action_parameter_schema(&guard, start_node_id)
}

/// Ensure a governed action pins an immutable board snapshot. If the working
/// board has changed after its current version was first published, preserve
/// the old snapshot and publish the draft at a fresh patch version.
async fn ensure_action_board_published_with_mode(
    app: &App,
    action: &mut lancegraph::OntologyActionDef,
    publish_draft: bool,
) -> flow_like_types::Result<Option<PreparedBoardSnapshot>> {
    if action.board_id.trim().is_empty() {
        return Ok(None);
    }
    let board = app
        .open_board_authoritative(action.board_id.clone(), None)
        .await
        .map_err(|_| {
            flow_like_types::anyhow!(
                "The action's implementation board '{}' could not be opened",
                action.board_id
            )
        })?;
    let (current, existing) = {
        let guard = board.lock().await;
        (guard.version, guard.get_versions(None).await?)
    };
    let mut pinned = match action.board_version {
        Some([major, minor, patch]) => (major, minor, patch),
        None if !publish_draft => {
            return Err(flow_like_types::anyhow!(
                "The persisted ontology action does not pin a board version"
            ));
        }
        None => {
            action.board_version = Some([current.0, current.1, current.2]);
            current
        }
    };
    let may_publish_current_draft = publish_draft
        && (pinned == current
            || (existing.contains(&pinned)
                && pinned.0 == current.0
                && pinned.1 == current.1
                && pinned.2 > current.2));
    if may_publish_current_draft {
        let start_node_id = action
            .start_node_id
            .as_deref()
            .map(str::trim)
            .filter(|node_id| !node_id.is_empty())
            .ok_or_else(|| flow_like_types::anyhow!("The ontology action has no start node"))?;
        let guard = board.lock().await;
        validated_action_parameter_schema(&guard, start_node_id)?;
    }
    let mut prepared = None;
    if pinned == current {
        let guard = board.lock().await;
        if existing.contains(&pinned) {
            if publish_draft && !guard.snapshot_matches_current(pinned, None).await? {
                let snapshot = guard.prepare_snapshot_at_fresh_patch_version(None).await?;
                pinned = snapshot.version();
                action.board_version = Some([pinned.0, pinned.1, pinned.2]);
                prepared = Some(snapshot);
            }
        } else {
            if !publish_draft {
                return Err(flow_like_types::anyhow!(
                    "The persisted action board version {}.{}.{} is missing",
                    pinned.0,
                    pinned.1,
                    pinned.2
                ));
            }
            if guard
                .snapshot_version_slot_is_compatible(pinned, None)
                .await?
            {
                guard.snapshot_at_version(pinned, None).await?;
            } else {
                let snapshot = guard.prepare_snapshot_at_fresh_patch_version(None).await?;
                pinned = snapshot.version();
                action.board_version = Some([pinned.0, pinned.1, pinned.2]);
                prepared = Some(snapshot);
            }
        }
    } else if !existing.contains(&pinned) {
        return Err(flow_like_types::anyhow!(
            "The action's board version {}.{}.{} no longer exists. Re-select the board in Data Studio.",
            pinned.0,
            pinned.1,
            pinned.2
        ));
    } else if publish_draft
        && pinned.0 == current.0
        && pinned.1 == current.1
        && pinned.2 > current.2
    {
        let guard = board.lock().await;
        if guard.snapshot_matches_current(pinned, None).await? {
            prepared = Some(guard.prepared_snapshot_at_version(pinned, None).await?);
        } else {
            let snapshot = guard.prepare_snapshot_at_fresh_patch_version(None).await?;
            pinned = snapshot.version();
            action.board_version = Some([pinned.0, pinned.1, pinned.2]);
            prepared = Some(snapshot);
        }
    }
    action.parameter_schema = derive_action_parameter_schema(app, action, pinned).await?;
    Ok(prepared)
}

fn validate_action_object_types(
    actions: &[lancegraph::OntologyActionDef],
    objects: &[NodeMappingDef],
) -> flow_like_types::Result<()> {
    for action in actions {
        let exists = objects.iter().any(|object| {
            object.id.as_deref() == Some(action.object_type.as_str())
                || object.api_name.as_deref() == Some(action.object_type.as_str())
                || object.label == action.object_type
        });
        if !exists {
            return Err(flow_like_types::anyhow!(
                "Ontology action '{}' references unknown object type '{}'",
                action.id,
                action.object_type
            ));
        }
        if let Some(schema) = &action.parameter_schema {
            flow_like_catalog::ontology_action_parameter_validator(schema).map_err(|error| {
                flow_like_types::anyhow!(
                    "Ontology action '{}' has an invalid parameter schema: {}",
                    action.id,
                    error
                )
            })?;
        }
    }
    Ok(())
}

async fn validate_overlay_for_save(
    connection: &Connection,
    overlay: &GraphOverlayDef,
) -> Result<(), TauriFunctionError> {
    let report = lancegraph::validate_overlay_definition(connection, overlay)
        .await
        .map_err(|error| TauriFunctionError::new(&format!("Overlay validation failed: {error}")))?;
    if report.ok {
        return Ok(());
    }

    let mut issues = report.issues;
    for mapping in report.mappings {
        for issue in mapping.issues {
            issues.push(format!("{} '{}': {}", mapping.kind, mapping.label, issue));
        }
    }
    Err(TauriFunctionError::new(&format!(
        "The overlay definition is invalid: {}",
        issues.join("; ")
    )))
}

async fn materialize_action_events(
    app_handle: &AppHandle,
    app_id: &str,
    ontology_id: &str,
    ontology_exposed: bool,
    objects: &[NodeMappingDef],
    edges: &[EdgeMappingDef],
    projection_mode: lancegraph::PropertyProjectionMode,
    actions: &mut [lancegraph::OntologyActionDef],
) -> flow_like_types::Result<Vec<PreparedBoardSnapshot>> {
    materialize_action_events_with_mode(
        app_handle,
        app_id,
        ontology_id,
        ontology_exposed,
        objects,
        edges,
        projection_mode,
        actions,
        true,
    )
    .await
}

async fn materialize_action_events_with_mode(
    app_handle: &AppHandle,
    app_id: &str,
    ontology_id: &str,
    ontology_exposed: bool,
    objects: &[NodeMappingDef],
    edges: &[EdgeMappingDef],
    projection_mode: lancegraph::PropertyProjectionMode,
    actions: &mut [lancegraph::OntologyActionDef],
    publish_drafts: bool,
) -> flow_like_types::Result<Vec<PreparedBoardSnapshot>> {
    if actions.is_empty() {
        return Ok(Vec::new());
    }

    let mut action_ids = std::collections::HashSet::with_capacity(actions.len());
    for action in actions.iter() {
        if action.id.trim().is_empty() || !action_ids.insert(action.id.clone()) {
            return Err(flow_like_types::anyhow!(
                "Ontology action IDs must be non-empty and unique"
            ));
        }
        if action.name.trim().is_empty() {
            return Err(flow_like_types::anyhow!(
                "Ontology actions must have a name"
            ));
        }
        if action.board_id.trim().is_empty() {
            return Err(flow_like_types::anyhow!(
                "The ontology action has no implementation board"
            ));
        }
        if action
            .start_node_id
            .as_deref()
            .is_none_or(|node_id| node_id.trim().is_empty())
        {
            return Err(flow_like_types::anyhow!(
                "The ontology action has no start node"
            ));
        }
    }

    let flow_like_state = TauriFlowLikeState::construct(app_handle)
        .await
        .map_err(|error| flow_like_types::anyhow!(error.to_string()))?;
    let mut app = App::load(app_id.to_string(), flow_like_state).await?;
    let connection = graph_connection(app_handle, app_id, false).await?;

    let mut app_changed = false;
    let mut republished_versions: std::collections::HashMap<(String, [u32; 3]), [u32; 3]> =
        std::collections::HashMap::new();
    let mut prepared_snapshots = Vec::new();
    for action in actions {
        if let Some(old_version) = action.board_version
            && let Some(new_version) =
                republished_versions.get(&(action.board_id.clone(), old_version))
        {
            action.board_version = Some(*new_version);
        }
        let requested_board_version = action.board_version;
        let start_node_id = action
            .start_node_id
            .clone()
            .filter(|node_id| !node_id.trim().is_empty())
            .ok_or_else(|| flow_like_types::anyhow!("The ontology action has no start node"))?;
        if action.board_id.trim().is_empty() {
            return Err(flow_like_types::anyhow!(
                "The ontology action has no implementation board"
            ));
        }
        let requested_event_id = action
            .event_id
            .clone()
            .filter(|event_id| !event_id.trim().is_empty());
        let existing = match requested_event_id.as_deref() {
            Some(event_id) => app.get_event(event_id, None).await.ok(),
            None => None,
        };
        // Resolve the governed data surface before creating an immutable board
        // snapshot. A broken mapping must not leave snapshot fragments behind.
        let projection = lancegraph::resolve_governed_object_projection_for_mappings(
            &connection,
            objects,
            edges,
            projection_mode,
            action,
        )
        .await?;
        // Board content can change independently of ontology metadata, so
        // reconcile the immutable implementation pin before the fast path.
        if let Some(prepared) =
            ensure_action_board_published_with_mode(&app, action, publish_drafts).await?
            && !prepared_snapshots.contains(&prepared)
        {
            prepared_snapshots.push(prepared);
        }
        if let (Some(old_version), Some(new_version)) =
            (requested_board_version, action.board_version)
            && old_version != new_version
        {
            republished_versions.insert((action.board_id.clone(), old_version), new_version);
        }
        let board_version = action.board_version.ok_or_else(|| {
            flow_like_types::anyhow!("The ontology action must pin an exact board version")
        })?;
        if let Some(event) = existing.as_ref()
            && managed_event_binding_is_current(
                event,
                ontology_id,
                ontology_exposed,
                objects,
                edges,
                projection_mode,
                action,
            )
        {
            action.event_id = Some(event.id.clone());
            if !app.events.contains(&event.id) {
                app.events.push(event.id.clone());
                app_changed = true;
            }
            continue;
        }
        let event_id = requested_event_id
            .filter(|_| {
                existing
                    .as_ref()
                    .is_some_and(|event| managed_event_matches(event, ontology_id, &action.id))
            })
            .unwrap_or_else(create_id);
        let now = std::time::SystemTime::now();
        let object = action_object(objects, action).ok_or_else(|| {
            flow_like_types::anyhow!(
                "Ontology action '{}' references unknown object type '{}'",
                action.id,
                action.object_type
            )
        })?;
        let contract_hash = lancegraph::ontology_action_contract_hash(
            ontology_id,
            ontology_exposed,
            action,
            object,
            &projection,
        )?;
        let mut event = Event {
            id: event_id,
            name: action.name.clone(),
            description: action.description.clone().unwrap_or_default(),
            board_id: action.board_id.clone(),
            board_version: Some((board_version[0], board_version[1], board_version[2])),
            node_id: start_node_id,
            variables: std::collections::HashMap::new(),
            config: flow_like_types::json::to_vec(&flow_like_types::json::json!({
                "managed_by": "ontology_action",
                "ontology_id": ontology_id,
                "action_id": action.id,
                "contract_hash": contract_hash,
                "object_projection": projection,
            }))
            .unwrap_or_default(),
            active: action.enabled,
            canary: None,
            priority: 0,
            event_type: "ontology_action".to_string(),
            notes: None,
            event_version: (0, 0, 0),
            created_at: now,
            updated_at: now,
            default_page_id: None,
            inputs: Vec::new(),
            route: None,
            is_default: false,
            execution_mode: EventExecutionMode::Local,
            exposure: EventExposure::Internal,
            correlation_mappings: None,
        };
        let event = event.upsert(&app, None, true).await?;
        if !app.events.contains(&event.id) {
            app.events.push(event.id.clone());
        }
        action.event_id = Some(event.id);
        app_changed = true;
    }

    if app_changed {
        app.updated_at = std::time::SystemTime::now();
        app.save().await?;
    }
    Ok(prepared_snapshots)
}

async fn commit_action_board_snapshots(
    app_handle: &AppHandle,
    app_id: &str,
    prepared_snapshots: &[PreparedBoardSnapshot],
) -> flow_like_types::Result<()> {
    if prepared_snapshots.is_empty() {
        return Ok(());
    }
    let flow_like_state = TauriFlowLikeState::construct(app_handle)
        .await
        .map_err(|error| flow_like_types::anyhow!(error.to_string()))?;
    let app = App::load(app_id.to_string(), flow_like_state).await?;
    for prepared in prepared_snapshots {
        let board = app
            .open_board_authoritative(prepared.board_id().to_string(), None)
            .await?;
        let committed = board
            .lock()
            .await
            .commit_prepared_snapshot(prepared, None)
            .await?;
        if !committed {
            tracing::info!(
                board_id = prepared.board_id(),
                version = ?prepared.version(),
                "Left a concurrently changed board draft at its current version"
            );
        }
    }
    Ok(())
}

async fn remove_action_events(
    app_handle: &AppHandle,
    app_id: &str,
    ontology_id: &str,
    removed_actions: &[lancegraph::OntologyActionDef],
) -> flow_like_types::Result<()> {
    if removed_actions.is_empty() {
        return Ok(());
    }
    let flow_like_state = TauriFlowLikeState::construct(app_handle)
        .await
        .map_err(|error| flow_like_types::anyhow!(error.to_string()))?;
    let mut app = App::load(app_id.to_string(), flow_like_state).await?;
    let mut removed_event_ids = Vec::new();
    for action in removed_actions {
        let Some(event_id) = action.event_id.as_deref() else {
            continue;
        };
        let Ok(event) = app.get_event(event_id, None).await else {
            continue;
        };
        if managed_event_matches(&event, ontology_id, &action.id) {
            event.delete(&app).await?;
            app.events.retain(|saved_id| saved_id != event_id);
            removed_event_ids.push(event_id.to_string());
        }
    }
    if !removed_event_ids.is_empty() {
        app.updated_at = std::time::SystemTime::now();
        app.save().await?;
    }
    Ok(())
}

async fn rollback_action_event_changes(
    app_handle: &AppHandle,
    app_id: &str,
    ontology_id: &str,
    ontology_exposed: bool,
    objects: &[NodeMappingDef],
    edges: &[EdgeMappingDef],
    projection_mode: lancegraph::PropertyProjectionMode,
    previous_actions: &[lancegraph::OntologyActionDef],
    attempted_actions: &[lancegraph::OntologyActionDef],
) -> flow_like_types::Result<()> {
    let newly_created = attempted_actions
        .iter()
        .filter(|attempted| {
            attempted.event_id.is_some()
                && previous_actions
                    .iter()
                    .find(|previous| previous.id == attempted.id)
                    .and_then(|previous| previous.event_id.as_ref())
                    != attempted.event_id.as_ref()
        })
        .cloned()
        .collect::<Vec<_>>();
    let removal_error = remove_action_events(app_handle, app_id, ontology_id, &newly_created)
        .await
        .err();
    if let Some(error) = removal_error.as_ref() {
        tracing::error!(%error, "Failed to remove newly materialized ontology action bindings");
    }

    let mut overwritten = previous_actions
        .iter()
        .filter(|previous| {
            previous.event_id.is_some()
                && attempted_actions.iter().any(|attempted| {
                    attempted.id == previous.id && attempted.event_id == previous.event_id
                })
        })
        .cloned()
        .collect::<Vec<_>>();
    // Restore exactly what the persisted ontology pins. Publishing the current
    // draft during compensation would make the rolled-back event disagree
    // with the ontology revision that actually won.
    let restore_result = materialize_action_events_with_mode(
        app_handle,
        app_id,
        ontology_id,
        ontology_exposed,
        objects,
        edges,
        projection_mode,
        &mut overwritten,
        false,
    )
    .await
    .map(|_| ());
    match (removal_error, restore_result) {
        (Some(error), _) => Err(error),
        (None, result) => result,
    }
}

#[tauri::command(async)]
pub async fn graph_list_overlays(
    app_handle: AppHandle,
    app_id: String,
    user_scoped: Option<bool>,
) -> Result<Vec<GraphOverlayDef>, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlays = lancegraph::list_overlays(&conn).await?;
    Ok(overlays)
}

#[tauri::command(async)]
pub async fn graph_list_imports(
    app_handle: AppHandle,
    app_id: String,
) -> Result<Vec<lancegraph::RemoteOntologyImportDef>, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, false).await?;
    let imports = lancegraph::list_ontology_imports(&conn).await?;
    Ok(imports)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateOverlayPayload {
    pub name: String,
    pub description: Option<String>,
    pub nodes: Vec<NodeMappingDef>,
    pub edges: Vec<EdgeMappingDef>,
    #[serde(default, alias = "object_views")]
    pub object_views: Vec<lancegraph::ObjectViewDef>,
    #[serde(default)]
    pub actions: Vec<lancegraph::OntologyActionDef>,
    #[serde(default)]
    pub exposed: bool,
    #[serde(default, alias = "bindings_enabled")]
    pub bindings_enabled: bool,
    #[serde(alias = "default_limit")]
    pub default_limit: Option<usize>,
}

#[tauri::command(async)]
pub async fn graph_create_overlay(
    app_handle: AppHandle,
    app_id: String,
    payload: CreateOverlayPayload,
    user_scoped: Option<bool>,
) -> Result<GraphOverlayDef, TauriFunctionError> {
    let user_scoped = user_scoped.unwrap_or(false);
    let conn = graph_connection(&app_handle, &app_id, user_scoped).await?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut overlay = GraphOverlayDef {
        id: create_id(),
        name: payload.name,
        description: payload.description,
        nodes: payload.nodes,
        edges: payload.edges,
        object_views: payload.object_views,
        actions: payload.actions,
        exposed: payload.exposed,
        bindings_enabled: payload.bindings_enabled,
        property_projection_mode: lancegraph::PropertyProjectionMode::Dynamic,
        default_limit: payload.default_limit.unwrap_or(DEFAULT_GRAPH_OVERLAY_LIMIT),
        created_at: now.clone(),
        updated_at: now,
    };
    if user_scoped && !overlay.actions.is_empty() {
        return Err(TauriFunctionError::new(
            "Executable ontology actions must use project scope",
        ));
    }
    for action in &mut overlay.actions {
        action.event_id = None;
    }
    validate_action_object_types(&overlay.actions, &overlay.nodes)?;
    validate_overlay_for_save(&conn, &overlay).await?;
    let prepared_snapshots = match materialize_action_events(
        &app_handle,
        &app_id,
        &overlay.id,
        overlay.exposed,
        &overlay.nodes,
        &overlay.edges,
        overlay.property_projection_mode,
        &mut overlay.actions,
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(error) => {
            if let Err(cleanup_error) =
                remove_action_events(&app_handle, &app_id, &overlay.id, &overlay.actions).await
            {
                tracing::error!(%cleanup_error, "Failed to roll back ontology action bindings");
            }
            return Err(error.into());
        }
    };
    if let Err(error) = lancegraph::save_overlay(&conn, &overlay).await {
        if let Err(cleanup_error) =
            remove_action_events(&app_handle, &app_id, &overlay.id, &overlay.actions).await
        {
            tracing::error!(%cleanup_error, "Failed to roll back ontology action bindings");
        }
        return Err(error.into());
    }
    if let Err(error) =
        commit_action_board_snapshots(&app_handle, &app_id, &prepared_snapshots).await
    {
        tracing::warn!(%error, "Could not advance a prepared action board draft");
    }
    Ok(overlay)
}

#[tauri::command(async)]
pub async fn graph_get_overlay(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    user_scoped: Option<bool>,
) -> Result<GraphOverlayDef, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    Ok(overlay)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OntologyObjectRefPayload {
    #[serde(alias = "object_type")]
    pub object_type: String,
    pub id: flow_like_types::Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareOntologyActionPayload {
    #[serde(alias = "object_refs")]
    pub object_refs: Vec<OntologyObjectRefPayload>,
    #[serde(default = "empty_action_parameters")]
    pub parameters: flow_like_types::Value,
    #[serde(default, alias = "idempotency_key")]
    pub idempotency_key: Option<String>,
}

fn empty_action_parameters() -> flow_like_types::Value {
    flow_like_types::json::json!({})
}

#[derive(Debug, Clone, Serialize)]
pub struct PreparedOntologyAction {
    pub event_id: String,
    pub payload: flow_like_types::Value,
}

/// Resolves an offline action through local ontology metadata, hydrates the
/// canonical objects, and returns the managed event plus run payload. The UI
/// executes that event through EventState so exact-version, OAuth, RPA, WASM,
/// runtime-variable, and intercom behavior stays on the established path.
#[tauri::command(async)]
pub async fn graph_prepare_ontology_action(
    app_handle: AppHandle,
    app_id: String,
    ontology_id: String,
    action_id: String,
    payload: PrepareOntologyActionPayload,
) -> Result<PreparedOntologyAction, TauriFunctionError> {
    let connection = graph_connection(&app_handle, &app_id, false).await?;
    let mut ontology = lancegraph::load_overlay(&connection, &ontology_id).await?;
    let previous_ontology = ontology.clone();
    let action_index = ontology
        .actions
        .iter()
        .position(|action| action.id == action_id && action.enabled)
        .ok_or_else(|| TauriFunctionError::new("Enabled ontology action not found"))?;
    let action = ontology.actions[action_index].clone();

    if payload.object_refs.is_empty() {
        return Err(TauriFunctionError::new(
            "At least one ontology object is required",
        ));
    }
    if (!action.allow_bulk && payload.object_refs.len() != 1) || payload.object_refs.len() > 100 {
        return Err(TauriFunctionError::new(if action.allow_bulk {
            "Bulk actions accept at most 100 objects"
        } else {
            "This action accepts exactly one object"
        }));
    }
    if !payload.parameters.is_object() {
        return Err(TauriFunctionError::new(
            "Action parameters must be an object",
        ));
    }
    if let Some(key) = payload.idempotency_key.as_deref()
        && (key.is_empty() || key.len() > 200)
    {
        return Err(TauriFunctionError::new(
            "Idempotency keys must contain between 1 and 200 characters",
        ));
    }
    let ids = payload
        .object_refs
        .iter()
        .map(|reference| {
            if reference.object_type != action.object_type {
                return Err(TauriFunctionError::new(&format!(
                    "Action '{}' requires object type '{}'",
                    action.id, action.object_type
                )));
            }
            Ok(reference.id.clone())
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Repair missing or tampered managed events lazily so older local
    // ontologies remain executable without trusting a stale event ID.
    let event_is_valid = if let Some(event_id) = ontology.actions[action_index].event_id.as_deref()
    {
        let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;
        match App::load(app_id.clone(), flow_like_state).await {
            Ok(app) => app.get_event(event_id, None).await.is_ok_and(|event| {
                managed_event_binding_is_current(
                    &event,
                    &ontology_id,
                    ontology.exposed,
                    &ontology.nodes,
                    &ontology.edges,
                    ontology.property_projection_mode,
                    &action,
                )
            }),
            Err(_) => false,
        }
    } else {
        false
    };
    if !event_is_valid {
        let prepared_snapshots = match materialize_action_events(
            &app_handle,
            &app_id,
            &ontology_id,
            ontology.exposed,
            &ontology.nodes,
            &ontology.edges,
            ontology.property_projection_mode,
            std::slice::from_mut(&mut ontology.actions[action_index]),
        )
        .await
        {
            Ok(prepared) => prepared,
            Err(error) => {
                let persisted = lancegraph::load_overlay(&connection, &ontology_id)
                    .await
                    .unwrap_or_else(|_| previous_ontology.clone());
                if let Err(rollback_error) = rollback_action_event_changes(
                    &app_handle,
                    &app_id,
                    &ontology_id,
                    persisted.exposed,
                    &persisted.nodes,
                    &persisted.edges,
                    persisted.property_projection_mode,
                    &persisted.actions,
                    std::slice::from_ref(&ontology.actions[action_index]),
                )
                .await
                {
                    tracing::error!(%rollback_error, "Failed to roll back partial ontology action repair");
                }
                return Err(error.into());
            }
        };
        ontology.updated_at = chrono::Utc::now().to_rfc3339();
        let save_result = lancegraph::save_overlay_if_unchanged(
            &connection,
            &ontology,
            &previous_ontology.updated_at,
        )
        .await;
        if !matches!(&save_result, Ok(true)) {
            let persisted = lancegraph::load_overlay(&connection, &ontology_id)
                .await
                .unwrap_or(previous_ontology);
            if persisted.updated_at != ontology.updated_at {
                if let Err(rollback_error) = rollback_action_event_changes(
                    &app_handle,
                    &app_id,
                    &ontology_id,
                    persisted.exposed,
                    &persisted.nodes,
                    &persisted.edges,
                    persisted.property_projection_mode,
                    &persisted.actions,
                    std::slice::from_ref(&ontology.actions[action_index]),
                )
                .await
                {
                    tracing::error!(%rollback_error, "Failed to roll back ontology action repair");
                }
                return match save_result {
                    Ok(false) => Err(TauriFunctionError::new(
                        "The ontology changed while its action binding was repaired. Try again.",
                    )),
                    Err(error) => Err(error.into()),
                    Ok(true) => unreachable!(),
                };
            }
        }
        if let Err(error) =
            commit_action_board_snapshots(&app_handle, &app_id, &prepared_snapshots).await
        {
            tracing::warn!(%error, "Could not advance a repaired action board draft");
        }
    }
    let event_id = ontology.actions[action_index]
        .event_id
        .clone()
        .ok_or_else(|| TauriFunctionError::new("Ontology action event is unavailable"))?;
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;
    let app = App::load(app_id.clone(), flow_like_state).await?;
    let event = app
        .get_event(&event_id, None)
        .await
        .map_err(|_| TauriFunctionError::new("Ontology action event is unavailable"))?;
    let bound_action = &ontology.actions[action_index];
    if !managed_event_binding_is_current(
        &event,
        &ontology_id,
        ontology.exposed,
        &ontology.nodes,
        &ontology.edges,
        ontology.property_projection_mode,
        bound_action,
    ) {
        return Err(TauriFunctionError::new(
            "The ontology action binding no longer matches its governed implementation",
        ));
    }
    // Lazy repair can publish a changed implementation board and derive a new
    // parameter schema. Validate against that repaired, persisted contract —
    // not the stale action clone captured before materialization.
    if let Some(schema) = &bound_action.parameter_schema {
        flow_like_catalog::validate_ontology_action_parameters(schema, &payload.parameters)
            .map_err(|error| {
                TauriFunctionError::new(&format!(
                    "Action parameters do not match the schema: {error}"
                ))
            })?;
    }
    let projection = lancegraph::governed_object_projection_from_event_config(&event.config)?;
    let objects = lancegraph::load_overlay_objects_with_projection(
        &connection,
        &ontology,
        bound_action,
        &projection,
        &ids,
    )
    .await?;
    let object_ids = ids
        .iter()
        .map(|id| match id {
            flow_like_types::Value::String(value) => value.clone(),
            _ => id.to_string(),
        })
        .collect::<Vec<_>>();
    let run_payload = flow_like_types::json::json!({
        "_ontology": {
            "ontology_id": ontology_id,
            "action_id": action.id,
            "object_type": action.object_type,
            "object_ids": object_ids,
            "idempotency_key": payload.idempotency_key,
        },
        "object": objects.first().cloned().unwrap_or(flow_like_types::Value::Null),
        "objects": objects,
        "parameters": payload.parameters,
    });

    Ok(PreparedOntologyAction {
        event_id,
        payload: run_payload,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOverlayPayload {
    #[serde(alias = "expected_updated_at")]
    pub expected_updated_at: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub nodes: Option<Vec<NodeMappingDef>>,
    pub edges: Option<Vec<EdgeMappingDef>>,
    #[serde(alias = "object_views")]
    pub object_views: Option<Vec<lancegraph::ObjectViewDef>>,
    pub actions: Option<Vec<lancegraph::OntologyActionDef>>,
    pub exposed: Option<bool>,
    #[serde(alias = "bindings_enabled")]
    pub bindings_enabled: Option<bool>,
    #[serde(alias = "default_limit")]
    pub default_limit: Option<usize>,
}

#[tauri::command(async)]
pub async fn graph_update_overlay(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    payload: UpdateOverlayPayload,
    user_scoped: Option<bool>,
) -> Result<GraphOverlayDef, TauriFunctionError> {
    let user_scoped = user_scoped.unwrap_or(false);
    let conn = graph_connection(&app_handle, &app_id, user_scoped).await?;
    let mut overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let previous_overlay = overlay.clone();
    let actions_supplied = payload.actions.is_some();
    let mut removed_action_events = Vec::new();
    let mut action_rollback = None;
    let mut prepared_action_snapshots = Vec::new();

    if let Some(name) = payload.name {
        overlay.name = name;
    }
    if let Some(desc) = payload.description {
        overlay.description = Some(desc);
    }
    if let Some(nodes) = payload.nodes {
        overlay.nodes = nodes;
    }
    if let Some(edges) = payload.edges {
        overlay.edges = edges;
    }
    if let Some(object_views) = payload.object_views {
        overlay.object_views = object_views;
    }
    if let Some(mut actions) = payload.actions {
        for action in &mut actions {
            action.event_id = previous_overlay
                .actions
                .iter()
                .find(|previous| previous.id == action.id)
                .and_then(|previous| previous.event_id.clone());
        }
        overlay.actions = actions;
    }
    if let Some(exposed) = payload.exposed {
        overlay.exposed = exposed;
    }
    if let Some(bindings_enabled) = payload.bindings_enabled {
        overlay.bindings_enabled = bindings_enabled;
    }
    if let Some(limit) = payload.default_limit {
        overlay.default_limit = limit;
    }

    let governed_contract_changed = actions_supplied
        || !lancegraph::ontology_action_contracts_equal(&previous_overlay, &overlay)
            .unwrap_or(false);
    if let Some(expected_updated_at) = payload.expected_updated_at.as_deref()
        && expected_updated_at != previous_overlay.updated_at
    {
        return Err(TauriFunctionError::new(
            "The ontology has changed. Refresh Data Studio before saving your edits.",
        ));
    }
    if governed_contract_changed && payload.expected_updated_at.is_none() {
        return Err(TauriFunctionError::new(
            "A current ontology revision is required for governed action changes. Refresh Data Studio and try again.",
        ));
    }
    if user_scoped && !overlay.actions.is_empty() {
        return Err(TauriFunctionError::new(
            "Executable ontology actions must use project scope",
        ));
    }
    // Mapping-only edits cannot leave an existing governed action pointing at
    // an object type that no longer exists in the ontology.
    validate_action_object_types(&overlay.actions, &overlay.nodes)?;
    validate_overlay_for_save(&conn, &overlay).await?;
    if governed_contract_changed && !overlay.actions.is_empty() {
        let mut reconciled_actions = overlay.actions.clone();
        let prepared = match materialize_action_events(
            &app_handle,
            &app_id,
            &overlay.id,
            overlay.exposed,
            &overlay.nodes,
            &overlay.edges,
            overlay.property_projection_mode,
            &mut reconciled_actions,
        )
        .await
        {
            Ok(prepared) => prepared,
            Err(error) => {
                let persisted = lancegraph::load_overlay(&conn, &overlay_id)
                    .await
                    .unwrap_or_else(|_| previous_overlay.clone());
                if let Err(rollback_error) = rollback_action_event_changes(
                    &app_handle,
                    &app_id,
                    &overlay.id,
                    persisted.exposed,
                    &persisted.nodes,
                    &persisted.edges,
                    persisted.property_projection_mode,
                    &persisted.actions,
                    &reconciled_actions,
                )
                .await
                {
                    tracing::error!(%rollback_error, "Failed to roll back ontology action bindings");
                }
                return Err(error.into());
            }
        };
        overlay.actions = reconciled_actions.clone();
        prepared_action_snapshots = prepared;
        action_rollback = Some(reconciled_actions);
    }
    if actions_supplied {
        removed_action_events = previous_overlay
            .actions
            .iter()
            .filter(|previous| {
                !overlay
                    .actions
                    .iter()
                    .any(|action| action.id == previous.id)
            })
            .cloned()
            .collect();
    }
    overlay.updated_at = chrono::Utc::now().to_rfc3339();

    let save_result =
        lancegraph::save_overlay_if_unchanged(&conn, &overlay, &previous_overlay.updated_at).await;
    if !matches!(&save_result, Ok(true)) {
        let persisted = lancegraph::load_overlay(&conn, &overlay_id)
            .await
            .unwrap_or_else(|_| previous_overlay.clone());
        if persisted.updated_at != overlay.updated_at {
            if let Some(attempted_actions) = action_rollback
                && let Err(rollback_error) = rollback_action_event_changes(
                    &app_handle,
                    &app_id,
                    &overlay.id,
                    persisted.exposed,
                    &persisted.nodes,
                    &persisted.edges,
                    persisted.property_projection_mode,
                    &persisted.actions,
                    &attempted_actions,
                )
                .await
            {
                tracing::error!(%rollback_error, "Failed to roll back ontology action bindings");
            }
            return match save_result {
                Ok(false) => Err(TauriFunctionError::new(
                    "The ontology changed while it was being saved. Refresh Data Studio and try again.",
                )),
                Err(error) => Err(error.into()),
                Ok(true) => unreachable!(),
            };
        }
    }
    if let Err(error) =
        commit_action_board_snapshots(&app_handle, &app_id, &prepared_action_snapshots).await
    {
        tracing::warn!(%error, "Could not advance a prepared action board draft");
    }
    if let Err(error) =
        remove_action_events(&app_handle, &app_id, &overlay.id, &removed_action_events).await
    {
        tracing::error!(%error, "Failed to clean up removed ontology action bindings");
    }
    Ok(overlay)
}

#[tauri::command(async)]
pub async fn graph_delete_overlay(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    user_scoped: Option<bool>,
) -> Result<(), TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    lancegraph::delete_overlay(&conn, &overlay_id).await?;
    if let Err(error) =
        remove_action_events(&app_handle, &app_id, &overlay_id, &overlay.actions).await
    {
        tracing::error!(%error, "Failed to clean up deleted ontology action bindings");
    }
    Ok(())
}

#[tauri::command(async)]
pub async fn graph_get_schema(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let store = LanceGraphStore::new(conn, overlay, None).await?;
    let schema = store.schema().await?;
    serde_json::to_value(schema).map_err(|e| e.into())
}

#[tauri::command(async)]
pub async fn graph_validate_overlay(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    user_scoped: Option<bool>,
    draft: Option<lancegraph::GraphOverlayDef>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = match draft {
        Some(draft) => draft,
        None => lancegraph::load_overlay(&conn, &overlay_id).await?,
    };
    let report = lancegraph::validate_overlay_definition(&conn, &overlay).await?;
    serde_json::to_value(report).map_err(|e| e.into())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CypherPayload {
    pub query: String,
    pub params: Option<serde_json::Map<String, serde_json::Value>>,
    pub limit: Option<usize>,
}

#[tauri::command(async)]
pub async fn graph_cypher(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    payload: CypherPayload,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let store = LanceGraphStore::new(conn, overlay, None).await?;
    let params = match payload.params {
        Some(map) => serde_json::Value::Object(map),
        None => serde_json::Value::Null,
    };
    let result = store
        .cypher(
            &payload.query,
            params,
            Some(payload.limit.unwrap_or(DEFAULT_GRAPH_QUERY_LIMIT)),
        )
        .await?;
    serde_json::to_value(result).map_err(|e| e.into())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertGraphElementsPayload {
    pub label: String,
    pub rows: Vec<serde_json::Value>,
}

#[tauri::command(async)]
pub async fn graph_upsert_nodes(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    payload: UpsertGraphElementsPayload,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let store = LanceGraphStore::new(conn, overlay, None).await?;
    let upserted = store.upsert_nodes(&payload.label, payload.rows).await?;
    Ok(serde_json::json!({ "upserted": upserted }))
}

#[tauri::command(async)]
pub async fn graph_upsert_edges(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    payload: UpsertGraphElementsPayload,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let store = LanceGraphStore::new(conn, overlay, None).await?;
    let upserted = store.upsert_edges(&payload.label, payload.rows).await?;
    Ok(serde_json::json!({ "upserted": upserted }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubgraphPayload {
    pub seeds: Vec<SubgraphSeed>,
    pub depth: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubgraphSeed {
    pub label: String,
    pub id: serde_json::Value,
}

#[tauri::command(async)]
pub async fn graph_subgraph(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    payload: SubgraphPayload,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let store = LanceGraphStore::new(conn, overlay, None).await?;
    let seeds: Vec<(String, serde_json::Value)> =
        payload.seeds.into_iter().map(|s| (s.label, s.id)).collect();
    let result = store
        .subgraph(
            seeds,
            payload.depth.unwrap_or(1),
            Some(payload.limit.unwrap_or(DEFAULT_GRAPH_QUERY_LIMIT)),
        )
        .await?;
    serde_json::to_value(result).map_err(|e| e.into())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchNodesPayload {
    pub query: String,
    pub limit: Option<usize>,
}

#[tauri::command(async)]
pub async fn graph_search_nodes(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    payload: SearchNodesPayload,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let store = LanceGraphStore::new(conn, overlay, None).await?;
    let result = store.search_nodes(&payload.query, payload.limit).await?;
    serde_json::to_value(result).map_err(|e| e.into())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeighborsPayload {
    pub label: String,
    #[serde(alias = "node_id")]
    pub node_id: serde_json::Value,
    pub depth: Option<usize>,
    pub direction: Option<String>,
    pub limit: Option<usize>,
    /// Relationship labels to follow. Omit or leave empty to follow all of them.
    #[serde(default, alias = "edge_labels")]
    pub edge_labels: Option<Vec<String>>,
}

#[tauri::command(async)]
pub async fn graph_neighbors(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    payload: NeighborsPayload,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let store = LanceGraphStore::new(conn, overlay, None).await?;
    let direction = match payload
        .direction
        .as_deref()
        .unwrap_or(DEFAULT_GRAPH_NEIGHBORS_DIRECTION)
    {
        "incoming" => TraversalDirection::Incoming,
        "both" => TraversalDirection::Both,
        _ => TraversalDirection::Outgoing,
    };
    let result = store
        .neighbors(
            &payload.label,
            payload.node_id,
            payload.depth.unwrap_or(1),
            direction,
            payload.limit,
            payload.edge_labels.as_deref(),
        )
        .await?;
    serde_json::to_value(result).map_err(|e| e.into())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayChildrenPayload {
    pub label: String,
    #[serde(alias = "node_id")]
    pub node_id: serde_json::Value,
    pub limit: Option<usize>,
}

#[tauri::command(async)]
pub async fn graph_overlay_children(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    payload: OverlayChildrenPayload,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let store = LanceGraphStore::new(conn, overlay, None).await?;
    let result = store
        .overlay_children(&payload.label, payload.node_id, payload.limit)
        .await?;
    serde_json::to_value(result).map_err(|e| e.into())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlPayload {
    pub query: String,
    pub limit: Option<usize>,
}

#[tauri::command(async)]
pub async fn graph_sql(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    payload: SqlPayload,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let store = LanceGraphStore::new(conn, overlay, None).await?;
    let result = store
        .sql(
            &payload.query,
            Some(payload.limit.unwrap_or(DEFAULT_GRAPH_QUERY_LIMIT)),
        )
        .await?;
    serde_json::to_value(result).map_err(|e| e.into())
}

#[tauri::command(async)]
pub async fn graph_sample(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    label: String,
    n: Option<usize>,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let result = lancegraph::sample_overlay(
        &conn,
        &overlay,
        &label,
        n.unwrap_or(DEFAULT_GRAPH_SAMPLE_SIZE).min(500),
    )
    .await?;
    serde_json::to_value(result).map_err(|e| e.into())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathsPayload {
    #[serde(alias = "from_label")]
    pub from_label: String,
    #[serde(alias = "from_id")]
    pub from_id: serde_json::Value,
    #[serde(alias = "to_label")]
    pub to_label: String,
    #[serde(alias = "to_id")]
    pub to_id: serde_json::Value,
    #[serde(alias = "max_depth")]
    pub max_depth: Option<usize>,
    pub limit: Option<usize>,
}

#[tauri::command(async)]
pub async fn graph_paths(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    payload: PathsPayload,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let store = LanceGraphStore::new(conn, overlay, None).await?;
    let result = store
        .shortest_paths(
            (payload.from_label, payload.from_id),
            (payload.to_label, payload.to_id),
            payload.max_depth.unwrap_or(4),
            payload.limit,
        )
        .await?;
    serde_json::to_value(result).map_err(|e| e.into())
}

#[tauri::command(async)]
pub async fn graph_analytics(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    limit: Option<usize>,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let store = LanceGraphStore::new(conn, overlay, None).await?;
    let result = store.analytics(limit).await?;
    serde_json::to_value(result).map_err(|e| e.into())
}
