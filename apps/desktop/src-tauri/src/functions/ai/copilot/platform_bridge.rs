//! Platform tool policy and frontend bridge dispatch.

use super::runtime::steering_messages;
use async_trait::async_trait;
use flow_like::flow::copilot::platform::PlatformToolBridge;
use flow_like_types::{channel::Channel as _, tokio_util::sync::CancellationToken};
use std::time::Duration;

/// Select which shared tool-spec surface validates and authorizes calls before they cross the
/// desktop frontend bridge. Board copilots intentionally use the scoped runtime definitions:
/// their `app_id` is injected by `FrontendToolContext`, whereas the global definitions require the
/// model to provide it explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FrontendPlatformToolSet {
    Global,
    BoardRuntime,
    DataStudio,
    Scout,
    Home,
    HomeReadOnly,
    BotTools,
}

pub(super) fn frontend_platform_tool_spec(
    tool_set: FrontendPlatformToolSet,
    tool_name: &str,
) -> Option<flow_like::flow::copilot::tool_spec::PlatformToolSpec> {
    use flow_like::flow::copilot::tool_spec::{
        find_cross_board_source_tool_spec, find_data_studio_tool_spec, find_global_tool_spec,
        find_home_tool_spec, find_home_tool_spec_for_access, find_runtime_execution_tool_spec,
        find_scout_tool_spec, find_workflow_context_tool_spec,
    };

    match tool_set {
        FrontendPlatformToolSet::Global => find_global_tool_spec(tool_name),
        FrontendPlatformToolSet::BoardRuntime => find_runtime_execution_tool_spec(tool_name)
            .or_else(|| find_workflow_context_tool_spec(tool_name))
            .or_else(|| find_cross_board_source_tool_spec(tool_name)),
        // The specialist sets are exactly what their loop advertises, so a tool outside them has no
        // spec here and is rejected as unadvertised rather than silently dispatched.
        FrontendPlatformToolSet::DataStudio => find_data_studio_tool_spec(tool_name),
        FrontendPlatformToolSet::Scout => find_scout_tool_spec(tool_name),
        FrontendPlatformToolSet::Home => find_home_tool_spec(tool_name),
        FrontendPlatformToolSet::HomeReadOnly => find_home_tool_spec_for_access(tool_name, true),
        FrontendPlatformToolSet::BotTools => {
            flow_like::flow::copilot::find_bot_tool_spec(tool_name)
        }
    }
}

pub(super) fn global_orchestrator_tool_scope_error(
    tool_set: FrontendPlatformToolSet,
    tool_name: &str,
) -> Option<String> {
    use flow_like::flow::copilot::tool_spec::{
        ARCHIVE_LOOKUP_TOOL, HTTP_REQUEST_TOOL, INTERNET_SEARCH_TOOL, OPEN_URL_TOOL,
    };

    (tool_set != FrontendPlatformToolSet::Global
        && matches!(
            tool_name,
            INTERNET_SEARCH_TOOL | OPEN_URL_TOOL | ARCHIVE_LOOKUP_TOOL | HTTP_REQUEST_TOOL
        ))
    .then(|| {
        serde_json::json!({
            "status": "error",
            "code": "global_orchestrator_tool_only",
            "tool": tool_name,
            "message": "Direct web and HTTP tools are available only to the top-level FlowPilot orchestrator."
        })
        .to_string()
    })
}

/// Desktop implementation of the platform tool bridge. Calls are validated and assigned an
/// approval policy from the selected shared spec set, then routed over the configured Tauri event
/// without blocking an async runtime worker.
pub(super) struct DesktopPlatformBridge {
    pub(super) bridge: crate::functions::ai::frontend_tool_bridge::FrontendToolBridge,
    pub(super) tool_set: FrontendPlatformToolSet,
    pub(super) cancellation: CancellationToken,
    /// True for global-chat runs, which drain steering text pushed onto their channel;
    /// Nested specialist runs share a channel with their owner and must not consume its inbox.
    pub(super) steerable: bool,
}

#[async_trait]
impl PlatformToolBridge for DesktopPlatformBridge {
    async fn drain_steering(&self) -> Vec<String> {
        if !self.steerable {
            return Vec::new();
        }
        steering_messages(self.bridge.channel().drain_inbound().await)
    }

    async fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled() || self.bridge.channel().is_cancelled().await
    }

    async fn call(&self, tool_name: &str, arguments: serde_json::Value) -> String {
        use crate::functions::ai::copilot_sdk_tools::approval_from_spec;
        use flow_like::flow::copilot::tool_spec::{HTTP_REQUEST_TOOL, missing_required_args};

        // Do not even enqueue a frontend event after the owning model run has ended. Cancellation
        // is checked again inside the blocking bridge scope to close the race after this preflight.
        if self.cancellation.is_cancelled() {
            return serde_json::json!({
                "status": "cancelled",
                "tool": tool_name,
                "message": "The owning FlowPilot run was cancelled before this tool could execute."
            })
            .to_string();
        }
        if let Some(error) = global_orchestrator_tool_scope_error(self.tool_set, tool_name) {
            return error;
        }
        let Some(spec) = frontend_platform_tool_spec(self.tool_set, tool_name) else {
            return serde_json::json!({
                "status": "error",
                "code": "platform_tool_not_advertised",
                "tool": tool_name,
                "retryable": false,
                "message": "This tool is unavailable in the active FlowPilot surface and was not executed."
            })
            .to_string();
        };

        // Reject calls with missing required arguments before any approval dialog or dispatch,
        // so the model retries with complete arguments (same guard as the SDK/MCP backends).
        if let Some(error) = missing_required_args(&spec, &arguments) {
            return serde_json::json!({ "status": "error", "error": error }).to_string();
        }

        // Approval + timeout come from the shared platform tool spec, so the Bits path enforces
        // exactly the same policy as the Copilot SDK / MCP backends.
        let approval = approval_from_spec(&spec, &arguments);
        let timeout = Duration::from_secs(spec.timeout_secs);
        let http_deadline = std::time::Instant::now().checked_add(timeout);

        let bridge = self.bridge.clone();
        let name = tool_name.to_string();
        let cancellation = self.cancellation.clone();
        let http_arguments = (tool_name == HTTP_REQUEST_TOOL).then(|| arguments.clone());
        match tokio::task::spawn_blocking(move || {
            crate::functions::ai::frontend_tool_bridge::with_frontend_tool_execution_scope(
                cancellation,
                None,
                || bridge.call_with_timeout(name, arguments, approval, timeout),
            )
        })
        .await
        {
            Ok(value) => {
                let value = if let Some(arguments) = http_arguments {
                    crate::functions::ai::frontend_tool_bridge::complete_http_request_approval(
                        &arguments,
                        value,
                        self.is_cancelled().await,
                        crate::functions::ai::frontend_tool_bridge::http_request_cancellation(
                            self.bridge.channel().clone(),
                            Some(self.cancellation.clone()),
                            http_deadline,
                        ),
                    )
                    .await
                } else {
                    value
                };
                serde_json::to_string(&value)
                    .unwrap_or_else(|_| "{\"status\":\"error\"}".to_string())
            }
            Err(err) => {
                serde_json::json!({ "status": "error", "error": err.to_string() }).to_string()
            }
        }
    }
}
