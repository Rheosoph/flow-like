#[cfg(not(feature = "execute"))]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like::flow::{
    execution::{
        LogLevel, context::ExecutionContext, internal_node::InternalNode, log::LogMessage,
    },
    pin::{Pin, PinType, ValueType},
    variable::VariableType,
};
use flow_like::flow::{
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType as NodeVariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::async_trait;
#[cfg(feature = "execute")]
use flow_like_types::json::{self, json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use super::{rest::RestAuthConfig, tls::TlsConfig};

const MCP_CONFIG_NODE_VERSION: u32 = 4;

#[cfg(feature = "execute")]
const MCP_SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

#[cfg(feature = "execute")]
const MCP_DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";

#[cfg(feature = "execute")]
const MCP_WELL_KNOWN_OAUTH_PATH: &str = "/.well-known/oauth-protected-resource";

#[cfg(feature = "execute")]
const MCP_SSE_HEARTBEAT_SECONDS: u64 = 15;

#[cfg(feature = "execute")]
const MCP_LEGACY_SESSION_QUERY_PARAM: &str = "sessionId";

#[cfg(feature = "execute")]
const MCP_EMPTY_STRING_HASH: &str = "16248035215404677707";

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct McpServerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default = "default_mcp_path")]
    pub path: String,
    #[serde(default)]
    pub timeout_seconds: u64,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,
    #[serde(default)]
    pub tls: TlsConfig,
    #[serde(default)]
    pub auth: RestAuthConfig,
    #[serde(default)]
    pub function_refs: Vec<String>,
    #[serde(default)]
    pub resources: Vec<McpResourceRegistration>,
    #[serde(default)]
    pub prompts: Vec<McpPromptRegistration>,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 0,
            path: default_mcp_path(),
            timeout_seconds: 0,
            max_connections: default_max_connections(),
            max_body_bytes: default_max_body_bytes(),
            tls: Default::default(),
            auth: Default::default(),
            function_refs: Vec::new(),
            resources: Vec::new(),
            prompts: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct McpResourceRegistration {
    pub uri: String,
    pub name: String,
    pub flow_path: FlowPath,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct McpPromptRegistration {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub template: String,
}

fn default_mcp_path() -> String {
    "/mcp".to_string()
}

fn default_max_connections() -> u32 {
    128
}

fn default_max_body_bytes() -> usize {
    10 * 1024 * 1024
}

fn flow_path_filename(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(path)
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateMcpServerConfigNode {}

impl CreateMcpServerConfigNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateMcpServerConfigNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mcp_server_config",
            "MCP Server Config",
            "Creates an MCP server config that function, resource, prompt, auth, and server nodes can compose.",
            "Web/MCP",
        );
        node.add_icon("/flow/icons/web.svg");
        node.add_input_pin("host", "Host", "Bind host", NodeVariableType::String)
            .set_default_value(Some(flow_like_types::json::json!("127.0.0.1")));
        node.add_input_pin("port", "Port", "Bind port", NodeVariableType::Integer)
            .set_default_value(Some(flow_like_types::json::json!(0)));
        node.add_input_pin("path", "Path", "MCP HTTP path", NodeVariableType::String)
            .set_default_value(Some(flow_like_types::json::json!("/mcp")));
        node.add_input_pin(
            "timeout_seconds",
            "Timeout Seconds",
            "Server lifetime timeout; zero means run until cancelled",
            NodeVariableType::Integer,
        )
        .set_default_value(Some(flow_like_types::json::json!(0)));
        node.add_input_pin(
            "max_connections",
            "Max Connections",
            "Maximum concurrent requests",
            NodeVariableType::Integer,
        )
        .set_default_value(Some(flow_like_types::json::json!(128)));
        node.add_input_pin(
            "max_body_bytes",
            "Max Body Bytes",
            "Maximum HTTP request body size",
            NodeVariableType::Integer,
        )
        .set_default_value(Some(flow_like_types::json::json!(10485760)));
        node.add_input_pin(
            "tls",
            "TLS",
            "TLS security config",
            NodeVariableType::Struct,
        )
        .set_schema::<TlsConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "config",
            "Config",
            "MCP server config",
            NodeVariableType::Struct,
        )
        .set_schema::<McpServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let host: String = context.evaluate_pin("host").await?;
        let port: i64 = context.evaluate_pin("port").await?;
        let path: String = context.evaluate_pin("path").await?;
        let timeout_seconds: i64 = context.evaluate_pin("timeout_seconds").await?;
        let max_connections: i64 = context.evaluate_pin("max_connections").await?;
        let max_body_bytes: i64 = context.evaluate_pin("max_body_bytes").await?;
        let tls: TlsConfig = context.evaluate_pin("tls").await.unwrap_or_default();

        let config = McpServerConfig {
            host,
            port: port.max(0) as u16,
            path: super::http_runtime::normalize_path(&path),
            timeout_seconds: timeout_seconds.max(0) as u64,
            max_connections: max_connections.max(0) as u32,
            max_body_bytes: max_body_bytes.max(0) as usize,
            tls,
            ..Default::default()
        };
        context
            .set_pin_value("config", flow_like_types::json::json!(config))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct RegisterMcpFunctionsNode {}

impl RegisterMcpFunctionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for RegisterMcpFunctionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mcp_register_functions",
            "Register MCP Functions",
            "Registers referenced Flow functions as MCP tools.",
            "Web/MCP",
        );
        node.add_icon("/flow/icons/web.svg");
        node.set_can_reference_fns(true);
        node.add_input_pin(
            "config_in",
            "Config",
            "MCP server config",
            NodeVariableType::Struct,
        )
        .set_schema::<McpServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "config_out",
            "Config",
            "Updated config",
            NodeVariableType::Struct,
        )
        .set_schema::<McpServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut config: McpServerConfig = context.evaluate_pin("config_in").await?;
        for node in context.get_referenced_functions().await? {
            let id = node.node_id().to_string();
            if !config.function_refs.contains(&id) {
                config.function_refs.push(id);
            }
        }
        context
            .set_pin_value("config_out", flow_like_types::json::json!(config))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct RegisterMcpAuthNode {}

impl RegisterMcpAuthNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for RegisterMcpAuthNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mcp_register_auth",
            "Register MCP Auth",
            "Registers MCP server authentication settings.",
            "Web/MCP",
        );
        node.add_icon("/flow/icons/web.svg");
        node.add_input_pin(
            "config_in",
            "Config",
            "MCP server config",
            NodeVariableType::Struct,
        )
        .set_schema::<McpServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("auth", "Auth", "Auth config", NodeVariableType::Struct)
            .set_schema::<RestAuthConfig>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "config_out",
            "Config",
            "Updated config",
            NodeVariableType::Struct,
        )
        .set_schema::<McpServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut config: McpServerConfig = context.evaluate_pin("config_in").await?;
        let auth: RestAuthConfig = context.evaluate_pin("auth").await?;
        config.auth = auth;
        context
            .set_pin_value("config_out", flow_like_types::json::json!(config))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct RegisterMcpResourceNode {}

impl RegisterMcpResourceNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for RegisterMcpResourceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mcp_register_resource",
            "Register MCP Resource",
            "Registers a FlowPath as an MCP resource.",
            "Web/MCP",
        );
        node.set_version(MCP_CONFIG_NODE_VERSION);
        node.add_icon("/flow/icons/web.svg");
        node.add_input_pin(
            "config_in",
            "Config",
            "MCP server config",
            NodeVariableType::Struct,
        )
        .set_schema::<McpServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "flow_path",
            "Flow Path",
            "Resource FlowPath",
            NodeVariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "uri",
            "URI",
            "MCP resource URI exposed to clients. Defaults to file://<flow path> when empty.",
            NodeVariableType::String,
        );
        node.add_input_pin(
            "name",
            "Name",
            "Resource display name. Defaults to the FlowPath filename when empty.",
            NodeVariableType::String,
        );
        node.add_input_pin(
            "description",
            "Description",
            "Optional description",
            NodeVariableType::String,
        );
        node.add_output_pin(
            "config_out",
            "Config",
            "Updated config",
            NodeVariableType::Struct,
        )
        .set_schema::<McpServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut config: McpServerConfig = context.evaluate_pin("config_in").await?;
        let flow_path: FlowPath = context.evaluate_pin("flow_path").await?;
        let uri: String = context.evaluate_pin("uri").await.unwrap_or_default();
        let name: String = context.evaluate_pin("name").await.unwrap_or_default();
        let description: Option<String> = context.evaluate_pin("description").await.ok();

        let filename = flow_path_filename(&flow_path.path);
        let uri = if uri.is_empty() {
            format!("file://{}", flow_path.path)
        } else {
            uri
        };
        let name = if name.is_empty() {
            filename.to_string()
        } else {
            name
        };
        let mime_type = Some(super::rest::guess_content_type(&flow_path.path).to_string());

        config.resources.push(McpResourceRegistration {
            uri,
            name,
            flow_path,
            description: description.filter(|value| !value.is_empty()),
            mime_type,
        });
        context
            .set_pin_value("config_out", flow_like_types::json::json!(config))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct RegisterMcpPromptNode {}

