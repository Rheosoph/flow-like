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
                    match ontologies
                        .into_iter()
                        .map(graph_overlay_from_def)
                        .collect::<flow_like_types::Result<Vec<_>>>()
                    {
                        Ok(ontologies) => {
                            let bindings =
                                flow_like_catalog::ontology_binding_nodes(&ontologies, &filtered);
                            filtered.extend(bindings);
                        }
                        Err(error) => tracing::warn!(
                            app_id,
                            %error,
                            "Could not decode Data Studio bindings; returning the base catalog"
                        ),
                    }
                }
                Err(error) => tracing::warn!(
                    app_id,
                    %error,
                    "Could not load Data Studio bindings; returning the base catalog"
                ),
            }
            match imports {
                Ok(imports) => {
                    match imports
                        .into_iter()
                        .map(remote_ontology_import_from_def)
                        .collect::<flow_like_types::Result<Vec<_>>>()
                    {
                        Ok(imports) => {
                            let bindings = flow_like_catalog::remote_ontology_binding_nodes(
                                &imports, &filtered,
                            );
                            filtered.extend(bindings);
                        }
                        Err(error) => tracing::warn!(
                            app_id,
                            %error,
                            "Could not decode remote Data Studio bindings; returning local bindings"
                        ),
                    }
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
