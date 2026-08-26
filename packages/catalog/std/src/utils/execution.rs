use flow_like::flow::{
    execution::{ExecutionEnvironment, ExecutionMode, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, async_trait, json::json};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionEnvironmentInfo {
    pub environment: String,
    pub execution_mode: String,
    pub run_id: String,
    pub app_id: Option<String>,
    pub board_id: Option<String>,
    pub node_id: String,
    pub user_id: Option<String>,
    /// Deprecated alias for user_id kept for older boards.
    pub user_sub: Option<String>,
    pub event_id: Option<String>,
    pub is_desktop: bool,
    pub is_server: bool,
    pub is_mobile: bool,
    pub is_browser_sandbox: bool,
    pub is_local: bool,
    pub is_remote: bool,
    pub is_event: bool,
    pub is_scheduled: bool,
    pub stream_state: bool,
    pub has_user_context: bool,
    pub is_technical_user: bool,
    pub token_available: bool,
    pub hub: Option<String>,
}

fn execution_scores() -> NodeScores {
    NodeScores::new()
        .set_privacy(4)
        .set_security(8)
        .set_performance(10)
        .set_governance(8)
        .set_reliability(10)
        .set_cost(10)
        .build()
}

fn user_id(context: &ExecutionContext) -> Option<String> {
    context
        .user_context()
        .and_then(|context| {
            let sub = context.sub.trim();
            if sub.is_empty() {
                None
            } else {
                Some(sub.to_string())
            }
        })
        .or_else(|| {
            context.execution_cache.as_ref().and_then(|cache| {
                let sub = cache.sub.trim();
                if sub.is_empty() {
                    None
                } else {
                    Some(sub.to_string())
                }
            })
        })
}

async fn execution_info(
    context: &ExecutionContext,
) -> flow_like_types::Result<ExecutionEnvironmentInfo> {
    let environment = context.execution_environment();
    let execution_mode = context.execution_mode();
    let user_context = context.user_context().cloned();
    let user_id = user_id(context);
    let hub = if context.profile.hub.trim().is_empty() {
        None
    } else {
        Some(context.profile.hub.clone())
    };

    Ok(ExecutionEnvironmentInfo {
        environment: environment.as_str().to_string(),
        execution_mode: execution_mode.as_str().to_string(),
        run_id: context.run_id().to_string(),
        app_id: context
            .execution_cache
            .as_ref()
            .map(|cache| cache.app_id.clone()),
        board_id: context
            .execution_cache
            .as_ref()
            .map(|cache| cache.board_id.clone()),
        node_id: context.id.to_string(),
        user_id: user_id.clone(),
        user_sub: user_id,
        event_id: context.event_id().await,
        is_desktop: matches!(environment, ExecutionEnvironment::Desktop),
        is_server: matches!(environment, ExecutionEnvironment::Server),
        is_mobile: matches!(environment, ExecutionEnvironment::Mobile),
        is_browser_sandbox: matches!(environment, ExecutionEnvironment::BrowserSandbox),
        is_local: environment.is_local(),
        is_remote: !environment.is_local(),
        is_event: matches!(
            execution_mode,
            ExecutionMode::Event | ExecutionMode::Scheduled
        ),
        is_scheduled: matches!(execution_mode, ExecutionMode::Scheduled),
        stream_state: context.stream_state,
        has_user_context: user_context.is_some(),
        is_technical_user: user_context
            .as_ref()
            .map(|context| context.is_technical())
            .unwrap_or(false),
        token_available: context
            .token
            .as_ref()
            .map(|token| !token.trim().is_empty())
            .unwrap_or(false),
        hub,
    })
}

/// Determine whether the current run is executing locally in the desktop app or on the server.
#[crate::register_node]
#[derive(Default)]
pub struct GetExecutionEnvironmentNode {}

impl GetExecutionEnvironmentNode {
    pub fn new() -> Self {
        GetExecutionEnvironmentNode {}
    }
}

#[async_trait]
impl NodeLogic for GetExecutionEnvironmentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_execution_get_environment",
            "Get Execution Environment",
            "Returns where and how the current run is executing.",
            "Utils/Execution",
        );
        node.set_flowscript_name("execution", "getEnvironment");
        node.add_icon("/flow/icons/computer.svg");

        node.add_output_pin(
            "environment",
            "Environment",
            "The execution environment: local, desktop, mobile, browser_sandbox, or server",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "local".to_string(),
                    "desktop".to_string(),
                    "mobile".to_string(),
                    "browser_sandbox".to_string(),
                    "server".to_string(),
                ])
                .build(),
        );

        node.add_output_pin(
            "execution_mode",
            "Execution Mode",
            "The execution mode: sync, async, event, or scheduled",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "sync".to_string(),
                    "async".to_string(),
                    "event".to_string(),
                    "scheduled".to_string(),
                ])
                .build(),
        );

        node.add_output_pin(
            "is_desktop",
            "Is Desktop",
            "True when the run is executing locally in the desktop app",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "is_server",
            "Is Server",
            "True when the run is executing on the server",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "is_mobile",
            "Is Mobile",
            "True when the run is executing on a mobile runtime",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "is_browser_sandbox",
            "Is Browser Sandbox",
            "True when the run is executing in a browser sandbox runtime",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "is_local",
            "Is Local",
            "True when the run has local/offline execution context",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "is_remote",
            "Is Remote",
            "True when the run does not have local/offline execution context",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "run_id",
            "Run ID",
            "Current run identifier",
            VariableType::String,
        );

        node.add_output_pin(
            "app_id",
            "App ID",
            "Current app identifier, if available",
            VariableType::String,
        );

        node.add_output_pin(
            "user_id",
            "User ID",
            "Current user identifier, if available",
            VariableType::String,
        );

        node.add_output_pin(
            "details",
            "Details",
            "Structured execution environment details",
            VariableType::Struct,
        )
        .set_schema::<ExecutionEnvironmentInfo>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_scores(execution_scores());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let details = execution_info(context).await?;

        context
            .set_pin_value("environment", json!(details.environment))
            .await?;
        context
            .set_pin_value("execution_mode", json!(details.execution_mode))
            .await?;
        context
            .set_pin_value("is_desktop", json!(details.is_desktop))
            .await?;
        context
            .set_pin_value("is_server", json!(details.is_server))
            .await?;
        context
            .set_pin_value("is_mobile", json!(details.is_mobile))
            .await?;
        context
            .set_pin_value("is_browser_sandbox", json!(details.is_browser_sandbox))
            .await?;
        context
            .set_pin_value("is_local", json!(details.is_local))
            .await?;
        context
            .set_pin_value("is_remote", json!(details.is_remote))
            .await?;
        context
            .set_pin_value("run_id", json!(details.run_id))
            .await?;
        context
            .set_pin_value(
                "app_id",
                json!(details.app_id.as_deref().unwrap_or_default()),
            )
            .await?;
        context
            .set_pin_value(
                "user_id",
                json!(details.user_id.as_deref().unwrap_or_default()),
            )
            .await?;
        context.set_pin_value("details", json!(details)).await?;

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IsLocalEnvironmentNode {}