impl RegisterMcpPromptNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for RegisterMcpPromptNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mcp_register_prompt",
            "Register MCP Prompt",
            "Registers a static MCP prompt template.",
            "Web/MCP",
        );
        node.add_icon("/flow/icons/web.svg");
        node.add_input_pin(
            "config_in",
            "Config",
            "MCP server config",
            NodeVariableType::Struct,
        )
        .set_schema::<McpServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("name", "Name", "Prompt name", NodeVariableType::String);
        node.add_input_pin(
            "description",
            "Description",
            "Optional description",
            NodeVariableType::String,
        );
        node.add_input_pin(
            "template",
            "Template",
            "Prompt template",
            NodeVariableType::String,
        );
        node.add_output_pin(
            "config_out",
            "Config",
            "Updated config",
            NodeVariableType::Struct,
        )
        .set_schema::<McpServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut config: McpServerConfig = context.evaluate_pin("config_in").await?;
        let name: String = context.evaluate_pin("name").await?;
        let description: Option<String> = context.evaluate_pin("description").await.ok();
        let template: String = context.evaluate_pin("template").await?;
        config.prompts.push(McpPromptRegistration {
            name,
            description: description.filter(|value| !value.is_empty()),
            template,
        });
        context
            .set_pin_value("config_out", flow_like_types::json::json!(config))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct McpServerNode {}

