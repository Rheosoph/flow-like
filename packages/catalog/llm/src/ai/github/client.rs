//! Copilot Client Nodes
//!
//! Nodes for managing the Copilot client lifecycle.
//!
//! Two client variants are provided:
//! - **Local Client**: Uses stdio connection to local Copilot CLI (desktop/local execution)
//! - **Server Client**: Uses TCP connection to remote Copilot endpoint (server/distributed execution)

#[cfg(feature = "execute")]
use super::COPILOT_CLIENT_PREFIX;
use super::{CopilotClientHandle, CopilotLogLevel};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, async_trait, json};
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use std::sync::Arc;

/// Configuration for local Copilot client (stdio-based)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct CopilotLocalClientConfig {
    /// Log level (Error, Warn, Info, Debug)
    #[serde(default)]
    pub log_level: CopilotLogLevel,
    /// Optional path to Copilot CLI executable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_path: Option<String>,
}

/// Configuration for server/remote Copilot client (TCP-based)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CopilotServerClientConfig {
    /// TCP endpoint URL (e.g., "tcp://localhost:3000")
    pub url: String,
    /// Log level (Error, Warn, Info, Debug)
    #[serde(default)]
    pub log_level: CopilotLogLevel,
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotLocalClientBuilderNode {}

#[async_trait]
impl NodeLogic for CopilotLocalClientBuilderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_local_client_builder",
            "Local Client Config",
            "Builds a local Copilot client configuration (stdio-based). Requires 'copilot' CLI to be installed and in PATH, or specify the CLI path explicitly.",
            "AI/GitHub/Copilot/Client",
        );
        node.set_flowscript_name("github.copilot", "localClientConfig");
        node.add_icon("/flow/icons/github.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(9)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "log_level",
            "Log Level",
            "Client log level",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "error".to_string(),
                    "warn".to_string(),
                    "info".to_string(),
                    "debug".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json::json!("error")));

        node.add_input_pin(
            "cli_path",
            "CLI Path",
            "Optional path to Copilot CLI executable. If not set, searches PATH and COPILOT_CLI_PATH env var.",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

        node.add_output_pin(
            "client_config",
            "Client Config",
            "Local client configuration",
            VariableType::Struct,
        )
        .set_schema::<CopilotLocalClientConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let log_level_str: String = context
            .evaluate_pin("log_level")
            .await
            .unwrap_or_else(|_| "error".to_string());

        let cli_path: String = context.evaluate_pin("cli_path").await.unwrap_or_default();

        let log_level = match log_level_str.to_lowercase().as_str() {
            "warn" => CopilotLogLevel::Warn,
            "info" => CopilotLogLevel::Info,
            "debug" => CopilotLogLevel::Debug,
            _ => CopilotLogLevel::Error,
        };

        let config = CopilotLocalClientConfig {
            log_level,
            cli_path: if cli_path.is_empty() {
                None
            } else {
                Some(cli_path)
            },
        };

        context
            .set_pin_value("client_config", json::json!(config))
            .await?;

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotServerClientBuilderNode {}

#[async_trait]
impl NodeLogic for CopilotServerClientBuilderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_server_client_builder",
            "Server Client Config",
            "Builds a server/remote Copilot client configuration (TCP-based)",
            "AI/GitHub/Copilot/Client",
        );
        node.set_flowscript_name("github.copilot", "serverClientConfig");
        node.add_icon("/flow/icons/github.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(6)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(8)
                .build(),
        );

        node.add_input_pin(
            "url",
            "URL",
            "TCP endpoint URL (e.g., tcp://localhost:3000)",
            VariableType::String,
        );

        node.add_input_pin(
            "log_level",
            "Log Level",
            "Client log level",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "error".to_string(),
                    "warn".to_string(),
                    "info".to_string(),
                    "debug".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json::json!("error")));

        node.add_output_pin(
            "client_config",
            "Client Config",
            "Server client configuration",
            VariableType::Struct,
        )
        .set_schema::<CopilotServerClientConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let url: String = context.evaluate_pin("url").await?;
        let log_level_str: String = context
            .evaluate_pin("log_level")
            .await
            .unwrap_or_else(|_| "error".to_string());

        let log_level = match log_level_str.to_lowercase().as_str() {
            "warn" => CopilotLogLevel::Warn,
            "info" => CopilotLogLevel::Info,
            "debug" => CopilotLogLevel::Debug,
            _ => CopilotLogLevel::Error,
        };

        let config = CopilotServerClientConfig { url, log_level };

        context
            .set_pin_value("client_config", json::json!(config))
            .await?;

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotLocalClientStartNode {}

