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
use flow_like_wasm_schema::manifest::PackageNodeEntry;
use sea_orm::{ColumnTrait, Condition, DatabaseConnection, EntityTrait, QueryFilter};
use std::sync::Arc;

/// An app's WASM node catalog together with a token that changes whenever the catalog does.
pub struct AppWasmNodes {
    pub nodes: Vec<Node>,
    /// Empty for apps without packages, otherwise a digest of every (package, node, version).
    pub fingerprint: String,
}

/// [`app_wasm_nodes`] behind a per-app cache keyed by the app's package pins.
///
/// The board sync endpoint needs this on every poll and the mutation path on every write; without
/// a cache each call pays a second database round trip that pulls every pinned package's node
/// definitions as JSON. The key embeds a digest of the installed `(package, version)` set, so an
/// install, update or removal is observed immediately and on every replica — a TTL would instead
/// keep serving the previous catalog to someone who just added a package and went straight into a
/// board, and no invalidation can reach the instances a Lambda deployment is holding.
pub async fn app_wasm_nodes_cached(
    state: &AppState,
    app_id: &str,
) -> Result<Arc<AppWasmNodes>, ApiError> {
    let packages = app_packages(state, app_id).await?;
    let key = format!("{app_id}\u{1f}{}", packages_epoch(&packages));
    if let Some(cached) = state.app_wasm_nodes_cache.get(&key) {
        return Ok(cached);
    }
    let nodes = wasm_nodes_for_packages(state, &packages).await?;
    let fingerprint = if nodes.is_empty() {
        String::new()
    } else {
        let mut hasher = blake3::Hasher::new();
        let mut identities: Vec<String> = nodes
            .iter()
            .map(|node| {
                format!(
                    "{}\u{1f}{}\u{1f}{:?}",
                    node.wasm
                        .as_ref()
                        .map(|wasm| wasm.package_id.as_str())
                        .unwrap_or_default(),
                    node.name,
                    node.version
                )
            })
            .collect();
        identities.sort();
        for identity in identities {
            hasher.update(identity.as_bytes());
            hasher.update(b"\0");
        }
        hasher.finalize().to_hex().to_string()
    };
    let entry = Arc::new(AppWasmNodes { nodes, fingerprint });
    state.app_wasm_nodes_cache.insert(key, entry.clone());
    Ok(entry)
}

/// The app's pinned, non-stale packages. This is the cheap half of a catalog resolve; the
/// expensive half is [`wasm_nodes_for_packages`], which the cache is there to skip.
async fn app_packages(state: &AppState, app_id: &str) -> Result<Vec<app_package::Model>, ApiError> {
    Ok(app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(app_id))
        .filter(app_package::Column::Stale.eq(false))
        .all(&state.db)
        .await?)
}

/// Identity of an app's package pins: any install, removal or version change moves it.
fn packages_epoch(packages: &[app_package::Model]) -> String {
    if packages.is_empty() {
        return String::new();
    }
    let mut pins: Vec<String> = packages
        .iter()
        .map(|pkg| format!("{}\u{1f}{}", pkg.package_id, pkg.version))
        .collect();
    pins.sort();
    let mut hasher = blake3::Hasher::new();
    for pin in pins {
        hasher.update(pin.as_bytes());
        hasher.update(b"\0");
    }
    hasher.finalize().to_hex().to_string()
}

pub async fn app_wasm_nodes(state: &AppState, app_id: &str) -> Result<Vec<Node>, ApiError> {
    let packages = app_packages(state, app_id).await?;
    wasm_nodes_for_packages(state, &packages).await
}

async fn wasm_nodes_for_packages(
    state: &AppState,
    packages: &[app_package::Model],
) -> Result<Vec<Node>, ApiError> {
    let pins: Vec<(String, String)> = packages
        .iter()
        .map(|pkg| (pkg.package_id.clone(), pkg.version.clone()))
        .collect();
    wasm_nodes_for_pins(&state.db, &pins).await
}