impl McpServerNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for McpServerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mcp_server",
            "MCP Server",
            "Starts an MCP server from a composed config.",
            "Web/MCP",
        );
        node.add_icon("/flow/icons/web.svg");
        node.set_version(MCP_CONFIG_NODE_VERSION);
        node.set_long_running(true);
        node.add_input_pin(
            "exec_in",
            "Execute",
            "Start server",
            NodeVariableType::Execution,
        );
        node.add_input_pin(
            "config",
            "Config",
            "MCP server config",
            NodeVariableType::Struct,
        )
        .set_schema::<McpServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "on_listening",
            "On Listening",
            "Fires when the server is ready",
            NodeVariableType::Execution,
        );
        node.add_output_pin(
            "local_addr",
            "Local Addr",
            "Bound address",
            NodeVariableType::String,
        );
        node.add_output_pin(
            "on_close",
            "On Close",
            "Fires when the server stops",
            NodeVariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Fires if the server cannot start",
            NodeVariableType::Execution,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use std::sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        };

        context.deactivate_exec_pin("on_listening").await?;
        context.deactivate_exec_pin("on_close").await?;
        context.activate_exec_pin("exec_error").await?;

        let config: McpServerConfig = context.evaluate_pin("config").await?;

        // Remote build: don't bind a socket. Emit the composed config so the
        // setup-collector on the API side can persist the registration, then
        // fire on_listening and return.
        #[cfg(all(feature = "remote", not(feature = "local")))]
        {
            if let Err(err) = super::remote::emit_remote_server_config(
                context,
                super::remote::RemoteServerKind::Mcp,
                &config,
            )
            .await
            {
                context.log_message(
                    &format!("MCP remote config emission failed: {}", err),
                    LogLevel::Error,
                );
                return Ok(());
            }
            context
                .set_pin_value("local_addr", json!(format!("remote://mcp/{}", config.host)))
                .await?;
            context.deactivate_exec_pin("exec_error").await?;
            context.activate_exec_pin("on_listening").await?;
            trigger_connected_exec(context, "on_listening", "MCP server (remote)").await;
            return Ok(());
        }

        #[cfg(not(all(feature = "remote", not(feature = "local"))))]
        {
            let addr = format!("{}:{}", config.host, config.port);
            let listener = match tokio::net::TcpListener::bind(&addr).await {
                Ok(listener) => listener,
                Err(err) => {
                    context.log_message(
                        &format!("MCP server bind failed on {}: {}", addr, err),
                        LogLevel::Error,
                    );
                    return Ok(());
                }
            };

            let tls_acceptor = match super::tls::server_acceptor(&config.tls) {
                Ok(acceptor) => acceptor,
                Err(err) => {
                    context.log_message(
                        &format!("MCP server TLS configuration failed: {}", err),
                        LogLevel::Error,
                    );
                    return Ok(());
                }
            };

            let tool_contexts = build_tool_contexts(context, &config.function_refs).await;
            let resources = preload_resources(context, &config.resources).await;
            let oauth_validator =
                match super::auth::build_oauth_validator(context, &config.auth).await {
                    Ok(validator) => validator,
                    Err(err) => {
                        context.log_message(
                            &format!("MCP server OAuth configuration failed: {}", err),
                            LogLevel::Error,
                        );
                        return Ok(());
                    }
                };
            let local_addr = listener.local_addr()?.to_string();
            context
                .set_pin_value("local_addr", json!(local_addr))
                .await?;
            context.deactivate_exec_pin("exec_error").await?;
            context.activate_exec_pin("on_listening").await?;
            trigger_connected_exec(context, "on_listening", "MCP server on_listening").await;

            let parent_node_id = context.node.node.lock().await.id.clone();
            let config = Arc::new(config);
            let tool_contexts = Arc::new(tool_contexts);
            let resources = Arc::new(resources);
            let oauth_validator = Arc::new(oauth_validator);
            let sessions: SessionMap = Arc::new(flow_like_types::sync::Mutex::new(HashMap::new()));
            let cancellation_token = context.get_cancellation_token();
            let active_connections = Arc::new(AtomicU32::new(0));
            let mut handles = Vec::new();
            let mut cancelled = false;

            loop {
                let accept = if config.timeout_seconds > 0 {
                    tokio::select! {
                        result = listener.accept() => Some(result),
                        _ = tokio::time::sleep(std::time::Duration::from_secs(config.timeout_seconds)) => {
                            context.log_message("MCP server timed out", LogLevel::Warn);
                            None
                        }
                        _ = super::wait_for_cancel(cancellation_token.clone()) => {
                            cancelled = true;
                            context.log_message("MCP server cancelled", LogLevel::Warn);
                            None
                        }
                    }
                } else {
                    tokio::select! {
                        result = listener.accept() => Some(result),
                        _ = super::wait_for_cancel(cancellation_token.clone()) => {
                            cancelled = true;
                            context.log_message("MCP server cancelled", LogLevel::Warn);
                            None
                        }
                    }
                };

                let Some(accept) = accept else {
                    break;
                };
                let (stream, remote_addr) = match accept {
                    Ok(pair) => pair,
                    Err(err) => {
                        context.log_message(&format!("MCP accept error: {}", err), LogLevel::Error);
                        continue;
                    }
                };

                if config.max_connections > 0
                    && active_connections.load(Ordering::Relaxed) >= config.max_connections
                {
                    use tokio::io::AsyncWriteExt;
                    let mut stream = stream;
                    let _ = stream
                    .write_all(
                        b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await;
                    let _ = stream.shutdown().await;
                    context.log_message(
                        "MCP server rejected request because max_connections was reached",
                        LogLevel::Warn,
                    );
                    continue;
                }

                let stream: super::tls::BoxedIo = if let Some(acceptor) = &tls_acceptor {
                    match acceptor.accept(stream).await {
                        Ok(stream) => Box::new(stream),
                        Err(err) => {
                            context.log_message(
                                &format!("MCP TLS handshake failed: {}", err),
                                LogLevel::Error,
                            );
                            continue;
                        }
                    }
                } else {
                    Box::new(stream)
                };

                active_connections.fetch_add(1, Ordering::Relaxed);
                let config = config.clone();
                let tool_contexts = tool_contexts.clone();
                let resources = resources.clone();
                let oauth_validator = oauth_validator.clone();
                let sessions = sessions.clone();
                let active_connections = active_connections.clone();
                let parent_node_id = parent_node_id.clone();
                let conn_cancel = cancellation_token.clone();
                handles.push(tokio::spawn(async move {
                    handle_connection(
                        stream,
                        remote_addr.to_string(),
                        config,
                        tool_contexts,
                        resources,
                        oauth_validator,
                        parent_node_id,
                        sessions,
                        conn_cancel,
                    )
                    .await;
                    active_connections.fetch_sub(1, Ordering::Relaxed);
                }));
            }

            for handle in handles {
                if !handle.is_finished() {
                    handle.abort();
                }
            }
            context.deactivate_exec_pin("on_listening").await?;
            context.activate_exec_pin("on_close").await?;
            trigger_connected_exec(context, "on_close", "MCP server on_close").await;

            if cancelled {
                return Err(flow_like_types::anyhow!("Execution was cancelled"));
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "MCP server requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
#[derive(Clone)]
struct ToolEntry {
    name: String,
    description: Option<String>,
    schema: flow_like_types::Value,
    context: super::http_runtime::SharedFunctionContext,
    argument_aliases: HashMap<String, String>,
}

#[cfg(feature = "execute")]
#[derive(Clone)]
struct CachedMcpResource {
    uri: String,
    name: String,
    description: Option<String>,
    mime_type: String,
    bytes: Vec<u8>,
}

#[cfg(feature = "execute")]
async fn build_tool_contexts(
    context: &ExecutionContext,
    function_refs: &[String],
) -> HashMap<String, ToolEntry> {
    let mut map: HashMap<String, ToolEntry> = HashMap::new();
    let board_refs = context
        .get_board()
        .await
        .map(|board| board.refs.clone())
        .unwrap_or_default();
    for id in function_refs {
        let Some(node) = context.nodes.get(id) else {
            continue;
        };
        let (base_name, description, schema, argument_aliases) =
            tool_metadata(node, &board_refs).await;
        let mut name = base_name.clone();
        let mut suffix = 2u32;
        while map.contains_key(&name) {
            name = format!("{}_{}", base_name, suffix);
            suffix += 1;
        }
        let shared = super::http_runtime::create_shared_function_context(context, node).await;
        map.insert(
            name.clone(),
            ToolEntry {
                name,
                description,
                schema,
                context: shared,
                argument_aliases,
            },
        );
    }
    map
}

#[cfg(feature = "execute")]
async fn tool_metadata(
    node: &std::sync::Arc<InternalNode>,
    board_refs: &HashMap<String, String>,
) -> (
    String,
    Option<String>,
    flow_like_types::Value,
    HashMap<String, String>,
) {
    let node_guard = node.node.lock().await;
    let name_source = if node_guard.friendly_name.trim().is_empty() {
        node_guard.name.as_str()
    } else {
        node_guard.friendly_name.as_str()
    };
    let name = super::http_runtime::sanitize_identifier(name_source);
    let description = resolved_mcp_description(&node_guard.description, board_refs);
    let has_non_payload_data_pin = node_guard.pins.values().any(|pin| {
        pin.pin_type == PinType::Output
            && pin.data_type != VariableType::Execution
            && pin.name != "payload"
            && pin.name != "_client"
    });

    let mut properties = json::Map::new();
    let mut argument_aliases = HashMap::new();
    let mut used_argument_names = HashSet::new();
    for pin in node_guard.pins.values() {
        if pin.pin_type != PinType::Output || pin.data_type == VariableType::Execution {
            continue;
        }
        if pin.name == "_client" {
            continue;
        }
        if pin.name == "payload" && has_non_payload_data_pin {
            continue;
        }
        let argument_name = unique_tool_argument_name(pin, &used_argument_names);
        used_argument_names.insert(argument_name.clone());
        register_tool_argument_aliases(&mut argument_aliases, &argument_name, pin);

        let resolved_schema = resolve_mcp_schema_ref(pin.schema.as_deref(), board_refs);
        let resolved_description =
            resolved_mcp_description(&pin.description, board_refs).unwrap_or_default();
        let mut schema = pin_schema(
            &pin.data_type,
            &pin.value_type,
            resolved_schema.as_deref(),
            &resolved_description,
        );
        if let Some(obj) = schema.as_object_mut()
            && !pin.friendly_name.trim().is_empty()
        {
            obj.entry("title".to_string())
                .or_insert_with(|| json!(pin.friendly_name.trim()));
        }
        properties.insert(argument_name, schema);
    }

    (
        name,
        description,
        json!({
            "type": "object",
            "properties": properties,
            "additionalProperties": true
        }),
        argument_aliases,
    )
}

#[cfg(feature = "execute")]
fn resolve_mcp_text_ref(value: &str, board_refs: &HashMap<String, String>) -> String {
    let trimmed = value.trim();
    if trimmed == MCP_EMPTY_STRING_HASH {
        return String::new();
    }
    board_refs
        .get(trimmed)
        .cloned()
        .unwrap_or_else(|| trimmed.to_string())
}

#[cfg(feature = "execute")]
fn resolved_mcp_description(
    description: &str,
    board_refs: &HashMap<String, String>,
) -> Option<String> {
    let resolved = resolve_mcp_text_ref(description, board_refs);
    let trimmed = resolved.trim();
    if trimmed.is_empty() || trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(feature = "execute")]
fn resolve_mcp_schema_ref(
    schema: Option<&str>,
    board_refs: &HashMap<String, String>,
) -> Option<String> {
    schema.map(|schema| resolve_mcp_text_ref(schema, board_refs))
}

#[cfg(feature = "execute")]
fn unique_tool_argument_name(pin: &Pin, used: &HashSet<String>) -> String {
    let friendly = super::http_runtime::sanitize_identifier(pin.friendly_name.trim());
    let raw = super::http_runtime::sanitize_identifier(pin.name.trim());

    for candidate in [&friendly, &raw] {
        if !candidate.is_empty() && !used.contains(candidate) {
            return candidate.clone();
        }
    }

    let base = if !friendly.is_empty() {
        friendly
    } else if !raw.is_empty() {
        raw
    } else {
        "arg".to_string()
    };
    let mut candidate = base.clone();
    let mut suffix = 2u32;
    while used.contains(&candidate) {
        candidate = format!("{}_{}", base, suffix);
        suffix += 1;
    }
    candidate
}

#[cfg(feature = "execute")]
fn register_tool_argument_aliases(
    aliases: &mut HashMap<String, String>,
    public_name: &str,
    pin: &Pin,
) {
    register_tool_argument_alias(aliases, public_name, &pin.name);
    register_tool_argument_alias(aliases, &pin.name, &pin.name);
    register_tool_argument_alias(
        aliases,
        &super::http_runtime::sanitize_identifier(&pin.name),
        &pin.name,
    );
    register_tool_argument_alias(aliases, &pin.friendly_name, &pin.name);
    register_tool_argument_alias(
        aliases,
        &super::http_runtime::sanitize_identifier(&pin.friendly_name),
        &pin.name,
    );
}

#[cfg(feature = "execute")]
fn register_tool_argument_alias(
    aliases: &mut HashMap<String, String>,
    alias: &str,
    pin_name: &str,
) {
    let trimmed = alias.trim();
    if trimmed.is_empty() {
        return;
    }
    aliases
        .entry(trimmed.to_string())
        .or_insert_with(|| pin_name.to_string());
}

#[cfg(feature = "execute")]
fn normalize_tool_arguments(
    arguments: flow_like_types::Value,
    tool: &ToolEntry,
) -> flow_like_types::Value {
    let Some(args) = arguments.as_object() else {
        return arguments;
    };

    let mut normalized = json::Map::new();
    for (key, value) in args {
        let target_key = tool
            .argument_aliases
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.clone());
        if target_key != *key && normalized.contains_key(&target_key) {
            continue;
        }
        normalized.insert(target_key, value.clone());
    }

    flow_like_types::Value::Object(normalized)
}

#[cfg(feature = "execute")]
fn pin_schema(
    data_type: &VariableType,
    value_type: &ValueType,
    schema: Option<&str>,
    description: &str,
) -> flow_like_types::Value {
    let mut base = match data_type {
        VariableType::String | VariableType::PathBuf | VariableType::Date => {
            json!({"type": "string"})
        }
        VariableType::Integer | VariableType::Byte => json!({"type": "integer"}),
        VariableType::Float => json!({"type": "number"}),
        VariableType::Boolean => json!({"type": "boolean"}),
        VariableType::Struct | VariableType::Generic => schema
            .and_then(|schema| json::from_str::<flow_like_types::Value>(schema).ok())
            .unwrap_or_else(|| json!({"type": "object"})),
        VariableType::Execution => json!({"type": "null"}),
    };
    if let Some(obj) = base.as_object_mut()
        && !description.is_empty()
    {
        obj.insert("description".to_string(), json!(description));
    }
    match value_type {
        ValueType::Array | ValueType::HashSet => json!({"type": "array", "items": base}),
        ValueType::HashMap => json!({"type": "object", "additionalProperties": base}),
        ValueType::Normal => base,
    }
}

#[cfg(feature = "execute")]
async fn preload_resources(
    context: &mut ExecutionContext,
    resources: &[McpResourceRegistration],
) -> Vec<CachedMcpResource> {
    let mut out = Vec::new();
    for resource in resources {
        match resource.flow_path.get(context, false).await {
            Ok(bytes) => out.push(CachedMcpResource {
                uri: resource.uri.clone(),
                name: resource.name.clone(),
                description: resource.description.clone(),
                mime_type: resource
                    .mime_type
                    .clone()
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| {
                        super::rest::guess_content_type(&resource.flow_path.path).to_string()
                    }),
                bytes,
            }),
            Err(err) => context.log_message(
                &format!("MCP resource preload failed for {}: {}", resource.uri, err),
                LogLevel::Error,
            ),
        }
    }
    out
}

#[cfg(feature = "execute")]
#[allow(dead_code)]
struct McpSession {
    protocol_version: String,
    client: flow_like_types::Value,
    sse_tx: tokio::sync::broadcast::Sender<String>,
    initialized: bool,
    created_at: std::time::Instant,
}

#[cfg(feature = "execute")]
type SessionMap = std::sync::Arc<
    flow_like_types::sync::Mutex<
        HashMap<String, std::sync::Arc<flow_like_types::sync::Mutex<McpSession>>>,
    >,
>;

#[cfg(feature = "execute")]
fn negotiate_protocol_version(requested: Option<&str>) -> String {
    match requested {
        Some(v) if MCP_SUPPORTED_PROTOCOL_VERSIONS.contains(&v) => v.to_string(),
        _ => MCP_DEFAULT_PROTOCOL_VERSION.to_string(),
    }
}

#[cfg(feature = "execute")]
fn cors_origin_header(origin: Option<&str>) -> String {
    origin.unwrap_or("*").to_string()
}

#[cfg(feature = "execute")]
fn apply_cors_headers(response: &mut super::http_runtime::HttpResponse, origin: Option<&str>) {
    response.headers.insert(
        "access-control-allow-origin".to_string(),
        cors_origin_header(origin),
    );
    response.headers.insert(
        "access-control-expose-headers".to_string(),
        "Mcp-Session-Id, MCP-Protocol-Version, WWW-Authenticate".to_string(),
    );
    if origin.is_some() {
        response
            .headers
            .insert("vary".to_string(), "Origin".to_string());
        response.headers.insert(
            "access-control-allow-credentials".to_string(),
            "true".to_string(),
        );
    }
}

#[cfg(feature = "execute")]
fn cors_preflight_response(
    request: &super::http_runtime::HttpRequest,
    origin: Option<&str>,
) -> super::http_runtime::HttpResponse {
    let mut response = super::http_runtime::HttpResponse::text(204, "");
    let requested_headers = request
        .headers
        .get("access-control-request-headers")
        .cloned()
        .unwrap_or_else(|| {
            "Content-Type, Authorization, Mcp-Session-Id, MCP-Protocol-Version, Last-Event-ID, Accept"
                .to_string()
        });
    response.headers.insert(
        "access-control-allow-methods".to_string(),
        "GET, POST, DELETE, OPTIONS".to_string(),
    );
    response.headers.insert(
        "access-control-allow-headers".to_string(),
        requested_headers,
    );
    response
        .headers
        .insert("access-control-max-age".to_string(), "86400".to_string());
    apply_cors_headers(&mut response, origin);
    response
}

#[cfg(feature = "execute")]
fn resource_url(config: &McpServerConfig, host_header: Option<&str>) -> String {
    let authority = host_header
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            let uses_default_port = (!config.tls.secure && config.port == 80)
                || (config.tls.secure && config.port == 443)
                || config.port == 0;
            if uses_default_port {
                config.host.clone()
            } else {
                format!("{}:{}", config.host, config.port)
            }
        });
    format!(
        "{}://{}{}",
        if config.tls.secure { "https" } else { "http" },
        authority,
        super::http_runtime::normalize_path(&config.path)
    )
}