#[async_trait]
impl NodeLogic for IsLocalEnvironmentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_execution_is_local_environment",
            "Is Local Environment",
            "Returns true when the current run is executing on a local/client runtime.",
            "Utils/Execution",
        );
        node.set_flowscript_name("execution", "isLocalEnvironment");
        node.add_icon("/flow/icons/computer.svg");
        node.add_output_pin(
            "is_local",
            "Is Local",
            "True for local, desktop, mobile, and browser sandbox execution",
            VariableType::Boolean,
        );
        node.set_scores(execution_scores());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context
            .set_pin_value(
                "is_local",
                json!(context.execution_environment().is_local()),
            )
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IsServerEnvironmentNode {}

#[async_trait]
impl NodeLogic for IsServerEnvironmentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_execution_is_server_environment",
            "Is Server Environment",
            "Returns true when the current run is executing on the server.",
            "Utils/Execution",
        );
        node.set_flowscript_name("execution", "isServerEnvironment");
        node.add_icon("/flow/icons/cloud.svg");
        node.add_output_pin(
            "is_server",
            "Is Server",
            "True for server-side execution",
            VariableType::Boolean,
        );
        node.set_scores(execution_scores());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context
            .set_pin_value(
                "is_server",
                json!(matches!(
                    context.execution_environment(),
                    ExecutionEnvironment::Server
                )),
            )
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IsMobileEnvironmentNode {}

