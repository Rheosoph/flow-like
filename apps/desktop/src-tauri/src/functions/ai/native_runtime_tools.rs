//! Runtime tools use the same services as interactive execution without a webview.

use super::{
    frontend_tool_bridge::FrontendToolContext, native_platform_tools::NativePlatformSession,
};
use serde_json::Value;
use tauri::AppHandle;

pub(crate) mod execution;
mod graph;
mod inspection;

pub(super) fn supports(tool: &str) -> bool {
    matches!(
        tool,
        "graph_overlay_tool"
            | "graph_query_tool"
            | "graph_element_tool"
            | "ontology_action_tool"
            | "execute_event"
            | "execute_node"
            | "query_execution_logs"
            | "run_board_tests"
            | "ui_inspect"
    )
}

pub(super) async fn execute(
    app: &AppHandle,
    context: Option<&FrontendToolContext>,
    tool: &str,
    args: &Value,
) -> Result<Value, String> {
    let session = NativePlatformSession::new(app, context).await?;
    let result = match tool {
        "graph_overlay_tool"
        | "graph_query_tool"
        | "graph_element_tool"
        | "ontology_action_tool" => graph::execute(&session, tool, args).await,
        "ui_inspect" => inspection::execute(&session, args).await,
        _ => execution::execute(&session, tool, args).await,
    }?;
    session.assert_current_profile().await?;
    Ok(result)
}