#[cfg(feature = "execute")]
fn add_www_authenticate(
    response: &mut super::http_runtime::HttpResponse,
    config: &McpServerConfig,
    host_header: Option<&str>,
) {
    let resource_uri = resource_url(config, host_header);
    let header = format!(
        "Bearer realm=\"mcp\", resource_metadata=\"{}{}\"",
        resource_uri.trim_end_matches('/'),
        MCP_WELL_KNOWN_OAUTH_PATH
    );
    response
        .headers
        .insert("www-authenticate".to_string(), header);
}

#[cfg(feature = "execute")]
fn oauth_metadata_response(
    config: &McpServerConfig,
    origin: Option<&str>,
    host_header: Option<&str>,
) -> super::http_runtime::HttpResponse {
    let resource = resource_url(config, host_header);
    let body = if let RestAuthConfig::OAuthBearer {
        issuer,
        oidc_discovery_url,
        required_scopes,
        ..
    } = &config.auth
    {
        let mut body = json!({
            "resource": resource,
            "bearer_methods_supported": ["header"],
            "resource_documentation": "https://modelcontextprotocol.io",
        });
        if let Some(map) = body.as_object_mut() {
            if oidc_discovery_url.is_some()
                && let Some(iss) = issuer.as_ref().filter(|v| !v.trim().is_empty())
            {
                map.insert(
                    "authorization_servers".to_string(),
                    json!([iss.trim().to_string()]),
                );
            }
            if !required_scopes.is_empty() {
                map.insert("scopes_supported".to_string(), json!(required_scopes));
            }
        }
        body
    } else {
        json!({
            "resource": resource,
            "bearer_methods_supported": [],
        })
    };
    let mut response = super::http_runtime::HttpResponse::json(200, body);
    apply_cors_headers(&mut response, origin);
    response
}

#[cfg(feature = "execute")]
fn parse_accept_types(accept: &str) -> (bool, bool) {
    let trimmed = accept.trim();
    if trimmed.is_empty() || trimmed.contains("*/*") {
        return (true, true);
    }
    let mut wants_json = false;
    let mut wants_sse = false;
    for piece in trimmed.split(',') {
        let mime = piece
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if mime == "application/json" || mime == "application/*" {
            wants_json = true;
        }
        if mime == "text/event-stream" || mime == "text/*" {
            wants_sse = true;
        }
    }
    (wants_json, wants_sse)
}

#[cfg(feature = "execute")]
fn request_session_id(request: &super::http_runtime::HttpRequest) -> Option<String> {
    request
        .headers
        .get("mcp-session-id")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            request
                .query
                .get(MCP_LEGACY_SESSION_QUERY_PARAM)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
}

#[cfg(feature = "execute")]
fn new_session_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

#[cfg(feature = "execute")]
async fn create_session(
    client: &flow_like_types::Value,
    protocol_version: &str,
    sessions: &SessionMap,
) -> (
    String,
    std::sync::Arc<flow_like_types::sync::Mutex<McpSession>>,
) {
    let session_id = new_session_id();
    let (tx, _rx) = tokio::sync::broadcast::channel::<String>(64);
    let session = std::sync::Arc::new(flow_like_types::sync::Mutex::new(McpSession {
        protocol_version: protocol_version.to_string(),
        client: client.clone(),
        sse_tx: tx,
        initialized: false,
        created_at: std::time::Instant::now(),
    }));
    sessions
        .lock()
        .await
        .insert(session_id.clone(), session.clone());
    (session_id, session)
}

#[cfg(feature = "execute")]
fn legacy_message_endpoint(
    config: &McpServerConfig,
    host_header: Option<&str>,
    session_id: &str,
) -> String {
    format!(
        "{}?{}={}",
        resource_url(config, host_header),
        MCP_LEGACY_SESSION_QUERY_PARAM,
        session_id
    )
}

#[cfg(feature = "execute")]
async fn handle_connection(
    mut stream: super::tls::BoxedIo,
    remote_addr: String,
    config: std::sync::Arc<McpServerConfig>,
    tools: std::sync::Arc<HashMap<String, ToolEntry>>,
    resources: std::sync::Arc<Vec<CachedMcpResource>>,
    oauth_validator: std::sync::Arc<Option<super::auth::OAuthValidator>>,
    parent_node_id: String,
    sessions: SessionMap,
    cancellation_token: Option<flow_like_types::tokio_util::sync::CancellationToken>,
) {
    let request = match super::http_runtime::read_http_request(
        &mut *stream,
        remote_addr.clone(),
        config.max_body_bytes,
    )
    .await
    {
        Ok(Some(request)) => request,
        Ok(None) => return,
        Err(err) => {
            let mut response =
                super::http_runtime::HttpResponse::text(400, format!("Bad request: {}", err));
            apply_cors_headers(&mut response, None);
            let _ = super::http_runtime::write_http_response(&mut *stream, response).await;
            return;
        }
    };

    let origin = request.headers.get("origin").cloned();
    let origin_ref = origin.as_deref();
    let normalized_path = super::http_runtime::normalize_path(&request.path);
    let mcp_path = super::http_runtime::normalize_path(&config.path);

    if request.method == "OPTIONS" {
        let response = cors_preflight_response(&request, origin_ref);
        let _ = super::http_runtime::write_http_response(&mut *stream, response).await;
        return;
    }

    if normalized_path == MCP_WELL_KNOWN_OAUTH_PATH && request.method == "GET" {
        let response = oauth_metadata_response(
            &config,
            origin_ref,
            request.headers.get("host").map(String::as_str),
        );
        let _ = super::http_runtime::write_http_response(&mut *stream, response).await;
        return;
    }

    if normalized_path != mcp_path {
        let mut response = super::http_runtime::HttpResponse::text(404, "Not Found");
        apply_cors_headers(&mut response, origin_ref);
        let _ = super::http_runtime::write_http_response(&mut *stream, response).await;
        return;
    }

    let client = match super::auth::authorize_client(
        &config.auth,
        oauth_validator.as_ref().as_ref(),
        &request,
        "mcp",
    ) {
        Ok(client) => client,
        Err(mut response) => {
            if matches!(config.auth, RestAuthConfig::OAuthBearer { .. }) {
                add_www_authenticate(
                    &mut response,
                    &config,
                    request.headers.get("host").map(String::as_str),
                );
            }
            apply_cors_headers(&mut response, origin_ref);
            let _ = super::http_runtime::write_http_response(&mut *stream, response).await;
            return;
        }
    };

    let session_id = request_session_id(&request);
    let is_legacy_session = !request.headers.contains_key("mcp-session-id")
        && request.query.contains_key(MCP_LEGACY_SESSION_QUERY_PARAM);

    match request.method.as_str() {
        "POST" => {
            handle_post_request(
                &mut *stream,
                request,
                &config,
                &tools,
                &resources,
                &parent_node_id,
                client,
                &sessions,
                session_id,
                origin,
                is_legacy_session,
            )
            .await;
        }
        "GET" => {
            handle_get_sse(
                &mut *stream,
                &request,
                &config,
                &sessions,
                session_id,
                &client,
                origin_ref,
                request.headers.get("host").map(String::as_str),
                cancellation_token,
            )
            .await;
        }
        "DELETE" => {
            let response = match session_id {
                Some(sid) => {
                    let mut map = sessions.lock().await;
                    if map.remove(&sid).is_some() {
                        let mut r = super::http_runtime::HttpResponse::text(204, "");
                        apply_cors_headers(&mut r, origin_ref);
                        r
                    } else {
                        let mut r =
                            super::http_runtime::HttpResponse::text(404, "Session not found");
                        apply_cors_headers(&mut r, origin_ref);
                        r
                    }
                }
                None => {
                    let mut r = super::http_runtime::HttpResponse::text(
                        400,
                        "Missing Mcp-Session-Id header",
                    );
                    apply_cors_headers(&mut r, origin_ref);
                    r
                }
            };
            let _ = super::http_runtime::write_http_response(&mut *stream, response).await;
        }
        _ => {
            let mut response = super::http_runtime::HttpResponse::text(405, "Method Not Allowed");
            response.headers.insert(
                "allow".to_string(),
                "GET, POST, DELETE, OPTIONS".to_string(),
            );
            apply_cors_headers(&mut response, origin_ref);
            let _ = super::http_runtime::write_http_response(&mut *stream, response).await;
        }
    }
}

