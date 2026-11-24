use std::sync::{Arc, Mutex, PoisonError};

use flow_like::app::App;
use flow_like::flow::node::Node;
use flow_like::flow_like_storage::databases::graph::lancegraph::{
    self, GraphOverlayDef, RemoteOntologyImportDef,
};
use flow_like::state::FlowNodeRegistryInner;
use flow_like_catalog::wire::{NodeIds, catalog_document, encode_section};
use tauri::AppHandle;

use crate::{
    functions::{
        TauriFunctionError,
        app::graph::{graph_connection, graph_overlay_from_def},
    },
    state::TauriFlowLikeState,
};

fn remote_ontology_import_from_def(
    definition: RemoteOntologyImportDef,
) -> flow_like_types::Result<flow_like_catalog::RemoteOntologyImport> {
    let value = flow_like_types::json::to_value(definition)?;
    Ok(flow_like_types::json::from_value(value)?)
}

/// The built-in nodes as they cross the bridge. Encoding them is most of the cost of a catalog
/// read and only changes with the registry, so the result is kept until its fingerprint moves.
/// It is the same for every app: an app's catalog names it by digest, and the webview asks for
/// the document itself only when it does not hold that digest yet.
#[derive(Clone)]
struct BuiltInCatalog {
    fingerprint: [u8; 32],
    digest: String,
    document: Arc<str>,
}

static BUILT_IN_CATALOG: Mutex<Option<BuiltInCatalog>> = Mutex::new(None);

fn built_in_catalog(registry: &FlowNodeRegistryInner) -> Result<BuiltInCatalog, serde_json::Error> {
    let fingerprint = registry.fingerprint();
    let mut cached = BUILT_IN_CATALOG
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(catalog) = cached.as_ref()
        && catalog.fingerprint == fingerprint
    {
        return Ok(catalog.clone());
    }

    let nodes = registry.get_nodes_shared();
    let built_in = nodes
        .iter()
        .filter(|node| node.wasm.is_none())
        .collect::<Vec<_>>();
    let section = serde_json::to_string(&encode_section(&built_in, NodeIds::Unstable)?)?;
    let digest = blake3::hash(section.as_bytes()).to_hex().to_string();
    let catalog = BuiltInCatalog {
        fingerprint,
        document: catalog_document(&digest, None, &section).into(),
        digest,
    };
    *cached = Some(catalog.clone());
    Ok(catalog)
}

/// One undecodable overlay must not drop every other ontology's binding nodes from the palette.
fn local_presets(app_id: &str, definitions: Vec<GraphOverlayDef>, catalog: &[Node]) -> Vec<Node> {
    let ontologies = definitions
        .into_iter()
        .filter_map(|def| {
            let overlay_id = def.id.clone();
            match graph_overlay_from_def(def) {
                Ok(overlay) => Some(overlay),
                Err(error) => {
                    tracing::warn!(
                        app_id,
                        overlay_id,
                        %error,
                        "Skipping undecodable ontology in Data Studio bindings"
                    );
                    None
                }
            }
        })
        .collect::<Vec<_>>();
    flow_like_catalog::ontology_binding_nodes(&ontologies, catalog)
}

fn remote_presets(
    app_id: &str,
    definitions: Vec<RemoteOntologyImportDef>,
    catalog: &[Node],
) -> Vec<Node> {
    let imports = definitions
        .into_iter()
        .filter_map(|def| {
            let import_id = def.id.clone();
            match remote_ontology_import_from_def(def) {
                Ok(import) => Some(import),
                Err(error) => {
                    tracing::warn!(
                        app_id,
                        import_id,
                        %error,
                        "Skipping undecodable remote ontology import"
                    );
                    None
                }
            }
        })
        .collect::<Vec<_>>();
    flow_like_catalog::remote_ontology_binding_nodes(&imports, catalog)
}

/// The presets an app's Data Studio ontologies add to the palette. A project database that
/// cannot be read leaves the catalog without them rather than failing it.
async fn data_studio_presets(handler: &AppHandle, app_id: &str, catalog: &[Node]) -> Vec<Node> {
    let connection = match graph_connection(handler, app_id, false).await {
        Ok(connection) => connection,
        Err(error) => {
            tracing::warn!(
                app_id,
                %error,
                "Could not open the project database for Data Studio bindings"
            );
            return Vec::new();
        }
    };
    let (ontologies, imports) = flow_like_types::tokio::join!(
        lancegraph::list_overlays(&connection),
        lancegraph::list_ontology_imports(&connection)
    );

    let mut presets = match ontologies {
        Ok(definitions) => local_presets(app_id, definitions, catalog),
        Err(error) => {
            tracing::warn!(
                app_id,
                %error,
                "Could not load Data Studio bindings; returning the base catalog"
            );
            Vec::new()
        }
    };
    match imports {
        Ok(definitions) => presets.extend(remote_presets(app_id, definitions, catalog)),
        Err(error) => tracing::warn!(
            app_id,
            %error,
            "Could not load remote Data Studio bindings; returning local bindings"
        ),
    }
    presets
}

/// A catalog document in the layout of `flow_like_catalog::wire`, which the webview expands
/// with `decodeCatalog`: what `app_id` adds to the built-in nodes (every registered package
/// node when no app is given), or with `built_in` the built-in nodes themselves.
#[tauri::command(async)]
pub async fn get_catalog(
    handler: AppHandle,
    app_id: Option<String>,
    built_in: Option<bool>,
) -> Result<tauri::ipc::Response, TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&handler).await?;
    let registry = state.node_registry.read().await.node_registry.clone();
    let catalog = built_in_catalog(&registry)?;
    if built_in.unwrap_or(false) {
        return Ok(tauri::ipc::Response::new(catalog.document.to_string()));
    }

    let nodes = registry.get_nodes_shared();
    let package_nodes = nodes.iter().filter(|node| node.wasm.is_some());
    let extras = match &app_id {
        None => package_nodes.cloned().collect::<Vec<_>>(),
        Some(app_id) => {
            let app = App::load(app_id.clone(), state.clone()).await?;
            let mut extras = package_nodes
                .filter(|node| {
                    node.wasm
                        .as_ref()
                        .is_some_and(|wasm| app.packages.contains_key(&wasm.package_id))
                })
                .cloned()
                .collect::<Vec<_>>();
            extras.extend(data_studio_presets(&handler, app_id, &nodes).await);
            extras
        }
    };
    let extras = serde_json::to_string(&encode_section(&extras, NodeIds::Stable)?)?;

    Ok(tauri::ipc::Response::new(catalog_document(
        &catalog.digest,
        None,
        &extras,
    )))
}