/// The `Node` definitions of exactly these `(package_id, version)` pins, from
/// the `nodes` column the compilation callback stored when the compiler
/// workload ran the module's `get_nodes`. This is the API's only view of a WASM
/// node: it never loads the module, and the definitions it rebuilds are
/// fingerprint-identical to the ones the executor derives from the running
/// module (see `both_node_builders_agree_on_every_fingerprint_input`), which is
/// what lets the API compile boards the executor will accept.
pub async fn wasm_nodes_for_pins(
    db: &DatabaseConnection,
    pins: &[(String, String)],
) -> Result<Vec<Node>, ApiError> {
    if pins.is_empty() {
        return Ok(Vec::new());
    }

    let mut pinned = Condition::any();
    for (package_id, version) in pins {
        pinned = pinned.add(
            Condition::all()
                .add(wasm_package_version::Column::PackageId.eq(package_id))
                .add(wasm_package_version::Column::Version.eq(version)),
        );
    }

    let mut nodes_by_pin: HashMap<(String, String), serde_json::Value> =
        wasm_package_version::Entity::find()
            .filter(pinned)
            .all(db)
            .await?
            .into_iter()
            .map(|record| ((record.package_id, record.version), record.nodes))
            .collect();

    let mut wasm_nodes: Vec<Node> = Vec::with_capacity(pins.len() * 5);

    for (package_id, version) in pins {
        let Some(nodes_value) = nodes_by_pin.remove(&(package_id.clone(), version.clone())) else {
            tracing::warn!(
                package_id = %package_id,
                version = %version,
                "app catalog: pinned WASM package version not found; skipping its nodes"
            );
            continue;
        };

        let entries: Vec<PackageNodeEntry> =
            serde_json::from_value(nodes_value).unwrap_or_default();

        for entry in &entries {
            wasm_nodes.push(package_node_to_node(entry, package_id));
        }
    }

    Ok(wasm_nodes)
}

fn package_node_to_node(entry: &PackageNodeEntry, package_id: &str) -> Node {
    let mut node = Node {
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
        // `build_node_from_definition` builds the same node for the executor
        // and leaves this unset; the two must agree or the registry
        // fingerprints they produce — and with them any compiled board
        // artifact bound to one — diverge. Entries written before
        // `definition_to_package_entry` stopped storing the module's ABI
        // version here still carry one, so the stored value is not read.
        version: None,
        wasm: Some(NodeWasm {
            package_id: package_id.to_string(),
            permissions: entry.permissions.clone(),
        }),
        namespace: None,
        alias: None,
        receiver: None,
    };
    node.ensure_flowscript_names();
    node
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

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_wasm::abi::{WasmNodeDefinition, WasmPinDefinition};
    use flow_like_wasm::{build_node_from_definition, definition_to_package_entry};

    fn definition() -> WasmNodeDefinition {
        WasmNodeDefinition {
            name: "youtube_transcript".to_string(),
            friendly_name: "YouTube Transcript".to_string(),
            description: "Fetch a transcript".to_string(),
            category: "Web".to_string(),
            icon: Some("video".to_string()),
            pins: vec![
                WasmPinDefinition {
                    name: "video_url".to_string(),
                    friendly_name: "Video URL".to_string(),
                    description: "The video to transcribe".to_string(),
                    pin_type: "Input".to_string(),
                    data_type: "String".to_string(),
                    default_value: None,
                    value_type: Some("Normal".to_string()),
                    schema: None,
                    valid_values: None,
                    range: None,
                    step: None,
                    sensitive: None,
                    enforce_schema: None,
                    enforce_generic_value_type: None,
                },
                WasmPinDefinition {
                    name: "transcript".to_string(),
                    friendly_name: "Transcript".to_string(),
                    description: "The transcript text".to_string(),
                    pin_type: "Output".to_string(),
                    data_type: "String".to_string(),
                    default_value: None,
                    value_type: Some("Normal".to_string()),
                    schema: None,
                    valid_values: None,
                    range: None,
                    step: None,
                    sensitive: None,
                    enforce_schema: None,
                    enforce_generic_value_type: None,
                },
            ],
            scores: None,
            long_running: None,
            docs: None,
            // A module built against a non-default host ABI: the case that used
            // to push the two builders apart.
            abi_version: Some(3),
            permissions: vec![],
        }
    }

    /// The executor overlays WASM nodes onto its registry via
    /// `build_node_from_definition`; the API reconstructs the same nodes from
    /// the stored `PackageNodeEntry`. `FlowNodeRegistryInner::fingerprint`
    /// hashes name, version and `semantic_hash` per node, so any disagreement
    /// on those three yields two different fingerprints for one package — and
    /// a compiled artifact one side writes that the other always rejects.
    #[test]
    fn both_node_builders_agree_on_every_fingerprint_input() {
        let definition = definition();

        let executor_node = build_node_from_definition(&definition);
        let api_node = package_node_to_node(
            &definition_to_package_entry(&definition),
            "com.flow-like.youtube",
        );

        assert_eq!(executor_node.name, api_node.name);
        assert_eq!(executor_node.version, api_node.version);
        assert_eq!(executor_node.semantic_hash(), api_node.semantic_hash());
    }

    #[test]
    fn abi_version_never_becomes_a_node_schema_version() {
        let definition = definition();
        assert_eq!(definition.abi_version, Some(3));

        assert_eq!(definition_to_package_entry(&definition).version, None);
        assert_eq!(build_node_from_definition(&definition).version, None);
    }
}