#[cfg(feature = "execute")]
#[allow(clippy::too_many_arguments)]
async fn handle_post_request<S>(
    stream: &mut S,
    request: super::http_runtime::HttpRequest,
    config: &McpServerConfig,
    tools: &HashMap<String, ToolEntry>,
    resources: &[CachedMcpResource],
    parent_node_id: &str,
    client: flow_like_types::Value,
    sessions: &SessionMap,
    session_id: Option<String>,
    origin: Option<String>,
    is_legacy_session: bool,
) where
    S: tokio::io::AsyncWrite + Unpin + Send + ?Sized,
{
    use tokio::io::AsyncWriteExt;
    let origin_ref = origin.as_deref();
    let accept = request.headers.get("accept").cloned().unwrap_or_default();
    let (wants_json, wants_sse) = parse_accept_types(&accept);

    let payload = match json::from_slice::<flow_like_types::Value>(&request.body) {
        Ok(v) => v,
        Err(_) => {
            let mut r = super::http_runtime::HttpResponse::json(
                400,
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}}),
            );
            apply_cors_headers(&mut r, origin_ref);
            let _ = super::http_runtime::write_http_response(stream, r).await;
            return;
        }
    };

    let (items, is_batch) = if let Some(arr) = payload.as_array() {
        (arr.clone(), true)
    } else {
        (vec![payload.clone()], false)
    };

    let has_initialize = items
        .iter()
        .any(|i| i.get("method").and_then(|m| m.as_str()) == Some("initialize"));

    let request_protocol_header = request.headers.get("mcp-protocol-version").cloned();

    let mut effective_session: Option<std::sync::Arc<flow_like_types::sync::Mutex<McpSession>>> =
        None;
    let mut assigned_session_id: Option<String> = None;

    if let Some(sid) = session_id.as_deref() {
        let map = sessions.lock().await;
        if let Some(session) = map.get(sid) {
            effective_session = Some(session.clone());
            assigned_session_id = Some(sid.to_string());
        } else if !has_initialize {
            drop(map);
            let mut r = super::http_runtime::HttpResponse::json(
                404,
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32001, "message": "Session not found"}}),
            );
            apply_cors_headers(&mut r, origin_ref);
            let _ = super::http_runtime::write_http_response(stream, r).await;
            return;
        }
    } else if !has_initialize {
        let mut r = super::http_runtime::HttpResponse::json(
            400,
            json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600, "message": "Missing Mcp-Session-Id header"}}),
        );
        apply_cors_headers(&mut r, origin_ref);
        let _ = super::http_runtime::write_http_response(stream, r).await;
        return;
    }

    if let Some(session) = &effective_session
        && let Some(client_protocol) = request_protocol_header.as_deref()
    {
        let session_locked = session.lock().await;
        if client_protocol != session_locked.protocol_version
            && !MCP_SUPPORTED_PROTOCOL_VERSIONS.contains(&client_protocol)
        {
            drop(session_locked);
            let mut r = super::http_runtime::HttpResponse::json(
                400,
                json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {
                        "code": -32600,
                        "message": format!("Unsupported MCP-Protocol-Version: {}", client_protocol)
                    }
                }),
            );
            apply_cors_headers(&mut r, origin_ref);
            let _ = super::http_runtime::write_http_response(stream, r).await;
            return;
        }
    }

    let mut responses: Vec<flow_like_types::Value> = Vec::new();
    for item in &items {
        let method = item.get("method").and_then(|v| v.as_str()).unwrap_or("");
        if method == "initialize" {
            let existing_session = assigned_session_id.as_ref().and_then(|sid| {
                effective_session
                    .as_ref()
                    .map(|session| (sid.clone(), session.clone()))
            });
            let (response, sid, session) =
                handle_initialize(item, &client, sessions, existing_session).await;
            assigned_session_id = Some(sid);
            effective_session = Some(session);
            if let Some(r) = response {
                responses.push(r);
            }
        } else {
            if method == "notifications/initialized"
                && let Some(session) = &effective_session
            {
                session.lock().await.initialized = true;
            }
            if let Some(response) =
                dispatch_json_rpc(item, config, tools, resources, parent_node_id, &client).await
            {
                responses.push(response);
            }
        }
    }

    if responses.is_empty() {
        let mut r = super::http_runtime::HttpResponse::text(202, "");
        if let Some(sid) = &assigned_session_id {
            r.headers.insert("mcp-session-id".to_string(), sid.clone());
        }
        apply_cors_headers(&mut r, origin_ref);
        let _ = super::http_runtime::write_http_response(stream, r).await;
        return;
    }

    if is_legacy_session && let Some(session) = &effective_session {
        let tx = session.lock().await.sse_tx.clone();
        for resp in &responses {
            let data = json::to_string(resp).unwrap_or_else(|_| "{}".to_string());
            let _ = tx.send(data);
        }
        let mut r = super::http_runtime::HttpResponse::text(202, "");
        if let Some(sid) = &assigned_session_id {
            r.headers.insert("mcp-session-id".to_string(), sid.clone());
        }
        apply_cors_headers(&mut r, origin_ref);
        let _ = super::http_runtime::write_http_response(stream, r).await;
        return;
    }

    let body_value = if is_batch {
        flow_like_types::Value::Array(responses.clone())
    } else {
        responses[0].clone()
    };

    if !wants_json && !wants_sse {
        let mut r = super::http_runtime::HttpResponse::text(
            406,
            "Not Acceptable: client must accept application/json or text/event-stream",
        );
        apply_cors_headers(&mut r, origin_ref);
        let _ = super::http_runtime::write_http_response(stream, r).await;
        return;
    }

    if wants_sse {
        let mut extra: Vec<(String, String)> = Vec::new();
        if let Some(sid) = &assigned_session_id {
            extra.push(("mcp-session-id".to_string(), sid.clone()));
        }
        extra.push((
            "access-control-allow-origin".to_string(),
            cors_origin_header(origin_ref),
        ));
        extra.push((
            "access-control-expose-headers".to_string(),
            "Mcp-Session-Id, MCP-Protocol-Version".to_string(),
        ));
        if origin_ref.is_some() {
            extra.push(("vary".to_string(), "Origin".to_string()));
            extra.push((
                "access-control-allow-credentials".to_string(),
                "true".to_string(),
            ));
        }
        if super::http_runtime::write_sse_response_head(stream, 200, &extra)
            .await
            .is_err()
        {
            return;
        }
        if is_batch {
            for resp in &responses {
                let data = json::to_string(resp).unwrap_or_else(|_| "{}".to_string());
                let event_id = uuid::Uuid::new_v4().simple().to_string();
                if super::http_runtime::write_sse_event(
                    stream,
                    Some("message"),
                    &data,
                    Some(&event_id),
                )
                .await
                .is_err()
                {
                    return;
                }
            }
        } else {
            let data = json::to_string(&body_value).unwrap_or_else(|_| "{}".to_string());
            let event_id = uuid::Uuid::new_v4().simple().to_string();
            let _ = super::http_runtime::write_sse_event(
                stream,
                Some("message"),
                &data,
                Some(&event_id),
            )
            .await;
        }
        let _ = stream.shutdown().await;
    } else {
        let mut r = super::http_runtime::HttpResponse::json(200, body_value);
        if let Some(sid) = &assigned_session_id {
            r.headers.insert("mcp-session-id".to_string(), sid.clone());
        }
        apply_cors_headers(&mut r, origin_ref);
        let _ = super::http_runtime::write_http_response(stream, r).await;
    }
}

#[cfg(feature = "execute")]
async fn handle_initialize(
    payload: &flow_like_types::Value,
    client: &flow_like_types::Value,
    sessions: &SessionMap,
    existing_session: Option<(
        String,
        std::sync::Arc<flow_like_types::sync::Mutex<McpSession>>,
    )>,
) -> (
    Option<flow_like_types::Value>,
    String,
    std::sync::Arc<flow_like_types::sync::Mutex<McpSession>>,
) {
    let id = payload.get("id").cloned();
    let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));
    let requested_protocol = params
        .get("protocolVersion")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let negotiated = negotiate_protocol_version(requested_protocol.as_deref());

    let (session_id, session) = if let Some((session_id, session)) = existing_session {
        {
            let mut session_locked = session.lock().await;
            session_locked.protocol_version = negotiated.clone();
            session_locked.client = client.clone();
            session_locked.initialized = false;
            session_locked.created_at = std::time::Instant::now();
        }
        (session_id, session)
    } else {
        create_session(client, &negotiated, sessions).await
    };

    let result = json!({
        "protocolVersion": negotiated,
        "capabilities": {
            "tools": {"listChanged": false},
            "resources": {"subscribe": false, "listChanged": false},
            "prompts": {"listChanged": false},
            "logging": {},
            "completions": {}
        },
        "serverInfo": {
            "name": "Flow Like MCP Server",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "Flow Like MCP server exposing flow functions as tools."
    });

    let response = id
        .as_ref()
        .map(|_| json!({"jsonrpc": "2.0", "id": id, "result": result}));
    (response, session_id, session)
}

