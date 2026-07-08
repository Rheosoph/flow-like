use std::collections::{HashMap, HashSet};

use crate::{
    entity::{app_package, wasm_package_version},
    error::ApiError,
    state::AppState,
};
use flow_like::flow::{
    board::{Board, commands::GenericCommand},
    node::{Node, NodeWasm},
};
use flow_like_wasm::manifest::PackageNodeEntry;
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter};

pub async fn app_wasm_nodes(state: &AppState, app_id: &str) -> Result<Vec<Node>, ApiError> {
    let packages = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(app_id))
        .filter(app_package::Column::Stale.eq(false))
        .all(&state.db)
        .await?;

    if packages.is_empty() {
        return Ok(Vec::new());
    }

    let mut pinned = Condition::any();
    for pkg in &packages {
        pinned = pinned.add(
            Condition::all()
                .add(wasm_package_version::Column::PackageId.eq(&pkg.package_id))
                .add(wasm_package_version::Column::Version.eq(&pkg.version)),
        );
    }

    let mut nodes_by_pin: HashMap<(String, String), serde_json::Value> =
        wasm_package_version::Entity::find()
            .filter(pinned)
            .all(&state.db)
            .await?
            .into_iter()
            .map(|record| ((record.package_id, record.version), record.nodes))
            .collect();

    let mut wasm_nodes: Vec<Node> = Vec::with_capacity(packages.len() * 5);

    for pkg in &packages {
        let key = (pkg.package_id.clone(), pkg.version.clone());
        let Some(nodes_value) = nodes_by_pin.remove(&key) else {
            tracing::warn!(
                package_id = %pkg.package_id,
                version = %pkg.version,
                "app catalog: pinned WASM package version not found; skipping its nodes"
            );
            continue;
        };

        let entries: Vec<PackageNodeEntry> =
            serde_json::from_value(nodes_value).unwrap_or_default();

        for entry in &entries {
            wasm_nodes.push(package_node_to_node(entry, &pkg.package_id));
        }
    }

    Ok(wasm_nodes)
}

fn package_node_to_node(entry: &PackageNodeEntry, package_id: &str) -> Node {
    Node {
        id: entry.id.clone(),
        name: entry.name.clone(),
        friendly_name: entry
            .friendly_name
            .clone()
            .unwrap_or_else(|| entry.name.clone()),
        description: entry.description.clone(),
        coordinates: None,
        category: entry.category.clone(),
        scores: entry.scores.clone(),
        pins: entry.pins.clone(),
        start: entry.start,
        icon: entry.icon.clone(),
        comment: None,
        long_running: entry.long_running,
        error: None,
        docs: entry.docs.clone(),
        event_callback: entry.event_callback,
        layer: None,
        hash: None,
        fn_refs: entry.fn_refs.clone(),
        oauth_providers: if entry.oauth_providers.is_empty() {
            None
        } else {
            Some(entry.oauth_providers.clone())
        },
        required_oauth_scopes: entry.required_oauth_scopes.clone(),
        only_offline: entry.only_offline,
        version: entry.version,
        wasm: Some(NodeWasm {
            package_id: package_id.to_string(),
            permissions: entry.permissions.clone(),
        }),
    }
}

pub fn hydrate_board_wasm_metadata(
    board: &mut Board,
    app_wasm_nodes: &[Node],
    builtin_nodes: &[Node],
) -> bool {
    if app_wasm_nodes.is_empty() {
        return false;
    }

    let lookup = WasmCatalogLookup::new(app_wasm_nodes, builtin_nodes);

    let mut changed = false;
    for node in board.nodes.values_mut() {
        changed |= hydrate_node_wasm_metadata(node, &lookup);
    }

    for layer in board.layers.values_mut() {
        for node in layer.nodes.values_mut() {
            changed |= hydrate_node_wasm_metadata(node, &lookup);
        }
    }

    changed
}

pub fn sanitize_wasm_command_metadata(
    commands: &mut [GenericCommand],
    app_wasm_nodes: &[Node],
    builtin_nodes: &[Node],
) -> Result<(), ApiError> {
    let lookup = WasmCatalogLookup::new(app_wasm_nodes, builtin_nodes);
    for command in commands {
        sanitize_command_wasm_metadata(command, &lookup)?;
    }

    Ok(())
}

