//! Shared helpers for prerun analysis (board + event endpoints).
//!
//! The bulk of prerun work is identical across endpoints: load the board,
//! walk every node (and every layer's nodes), and extract runtime variables,
//! OAuth requirements, and WASM package metadata. The result depends only on
//! `(app_id, board_id, version)` — not on the caller — so frontends are
//! expected to cache responses and revalidate in the background.
//!
//! The payload carries a stable `signature` hash so callers can detect drift
//! when revalidating and react (rerun / cancel / prompt) on divergence.

use blake3::Hasher;
use flow_like::flow::{
    board::{Board, ExecutionMode},
    node::{Node, NodePermission},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use utoipa::ToSchema;

/// A runtime-configured variable that needs a value before execution.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeVariable {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub data_type: String,
    pub value_type: String,
    pub secret: bool,
    pub schema: Option<String>,
}

/// OAuth provider requirement collected from the board's nodes.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OAuthRequirement {
    pub provider_id: String,
    pub scopes: Vec<String>,
}

/// Board-derived prerun data — everything that depends only on
/// `(app_id, board_id, version)` and is safe to share across users.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PrerunPayload {
    pub runtime_variables: Vec<RuntimeVariable>,
    pub oauth_requirements: Vec<OAuthRequirement>,
    pub requires_local_execution: bool,
    #[schema(value_type = String)]
    pub execution_mode: ExecutionMode,
    pub has_wasm_nodes: bool,
    pub wasm_package_ids: Vec<String>,
    pub wasm_package_permissions: HashMap<String, Vec<NodePermission>>,
    /// Stable hash over the board-derived fields. Frontend uses this
    /// to detect drift when revalidating in the background.
    pub signature: String,
}

pub fn parse_version(version_str: &str) -> Option<(u32, u32, u32)> {
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

pub fn compute_payload(board: &Board) -> PrerunPayload {
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

    let mut oauth_scopes: HashMap<String, Vec<String>> = HashMap::new();
    let mut requires_local_execution = false;
    let mut wasm_package_ids: Vec<String> = Vec::new();
    let mut wasm_package_permissions: HashMap<String, Vec<NodePermission>> = HashMap::new();

    for node in board.nodes.values() {
        accumulate_node(
            node,
            &mut oauth_scopes,
            &mut requires_local_execution,
            &mut wasm_package_ids,
            &mut wasm_package_permissions,
        );
    }
    for layer in board.layers.values() {
        for node in layer.nodes.values() {
            accumulate_node(
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

    let mut payload = PrerunPayload {
        runtime_variables,
        oauth_requirements,
        requires_local_execution,
        execution_mode: board.execution_mode.clone(),
        has_wasm_nodes: !wasm_package_ids.is_empty(),
        wasm_package_ids,
        wasm_package_permissions,
        signature: String::new(),
    };
    payload.signature = compute_signature(&payload);
    payload
}

fn accumulate_node(
    node: &Node,
    oauth_scopes: &mut HashMap<String, Vec<String>>,
    requires_local: &mut bool,
    wasm_ids: &mut Vec<String>,
    wasm_perms: &mut HashMap<String, Vec<NodePermission>>,
) {
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
    if node.only_offline {
        *requires_local = true;
    }
    if let Some(providers) = &node.oauth_providers {
        for provider_id in providers {
            oauth_scopes.entry(provider_id.clone()).or_default();
        }
    }
    // required_oauth_scopes only contributes scopes for providers already
    // registered via oauth_providers — it's informational, not a trigger.
    if let Some(required_scopes) = &node.required_oauth_scopes {
        for (provider_id, scopes) in required_scopes {
            if let Some(entry) = oauth_scopes.get_mut(provider_id) {
                for scope in scopes {
                    if !entry.contains(scope) {
                        entry.push(scope.clone());
                    }
                }
            }
        }
    }
}

/// Stable hash over every signature-relevant field, with deterministic ordering
/// so HashMap/Vec insertion order can't shift the hash.
fn compute_signature(payload: &PrerunPayload) -> String {
    let mut h = Hasher::new();

    h.update(format!("{:?}", payload.execution_mode).as_bytes());
    h.update(&[payload.requires_local_execution as u8]);
    h.update(&[payload.has_wasm_nodes as u8]);

    let mut vars: Vec<&RuntimeVariable> = payload.runtime_variables.iter().collect();
    vars.sort_by(|a, b| a.id.cmp(&b.id));
    for v in vars {
        h.update(b"|var|");
        h.update(v.id.as_bytes());
        h.update(b"|");
        h.update(v.name.as_bytes());
        h.update(b"|");
        h.update(v.data_type.as_bytes());
        h.update(b"|");
        h.update(v.value_type.as_bytes());
        h.update(b"|");
        h.update(&[v.secret as u8]);
        h.update(b"|");
        if let Some(d) = &v.description {
            h.update(d.as_bytes());
        }
        h.update(b"|");
        if let Some(s) = &v.schema {
            h.update(s.as_bytes());
        }
    }

    let mut providers: Vec<&OAuthRequirement> = payload.oauth_requirements.iter().collect();
    providers.sort_by(|a, b| a.provider_id.cmp(&b.provider_id));
    for p in providers {
        h.update(b"|oauth|");
        h.update(p.provider_id.as_bytes());
        let mut scopes = p.scopes.clone();
        scopes.sort();
        for s in scopes {
            h.update(b"|");
            h.update(s.as_bytes());
        }
    }

    let mut ids = payload.wasm_package_ids.clone();
    ids.sort();
    for id in ids {
        h.update(b"|wasm|");
        h.update(id.as_bytes());
    }

    let sorted: BTreeMap<&String, &Vec<NodePermission>> =
        payload.wasm_package_permissions.iter().collect();
    for (pkg_id, perms) in sorted {
        h.update(b"|wp|");
        h.update(pkg_id.as_bytes());
        let mut perm_strs: Vec<String> = perms.iter().map(|p| format!("{:?}", p)).collect();
        perm_strs.sort();
        for p in perm_strs {
            h.update(b"|");
            h.update(p.as_bytes());
        }
    }

    h.finalize().to_hex().to_string()
}
