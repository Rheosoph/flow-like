use flow_like::flow::{
    execution::context::ExecutionContext,
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
    pub is_desktop: bool,
    pub is_server: bool,
    pub is_local: bool,
    pub is_remote: bool,
    pub has_user_context: bool,
    pub user_sub: Option<String>,
    pub is_technical_user: bool,
    pub token_available: bool,
    pub hub: Option<String>,
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
            "Determines whether the current run is executing in the desktop app/local runtime or on the server.",
            "Utils/Execution",
        );
        node.add_icon("/flow/icons/computer.svg");

        node.add_output_pin(
            "environment",
            "Environment",
            "The execution environment: desktop or server",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["desktop".to_string(), "server".to_string()])
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
            "details",
            "Details",
            "Structured execution environment details",
            VariableType::Struct,
        )
        .set_schema::<ExecutionEnvironmentInfo>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_scores(
            NodeScores::new()
                .set_privacy(3)
                .set_security(8)
                .set_performance(10)
                .set_governance(8)
                .set_reliability(10)
                .set_cost(10)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let user_context = context.user_context().cloned();
        let is_local = user_context
            .as_ref()
            .map(|context| context.is_offline())
            .unwrap_or(false);
        let environment = if is_local { "desktop" } else { "server" };
        let hub = if context.profile.hub.trim().is_empty() {
            None
        } else {
            Some(context.profile.hub.clone())
        };

        let details = ExecutionEnvironmentInfo {
            environment: environment.to_string(),
            is_desktop: is_local,
            is_server: !is_local,
            is_local,
            is_remote: !is_local,
            has_user_context: user_context.is_some(),
            user_sub: user_context.as_ref().map(|context| context.sub.clone()),
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
        };

        context
            .set_pin_value("environment", json!(details.environment))
            .await?;
        context
            .set_pin_value("is_desktop", json!(details.is_desktop))
            .await?;
        context
            .set_pin_value("is_server", json!(details.is_server))
            .await?;
        context
            .set_pin_value("is_local", json!(details.is_local))
            .await?;
        context
            .set_pin_value("is_remote", json!(details.is_remote))
            .await?;
        context.set_pin_value("details", json!(details)).await?;

        Ok(())
    }
}
