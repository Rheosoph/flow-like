use std::collections::HashSet;

use flow_like::app::App;
use flow_like::flow::node::Node;
use tauri::AppHandle;

use crate::{
    functions::{
        TauriFunctionError,
        app::graph::{graph_connection, graph_overlay_from_def},
    },
    state::TauriFlowLikeState,
};

fn remote_ontology_import_from_def(
    definition: flow_like::flow_like_storage::databases::graph::lancegraph::RemoteOntologyImportDef,
) -> flow_like_types::Result<flow_like_catalog::RemoteOntologyImport> {
    let value = flow_like_types::json::to_value(definition)?;
    Ok(flow_like_types::json::from_value(value)?)
}

#[tauri::command(async)]
pub async fn get_catalog(
    handler: AppHandle,
    app_id: Option<String>,
) -> Result<Vec<Node>, TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&handler).await?;
    let all_nodes = state.node_registry.read().await.get_nodes()?;

    let Some(app_id) = app_id else {
        return Ok(all_nodes);
    };

    let app = App::load(app_id.clone(), state.clone()).await?;
    let allowed_packages: HashSet<&String> = app.packages.keys().collect();

    let mut filtered = all_nodes
        .into_iter()
        .filter(|node| match &node.wasm {
            None => true,
            Some(wasm) => allowed_packages.contains(&wasm.package_id),
        })
        .collect::<Vec<_>>();

    match graph_connection(&handler, &app_id, false).await {
        Ok(connection) => {
            let (ontologies, imports) = flow_like_types::tokio::join!(
                flow_like::flow_like_storage::databases::graph::lancegraph::list_overlays(
                    &connection
                ),
                flow_like::flow_like_storage::databases::graph::lancegraph::list_ontology_imports(
                    &connection
                )
            );
            match ontologies {
                Ok(ontologies) => {
                    // One undecodable overlay must not drop every other
                    // ontology's binding nodes from the palette.
                    let ontologies = ontologies
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
                    let bindings =
                        flow_like_catalog::ontology_binding_nodes(&ontologies, &filtered);
                    filtered.extend(bindings);
                }
                Err(error) => tracing::warn!(
                    app_id,
                    %error,
                    "Could not load Data Studio bindings; returning the base catalog"
                ),
            }
            match imports {
                Ok(imports) => {
                    let imports = imports
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
                    let bindings =
                        flow_like_catalog::remote_ontology_binding_nodes(&imports, &filtered);
                    filtered.extend(bindings);
                }
                Err(error) => tracing::warn!(
                    app_id,
                    %error,
                    "Could not load remote Data Studio bindings; returning local bindings"
                ),
            }
        }
        Err(error) => tracing::warn!(
            app_id,
            %error,
            "Could not open the project database for Data Studio bindings"
        ),
    }

    Ok(filtered)
}
