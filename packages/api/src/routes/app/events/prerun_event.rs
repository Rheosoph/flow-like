//! Pre-run analysis endpoint for events
//!
//! Returns information needed before executing an event:
//! - Runtime-configured variables that need values
//! - Required OAuth providers and scopes

use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::board::ExecutionMode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

use super::db::get_event_from_db;

/// Query parameters for pre-run analysis
#[derive(Debug, Deserialize, ToSchema)]
pub struct PrerunEventQuery {
    /// Board version as tuple (major, minor, patch) - defaults to latest
    pub version: Option<String>,
}

/// A runtime-configured variable that needs a value before execution
#[derive(Debug, Serialize, ToSchema)]
pub struct RuntimeVariable {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub data_type: String,
    pub value_type: String,
    pub secret: bool,
    pub schema: Option<String>,
}

/// OAuth provider requirement
#[derive(Debug, Serialize, ToSchema)]
pub struct OAuthRequirement {
    pub provider_id: String,
    pub scopes: Vec<String>,
}

/// Response from pre-run analysis
#[derive(Debug, Serialize, ToSchema)]
pub struct PrerunEventResponse {
    /// ID of the board this event triggers
    pub board_id: String,
    /// Variables that are marked as runtime_configured (need user-provided values)
    pub runtime_variables: Vec<RuntimeVariable>,
    /// OAuth providers required by nodes in this board
    pub oauth_requirements: Vec<OAuthRequirement>,
    /// Whether the event can only run locally (has offline-only nodes)
    pub requires_local_execution: bool,
    /// Board's execution mode setting (Hybrid, Remote, Local)
    #[schema(value_type = String)]
    pub execution_mode: ExecutionMode,
    /// Whether the user can execute locally (has ReadBoards permission)
    /// If false, execution must happen on server
    pub can_execute_locally: bool,
    /// Whether the board contains any WASM (external) nodes
    pub has_wasm_nodes: bool,
    /// package_id values of all WASM nodes present in the board
    pub wasm_package_ids: Vec<String>,
    /// Per-package permissions declared by WASM nodes (package_id -> list of permission strings)
    pub wasm_package_permissions: HashMap<String, Vec<String>>,
}

fn parse_version(version_str: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<&str> = version_str.split('_').collect();
    if parts.len() == 3 {
        let major = parts[0].parse().ok()?;
        let minor = parts[1].parse().ok()?;
        let patch = parts[2].parse().ok()?;
        Some((major, minor, patch))
    } else {
        None
    }
}

/// Analyze an event to determine what's needed before execution.
///
/// Returns runtime-configured variables and OAuth requirements from the event's board.
#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/prerun",
    tag = "events",
    description = "Get pre-run requirements for an event.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID"),
        ("version" = Option<String>, Query, description = "Version in MAJOR_MINOR_PATCH format")
    ),
    responses(
        (status = 200, description = "Pre-run requirements", body = PrerunEventResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/events/{event_id}/prerun",
    skip(state, user)
)]
pub async fn prerun_event(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Query(query): Query<PrerunEventQuery>,
) -> Result<Json<PrerunEventResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);
    let sub = permission.sub()?;

    // Check if user can execute locally (has ReadBoards permission)
    let can_execute_locally = permission.has_permission(RolePermissions::ReadBoards);

    let version = query.version.as_ref().and_then(|v| parse_version(v));

    // Get the event from database (validates event belongs to this app)
    let event = get_event_from_db(&state.db, &event_id, &app_id).await?;
    let board_id = event.board_id.clone();

    // Get the board from the event
    let board = state
        .master_board(&sub, &app_id, &board_id, &state, version)
        .await?;

    // Collect runtime-configured variables
    let runtime_variables: Vec<RuntimeVariable> = board
        .variables
        .values()
        .filter(|v| v.runtime_configured)
        .map(|v| RuntimeVariable {
            id: v.id.clone(),
            name: v.name.clone(),
            description: v.description.clone(),
            data_type: format!("{:?}", v.data_type),
            value_type: format!("{:?}", v.value_type),
            secret: v.secret,
            schema: v.schema.clone(),
        })
        .collect();

    // Collect OAuth requirements and WASM info from all nodes (including layers)
    let mut oauth_scopes: HashMap<String, Vec<String>> = HashMap::new();
    let mut requires_local_execution = false;
    let mut wasm_package_ids: Vec<String> = Vec::new();
    let mut wasm_package_permissions: HashMap<String, Vec<String>> = HashMap::new();

    let process_node = |node: &flow_like::flow::node::Node,
                        oauth_scopes: &mut HashMap<String, Vec<String>>,
                        requires_local: &mut bool,
                        wasm_ids: &mut Vec<String>,
                        wasm_perms: &mut HashMap<String, Vec<String>>| {
        // Collect WASM (external) node package IDs
        if let Some(wasm) = &node.wasm {
            if !wasm_ids.contains(&wasm.package_id) {
                wasm_ids.push(wasm.package_id.clone());
            }
            if !wasm.permissions.is_empty() {
                let entry = wasm_perms.entry(wasm.package_id.clone()).or_default();
                for perm in &wasm.permissions {
                    if !entry.contains(perm) {
                        entry.push(perm.clone());
                    }
                }
            }
        }
        // Check if node requires local execution
        if node.only_offline {
            *requires_local = true;
        }

        // Collect OAuth provider IDs
        if let Some(providers) = &node.oauth_providers {
            for provider_id in providers {
                oauth_scopes.entry(provider_id.clone()).or_default();
            }
        }

        // Collect required scopes - only for providers already registered via oauth_providers
        // required_oauth_scopes is informational - it documents what scopes a node needs
        // IF OAuth is used, but shouldn't trigger OAuth by itself
        if let Some(required_scopes) = &node.required_oauth_scopes {
            for (provider_id, scopes) in required_scopes {
                // Only add scopes if this provider was already registered by an OAuth provider node
                if let Some(entry) = oauth_scopes.get_mut(provider_id) {
                    for scope in scopes {
                        if !entry.contains(scope) {
                            entry.push(scope.clone());
                        }
                    }
                }
            }
        }
    };

    // Process main board nodes
    for node in board.nodes.values() {
        process_node(
            node,
            &mut oauth_scopes,
            &mut requires_local_execution,
            &mut wasm_package_ids,
            &mut wasm_package_permissions,
        );
    }

    // Process layer nodes
    for layer in board.layers.values() {
        for node in layer.nodes.values() {
            process_node(
                node,
                &mut oauth_scopes,
                &mut requires_local_execution,
                &mut wasm_package_ids,
                &mut wasm_package_permissions,
            );
        }
    }

    let oauth_requirements: Vec<OAuthRequirement> = oauth_scopes
        .into_iter()
        .map(|(provider_id, scopes)| OAuthRequirement {
            provider_id,
            scopes,
        })
        .collect();

    Ok(Json(PrerunEventResponse {
        board_id,
        runtime_variables,
        oauth_requirements,
        requires_local_execution,
        execution_mode: board.execution_mode.clone(),
        can_execute_locally,
        has_wasm_nodes: !wasm_package_ids.is_empty(),
        wasm_package_ids,
        wasm_package_permissions,
    }))
}