#[async_trait]
impl NodeLogic for CopilotLocalClientStartNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_local_client_start",
            "Start Local Client",
            "Starts a local Copilot client using stdio. Requires 'copilot' CLI installed.",
            "AI/GitHub/Copilot/Client",
        );
        node.set_flowscript_name("github.copilot", "startLocalClient");
        node.add_icon("/flow/icons/github.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(7)
                .set_governance(7)
                .set_reliability(7)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);

        node.add_input_pin(
            "client_config",
            "Client Config",
            "Local client configuration",
            VariableType::Struct,
        )
        .set_schema::<CopilotLocalClientConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after client starts",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Fires if client fails to start",
            VariableType::Execution,
        );

        node.add_output_pin(
            "client",
            "Client",
            "Running client handle",
            VariableType::Struct,
        )
        .set_schema::<CopilotClientHandle>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error message if startup fails",
            VariableType::String,
        );

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use super::CachedCopilotClient;
        use copilot_sdk::{Client, LogLevel};
        use flow_like::flow::execution::LogLevel as FlowLogLevel;
        use flow_like_types::Cacheable;
        use std::path::PathBuf;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let config: CopilotLocalClientConfig = context.evaluate_pin("client_config").await?;

        context.log_message("Starting local Copilot client...", FlowLogLevel::Info);

        let log_level = match config.log_level {
            CopilotLogLevel::Error => LogLevel::Error,
            CopilotLogLevel::Warn => LogLevel::Warn,
            CopilotLogLevel::Info => LogLevel::Info,
            CopilotLogLevel::Debug => LogLevel::Debug,
        };

        let mut builder = Client::builder().use_stdio(true).log_level(log_level);

        if let Some(ref cli_path) = config.cli_path {
            builder = builder.cli_path(PathBuf::from(cli_path));
        }

        let client = match builder.build() {
            Ok(c) => c,
            Err(e) => {
                let error_msg = format!(
                    "Failed to build local Copilot client: {}. Make sure 'copilot' CLI is installed and in PATH, or set COPILOT_CLI_PATH environment variable.",
                    e
                );
                context.log_message(&error_msg, FlowLogLevel::Error);
                context
                    .set_pin_value("error_message", json::json!(error_msg))
                    .await?;
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        if let Err(e) = client.start().await {
            let error_msg = format!(
                "Failed to start local Copilot client: {}. Ensure 'copilot' CLI is authenticated.",
                e
            );
            context.log_message(&error_msg, FlowLogLevel::Error);
            context
                .set_pin_value("error_message", json::json!(error_msg))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        context.log_message("Local Copilot client started", FlowLogLevel::Info);

        let cache_key = format!("{}local_{}", COPILOT_CLIENT_PREFIX, uuid::Uuid::new_v4());
        let cached = CachedCopilotClient { client };
        let cacheable: Arc<dyn Cacheable> = Arc::new(cached);
        context
            .cache
            .write()
            .await
            .insert(cache_key.clone(), cacheable);

        let handle = CopilotClientHandle { cache_key };

        context.set_pin_value("client", json::json!(handle)).await?;
        context.activate_exec_pin("exec_out").await?;

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
pub struct CopilotServerClientStartNode {}

#[async_trait]
impl NodeLogic for CopilotServerClientStartNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_server_client_start",
            "Start Server Client",
            "Starts a server/remote Copilot client using TCP",
            "AI/GitHub/Copilot/Client",
        );
        node.set_flowscript_name("github.copilot", "startServerClient");
        node.add_icon("/flow/icons/github.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(6)
                .set_performance(7)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(7)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);

        node.add_input_pin(
            "client_config",
            "Client Config",
            "Server client configuration",
            VariableType::Struct,
        )
        .set_schema::<CopilotServerClientConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after client starts",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Fires if client fails to start",
            VariableType::Execution,
        );

        node.add_output_pin(
            "client",
            "Client",
            "Running client handle",
            VariableType::Struct,
        )
        .set_schema::<CopilotClientHandle>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error message if connection fails",
            VariableType::String,
        );

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use super::CachedCopilotClient;
        use copilot_sdk::{Client, LogLevel};
        use flow_like::flow::execution::LogLevel as FlowLogLevel;
        use flow_like_types::Cacheable;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let config: CopilotServerClientConfig = context.evaluate_pin("client_config").await?;

        context.log_message(
            &format!("Connecting to remote Copilot at {}...", config.url),
            FlowLogLevel::Info,
        );

        let log_level = match config.log_level {
            CopilotLogLevel::Error => LogLevel::Error,
            CopilotLogLevel::Warn => LogLevel::Warn,
            CopilotLogLevel::Info => LogLevel::Info,
            CopilotLogLevel::Debug => LogLevel::Debug,
        };

        let client = match Client::builder()
            .use_stdio(false)
            .cli_url(config.url.clone())
            .log_level(log_level)
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                let error_msg = format!("Failed to build server Copilot client: {}", e);
                context.log_message(&error_msg, FlowLogLevel::Error);
                context
                    .set_pin_value("error_message", json::json!(error_msg))
                    .await?;
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        if let Err(e) = client.start().await {
            let error_msg = format!(
                "Failed to connect to remote Copilot at {}: {}",
                config.url, e
            );
            context.log_message(&error_msg, FlowLogLevel::Error);
            context
                .set_pin_value("error_message", json::json!(error_msg))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        context.log_message(
            &format!("Connected to remote Copilot at {}", config.url),
            FlowLogLevel::Info,
        );

        let cache_key = format!("{}server_{}", COPILOT_CLIENT_PREFIX, uuid::Uuid::new_v4());
        let cached = CachedCopilotClient { client };
        let cacheable: Arc<dyn Cacheable> = Arc::new(cached);
        context
            .cache
            .write()
            .await
            .insert(cache_key.clone(), cacheable);

        let handle = CopilotClientHandle { cache_key };

        context.set_pin_value("client", json::json!(handle)).await?;
        context.activate_exec_pin("exec_out").await?;

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
pub struct CopilotClientStopNode {}

#[async_trait]
impl NodeLogic for CopilotClientStopNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_client_stop",
            "Stop Client",
            "Gracefully stops a running Copilot client (local or server)",
            "AI/GitHub/Copilot/Client",
        );
        node.set_flowscript_name("github.copilot", "stop");
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

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);

        node.add_input_pin(
            "client",
            "Client",
            "Client handle to stop",
            VariableType::Struct,
        )
        .set_schema::<CopilotClientHandle>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after client stops",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use super::CachedCopilotClient;
        use flow_like::flow::execution::LogLevel;

        context.deactivate_exec_pin("exec_out").await?;

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

        context.log_message("Stopping Copilot client...", LogLevel::Info);

        let stop_errors = cached_client.client.stop().await;
        for e in &stop_errors {
            context.log_message(
                &format!("Copilot client stop warning: {}", e),
                LogLevel::Warn,
            );
        }

        context.cache.write().await.remove(&handle.cache_key);

        context.log_message("Copilot client stopped", LogLevel::Info);
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "GitHub Copilot integration requires the 'execute' feature"
        ))
    }
}