#[async_trait]
impl NodeLogic for IsMobileEnvironmentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_execution_is_mobile_environment",
            "Is Mobile Environment",
            "Returns true when the current run is executing on a mobile runtime.",
            "Utils/Execution",
        );
        node.set_flowscript_name("execution", "isMobileEnvironment");
        node.add_icon("/flow/icons/smartphone.svg");
        node.add_output_pin(
            "is_mobile",
            "Is Mobile",
            "True for mobile execution",
            VariableType::Boolean,
        );
        node.set_scores(execution_scores());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context
            .set_pin_value(
                "is_mobile",
                json!(matches!(
                    context.execution_environment(),
                    ExecutionEnvironment::Mobile
                )),
            )
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct GetRunIdNode {}

#[async_trait]
impl NodeLogic for GetRunIdNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_execution_get_run_id",
            "Get Run ID",
            "Returns the current execution run identifier.",
            "Utils/Execution",
        );
        node.set_flowscript_name("execution", "getRunId");
        node.add_icon("/flow/icons/hash.svg");
        node.add_output_pin(
            "run_id",
            "Run ID",
            "Current run identifier",
            VariableType::String,
        );
        node.set_scores(execution_scores());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context
            .set_pin_value("run_id", json!(context.run_id()))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct GetAppIdNode {}

#[async_trait]
impl NodeLogic for GetAppIdNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_execution_get_app_id",
            "Get App ID",
            "Returns the current app identifier.",
            "Utils/Execution",
        );
        node.set_flowscript_name("execution", "getAppId");
        node.add_icon("/flow/icons/box.svg");
        node.add_output_pin(
            "app_id",
            "App ID",
            "Current app identifier",
            VariableType::String,
        );
        node.set_scores(execution_scores());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let app_id = context
            .execution_cache
            .as_ref()
            .map(|cache| cache.app_id.as_str())
            .unwrap_or_default();
        context.set_pin_value("app_id", json!(app_id)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct GetUserIdNode {}

#[async_trait]
impl NodeLogic for GetUserIdNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_execution_get_user_id",
            "Get User ID",
            "Returns the current user identifier, when available.",
            "Utils/Execution",
        );
        node.set_flowscript_name("execution", "getUserId");
        node.add_icon("/flow/icons/user.svg");
        node.add_output_pin(
            "user_id",
            "User ID",
            "Current user identifier, or empty when unavailable",
            VariableType::String,
        );
        node.set_scores(execution_scores());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context
            .set_pin_value("user_id", json!(user_id(context).unwrap_or_default()))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct GetExecutionModeNode {}

#[async_trait]
impl NodeLogic for GetExecutionModeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_execution_get_mode",
            "Get Execution Mode",
            "Returns the current execution mode.",
            "Utils/Execution",
        );
        node.set_flowscript_name("execution", "getMode");
        node.add_icon("/flow/icons/play.svg");
        node.add_output_pin(
            "mode",
            "Mode",
            "The execution mode: sync, async, event, or scheduled",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "sync".to_string(),
                    "async".to_string(),
                    "event".to_string(),
                    "scheduled".to_string(),
                ])
                .build(),
        );
        node.set_scores(execution_scores());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context
            .set_pin_value("mode", json!(context.execution_mode().as_str()))
            .await?;
        Ok(())
    }
}
