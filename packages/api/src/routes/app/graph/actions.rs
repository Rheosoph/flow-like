use std::{
    collections::{HashMap, HashSet},
    time::SystemTime,
};

use axum::{
    Extension, Json,
    extract::{Path, State},
    response::Response,
};
use flow_like::app::App;
use flow_like::flow::event::{Event, EventExecutionMode, EventExposure};
use flow_like_storage::databases::graph::lancegraph::{self, OntologyActionDef};
use flow_like_types::{Value, create_id, json::json};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    credentials::CredentialsAccess,
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::events::{
        db::{get_event_from_db, sync_event_to_db},
        invoke_event::{InvokeEventQuery, InvokeEventRequest, invoke_resolved_event},
    },
    routes::app::prerun_shared::{OAuthRequirement, compute_payload},
    state::AppState,
};

const ONTOLOGY_ACTION_EVENT_TYPE: &str = "ontology_action";
const MAX_ACTION_OBJECTS: usize = 100;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct OntologyObjectRef {
    pub object_type: String,
    #[schema(value_type = Object)]
    pub id: Value,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct InvokeOntologyActionRequest {
    pub object_refs: Vec<OntologyObjectRef>,
    #[serde(default = "empty_object")]
    #[schema(value_type = Object)]
    pub parameters: Value,
    /// Client retry key. It is included in run correlation and the governed
    /// payload; durable duplicate suppression is intentionally left to the
    /// action workflow until run-level idempotency is persisted by the API.
    #[serde(default)]
    pub idempotency_key: Option<String>,
    /// User token forwarded to nodes in the action workflow.
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub oauth_tokens: Option<HashMap<String, Value>>,
    #[serde(default)]
    pub profile_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OntologyActionPrerunResponse {
    pub oauth_requirements: Vec<OAuthRequirement>,
    pub signature: String,
}

fn empty_object() -> Value {
    json!({})
}

fn action_event(
    ontology_id: &str,
    ontology_exposed: bool,
    objects: &[lancegraph::NodeMappingDef],
    action: &OntologyActionDef,
    event_id: String,
) -> Result<Event, ApiError> {
    let start_node_id = action
        .start_node_id
        .clone()
        .filter(|node_id| !node_id.trim().is_empty())
        .ok_or_else(|| ApiError::bad_request("The ontology action has no start node"))?;
    if action.board_id.trim().is_empty() {
        return Err(ApiError::bad_request(
            "The ontology action has no implementation board",
        ));
    }
    let board_version = action.board_version.ok_or_else(|| {
        ApiError::bad_request("The ontology action must pin an exact board version")
    })?;
    let object = action_object(objects, action).ok_or_else(|| {
        ApiError::bad_request(format!(
            "Ontology action '{}' references unknown object type '{}'",
            action.id, action.object_type
        ))
    })?;
    let contract_hash =
        lancegraph::ontology_action_contract_hash(ontology_id, ontology_exposed, action, object)
            .map_err(|error| {
                ApiError::internal(format!("Could not hash action contract: {error}"))
            })?;

    let now = SystemTime::now();
    Ok(Event {
        id: event_id,
        name: action.name.clone(),
        description: action.description.clone().unwrap_or_default(),
        board_id: action.board_id.clone(),
        board_version: Some((board_version[0], board_version[1], board_version[2])),
        node_id: start_node_id,
        variables: HashMap::new(),
        config: serde_json::to_vec(&json!({
            "managed_by": ONTOLOGY_ACTION_EVENT_TYPE,
            "ontology_id": ontology_id,
            "action_id": action.id,
            "contract_hash": contract_hash,
        }))
        .unwrap_or_default(),
        active: action.enabled,
        canary: None,
        priority: 0,
        event_type: ONTOLOGY_ACTION_EVENT_TYPE.to_string(),
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
    })
}

/// Ensures the exact board version a governed action pins exists as an
/// immutable snapshot, then derives the action's parameter schema from that
/// pinned board. The Data Studio editor pins the board's working version, which
/// is not published until now; without a snapshot the managed event's
/// `validate_event_references` fails when it tries to load the version.
///
/// The derived schema is authoritative: it is read from the start node's
/// `parameters` struct pin on the pinned board that actually executes, so a
/// client cannot pin a board doing X while advertising a schema for Y. The
/// client-supplied `parameter_schema` is ignored for materialized actions and
/// overwritten with the board-derived value (or `None` when the start node has
/// no typed `parameters` pin — invoke then accepts any object payload).
async fn ensure_action_board_published(
    app: &App,
    action: &mut OntologyActionDef,
) -> Result<(), ApiError> {
    if action.board_id.trim().is_empty() {
        // action_event surfaces the missing-implementation error with context.
        return Ok(());
    }
    let board = app
        .open_board(action.board_id.clone(), Some(false), None)
        .await
        .map_err(|_| {
            ApiError::bad_request(format!(
                "The action's implementation board '{}' could not be opened",
                action.board_id
            ))
        })?;
    let (current, existing) = {
        let guard = board.lock().await;
        let current = guard.version;
        let existing = guard.get_versions(None).await.unwrap_or_default();
        (current, existing)
    };
    let pinned = match action.board_version {
        Some([maj, min, pat]) => (maj, min, pat),
        None => {
            action.board_version = Some([current.0, current.1, current.2]);
            current
        }
    };
    if pinned == current {
        // Pinning the working draft: always re-snapshot so the immutable version
        // reflects the latest edits. `snapshot_at_version` does not bump the
        // board version, so a prior save can leave a stale snapshot at this
        // number that is missing nodes added since — the managed event's
        // `validate_event_references` then fails with "node not found in board".
        board
            .lock()
            .await
            .snapshot_at_version(pinned, None)
            .await
            .map_err(|error| {
                ApiError::internal(format!(
                    "Could not publish the action's board version: {error}"
                ))
            })?;
    } else if !existing.contains(&pinned) {
        return Err(ApiError::bad_request(format!(
            "The action's board version {}.{}.{} no longer exists. Re-select the board in Data Studio.",
            pinned.0, pinned.1, pinned.2
        )));
    }
    action.parameter_schema = derive_action_parameter_schema(app, action, pinned).await?;
    Ok(())
}

/// Reads the authoritative parameter schema from the pinned board version's
/// start node. Loading the pinned snapshot (rather than the working board)
/// guarantees the schema matches the exact implementation that executes, even
/// when the action pins an older published version.
async fn derive_action_parameter_schema(
    app: &App,
    action: &OntologyActionDef,
    pinned: (u32, u32, u32),
) -> Result<Option<Value>, ApiError> {
    let Some(start_node_id) = action
        .start_node_id
        .as_deref()
        .map(str::trim)
        .filter(|node_id| !node_id.is_empty())
    else {
        return Ok(None);
    };
    let board = app
        .open_board(action.board_id.clone(), Some(false), Some(pinned))
        .await
        .map_err(|error| {
            ApiError::internal(format!(
                "Could not load the action's pinned board version to derive its parameter schema: {error}"
            ))
        })?;
    let schema = board.lock().await.action_parameter_schema(start_node_id);
    Ok(schema)
}

fn managed_event_matches(event: &Event, ontology_id: &str, action_id: &str) -> bool {
    if event.event_type != ONTOLOGY_ACTION_EVENT_TYPE {
        return false;
    }
    serde_json::from_slice::<Value>(&event.config)
        .ok()
        .is_some_and(|config| {
            config.get("managed_by").and_then(Value::as_str) == Some(ONTOLOGY_ACTION_EVENT_TYPE)
                && config.get("ontology_id").and_then(Value::as_str) == Some(ontology_id)
                && config.get("action_id").and_then(Value::as_str) == Some(action_id)
        })
}

fn managed_event_binding_is_current(
    event: &Event,
    ontology_id: &str,
    ontology_exposed: bool,
    objects: &[lancegraph::NodeMappingDef],
    action: &OntologyActionDef,
) -> bool {
    let Some(object) = action_object(objects, action) else {
        return false;
    };
    let Ok(contract_hash) =
        lancegraph::ontology_action_contract_hash(ontology_id, ontology_exposed, action, object)
    else {
        return false;
    };
    let saved_contract_hash = serde_json::from_slice::<Value>(&event.config)
        .ok()
        .and_then(|config| {
            config
                .get("contract_hash")
                .and_then(Value::as_str)
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
    objects: &'a [lancegraph::NodeMappingDef],
    action: &OntologyActionDef,
) -> Option<&'a lancegraph::NodeMappingDef> {
    objects.iter().find(|object| {
        object.id.as_deref() == Some(action.object_type.as_str())
            || object.api_name.as_deref() == Some(action.object_type.as_str())
            || object.label == action.object_type
    })
}

pub(crate) fn validate_action_object_types<F>(
    actions: &[OntologyActionDef],
    object_type_exists: F,
) -> Result<(), ApiError>
where
    F: Fn(&str) -> bool,
{
    for action in actions {
        if !object_type_exists(&action.object_type) {
            return Err(ApiError::bad_request(format!(
                "Ontology action '{}' references unknown object type '{}'",
                action.id, action.object_type
            )));
        }
        if let Some(schema) = &action.parameter_schema {
            flow_like_catalog_core::ontology_action_parameter_validator(schema).map_err(
                |error| {
                    ApiError::bad_request(format!(
                        "Ontology action '{}' has an invalid parameter schema: {error}",
                        action.id
                    ))
                },
            )?;
        }
    }
    Ok(())
}

/// Creates or updates the internal, version-pinned events that make ontology
/// actions executable through the existing event runtime. Callers must already
/// have checked graph-write and event-write permissions.
pub(crate) async fn materialize_action_events(
    state: &AppState,
    sub: &str,
    app_id: &str,
    ontology_id: &str,
    ontology_exposed: bool,
    objects: &[lancegraph::NodeMappingDef],
    actions: &mut [OntologyActionDef],
) -> Result<(), ApiError> {
    if actions.is_empty() {
        return Ok(());
    }

    let mut action_ids = HashSet::with_capacity(actions.len());
    for action in actions.iter() {
        if action.id.trim().is_empty() || !action_ids.insert(action.id.clone()) {
            return Err(ApiError::bad_request(
                "Ontology action IDs must be non-empty and unique",
            ));
        }
        if action.name.trim().is_empty() {
            return Err(ApiError::bad_request("Ontology actions must have a name"));
        }
    }

    let mut app = state
        .scoped_app(sub, app_id, state, CredentialsAccess::EditApp)
        .await?;
    let mut events_to_sync = Vec::with_capacity(actions.len());
    let mut app_changed = false;

    for action in actions {
        let requested_event_id = action
            .event_id
            .clone()
            .filter(|event_id| !event_id.trim().is_empty());
        let existing = match requested_event_id.as_deref() {
            Some(event_id) => app.get_event(event_id, None).await.ok(),
            None => None,
        };
        if let Some(event) = existing.as_ref()
            && managed_event_binding_is_current(
                &event,
                ontology_id,
                ontology_exposed,
                objects,
                action,
            )
        {
            action.event_id = Some(event.id.clone());
            events_to_sync.push(event.clone());
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
        ensure_action_board_published(&app, action).await?;
        let mut event = action_event(ontology_id, ontology_exposed, objects, action, event_id)?;
        let event = event.upsert(&app, None, true).await?;
        if !app.events.contains(&event.id) {
            app.events.push(event.id.clone());
        }
        action.event_id = Some(event.id.clone());
        events_to_sync.push(event);
        app_changed = true;
    }

    if app_changed {
        app.updated_at = SystemTime::now();
        app.save().await?;
    }
    for event in &events_to_sync {
        // Ontology action events have no inbound sink. Sync only the fast event
        // lookup mirror; the generic sink mapper treats unknown event types as
        // HTTP and would accidentally publish an endpoint.
        sync_event_to_db(&state.db, app_id, event).await?;
    }
    Ok(())
}

/// Removes materialized events for actions that no longer exist. Ownership is
/// verified from the event's managed metadata before deletion, so a stale or
/// malicious `event_id` can never delete an unrelated project event.
pub(crate) async fn remove_action_events(
    state: &AppState,
    sub: &str,
    app_id: &str,
    ontology_id: &str,
    removed_actions: &[OntologyActionDef],
) -> Result<(), ApiError> {
    if removed_actions.is_empty() {
        return Ok(());
    }

    let mut app = state
        .scoped_app(sub, app_id, state, CredentialsAccess::EditApp)
        .await?;
    let mut removed_event_ids = Vec::new();
    for action in removed_actions {
        let Some(event_id) = action.event_id.as_deref() else {
            continue;
        };
        let Ok(event) = app.get_event(event_id, None).await else {
            continue;
        };
        if !managed_event_matches(&event, ontology_id, &action.id) {
            continue;
        }
        event.delete(&app).await?;
        app.events.retain(|saved_id| saved_id != event_id);
        removed_event_ids.push(event_id.to_string());
    }
    if !removed_event_ids.is_empty() {
        app.updated_at = SystemTime::now();
        app.save().await?;
    }
    for event_id in removed_event_ids {
        crate::routes::app::events::db::delete_event_with_sink(&state.db, state, &event_id).await?;
    }
    Ok(())
}

/// Best-effort compensation when ontology persistence fails after event files
/// have already been updated. Newly created bindings are removed and reused
/// bindings are restored to their previous definition.
pub(crate) async fn rollback_action_event_changes(
    state: &AppState,
    sub: &str,
    app_id: &str,
    ontology_id: &str,
    ontology_exposed: bool,
    objects: &[lancegraph::NodeMappingDef],
    previous_actions: &[OntologyActionDef],
    attempted_actions: &[OntologyActionDef],
) -> Result<(), ApiError> {
    let newly_materialized = attempted_actions
        .iter()
        .filter(|attempted| {
            let previous_event_id = previous_actions
                .iter()
                .find(|previous| previous.id == attempted.id)
                .and_then(|previous| previous.event_id.as_deref());
            attempted.event_id.as_deref() != previous_event_id
        })
        .cloned()
        .collect::<Vec<_>>();
    let removal_error = remove_action_events(state, sub, app_id, ontology_id, &newly_materialized)
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
    let restore_result = materialize_action_events(
        state,
        sub,
        app_id,
        ontology_id,
        ontology_exposed,
        objects,
        &mut overwritten,
    )
    .await;
    match (removal_error, restore_result) {
        (Some(error), _) => Err(error),
        (None, result) => result,
    }
}

fn validate_request(
    action: &OntologyActionDef,
    request: &InvokeOntologyActionRequest,
) -> Result<Vec<Value>, ApiError> {
    if request.object_refs.is_empty() {
        return Err(ApiError::bad_request(
            "At least one ontology object is required",
        ));
    }
    if (!action.allow_bulk && request.object_refs.len() != 1)
        || request.object_refs.len() > MAX_ACTION_OBJECTS
    {
        return Err(ApiError::bad_request(if action.allow_bulk {
            format!("Bulk actions accept at most {} objects", MAX_ACTION_OBJECTS)
        } else {
            "This action accepts exactly one object".to_string()
        }));
    }
    if !request.parameters.is_object() {
        return Err(ApiError::bad_request("Action parameters must be an object"));
    }
    if let Some(key) = request.idempotency_key.as_deref()
        && (key.is_empty() || key.len() > 200)
    {
        return Err(ApiError::bad_request(
            "Idempotency keys must contain between 1 and 200 characters",
        ));
    }
    if let Some(schema) = &action.parameter_schema {
        flow_like_catalog_core::validate_ontology_action_parameters(schema, &request.parameters)
            .map_err(|error| {
                ApiError::bad_request(format!(
                    "Action parameters do not match the schema: {error}"
                ))
            })?;
    }

    request
        .object_refs
        .iter()
        .map(|reference| {
            if reference.object_type != action.object_type {
                return Err(ApiError::bad_request(format!(
                    "Action '{}' requires object type '{}'",
                    action.id, action.object_type
                )));
            }
            Ok(reference.id.clone())
        })
        .collect()
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/graph/{ontology_id}/actions/{action_id}/prerun",
    tag = "graph",
    description = "Resolve OAuth requirements for a governed ontology action without exposing its board.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("ontology_id" = String, Path, description = "Ontology ID"),
        ("action_id" = String, Path, description = "Ontology action ID")
    ),
    responses(
        (status = 200, description = "Governed action pre-run requirements", body = OntologyActionPrerunResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Ontology or action not found"),
        (status = 409, description = "Action binding needs repair")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/graph/{ontology_id}/actions/{action_id}/prerun",
    skip(state, user)
)]
pub async fn prerun_ontology_action(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, ontology_id, action_id)): Path<(String, String, String)>,
) -> Result<Json<OntologyActionPrerunResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);
    if !permission.has_permission(RolePermissions::ReadFiles)
        && !permission.has_permission(RolePermissions::ReadDatabase)
    {
        return Err(ApiError::forbidden(
            "Ontology action execution requires access to the governed object data",
        ));
    }
    let sub = permission.sub()?;
    let credentials = state.master_credentials().await?;
    let connection = credentials.to_db(&app_id).await?.execute().await?;
    let ontology = lancegraph::load_overlay(&connection, &ontology_id)
        .await
        .map_err(|_| ApiError::not_found("Ontology not found"))?;
    if user.is_connected_app() && !ontology.exposed {
        return Err(ApiError::forbidden(
            "This ontology is not exposed to connected projects",
        ));
    }
    let action = ontology
        .actions
        .iter()
        .find(|action| action.id == action_id && action.enabled)
        .ok_or_else(|| ApiError::not_found("Enabled ontology action not found"))?;
    if user.is_connected_app() && !action.exposed {
        return Err(ApiError::forbidden(
            "This ontology action is not exposed to connected projects",
        ));
    }
    let event_id = action.event_id.as_deref().ok_or_else(|| {
        ApiError::conflict(
            "The ontology action binding is not materialized. Repair it in Data Studio.",
        )
    })?;
    let event = get_event_from_db(&state.db, event_id, &app_id)
        .await
        .map_err(|_| {
            ApiError::conflict("The ontology action binding is stale. Repair it in Data Studio.")
        })?;
    if !managed_event_binding_is_current(
        &event,
        &ontology_id,
        ontology.exposed,
        &ontology.nodes,
        action,
    ) {
        return Err(ApiError::conflict(
            "The ontology action binding no longer matches its governed implementation. Repair it in Data Studio.",
        ));
    }
    let version = action
        .board_version
        .map(|version| (version[0], version[1], version[2]));
    let board = state
        .master_board(&sub, &app_id, &action.board_id, &state, version)
        .await?;
    let payload = compute_payload(&board);
    Ok(Json(OntologyActionPrerunResponse {
        oauth_requirements: payload.oauth_requirements,
        signature: payload.signature,
    }))
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/graph/{ontology_id}/actions/{action_id}/invoke",
    tag = "graph",
    description = "Invoke an enabled ontology action. The saved action is resolved server-side; callers never supply board implementation coordinates.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("ontology_id" = String, Path, description = "Ontology ID"),
        ("action_id" = String, Path, description = "Ontology action ID")
    ),
    request_body = InvokeOntologyActionRequest,
    responses(
        (status = 200, description = "Action execution SSE stream", body = String, content_type = "text/event-stream"),
        (status = 400, description = "Invalid action input"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Ontology, action, or object not found")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/graph/{ontology_id}/actions/{action_id}/invoke",
    skip(state, user, request)
)]
pub async fn invoke_ontology_action(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, ontology_id, action_id)): Path<(String, String, String)>,
    Json(request): Json<InvokeOntologyActionRequest>,
) -> Result<Response, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);
    if !permission.has_permission(RolePermissions::ReadFiles)
        && !permission.has_permission(RolePermissions::ReadDatabase)
    {
        return Err(ApiError::forbidden(
            "Ontology action execution requires access to the governed object data",
        ));
    }

    let credentials = state.master_credentials().await?;
    let connection = credentials.to_db(&app_id).await?.execute().await?;
    let ontology = lancegraph::load_overlay(&connection, &ontology_id)
        .await
        .map_err(|_| ApiError::not_found("Ontology not found"))?;
    if user.is_connected_app() && !ontology.exposed {
        return Err(ApiError::forbidden(
            "This ontology is not exposed to connected projects",
        ));
    }
    let action = ontology
        .actions
        .iter()
        .find(|action| action.id == action_id && action.enabled)
        .cloned()
        .ok_or_else(|| ApiError::not_found("Enabled ontology action not found"))?;
    if user.is_connected_app() && !action.exposed {
        return Err(ApiError::forbidden(
            "This ontology action is not exposed to connected projects",
        ));
    }
    let ids = validate_request(&action, &request)?;
    let objects =
        lancegraph::load_overlay_objects(&connection, &ontology, &action.object_type, &ids)
            .await
            .map_err(|error| ApiError::bad_request(error.to_string()))?;

    let event_id = action.event_id.as_deref().ok_or_else(|| {
        ApiError::conflict(
            "The ontology action binding is not materialized. Save the action in Data Studio before invoking it.",
        )
    })?;
    let event = get_event_from_db(&state.db, event_id, &app_id)
        .await
        .map_err(|_| {
            ApiError::conflict(
                "The ontology action binding is stale. Save the action in Data Studio to repair it.",
            )
        })?;
    if !managed_event_binding_is_current(
        &event,
        &ontology_id,
        ontology.exposed,
        &ontology.nodes,
        &action,
    ) {
        return Err(ApiError::conflict(
            "The ontology action binding no longer matches its governed implementation. Save it in Data Studio to repair it.",
        ));
    }

    let object_ids = ids
        .iter()
        .map(|id| match id {
            Value::String(value) => value.clone(),
            _ => id.to_string(),
        })
        .collect::<Vec<_>>();
    let payload = json!({
        "_ontology": {
            "ontology_id": ontology_id,
            "action_id": action.id,
            "object_type": action.object_type,
            "object_ids": object_ids,
            "idempotency_key": request.idempotency_key,
        },
        "object": objects.first().cloned().unwrap_or(Value::Null),
        "objects": objects,
        "parameters": request.parameters,
    });
    let mut correlation = HashMap::from([
        ("ontology_id".to_string(), ontology.id.clone()),
        ("ontology_action_id".to_string(), action.id.clone()),
    ]);
    if object_ids.len() == 1 {
        correlation.insert("ontology_object_id".to_string(), object_ids[0].clone());
    }
    if let Some(key) = request.idempotency_key.clone() {
        correlation.insert("idempotency_key".to_string(), key);
    }

    invoke_resolved_event(
        state,
        user,
        app_id,
        event,
        InvokeEventQuery::default(),
        InvokeEventRequest {
            version: action
                .board_version
                .map(|version| format!("{}_{}_{}", version[0], version[1], version[2])),
            payload: Some(payload),
            token: request.token,
            oauth_tokens: request.oauth_tokens,
            // Ontology actions accept only their declared parameter schema.
            // Raw board-variable overrides would bypass that governed contract.
            runtime_variables: None,
            profile_id: request.profile_id,
            correlation: Some(correlation),
        },
    )
    .await
}