#[cfg(feature = "execute")]
async fn handle_get_sse<S>(
    stream: &mut S,
    request: &super::http_runtime::HttpRequest,
    config: &McpServerConfig,
    sessions: &SessionMap,
    session_id: Option<String>,
    client: &flow_like_types::Value,
    origin: Option<&str>,
    host_header: Option<&str>,
    cancellation_token: Option<flow_like_types::tokio_util::sync::CancellationToken>,
) where
    S: tokio::io::AsyncWrite + Unpin + Send + ?Sized,
{
    use tokio::io::AsyncWriteExt;
    let accept = request.headers.get("accept").cloned().unwrap_or_default();
    let (_, wants_sse) = parse_accept_types(&accept);
    if !wants_sse {
        let mut r = super::http_runtime::HttpResponse::text(
            406,
            "Not Acceptable: GET requires Accept: text/event-stream",
        );
        apply_cors_headers(&mut r, origin);
        let _ = super::http_runtime::write_http_response(stream, r).await;
        return;
    }

    let (sid, session, legacy_endpoint) = if let Some(sid) = session_id {
        let session = {
            let map = sessions.lock().await;
            map.get(&sid).cloned()
        };
        let Some(session) = session else {
            let mut r = super::http_runtime::HttpResponse::text(404, "Session not found");
            apply_cors_headers(&mut r, origin);
            let _ = super::http_runtime::write_http_response(stream, r).await;
            return;
        };
        (sid, session, None)
    } else {
        let (sid, session) = create_session(client, MCP_DEFAULT_PROTOCOL_VERSION, sessions).await;
        (
            sid.clone(),
            session,
            Some(legacy_message_endpoint(config, host_header, &sid)),
        )
    };

    let mut extra: Vec<(String, String)> = vec![
        ("mcp-session-id".to_string(), sid.clone()),
        (
            "access-control-allow-origin".to_string(),
            cors_origin_header(origin),
        ),
        (
            "access-control-expose-headers".to_string(),
            "Mcp-Session-Id, MCP-Protocol-Version".to_string(),
        ),
    ];
    if origin.is_some() {
        extra.push(("vary".to_string(), "Origin".to_string()));
        extra.push((
            "access-control-allow-credentials".to_string(),
            "true".to_string(),
        ));
    }
    if super::http_runtime::write_sse_response_head(stream, 200, &extra)
        .await
        .is_err()
    {
        return;
    }

    if let Some(endpoint) = legacy_endpoint.as_deref()
        && super::http_runtime::write_sse_event(stream, Some("endpoint"), endpoint, None)
            .await
            .is_err()
    {
        return;
    }

    let mut rx = session.lock().await.sse_tx.subscribe();
    let mut heartbeat =
        tokio::time::interval(std::time::Duration::from_secs(MCP_SSE_HEARTBEAT_SECONDS));
    heartbeat.tick().await;

    loop {
        tokio::select! {
            _ = super::wait_for_cancel(cancellation_token.clone()) => break,
            _ = heartbeat.tick() => {
                if super::http_runtime::write_sse_comment(stream, "keepalive").await.is_err() {
                    break;
                }
            }
            msg = rx.recv() => {
                match msg {
                    Ok(data) => {
                        let event_id = uuid::Uuid::new_v4().simple().to_string();
                        if super::http_runtime::write_sse_event(
                            stream,
                            Some("message"),
                            &data,
                            Some(&event_id),
                        )
                        .await
                        .is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    let _ = stream.shutdown().await;
}

#[cfg(feature = "execute")]
async fn dispatch_json_rpc(
    payload: &flow_like_types::Value,
    config: &McpServerConfig,
    tools: &HashMap<String, ToolEntry>,
    resources: &[CachedMcpResource],
    parent_node_id: &str,
    client: &flow_like_types::Value,
) -> Option<flow_like_types::Value> {
    let id = payload.get("id").cloned();
    let method = payload
        .get("method")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));
    let notification = id.is_none();

    let result = match method {
        // initialize is handled separately by handle_initialize so a session can be created
        "initialize" => {
            return Some(json_rpc_error(
                id,
                -32600,
                "initialize must be handled at the transport layer",
            ));
        }
        "notifications/initialized"
        | "notifications/cancelled"
        | "notifications/roots/list_changed"
        | "notifications/progress" => return None,
        "ping" => json!({}),
        "tools/list" => json!({
            "tools": tools.values().map(|tool| {
                let mut entry = json::Map::new();
                entry.insert("name".to_string(), json!(tool.name));
                if let Some(desc) = &tool.description {
                    entry.insert("description".to_string(), json!(desc));
                }
                entry.insert("inputSchema".to_string(), tool.schema.clone());
                flow_like_types::Value::Object(entry)
            }).collect::<Vec<_>>()
        }),
        "tools/call" => {
            return Some(tool_call_response(id, params, tools, parent_node_id, client).await);
        }
        "resources/list" => json!({
            "resources": resources.iter().map(|resource| {
                let mut entry = json::Map::new();
                entry.insert("uri".to_string(), json!(resource.uri));
                entry.insert("name".to_string(), json!(resource.name));
                if let Some(desc) = &resource.description {
                    entry.insert("description".to_string(), json!(desc));
                }
                entry.insert("mimeType".to_string(), json!(resource.mime_type));
                flow_like_types::Value::Object(entry)
            }).collect::<Vec<_>>()
        }),
        "resources/templates/list" => json!({"resourceTemplates": []}),
        "resources/read" => match resource_read_result(&params, resources) {
            Ok(value) => value,
            Err((code, message)) => return Some(json_rpc_error(id, code, &message)),
        },
        "resources/subscribe" | "resources/unsubscribe" => json!({}),
        "prompts/list" => json!({
            "prompts": config.prompts.iter().map(|prompt| {
                let mut entry = json::Map::new();
                entry.insert("name".to_string(), json!(prompt.name));
                if let Some(desc) = &prompt.description {
                    entry.insert("description".to_string(), json!(desc));
                }
                entry.insert("arguments".to_string(), json!(prompt_argument_specs(&prompt.template)));
                flow_like_types::Value::Object(entry)
            }).collect::<Vec<_>>()
        }),
        "prompts/get" => match prompt_get_result(&params, &config.prompts) {
            Ok(value) => value,
            Err((code, message)) => return Some(json_rpc_error(id, code, &message)),
        },
        "logging/setLevel" => json!({}),
        "completion/complete" => json!({
            "completion": {
                "values": [],
                "total": 0,
                "hasMore": false
            }
        }),
        _ => {
            if notification {
                return None;
            }
            return Some(json_rpc_error(id, -32601, "Method not found"));
        }
    };

    if notification {
        None
    } else {
        Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }
}

#[cfg(feature = "execute")]
async fn tool_call_response(
    id: Option<flow_like_types::Value>,
    params: flow_like_types::Value,
    tools: &HashMap<String, ToolEntry>,
    parent_node_id: &str,
    client: &flow_like_types::Value,
) -> flow_like_types::Value {
    let name = params
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let Some(tool) = tools.get(name) else {
        return json_rpc_error(id, -32602, &format!("Unknown tool: {}", name));
    };

    let normalized_arguments = normalize_tool_arguments(arguments, tool);
    let mut args = normalized_arguments
        .as_object()
        .cloned()
        .unwrap_or_default();
    args.insert(
        "payload".to_string(),
        super::auth::payload_with_client(normalized_arguments, client),
    );
    args.insert("_client".to_string(), client.clone());
    match super::http_runtime::trigger_shared_function_context(
        &tool.context,
        &flow_like_types::Value::Object(args),
        parent_node_id,
        "MCP tool handler",
    )
    .await
    {
        Ok(value) => {
            let text = result_text(&value);
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{"type": "text", "text": text}],
                    "isError": false
                }
            })
        }
        Err(err) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{"type": "text", "text": err.to_string()}],
                "isError": true
            }
        }),
    }
}

#[cfg(feature = "execute")]
fn resource_read_result(
    params: &flow_like_types::Value,
    resources: &[CachedMcpResource],
) -> Result<flow_like_types::Value, (i64, String)> {
    let uri = params
        .get("uri")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if uri.is_empty() {
        return Err((-32602, "Missing required parameter: uri".to_string()));
    }
    let Some(resource) = resources.iter().find(|resource| resource.uri == uri) else {
        return Err((-32002, format!("Resource not found: {}", uri)));
    };

    let content = match std::str::from_utf8(&resource.bytes) {
        Ok(text) => json!({
            "uri": resource.uri,
            "mimeType": resource.mime_type,
            "text": text
        }),
        Err(_) => {
            use base64::Engine;
            json!({
                "uri": resource.uri,
                "mimeType": resource.mime_type,
                "blob": base64::engine::general_purpose::STANDARD.encode(&resource.bytes)
            })
        }
    };
    Ok(json!({"contents": [content]}))
}

