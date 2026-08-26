//! Copilot Session Nodes
//!
//! Nodes for creating, managing, and configuring Copilot sessions.

#[cfg(feature = "execute")]
use super::COPILOT_SESSION_PREFIX;
#[cfg(feature = "execute")]
use super::CachedCopilotSession;
use super::{
    CopilotClientHandle, CopilotSessionConfig, CopilotSessionHandle, CopilotToolConfig,
    CustomAgentConfig, InfiniteSessionConfig, ProviderConfig, SystemMessageConfig,
    SystemMessageMode,
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json};
use std::collections::HashMap;

/// Session builder for constructing complete session configurations
#[crate::register_node]
#[derive(Default)]
pub struct CopilotSessionBuilderNode {}

#[async_trait]
impl NodeLogic for CopilotSessionBuilderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_session_builder",
            "Session Builder",
            "Builds a complete Copilot session configuration with all options",
            "AI/GitHub/Copilot/Session",
        );
        node.set_flowscript_name("github.copilot", "sessionConfig");
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
            "model",
            "Model",
            "Optional model ID to use (e.g., gpt-4o)",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

        node.add_input_pin(
            "streaming",
            "Streaming",
            "Enable streaming responses",
            VariableType::Boolean,
        )
        .set_default_value(Some(json::json!(true)));

        node.add_input_pin(
            "system_message",
            "System Message",
            "Optional system message content",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

        node.add_input_pin(
            "system_mode",
            "System Mode",
            "Replace or Append to default system message",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["replace".to_string(), "append".to_string()])
                .build(),
        )
        .set_default_value(Some(json::json!("replace")));

        node.add_input_pin(
            "infinite_enabled",
            "Infinite Session",
            "Enable infinite sessions with automatic context compaction",
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

        node.add_input_pin(
            "provider",
            "Provider",
            "Optional BYOK provider configuration",
            VariableType::Struct,
        )
        .set_schema::<ProviderConfig>();

        node.add_input_pin(
            "tools",
            "Tools",
            "Optional array of tool configurations",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<CopilotToolConfig>();

        node.add_input_pin(
            "custom_agents",
            "Custom Agents",
            "Optional array of custom agent configurations",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<CustomAgentConfig>();

        node.add_input_pin(
            "mcp_servers",
            "MCP Servers",
            "Optional MCP servers configuration (JSON object)",
            VariableType::Generic,
        );

        node.add_output_pin(
            "config",
            "Config",
            "Complete session configuration",
            VariableType::Struct,
        )
        .set_schema::<CopilotSessionConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let model: String = context.evaluate_pin("model").await.unwrap_or_default();
        let streaming: bool = context.evaluate_pin("streaming").await.unwrap_or(true);
        let system_message: String = context
            .evaluate_pin("system_message")
            .await
            .unwrap_or_default();
        let system_mode: String = context
            .evaluate_pin("system_mode")
            .await
            .unwrap_or_else(|_| "replace".to_string());
        let infinite_enabled: bool = context
            .evaluate_pin("infinite_enabled")
            .await
            .unwrap_or(true);
        let background_threshold: f64 = context
            .evaluate_pin("background_threshold")
            .await
            .unwrap_or(0.7);
        let exhaustion_threshold: f64 = context
            .evaluate_pin("exhaustion_threshold")
            .await
            .unwrap_or(0.9);
        let provider: Option<ProviderConfig> = context.evaluate_pin("provider").await.ok();
        let tools: Vec<CopilotToolConfig> = context.evaluate_pin("tools").await.unwrap_or_default();
        let custom_agents: Vec<CustomAgentConfig> = context
            .evaluate_pin("custom_agents")
            .await
            .unwrap_or_default();
        let mcp_servers: HashMap<String, flow_like_types::Value> = context
            .evaluate_pin("mcp_servers")
            .await
            .unwrap_or_default();

        let system_msg_config = if system_message.is_empty() {
            None
        } else {
            let mode = match system_mode.to_lowercase().as_str() {
                "append" => SystemMessageMode::Append,
                _ => SystemMessageMode::Replace,
            };
            Some(SystemMessageConfig {
                content: system_message,
                mode,
            })
        };

        let infinite_config = if infinite_enabled {
            Some(InfiniteSessionConfig {
                enabled: true,
                background_compaction_threshold: Some(background_threshold),
                buffer_exhaustion_threshold: Some(exhaustion_threshold),
            })
        } else {
            None
        };

        let config = CopilotSessionConfig {
            model: if model.is_empty() { None } else { Some(model) },
            streaming,
            request_permission: None,
            system_message: system_msg_config,
            infinite_sessions: infinite_config,
            mcp_servers,
            custom_agents,
            tools,
            provider,
        };

        context.set_pin_value("config", json::json!(config)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotCreateSessionNode {}

#[async_trait]
impl NodeLogic for CopilotCreateSessionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_create_session",
            "Create Session",
            "Creates a new Copilot chat session",
            "AI/GitHub/Copilot/Session",
        );
        node.set_flowscript_name("github.copilot", "createSession");
        node.set_receiver("client");
        node.add_icon("/flow/icons/github.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(7)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(8)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);

        node.add_input_pin(
            "client",
            "Client",
            "Running Copilot client",
            VariableType::Struct,
        )
        .set_schema::<CopilotClientHandle>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "config",
            "Config",
            "Session configuration (from Session Builder)",
            VariableType::Struct,
        )
        .set_schema::<CopilotSessionConfig>();

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after session is created",
            VariableType::Execution,
        );

        node.add_output_pin("session", "Session", "Session handle", VariableType::Struct)
            .set_schema::<CopilotSessionHandle>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use super::CachedCopilotClient;
        use copilot_sdk::SessionConfig;
        use flow_like::flow::execution::LogLevel;
        use flow_like_types::Cacheable;
        use std::sync::Arc;

        context.deactivate_exec_pin("exec_out").await?;

        let client_handle: CopilotClientHandle = context.evaluate_pin("client").await?;
        let config: Option<CopilotSessionConfig> = context.evaluate_pin("config").await.ok();

        let cached = {
            let cache = context.cache.read().await;
            cache.get(&client_handle.cache_key).cloned()
        };
        let cached =
            cached.ok_or_else(|| flow_like_types::anyhow!("Copilot client not found in cache"))?;
        let cached_client = cached
            .as_any()
            .downcast_ref::<CachedCopilotClient>()
            .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast cached client"))?;

        context.log_message("Creating Copilot session...", LogLevel::Info);

        let sdk_config = if let Some(cfg) = config {
            let mut session_config = SessionConfig {
                model: cfg.model,
                streaming: cfg.streaming,
                ..Default::default()
            };

            if let Some(sys_msg) = cfg.system_message {
                session_config.system_message = Some(copilot_sdk::SystemMessageConfig {
                    content: Some(sys_msg.content),
                    mode: Some(match sys_msg.mode {
                        super::SystemMessageMode::Append => copilot_sdk::SystemMessageMode::Append,
                        super::SystemMessageMode::Replace => {
                            copilot_sdk::SystemMessageMode::Replace
                        }
                    }),
                });
            }

            if let Some(infinite) = cfg.infinite_sessions
                && infinite.enabled
            {
                session_config.infinite_sessions =
                    Some(copilot_sdk::InfiniteSessionConfig::enabled());
            }

            if let Some(provider) = cfg.provider {
                session_config.provider = Some(copilot_sdk::ProviderConfig {
                    base_url: provider.base_url,
                    api_key: provider.api_key,
                    provider_type: None,
                    wire_api: None,
                    bearer_token: None,
                    azure: None,
                });
                if let Some(model) = provider.model {
                    session_config.model = Some(model);
                }
            }

            if !cfg.tools.is_empty() {
                session_config.tools = cfg
                    .tools
                    .into_iter()
                    .map(|t| {
                        copilot_sdk::Tool::new(&t.name)
                            .description(&t.description)
                            .schema(t.schema)
                    })
                    .collect();
            }

            if !cfg.custom_agents.is_empty() {
                session_config.custom_agents = Some(
                    cfg.custom_agents
                        .into_iter()
                        .map(|a| copilot_sdk::CustomAgentConfig {
                            name: a.name,
                            prompt: a.prompt,
                            display_name: a.display_name,
                            description: a.description,
                            tools: None,
                            mcp_servers: if a.mcp_servers.is_empty() {
                                None
                            } else {
                                Some(a.mcp_servers)
                            },
                            infer: None,
                        })
                        .collect(),
                );
            }

            if !cfg.mcp_servers.is_empty() {
                session_config.mcp_servers = Some(cfg.mcp_servers);
            }

            session_config
        } else {
            SessionConfig::default()
        };

        let session = cached_client
            .client
            .create_session(sdk_config)
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to create session: {}", e))?;

        let session_id = session.session_id().to_string();
        let cache_key = format!("{}{}", COPILOT_SESSION_PREFIX, uuid::Uuid::new_v4());

        context.log_message(&format!("Created session: {}", session_id), LogLevel::Info);

        let cached_session = CachedCopilotSession { session };
        let cacheable: Arc<dyn Cacheable> = Arc::new(cached_session);
        context
            .cache
            .write()
            .await
            .insert(cache_key.clone(), cacheable);

        let handle = CopilotSessionHandle {
            cache_key,
            session_id,
            client_key: client_handle.cache_key,
        };

        context
            .set_pin_value("session", json::json!(handle))
            .await?;
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
pub struct CopilotDestroySessionNode {}

#[async_trait]
impl NodeLogic for CopilotDestroySessionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_destroy_session",
            "Destroy Session",
            "Destroys a Copilot session",
            "AI/GitHub/Copilot/Session",
        );
        node.set_flowscript_name("github.copilot", "destroySession");
        node.set_receiver("session");
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
            "session",
            "Session",
            "Session handle to destroy",
            VariableType::Struct,
        )
        .set_schema::<CopilotSessionHandle>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after session is destroyed",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use super::CachedCopilotSession;
        use flow_like::flow::execution::LogLevel;

        context.deactivate_exec_pin("exec_out").await?;

        let handle: CopilotSessionHandle = context.evaluate_pin("session").await?;

        let cached = {
            let cache = context.cache.read().await;
            cache.get(&handle.cache_key).cloned()
        };

        if let Some(cached) = cached
            && let Some(cached_session) = cached.as_any().downcast_ref::<CachedCopilotSession>()
            && let Err(e) = cached_session.session.destroy().await
        {
            context.log_message(
                &format!("Warning: Failed to destroy session: {}", e),
                LogLevel::Warn,
            );
        }

        context.cache.write().await.remove(&handle.cache_key);
        context.log_message("Session destroyed", LogLevel::Info);
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
