//! Copilot Utility Nodes
//!
//! Helper utilities for Copilot integration.

use super::CopilotClientHandle;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json};

#[crate::register_node]
#[derive(Default)]
pub struct CopilotGetVersionNode {}

#[async_trait]
impl NodeLogic for CopilotGetVersionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_get_version",
            "Get Version",
            "Gets the version of the Copilot CLI",
            "AI/GitHub/Copilot/Utilities",
        );
        node.set_flowscript_name("github.copilot", "getVersion");
        node.set_receiver("client");
        node.add_icon("/flow/icons/github.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(9)
                .set_governance(10)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "client",
            "Client",
            "Copilot client handle",
            VariableType::Struct,
        )
        .set_schema::<CopilotClientHandle>();

        node.add_output_pin(
            "version",
            "Version",
            "CLI version string",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use super::CachedCopilotClient;

        let handle: CopilotClientHandle = context.evaluate_pin("client").await?;

        let cached = {
            let cache = context.cache.read().await;
            cache.get(&handle.cache_key).cloned()
        };
        let cached =
            cached.ok_or_else(|| flow_like_types::anyhow!("Copilot client not found in cache"))?;
        let cached_client = cached
            .as_any()
            .downcast_ref::<CachedCopilotClient>()
            .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast cached client"))?;

        let status = cached_client
            .client
            .get_status()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to get status: {}", e))?;

        context
            .set_pin_value("version", json::json!(status.version))
            .await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "GitHub Copilot integration requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotGetModelsNode {}

#[async_trait]
impl NodeLogic for CopilotGetModelsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_get_models",
            "Get Models",
            "Lists available Copilot models",
            "AI/GitHub/Copilot/Utilities",
        );
        node.set_flowscript_name("github.copilot", "getModels");
        node.set_receiver("client");
        node.add_icon("/flow/icons/github.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(7)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin(
            "client",
            "Client",
            "Copilot client handle",
            VariableType::Struct,
        )
        .set_schema::<CopilotClientHandle>();

        node.add_output_pin(
            "models",
            "Models",
            "Array of available model names",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use super::CachedCopilotClient;

        let handle: CopilotClientHandle = context.evaluate_pin("client").await?;

        let cached = {
            let cache = context.cache.read().await;
            cache.get(&handle.cache_key).cloned()
        };
        let cached =
            cached.ok_or_else(|| flow_like_types::anyhow!("Copilot client not found in cache"))?;
        let cached_client = cached
            .as_any()
            .downcast_ref::<CachedCopilotClient>()
            .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast cached client"))?;

        let models = cached_client
            .client
            .list_models()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to list models: {}", e))?;

        let model_names: Vec<String> = models.into_iter().map(|m| m.name.clone()).collect();
        context
            .set_pin_value("models", json::json!(model_names))
            .await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "GitHub Copilot integration requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotGetAuthStatusNode {}

#[async_trait]
impl NodeLogic for CopilotGetAuthStatusNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_get_auth_status",
            "Get Auth Status",
            "Gets the authentication status of the Copilot client",
            "AI/GitHub/Copilot/Utilities",
        );
        node.set_flowscript_name("github.copilot", "getAuthStatus");
        node.set_receiver("client");
        node.add_icon("/flow/icons/github.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(9)
                .set_governance(8)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "client",
            "Client",
            "Copilot client handle",
            VariableType::Struct,
        )
        .set_schema::<CopilotClientHandle>();

        node.add_output_pin(
            "is_authenticated",
            "Is Authenticated",
            "Whether the user is authenticated",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "login",
            "Login",
            "GitHub username if authenticated",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use super::CachedCopilotClient;

        let handle: CopilotClientHandle = context.evaluate_pin("client").await?;

        let cached = {
            let cache = context.cache.read().await;
            cache.get(&handle.cache_key).cloned()
        };
        let cached =
            cached.ok_or_else(|| flow_like_types::anyhow!("Copilot client not found in cache"))?;
        let cached_client = cached
            .as_any()
            .downcast_ref::<CachedCopilotClient>()
            .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast cached client"))?;

        let auth_status = cached_client
            .client
            .get_auth_status()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to get auth status: {}", e))?;

        context
            .set_pin_value(
                "is_authenticated",
                json::json!(auth_status.is_authenticated),
            )
            .await?;
        context
            .set_pin_value("login", json::json!(auth_status.login.unwrap_or_default()))
            .await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "GitHub Copilot integration requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotClientStatusNode {}

#[async_trait]
impl NodeLogic for CopilotClientStatusNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_client_status",
            "Client Status",
            "Checks if a Copilot client is connected and ready",
            "AI/GitHub/Copilot/Utilities",
        );
        node.set_flowscript_name("github.copilot", "status");
        node.set_receiver("client");
        node.add_icon("/flow/icons/github.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(10)
                .set_governance(10)
                .set_reliability(10)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "client",
            "Client",
            "Copilot client handle",
            VariableType::Struct,
        )
        .set_schema::<CopilotClientHandle>();

        node.add_output_pin(
            "is_connected",
            "Is Connected",
            "Whether the client is connected",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "client_id",
            "Client ID",
            "Client identifier",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let handle: CopilotClientHandle = context.evaluate_pin("client").await?;

        let is_connected = {
            let cache = context.cache.read().await;
            cache.get(&handle.cache_key).is_some()
        };

        context
            .set_pin_value("is_connected", json::json!(is_connected))
            .await?;
        context
            .set_pin_value("client_id", json::json!(handle.cache_key))
            .await?;
        Ok(())
    }
}