#[cfg(feature = "execute")]
fn prompt_argument_specs(template: &str) -> Vec<flow_like_types::Value> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    let bytes = template.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let start = i + 2;
            if let Some(end) = template[start..].find("}}") {
                let name = template[start..start + end].trim();
                if !name.is_empty() && seen.insert(name.to_string()) {
                    out.push(json!({
                        "name": name,
                        "required": true
                    }));
                }
                i = start + end + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

#[cfg(feature = "execute")]
fn substitute_prompt_template(template: &str, arguments: &flow_like_types::Value) -> String {
    let map = arguments.as_object().cloned().unwrap_or_default();
    let mut output = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let start = i + 2;
            if let Some(rel_end) = template[start..].find("}}") {
                let name = template[start..start + rel_end].trim();
                if let Some(value) = map.get(name) {
                    match value {
                        flow_like_types::Value::String(s) => output.push_str(s),
                        other => output.push_str(&json::to_string(other).unwrap_or_default()),
                    }
                } else {
                    output.push_str(&template[i..start + rel_end + 2]);
                }
                i = start + rel_end + 2;
                continue;
            }
        }
        output.push(template[i..].chars().next().unwrap_or(' '));
        i += template[i..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(1);
    }
    output
}

#[cfg(feature = "execute")]
fn prompt_get_result(
    params: &flow_like_types::Value,
    prompts: &[McpPromptRegistration],
) -> Result<flow_like_types::Value, (i64, String)> {
    let name = params
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if name.is_empty() {
        return Err((-32602, "Missing required parameter: name".to_string()));
    }
    let Some(prompt) = prompts.iter().find(|prompt| prompt.name == name) else {
        return Err((-32602, format!("Prompt not found: {}", name)));
    };
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let rendered = substitute_prompt_template(&prompt.template, &arguments);
    Ok(json!({
        "description": prompt.description,
        "messages": [{
            "role": "user",
            "content": {"type": "text", "text": rendered}
        }]
    }))
}

#[cfg(feature = "execute")]
fn result_text(value: &flow_like_types::Value) -> String {
    value
        .as_str()
        .map(ToString::to_string)
        .unwrap_or_else(|| json::to_string(value).unwrap_or_default())
}