fn hydrate_node_wasm_metadata(node: &mut Node, lookup: &WasmCatalogLookup<'_>) -> bool {
    let Ok(catalog_node) = lookup.resolve(node) else {
        return false;
    };
    let Some(catalog_node) = catalog_node else {
        return false;
    };

    apply_catalog_wasm_metadata(node, catalog_node)
}

fn sanitize_command_wasm_metadata(
    command: &mut GenericCommand,
    lookup: &WasmCatalogLookup<'_>,
) -> Result<(), ApiError> {
    match command {
        GenericCommand::AddNode(command) => sanitize_node_wasm_metadata(&mut command.node, lookup)?,
        GenericCommand::UpdateNode(command) => {
            sanitize_node_wasm_metadata(&mut command.node, lookup)?
        }
        GenericCommand::CopyPaste(command) => {
            for node in &mut command.original_nodes {
                sanitize_node_wasm_metadata(node, lookup)?;
            }
            for node in &mut command.new_nodes {
                sanitize_node_wasm_metadata(node, lookup)?;
            }
            for layer in &mut command.original_layers {
                for node in layer.nodes.values_mut() {
                    sanitize_node_wasm_metadata(node, lookup)?;
                }
            }
            for layer in &mut command.new_layers {
                for node in layer.nodes.values_mut() {
                    sanitize_node_wasm_metadata(node, lookup)?;
                }
            }
        }
        _ => {}
    }

    Ok(())
}

fn sanitize_node_wasm_metadata(
    node: &mut Node,
    lookup: &WasmCatalogLookup<'_>,
) -> Result<(), ApiError> {
    let Some(catalog_node) = lookup.resolve(node)? else {
        return Ok(());
    };

    apply_catalog_wasm_metadata(node, catalog_node);
    Ok(())
}

fn apply_catalog_wasm_metadata(node: &mut Node, catalog_node: &Node) -> bool {
    let Some(catalog_wasm) = &catalog_node.wasm else {
        return false;
    };
    let needs_update = match node.wasm.as_ref() {
        Some(existing) => {
            existing.package_id != catalog_wasm.package_id
                || existing.permissions != catalog_wasm.permissions
        }
        None => true,
    };

    if !needs_update {
        return false;
    }

    node.wasm = Some(catalog_wasm.clone());
    true
}

struct WasmCatalogLookup<'a> {
    builtin_names: HashSet<&'a str>,
    wasm_by_name: HashMap<&'a str, &'a Node>,
    wasm_by_package_and_name: HashMap<(&'a str, &'a str), &'a Node>,
}

impl<'a> WasmCatalogLookup<'a> {
    fn new(app_wasm_nodes: &'a [Node], builtin_nodes: &'a [Node]) -> Self {
        let builtin_names: HashSet<&str> = builtin_nodes
            .iter()
            .map(|node| node.name.as_str())
            .collect();
        let mut wasm_by_name: HashMap<&str, &Node> = HashMap::with_capacity(app_wasm_nodes.len());
        let mut wasm_by_package_and_name: HashMap<(&str, &str), &Node> =
            HashMap::with_capacity(app_wasm_nodes.len());

        for node in app_wasm_nodes {
            let Some(wasm) = &node.wasm else {
                continue;
            };
            wasm_by_name.entry(node.name.as_str()).or_insert(node);
            wasm_by_package_and_name.insert((wasm.package_id.as_str(), node.name.as_str()), node);
        }

        Self {
            builtin_names,
            wasm_by_name,
            wasm_by_package_and_name,
        }
    }

    fn resolve(&self, node: &Node) -> Result<Option<&'a Node>, ApiError> {
        if let Some(wasm) = &node.wasm {
            return self
                .wasm_by_package_and_name
                .get(&(wasm.package_id.as_str(), node.name.as_str()))
                .copied()
                .map(Some)
                .ok_or_else(|| {
                    ApiError::bad_request(format!(
                        "WASM node '{}' references package '{}', but that package/node pair is not linked to this app",
                        node.name, wasm.package_id
                    ))
                });
        }

        if self.builtin_names.contains(node.name.as_str()) {
            return Ok(None);
        }

        Ok(self.wasm_by_name.get(node.name.as_str()).copied())
    }
}
