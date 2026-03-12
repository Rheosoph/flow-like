use std::collections::HashSet;

use flow_like::app::App;
use flow_like::flow::node::Node;
use tauri::AppHandle;

use crate::{functions::TauriFunctionError, state::TauriFlowLikeState};

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

    let app = App::load(app_id, state.clone()).await?;
    let allowed_packages: HashSet<&String> = app.packages.keys().collect();

    let filtered = all_nodes
        .into_iter()
        .filter(|node| match &node.wasm {
            None => true,
            Some(wasm) => allowed_packages.contains(&wasm.package_id),
        })
        .collect();

    Ok(filtered)
}