#[cfg(feature = "execute")]
fn json_rpc_error(
    id: Option<flow_like_types::Value>,
    code: i64,
    message: &str,
) -> flow_like_types::Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[cfg(feature = "execute")]
async fn trigger_connected_exec(context: &mut ExecutionContext, pin_name: &str, log_name: &str) {
    let Ok(pin) = context.get_pin_by_name(pin_name).await else {
        return;
    };

    for node in pin.get_connected_nodes() {
        let mut sub = context.create_sub_context(&node).await;
        sub.delegated = true;
        let mut message = LogMessage::new(log_name, LogLevel::Debug, None);
        let _ = InternalNode::trigger(&mut sub, &mut None, true).await;
        message.end();
        sub.log(message);
        sub.end_trace();
        context.push_sub_context(&mut sub);
    }
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use crate::web::test_support::{internal_node, internal_node_with_logic, test_context};
    use flow_like::flow::node::NodeLogic;
    use flow_like_types::{Value, async_trait, json::json};
    use std::sync::Arc;

    #[derive(Default)]
    struct McpEchoLogic;

    #[async_trait]
    impl NodeLogic for McpEchoLogic {
        fn get_node(&self) -> Node {
            Node::new("mcp_echo", "MCP Echo", "MCP echo test", "Tests")
        }

        async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            let message: String = context.evaluate_pin("message").await?;
            let client: Value = context.evaluate_pin("_client").await?;
            context.set_result(json!({
                "echo": message,
                "client": client
            }));
            Ok(())
        }
    }

    fn mcp_handler() -> Arc<InternalNode> {
        let mut node = Node::new(
            "test_mcp_handler",
            "Test MCP Handler",
            "Handler test node",
            "Tests",
        );
        node.add_output_pin(
            "message",
            "Message",
            "Message to echo",
            VariableType::String,
        );
        node.add_output_pin("_client", "Client", "Client", VariableType::Struct);
        internal_node_with_logic(node, Arc::new(McpEchoLogic))
    }

    const MCP_TOOL_DESC_REF: &str = "8701601262111675572";
    const MCP_TOOL_PIN_REF_NAME: &str = "992233445566778899";
    const MCP_TOOL_PIN_DESC_REF: &str = "331144225566778899";

    #[derive(Default)]
    struct McpRefLogic;

    #[async_trait]
    impl NodeLogic for McpRefLogic {
        fn get_node(&self) -> Node {
            Node::new("mcp_ref_echo", "MCP Ref Echo", "MCP ref echo test", "Tests")
        }

        async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            let value: String = context.evaluate_pin(MCP_TOOL_PIN_REF_NAME).await?;
            context.set_result(json!({ "echo": value }));
            Ok(())
        }
    }

    fn mcp_ref_handler() -> Arc<InternalNode> {
        let mut node = Node::new("cool_tool", "Cool Tool", MCP_TOOL_DESC_REF, "Tests");
        node.add_output_pin(
            MCP_TOOL_PIN_REF_NAME,
            "Name",
            MCP_TOOL_PIN_DESC_REF,
            VariableType::String,
        );
        internal_node_with_logic(node, Arc::new(McpRefLogic))
    }

    #[tokio::test]
    async fn mcp_tools_list_uses_registered_function_pin_schema() {
        let handler = mcp_handler();
        let parent = internal_node(McpServerNode::new().get_node());
        let context = test_context(parent, vec![handler.clone()]).await;
        let tools = build_tool_contexts(&context, &[handler.node_id().to_string()]).await;
        let request = super::super::http_runtime::HttpRequest {
            method: "POST".to_string(),
            path: "/mcp".to_string(),
            query: HashMap::new(),
            headers: HashMap::new(),
            body: flow_like_types::json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/list"
            }))
            .unwrap(),
            remote_addr: "127.0.0.1:1234".to_string(),
        };

        let client = super::super::auth::client_metadata(&request, "mcp", None);
        let payload: Value = flow_like_types::json::from_slice(&request.body).unwrap();
        let response = dispatch_json_rpc(
            &payload,
            &McpServerConfig::default(),
            &tools,
            &[],
            "parent",
            &client,
        )
        .await
        .expect("tools/list response");
        assert_eq!(
            response["result"]["tools"][0]["name"],
            json!("test_mcp_handler")
        );
        assert_eq!(
            response["result"]["tools"][0]["inputSchema"]["properties"]["message"]["type"],
            json!("string")
        );
        assert!(
            response["result"]["tools"][0]["inputSchema"]["properties"]
                .get("_client")
                .is_none()
        );
    }

    #[tokio::test]
    async fn mcp_tool_call_injects_client_metadata_and_returns_text_content() {
        let handler = mcp_handler();
        let parent = internal_node(McpServerNode::new().get_node());
        let context = test_context(parent, vec![handler.clone()]).await;
        let tools = build_tool_contexts(&context, &[handler.node_id().to_string()]).await;
        let request = super::super::http_runtime::HttpRequest {
            method: "POST".to_string(),
            path: "/mcp".to_string(),
            query: HashMap::new(),
            headers: HashMap::new(),
            body: flow_like_types::json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "test_mcp_handler",
                    "arguments": {
                        "message": "hello",
                        "_client": "attacker"
                    }
                }
            }))
            .unwrap(),
            remote_addr: "127.0.0.1:5678".to_string(),
        };

        let client = super::super::auth::client_metadata(&request, "mcp", None);
        let payload: Value = flow_like_types::json::from_slice(&request.body).unwrap();
        let response = dispatch_json_rpc(
            &payload,
            &McpServerConfig::default(),
            &tools,
            &[],
            "parent",
            &client,
        )
        .await
        .expect("tools/call response");
        assert_eq!(response["result"]["isError"], json!(false));
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default();
        let parsed: Value = flow_like_types::json::from_str(text).unwrap();
        assert_eq!(parsed["echo"], json!("hello"));
        assert_eq!(parsed["client"]["protocol"], json!("mcp"));
        assert_eq!(parsed["client"]["remote_addr"], json!("127.0.0.1:5678"));
        // Spec compliance: tools without outputSchema must NOT include structuredContent
        assert!(response["result"].get("structuredContent").is_none());
    }

    #[tokio::test]
    async fn mcp_tool_metadata_resolves_ref_descriptions_and_friendly_pin_names() {
        let handler = mcp_ref_handler();
        let refs = HashMap::from([
            (
                MCP_TOOL_DESC_REF.to_string(),
                "Resolved tool description".to_string(),
            ),
            (
                MCP_TOOL_PIN_DESC_REF.to_string(),
                "Resolved pin description".to_string(),
            ),
        ]);

        let (name, description, schema, argument_aliases) = tool_metadata(&handler, &refs).await;

        assert_eq!(name, "cool_tool");
        assert_eq!(description, Some("Resolved tool description".to_string()));
        assert_eq!(
            schema["properties"]["name"]["description"],
            json!("Resolved pin description")
        );
        assert_eq!(
            argument_aliases.get("name"),
            Some(&MCP_TOOL_PIN_REF_NAME.to_string())
        );
    }

    #[tokio::test]
    async fn mcp_tool_call_normalizes_friendly_argument_names_to_raw_pin_names() {
        let handler = mcp_ref_handler();
        let parent = internal_node(McpServerNode::new().get_node());
        let context = test_context(parent, vec![handler.clone()]).await;
        let (name, description, schema, argument_aliases) =
            tool_metadata(&handler, &HashMap::new()).await;
        let mut tools = HashMap::new();
        tools.insert(
            name.clone(),
            ToolEntry {
                name: name.clone(),
                description,
                schema,
                context: super::super::http_runtime::create_shared_function_context(
                    &context, &handler,
                )
                .await,
                argument_aliases,
            },
        );

        let response = tool_call_response(
            Some(json!(1)),
            json!({
                "name": name,
                "arguments": {
                    "name": "Felix"
                }
            }),
            &tools,
            "parent",
            &json!({}),
        )
        .await;

        let text = response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default();
        let parsed: Value = flow_like_types::json::from_str(text).unwrap();
        assert_eq!(parsed["echo"], json!("Felix"));
    }

    #[tokio::test]
    async fn mcp_initialize_negotiates_protocol_and_creates_session() {
        let sessions: SessionMap = Arc::new(flow_like_types::sync::Mutex::new(HashMap::new()));
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {"protocolVersion": "2025-06-18"}
        });
        let (response, sid, _session) =
            handle_initialize(&payload, &json!({}), &sessions, None).await;
        let response = response.expect("initialize must respond");
        assert_eq!(response["result"]["protocolVersion"], json!("2025-06-18"));
        assert!(!sid.is_empty());
        assert!(sessions.lock().await.contains_key(&sid));
    }

    #[tokio::test]
    async fn mcp_initialize_falls_back_to_default_protocol() {
        let sessions: SessionMap = Arc::new(flow_like_types::sync::Mutex::new(HashMap::new()));
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {"protocolVersion": "1999-01-01"}
        });
        let (response, _sid, _session) =
            handle_initialize(&payload, &json!({}), &sessions, None).await;
        let response = response.unwrap();
        assert_eq!(
            response["result"]["protocolVersion"],
            json!(MCP_DEFAULT_PROTOCOL_VERSION)
        );
    }

    #[tokio::test]
    async fn mcp_initialize_reuses_existing_legacy_session() {
        let sessions: SessionMap = Arc::new(flow_like_types::sync::Mutex::new(HashMap::new()));
        let client = json!({"kind": "legacy"});
        let (legacy_sid, legacy_session) =
            create_session(&client, MCP_DEFAULT_PROTOCOL_VERSION, &sessions).await;
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {"protocolVersion": "2025-03-26"}
        });

        let (response, sid, session) = handle_initialize(
            &payload,
            &client,
            &sessions,
            Some((legacy_sid.clone(), legacy_session.clone())),
        )
        .await;

        let response = response.expect("initialize must respond");
        assert_eq!(sid, legacy_sid);
        assert!(std::sync::Arc::ptr_eq(&session, &legacy_session));
        assert_eq!(response["result"]["protocolVersion"], json!("2025-03-26"));
        assert_eq!(sessions.lock().await.len(), 1);
    }

    #[test]
    fn mcp_request_session_id_accepts_legacy_query_param() {
        let request = super::super::http_runtime::HttpRequest {
            method: "POST".to_string(),
            path: "/mcp".to_string(),
            query: HashMap::from([(
                MCP_LEGACY_SESSION_QUERY_PARAM.to_string(),
                "abc123".to_string(),
            )]),
            headers: HashMap::new(),
            body: Vec::new(),
            remote_addr: "127.0.0.1:1234".to_string(),
        };

        assert_eq!(request_session_id(&request), Some("abc123".to_string()));
    }

    #[test]
    fn mcp_legacy_message_endpoint_uses_query_session_id() {
        let config = McpServerConfig {
            host: "127.0.0.1".to_string(),
            port: 5555,
            ..McpServerConfig::default()
        };

        assert_eq!(
            legacy_message_endpoint(&config, Some("127.0.0.1:5555"), "abc123"),
            "http://127.0.0.1:5555/mcp?sessionId=abc123"
        );
    }

    #[tokio::test]
    async fn mcp_resources_read_unknown_returns_jsonrpc_error() {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "resources/read",
            "params": {"uri": "file:///nonexistent"}
        });
        let response = dispatch_json_rpc(
            &payload,
            &McpServerConfig::default(),
            &HashMap::new(),
            &[],
            "parent",
            &json!({}),
        )
        .await
        .expect("resources/read response");
        assert_eq!(response["error"]["code"], json!(-32002));
    }

    #[tokio::test]
    async fn mcp_completion_complete_returns_well_formed_shape() {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "completion/complete",
            "params": {}
        });
        let response = dispatch_json_rpc(
            &payload,
            &McpServerConfig::default(),
            &HashMap::new(),
            &[],
            "parent",
            &json!({}),
        )
        .await
        .expect("completion/complete response");
        assert_eq!(response["result"]["completion"]["values"], json!([]));
        assert_eq!(response["result"]["completion"]["total"], json!(0));
        assert_eq!(response["result"]["completion"]["hasMore"], json!(false));
    }

    #[tokio::test]
    async fn mcp_prompt_substitution_replaces_placeholders() {
        let mut config = McpServerConfig::default();
        config.prompts.push(McpPromptRegistration {
            name: "greet".to_string(),
            description: Some("greeting".to_string()),
            template: "Hello {{ name }}, welcome to {{ place }}!".to_string(),
        });
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "prompts/get",
            "params": {
                "name": "greet",
                "arguments": {"name": "World", "place": "Earth"}
            }
        });
        let response = dispatch_json_rpc(
            &payload,
            &config,
            &HashMap::new(),
            &[],
            "parent",
            &json!({}),
        )
        .await
        .unwrap();
        assert_eq!(
            response["result"]["messages"][0]["content"]["text"],
            json!("Hello World, welcome to Earth!")
        );
    }

    #[tokio::test]
    async fn mcp_prompts_list_exposes_template_arguments() {
        let mut config = McpServerConfig::default();
        config.prompts.push(McpPromptRegistration {
            name: "greet".to_string(),
            description: None,
            template: "Hi {{ name }} from {{ place }} and {{ name }}".to_string(),
        });
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 12,
            "method": "prompts/list"
        });
        let response = dispatch_json_rpc(
            &payload,
            &config,
            &HashMap::new(),
            &[],
            "parent",
            &json!({}),
        )
        .await
        .unwrap();
        let args = response["result"]["prompts"][0]["arguments"]
            .as_array()
            .unwrap();
        assert_eq!(args.len(), 2);
        assert_eq!(args[0]["name"], json!("name"));
        assert_eq!(args[1]["name"], json!("place"));
    }

    #[tokio::test]
    async fn mcp_accept_parser_handles_streamable_http_combinations() {
        let (json, sse) = parse_accept_types("application/json, text/event-stream");
        assert!(json && sse);
        let (json, sse) = parse_accept_types("text/event-stream");
        assert!(!json && sse);
        let (json, sse) = parse_accept_types("application/json");
        assert!(json && !sse);
        let (json, sse) = parse_accept_types("*/*");
        assert!(json && sse);
        let (json, sse) = parse_accept_types("");
        assert!(json && sse);
        let (json, sse) = parse_accept_types("text/html");
        assert!(!json && !sse);
    }

    #[test]
    fn mcp_oauth_metadata_omits_authorization_servers_for_jwks_only_auth() {
        let config = McpServerConfig {
            auth: RestAuthConfig::OAuthBearer {
                issuer: Some("https://issuer.example".to_string()),
                audience: None,
                required_scopes: vec!["read".to_string()],
                jwks_url: Some("https://issuer.example/jwks.json".to_string()),
                jwks_flow_path: None,
                oidc_discovery_url: None,
            },
            ..McpServerConfig::default()
        };

        let response = oauth_metadata_response(&config, None, None);
        let body: Value = flow_like_types::json::from_slice(&response.body).unwrap();

        assert!(body.get("authorization_servers").is_none());
        assert_eq!(body["scopes_supported"], json!(["read"]));
    }

    #[test]
    fn mcp_oauth_metadata_includes_authorization_servers_for_oidc_auth() {
        let config = McpServerConfig {
            auth: RestAuthConfig::OAuthBearer {
                issuer: Some("https://issuer.example".to_string()),
                audience: None,
                required_scopes: Vec::new(),
                jwks_url: None,
                jwks_flow_path: None,
                oidc_discovery_url: Some(
                    "https://issuer.example/.well-known/openid-configuration".to_string(),
                ),
            },
            ..McpServerConfig::default()
        };

        let response = oauth_metadata_response(&config, None, None);
        let body: Value = flow_like_types::json::from_slice(&response.body).unwrap();

        assert_eq!(
            body["authorization_servers"],
            json!(["https://issuer.example"])
        );
    }

    #[test]
    fn mcp_oauth_metadata_uses_request_host_header_for_resource_url() {
        let config = McpServerConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            ..McpServerConfig::default()
        };

        let response = oauth_metadata_response(&config, None, Some("127.0.0.1:5555"));
        let body: Value = flow_like_types::json::from_slice(&response.body).unwrap();

        assert_eq!(body["resource"], json!("http://127.0.0.1:5555/mcp"));
    }

    #[test]
    fn mcp_www_authenticate_uses_request_host_header_for_resource_metadata() {
        let config = McpServerConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            ..McpServerConfig::default()
        };
        let mut response = super::super::http_runtime::HttpResponse::text(401, "Unauthorized");

        add_www_authenticate(&mut response, &config, Some("127.0.0.1:5555"));

        assert_eq!(
            response.headers.get("www-authenticate"),
            Some(
                &"Bearer realm=\"mcp\", resource_metadata=\"http://127.0.0.1:5555/mcp/.well-known/oauth-protected-resource\"".to_string(),
            )
        );
    }
}
