//! Pre-run analysis endpoint for boards
//!
//! Returns information needed before executing a board:
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
use flow_like::flow::{board::ExecutionMode, node::NodePermission};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};

/// Query parameters for pre-run analysis
#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct PrerunBoardQuery {
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
pub struct PrerunBoardResponse {
    /// Variables that are marked as runtime_configured (need user-provided values)
    pub runtime_variables: Vec<RuntimeVariable>,
    /// OAuth providers required by nodes in this board
    pub oauth_requirements: Vec<OAuthRequirement>,
    /// Whether the board can only run locally (has offline-only nodes)
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
    /// Per-package permissions declared by WASM nodes (package_id -> list of permissions)
    pub wasm_package_permissions: HashMap<String, Vec<NodePermission>>,
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

/// Analyze a board to determine what's needed before execution.
///
/// Returns runtime-configured variables and OAuth requirements.
#[utoipa::path(
    get,
    path = "/apps/{app_id}/board/{board_id}/prerun",
    tag = "execution",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID"),
        PrerunBoardQuery
    ),
    responses(
        (status = 200, description = "Pre-run analysis results", body = PrerunBoardResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Board not found")
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/board/{board_id}/prerun", skip(state, user))]
pub async fn prerun_board(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Query(query): Query<PrerunBoardQuery>,
) -> Result<Json<PrerunBoardResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteBoards);
    let sub = permission.sub()?;

    // Check if user can execute locally (has ReadBoards permission)
    let can_execute_locally = permission.has_permission(RolePermissions::ReadBoards);

    let version = query.version.as_ref().and_then(|v| parse_version(v));

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
    let mut wasm_package_permissions: HashMap<String, Vec<NodePermission>> = HashMap::new();

    let process_node =
        |node: &flow_like::flow::node::Node,
         oauth_scopes: &mut HashMap<String, Vec<String>>,
         requires_local: &mut bool,
         wasm_ids: &mut Vec<String>,
         wasm_perms: &mut HashMap<String, Vec<NodePermission>>| {
            // Collect WASM (external) node package IDs
            if let Some(wasm) = &node.wasm {
                if !wasm_ids.contains(&wasm.package_id) {
                    wasm_ids.push(wasm.package_id.clone());
                }
                if !wasm.permissions.is_empty() {
                    let entry = wasm_perms.entry(wasm.package_id.clone()).or_default();
                    for perm in &wasm.permissions {
                        if !entry.contains(perm) {
                            entry.push(*perm);
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

    Ok(Json(PrerunBoardResponse {
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
