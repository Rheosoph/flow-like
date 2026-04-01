//! Copilot Configuration Nodes
//!
//! Nodes for advanced session configuration.

use super::{
    CustomAgentConfig, InfiniteSessionConfig, ProviderConfig, SystemMessageConfig,
    SystemMessageMode,
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json};

#[crate::register_node]
#[derive(Default)]
pub struct CopilotInfiniteSessionNode {}

#[async_trait]
impl NodeLogic for CopilotInfiniteSessionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_infinite_session",
            "Infinite Session Config",
            "Configures infinite session with automatic context compaction",
            "AI/GitHub/Copilot/Config",
        );
        node.add_icon("/flow/icons/github.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(8)
                .set_governance(10)
                .set_reliability(9)
                .set_cost(7)
                .build(),
        );

        node.add_input_pin(
            "enabled",
            "Enabled",
            "Enable infinite sessions",
            VariableType::Boolean,
        )
        .set_default_value(Some(json::json!(true)));

        node.add_input_pin(
            "background_threshold",
            "Background Threshold",
            "Background compaction threshold (0.0-1.0)",
            VariableType::Float,
        )
        .set_default_value(Some(json::json!(0.7)));

        node.add_input_pin(
            "exhaustion_threshold",
            "Exhaustion Threshold",
            "Buffer exhaustion threshold (0.0-1.0)",
            VariableType::Float,
        )
        .set_default_value(Some(json::json!(0.9)));

        node.add_output_pin(
            "config",
            "Config",
            "Infinite session configuration",
            VariableType::Struct,
        )
        .set_schema::<InfiniteSessionConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let enabled: bool = context.evaluate_pin("enabled").await.unwrap_or(true);
        let background_threshold: f64 = context
            .evaluate_pin("background_threshold")
            .await
            .unwrap_or(0.7);
        let exhaustion_threshold: f64 = context
            .evaluate_pin("exhaustion_threshold")
            .await
            .unwrap_or(0.9);

        let config = InfiniteSessionConfig {
            enabled,
            background_compaction_threshold: Some(background_threshold),
            buffer_exhaustion_threshold: Some(exhaustion_threshold),
        };

        context.set_pin_value("config", json::json!(config)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotSystemMessageNode {}

#[async_trait]
impl NodeLogic for CopilotSystemMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_system_message",
            "System Message Config",
            "Configures the system message for the session",
            "AI/GitHub/Copilot/Config",
        );
        node.add_icon("/flow/icons/github.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(9)
                .set_performance(10)
                .set_governance(9)
                .set_reliability(10)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "content",
            "Content",
            "System message content",
            VariableType::String,
        );

        node.add_input_pin(
            "mode",
            "Mode",
            "Replace or Append to default system message",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["replace".to_string(), "append".to_string()])
                .build(),
        )
        .set_default_value(Some(json::json!("replace")));

        node.add_output_pin(
            "config",
            "Config",
            "System message configuration",
            VariableType::Struct,
        )
        .set_schema::<SystemMessageConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let content: String = context.evaluate_pin("content").await?;
        let mode_str: String = context
            .evaluate_pin("mode")
            .await
            .unwrap_or_else(|_| "replace".to_string());

        let mode = match mode_str.to_lowercase().as_str() {
            "append" => SystemMessageMode::Append,
            _ => SystemMessageMode::Replace,
        };

        let config = SystemMessageConfig { content, mode };

        context.set_pin_value("config", json::json!(config)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotCustomAgentNode {}

#[async_trait]
impl NodeLogic for CopilotCustomAgentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_custom_agent",
            "Custom Agent Config",
            "Configures a custom agent",
            "AI/GitHub/Copilot/Config",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/github.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(10)
                .set_governance(8)
                .set_reliability(10)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("name", "Name", "Agent identifier", VariableType::String);

        node.add_input_pin(
            "display_name",
            "Display Name",
            "Human-readable agent name",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

        node.add_input_pin(
            "description",
            "Description",
            "Agent description",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

        node.add_input_pin(
            "prompt",
            "Prompt",
            "Agent system prompt",
            VariableType::String,
        );

        node.add_output_pin(
            "agent",
            "Agent",
            "Custom agent configuration",
            VariableType::Struct,
        )
        .set_schema::<CustomAgentConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let name: String = context.evaluate_pin("name").await?;
        let display_name: String = context
            .evaluate_pin("display_name")
            .await
            .unwrap_or_default();
        let description: String = context
            .evaluate_pin("description")
            .await
            .unwrap_or_default();
        let prompt: String = context.evaluate_pin("prompt").await?;

        let agent = CustomAgentConfig {
            name,
            display_name: if display_name.is_empty() {
                None
            } else {
                Some(display_name)
            },
            description: if description.is_empty() {
                None
            } else {
                Some(description)
            },
            prompt,
            model: None,
            mcp_servers: Default::default(),
        };

        context.set_pin_value("agent", json::json!(agent)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotProviderConfigNode {}

#[async_trait]
impl NodeLogic for CopilotProviderConfigNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_provider_config",
            "Provider Config (BYOK)",
            "Configures a custom provider (Bring Your Own Key)",
            "AI/GitHub/Copilot/Config",
        );
        node.add_icon("/flow/icons/github.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(10)
                .set_governance(6)
                .set_reliability(9)
                .set_cost(8)
                .build(),
        );

        node.add_input_pin(
            "base_url",
            "Base URL",
            "Provider API base URL (e.g., https://api.openai.com/v1)",
            VariableType::String,
        );

        node.add_input_pin(
            "api_key",
            "API Key",
            "API key for authentication",
            VariableType::String,
        );

        node.add_input_pin("model", "Model", "Model ID to use", VariableType::String)
            .set_default_value(Some(json::json!("")));

        node.add_output_pin(
            "config",
            "Config",
            "Provider configuration",
            VariableType::Struct,
        )
        .set_schema::<ProviderConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let base_url: String = context.evaluate_pin("base_url").await?;
        let api_key: String = context.evaluate_pin("api_key").await?;
        let model: String = context.evaluate_pin("model").await.unwrap_or_default();

        let config = ProviderConfig {
            base_url,
            api_key: if api_key.is_empty() {
                None
            } else {
                Some(api_key)
            },
            model: if model.is_empty() { None } else { Some(model) },
        };

        context.set_pin_value("config", json::json!(config)).await?;
        Ok(())
    }
}
