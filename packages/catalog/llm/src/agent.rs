use crate::embedding::CachedEmbeddingModel;
use flow_like::bit::Bit;
use flow_like_model_provider::history::{History, Tool};
use flow_like_types::JsonSchema;
use memory::MemoryConfig;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub mod add_datafusion;
pub mod from_model;
pub mod helpers;
pub mod invoke;
pub mod lazy_register_tools;
pub mod memory;
pub mod register_mcp_tools;
pub mod register_remote_mcp_tools;
pub mod register_thinking;
pub mod register_tools;
pub mod set_system_prompt;
pub mod simple;
pub mod stream_invoke;

/// MCP server registration with optional tool filtering
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpServerConfig {
    /// URI of the MCP server
    pub uri: String,

    /// Optional tool filter - if None, all tools are used
    /// If Some, only tools in this set are used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_filter: Option<HashSet<String>>,

    /// Optional bearer token (without the `Bearer ` prefix) sent in the
    /// Authorization header of every request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header: Option<String>,

    /// Connected app whose short-lived bearer should be resolved immediately
    /// before opening the MCP transport. Keeping identity instead of a token in
    /// the serialized agent avoids freezing an expiring credential in a pin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_app_id: Option<String>,

    /// MCP event hosted by `remote_app_id`. Remote transports reconstruct
    /// their URI from this validated identity and the freshly minted session;
    /// the serialized `uri` is never trusted as a bearer-token destination.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_event_id: Option<String>,

    /// Additional headers included with every MCP request. Connected-app MCP
    /// proxies use this for registration auth while `auth_header` remains the
    /// app-connection bearer token.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub custom_headers: HashMap<String, String>,
}

/// Builds the streamable-HTTP transport config for an MCP server,
/// attaching the configured Authorization header to every request.
#[cfg(feature = "execute")]
pub(crate) fn mcp_transport_config(
    config: &McpServerConfig,
) -> rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig {
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;

    let mut transport_config = StreamableHttpClientTransportConfig::with_uri(config.uri.clone());
    // RMCP expects the raw bearer token and adds the `Bearer` scheme itself.
    // Accept the legacy serialized form to avoid double-prefixing saved agents.
    transport_config.auth_header = config
        .auth_header
        .as_deref()
        .map(|value| value.strip_prefix("Bearer ").unwrap_or(value).to_string());
    transport_config.custom_headers = config
        .custom_headers
        .iter()
        .filter_map(|(name, value)| {
            match (
                flow_like_types::reqwest::header::HeaderName::from_bytes(name.as_bytes()),
                flow_like_types::reqwest::header::HeaderValue::from_bytes(value.as_bytes()),
            ) {
                (Ok(name), Ok(value)) => Some((name, value)),
                _ => {
                    // A dropped registration header would otherwise surface only
                    // as a downstream 401 with no cause; log which header so a
                    // stray control byte in a pasted credential is diagnosable.
                    tracing::warn!(
                        header = %name,
                        "Skipping MCP custom header that is not a valid HTTP header name/value"
                    );
                    None
                }
            }
        })
        .collect();
    transport_config
}

/// Builds an MCP transport and resolves a connected-app bearer at the last
/// responsible moment. The underlying session helper is run-scoped,
/// expiry-aware, and single-flight, so this is cheap for repeated agents.
#[cfg(feature = "execute")]
pub(crate) async fn mcp_transport_config_for_execution(
    config: &McpServerConfig,
    context: &flow_like::flow::execution::context::ExecutionContext,
) -> flow_like_types::Result<
    rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig,
> {
    match (
        config.remote_app_id.as_deref(),
        config.remote_event_id.as_deref(),
    ) {
        (None, None) => Ok(mcp_transport_config(config)),
        (Some(remote_app_id), Some(remote_event_id)) => {
            let remote_app_id = flow_like_catalog_data::remote_util::validate_path_id(
                remote_app_id,
                "remote project",
            )?;
            let remote_event_id = flow_like_catalog_data::remote_util::validate_path_id(
                remote_event_id,
                "remote event",
            )?;
            let session = flow_like_catalog_data::remote_util::remote_app_session_for_mcp(
                context,
                &remote_app_id,
            )
            .await?;
            let trusted_uri = session.url(&format!("events/{remote_event_id}/mcp"));
            Ok(mcp_transport_config_with_remote_credentials(
                config,
                trusted_uri,
                session.token,
            ))
        }
        _ => Err(flow_like_types::anyhow!(
            "Remote MCP configuration requires both a remote project and event"
        )),
    }
}

#[cfg(feature = "execute")]
fn mcp_transport_config_with_remote_credentials(
    config: &McpServerConfig,
    trusted_uri: String,
    token: String,
) -> rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig {
    let mut trusted = config.clone();
    trusted.uri = trusted_uri;
    trusted.auth_header = Some(token);
    mcp_transport_config(&trusted)
}

/// DataFusion session context for SQL-based data analysis
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DataFusionContext {
    /// Cache key to look up the session in ExecutionContext.cache
    pub session_cache_key: String,

    /// User-provided description of what this data represents
    /// e.g., "Sales data from 2020-2024 including customer demographics"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Per-table descriptions for better LLM understanding
    /// Key is the table name, value is description
    #[serde(default)]
    pub table_descriptions: HashMap<String, String>,

    /// Example SQL queries that work well with this data
    #[serde(default)]
    pub example_queries: Vec<String>,

    /// Auto-discovered table schemas (populated at runtime)
    /// Key is table name, value is schema description
    #[serde(default)]
    pub table_schemas: HashMap<String, String>,
}

impl DataFusionContext {
    pub fn new(session_cache_key: String) -> Self {
        Self {
            session_cache_key,
            description: None,
            table_descriptions: HashMap::new(),
            example_queries: Vec::new(),
            table_schemas: HashMap::new(),
        }
    }

    /// Generate system prompt extension for this DataFusion context
    pub fn generate_system_prompt_extension(&self) -> String {
        let mut prompt = String::new();

        if let Some(desc) = &self.description {
            prompt.push_str(&format!("**Data Context:** {}\n\n", desc));
        }

        if !self.table_schemas.is_empty() {
            prompt.push_str("**Available Tables:**\n");
            for (table_name, schema) in &self.table_schemas {
                if let Some(table_desc) = self.table_descriptions.get(table_name) {
                    prompt.push_str(&format!("- `{}`: {}\n", table_name, table_desc));
                } else {
                    prompt.push_str(&format!("- `{}`\n", table_name));
                }
                prompt.push_str(&format!("  Schema: {}\n", schema));
            }
            prompt.push('\n');
        }

        if !self.example_queries.is_empty() {
            prompt.push_str("**Example Queries:**\n```sql\n");
            for query in &self.example_queries {
                prompt.push_str(&format!("{}\n", query));
            }
            prompt.push_str("```\n\n");
        }

        prompt
    }
}

/// Reference to a lazy function tool index stored in a vector DB.
/// Allows agents to do hybrid search over a large pool of tools at execution time
/// instead of loading all tool schemas into the context upfront.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LazyFunctionRef {
    /// Cache key used to look up the LanceDB connection
    pub db_cache_key: String,
}

/// Context management strategy for infinite context mode
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq)]
pub enum ContextManagementMode {
    /// Sliding window truncation - removes oldest messages to fit budget.
    /// Fast, deterministic, no extra API costs. May lose important early context.
    #[default]
    Truncate,
    /// LLM summarization - compresses old messages into a summary.
    /// Preserves key information but adds latency and API cost.
    Summarize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Agent {
    /// The LLM model id backing this agent
    pub model: Bit,

    /// Model display name
    pub model_display_name: Option<String>,

    /// Maximum number of iterations/tool calls before stopping
    pub max_iterations: u64,

    /// System prompt for the agent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,

    /// Registered tools (function calling schemas for non-function tools)
    #[serde(default)]
    pub tools: Vec<Tool>,

    /// Function references (node_id -> node_name mapping)
    /// These are converted to tools at execution time to keep data slim
    #[serde(default)]
    pub function_refs: HashMap<String, String>,

    /// MCP servers with optional tool filtering
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,

    /// Whether the thinking tool is enabled
    #[serde(default)]
    pub thinking_enabled: bool,

    /// Optional conversation history to initialize with
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<History>,

    /// DataFusion sessions for SQL-based data analysis
    /// Multiple sessions can be added to give the agent access to different data sources
    #[serde(default)]
    pub datafusion_contexts: Vec<DataFusionContext>,

    /// Enable infinite context mode with automatic context window management.
    /// When enabled, applies the selected context management strategy.
    #[serde(default)]
    pub infinite_context: bool,

    /// Strategy for managing context when it exceeds the token budget.
    /// - Truncate: Sliding window, removes oldest messages (fast, no extra cost)
    /// - Summarize: LLM compresses old messages (preserves info, adds latency/cost)
    #[serde(default)]
    pub context_management_mode: ContextManagementMode,

    /// Maximum tokens to retain when truncating history in infinite context mode.
    /// Defaults to 32000 tokens if not specified. Only used when infinite_context is true.
    #[serde(default)]
    pub max_context_tokens: Option<u32>,

    /// Lazy function references backed by a vector DB index.
    /// At execution time the agent can search this index to dynamically discover
    /// and load only the tools it actually needs, keeping the context window lean.
    #[serde(default)]
    pub lazy_function_refs: Vec<LazyFunctionRef>,

    /// Embedding model shared across all lazy function tool indexes.
    /// The model's cache key is encoded into the vector DB table name, so
    /// swapping the model automatically uses a fresh table (old embeddings are abandoned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lazy_embedding_model: Option<CachedEmbeddingModel>,

    /// Persistent memory configuration. When set, the agent gains built-in
    /// `_memory_search`, `_memory_store`, and `_memory_compress` tools to
    /// autonomously store, recall, and compress observations across conversations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryConfig>,
}

impl Agent {
    /// Create a new agent from model id with default configuration
    pub fn new(model: Bit, max_iterations: u64) -> Self {
        Self {
            model,
            model_display_name: None,
            max_iterations,
            system_prompt: None,
            tools: Vec::new(),
            function_refs: HashMap::new(),
            mcp_servers: Vec::new(),
            thinking_enabled: false,
            history: None,
            datafusion_contexts: Vec::new(),
            infinite_context: false,
            context_management_mode: ContextManagementMode::default(),
            max_context_tokens: None,
            lazy_function_refs: Vec::new(),
            lazy_embedding_model: None,
            memory: None,
        }
    }

    /// Add a tool to this agent (for non-function tools)
    pub fn add_tool(&mut self, tool: Tool) {
        self.tools.push(tool);
    }

    /// Add a function reference (node_id -> node_name)
    pub fn add_function_ref(&mut self, node_id: String, node_name: String) {
        self.function_refs.insert(node_id, node_name);
    }

    /// Add an MCP server configuration
    pub fn add_mcp_server(&mut self, config: McpServerConfig) {
        self.mcp_servers.push(config);
    }

    /// Enable the thinking tool
    pub fn enable_thinking(&mut self) {
        self.thinking_enabled = true;
    }

    /// Add a DataFusion context for SQL data analysis
    pub fn add_datafusion_context(&mut self, context: DataFusionContext) {
        self.datafusion_contexts.push(context);
    }

    /// Enable infinite context mode with optional token limit
    pub fn enable_infinite_context(&mut self, max_tokens: Option<u32>) {
        self.infinite_context = true;
        self.max_context_tokens = max_tokens;
    }

    /// Set the context management mode (truncate vs summarize)
    pub fn set_context_management_mode(&mut self, mode: ContextManagementMode) {
        self.context_management_mode = mode;
    }
    /// Set the system prompt
    pub fn set_system_prompt(&mut self, prompt: String) {
        self.system_prompt = Some(prompt);
    }

    /// Set conversation history
    pub fn set_history(&mut self, history: History) {
        self.history = Some(history);
    }

    /// Get the effective system prompt (from history or direct field)
    /// Now also includes DataFusion context extensions
    pub fn get_system_prompt(&self) -> Option<String> {
        let mut base_prompt = if let Some(prompt) = &self.system_prompt {
            prompt.clone()
        } else if let Some(history) = &self.history {
            history.get_system_prompt().unwrap_or_default()
        } else {
            String::new()
        };

        // Append DataFusion context information
        if !self.datafusion_contexts.is_empty() {
            base_prompt.push_str("\n\n## Data Analysis Capabilities\n\n");
            base_prompt.push_str("You have access to SQL databases for data analysis. Use the `list_tables`, `describe_table`, and `execute_sql` tools to explore and query data.\n\n");

            for (i, df_ctx) in self.datafusion_contexts.iter().enumerate() {
                if self.datafusion_contexts.len() > 1 {
                    base_prompt.push_str(&format!("### Data Source {}\n", i + 1));
                }
                base_prompt.push_str(&df_ctx.generate_system_prompt_extension());
            }

            base_prompt.push_str("**Best Practices:**\n");
            base_prompt.push_str("1. Use `list_tables` to discover available tables\n");
            base_prompt.push_str("2. Use `describe_table` to understand schema before querying\n");
            base_prompt.push_str("3. Use LIMIT to avoid overwhelming output\n");
            base_prompt.push_str("4. Prefer aggregations and summaries over raw data dumps\n");
        }

        if !self.lazy_function_refs.is_empty() {
            base_prompt.push_str("\n\n## Tool Discovery\n\n");
            base_prompt.push_str(
                "You have access to a large pool of tools beyond what is listed in your current tool set. \
                 If you need a capability that you don't see in your available tools, call the `_lazy_search_tools` \
                 tool with a description of what you need. Matching tools will be added to your available tools \
                 for subsequent calls.\n\
                 - Only search when you genuinely need a tool you don't already have.\n\
                 - Be specific in your search query for best results.\n\
                 - After searching, call the discovered tool directly in your next step.\n",
            );
        }

        if self.memory.is_some() {
            base_prompt.push_str("\n\n## Memory\n\n");
            base_prompt.push_str(
                "You have persistent memory across conversations. Use these tools proactively:\n\n\
                 - `_memory_search`: Search your memory for relevant context.\n\
                   Call this at the START of each conversation to recall relevant context.\n\
                   Parameters: query (string), role_filter (optional, one of: \"user\", \"assistant\", \"observation\", \"summary\", \"context\")\n\n\
                 - `_memory_store`: Store important facts, user preferences, decisions, and context.\n\
                   Call this for any information worth remembering across conversations.\n\
                   Parameters: content (string), role (one of: \"user\", \"assistant\", \"observation\")\n\n\
                 Memory compression happens automatically — you do not need to manage it.\n\
                 When older conversation messages are evicted from context, they are also stored \
                 to memory automatically so you can retrieve them with `_memory_search`.\n\n\
                 **Guidelines:**\n\
                 - ALWAYS search memory at the start of a new conversation\n\
                 - If the user asks you to remember something, call `_memory_store` immediately\n\
                 - Do not merely say you will remember something; store it with `_memory_store`\n\
                 - Store key facts, preferences, and decisions after learning them\n\
                 - Don't store trivial or transient information\n",
            );
        }

        if base_prompt.trim().is_empty() {
            None
        } else {
            Some(base_prompt)
        }
    }

    /// Add a lazy function reference (points to a vector DB index)
    pub fn add_lazy_function_ref(&mut self, lazy_ref: LazyFunctionRef) {
        self.lazy_function_refs.push(lazy_ref);
    }

    /// Set the embedding model used for all lazy function tool indexes
    pub fn set_lazy_embedding_model(&mut self, model: CachedEmbeddingModel) {
        self.lazy_embedding_model = Some(model);
    }

    /// Check if this agent has any DataFusion contexts
    pub fn has_datafusion_contexts(&self) -> bool {
        !self.datafusion_contexts.is_empty()
    }

    /// Set persistent memory configuration
    pub fn set_memory(&mut self, config: MemoryConfig) {
        self.memory = Some(config);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generative::embedding::CachedEmbeddingModel;
    use flow_like::bit::{Bit, BitTypes};
    use flow_like_catalog_core::NodeDBConnection;

    fn memory_config() -> MemoryConfig {
        MemoryConfig::new(
            NodeDBConnection {
                cache_key: "memory-db".to_string(),
            },
            CachedEmbeddingModel {
                cache_key: "embedding-model".to_string(),
                model_type: BitTypes::Embedding,
            },
        )
    }

    #[test]
    fn memory_prompt_is_added_without_base_system_prompt() {
        let mut agent = Agent::new(Bit::default(), 4);
        agent.set_memory(memory_config());

        let prompt = agent
            .get_system_prompt()
            .expect("memory-enabled agent should get memory instructions");

        assert!(prompt.contains("## Memory"));
        assert!(prompt.contains("_memory_search"));
        assert!(prompt.contains("_memory_store"));
        assert!(prompt.contains("call `_memory_store` immediately"));
    }

    #[test]
    fn empty_agent_without_capabilities_has_no_system_prompt() {
        let agent = Agent::new(Bit::default(), 4);

        assert!(agent.get_system_prompt().is_none());
    }

    #[test]
    fn remote_mcp_config_serializes_identity_without_a_bearer() {
        let config = McpServerConfig {
            uri: "https://example.invalid/mcp".to_string(),
            tool_filter: None,
            auth_header: None,
            remote_app_id: Some("remote-app".to_string()),
            remote_event_id: Some("remote-event".to_string()),
            custom_headers: HashMap::new(),
        };

        let serialized = flow_like_types::json::to_value(&config).unwrap();
        assert_eq!(
            serialized
                .get("remote_app_id")
                .and_then(|value| value.as_str()),
            Some("remote-app")
        );
        assert_eq!(
            serialized
                .get("remote_event_id")
                .and_then(|value| value.as_str()),
            Some("remote-event")
        );
        assert!(serialized.get("auth_header").is_none());

        let legacy: McpServerConfig = flow_like_types::json::from_value(
            flow_like_types::json::json!({ "uri": "https://example.invalid/mcp" }),
        )
        .unwrap();
        assert!(legacy.remote_app_id.is_none());
        assert!(legacy.remote_event_id.is_none());
    }

    #[cfg(feature = "execute")]
    #[test]
    fn remote_mcp_bearer_ignores_a_serialized_untrusted_uri() {
        let crafted = McpServerConfig {
            uri: "https://attacker.invalid/collect".to_string(),
            tool_filter: None,
            auth_header: Some("attacker-controlled".to_string()),
            remote_app_id: Some("remote-app".to_string()),
            remote_event_id: Some("remote-event".to_string()),
            custom_headers: HashMap::new(),
        };

        let transport = mcp_transport_config_with_remote_credentials(
            &crafted,
            "https://hub.invalid/api/v1/apps/remote-app/events/remote-event/mcp".to_string(),
            "fresh-connection-token".to_string(),
        );

        assert_eq!(
            transport.uri.as_ref(),
            "https://hub.invalid/api/v1/apps/remote-app/events/remote-event/mcp"
        );
        assert_eq!(
            transport.auth_header.as_deref(),
            Some("fresh-connection-token")
        );
    }

    #[cfg(feature = "execute")]
    #[test]
    fn mcp_transport_keeps_connection_bearer_and_registration_headers_separate() {
        let config = McpServerConfig {
            uri: "https://example.invalid/mcp".to_string(),
            tool_filter: None,
            auth_header: Some("connection-token".to_string()),
            remote_app_id: None,
            remote_event_id: None,
            custom_headers: HashMap::from([
                (
                    "x-flow-like-event-authorization".to_string(),
                    "Bearer registration-token".to_string(),
                ),
                ("x-api-key".to_string(), "secret".to_string()),
            ]),
        };

        let transport = mcp_transport_config(&config);
        let event_auth_header = flow_like_types::reqwest::header::HeaderName::from_static(
            "x-flow-like-event-authorization",
        );
        let api_key_header = flow_like_types::reqwest::header::HeaderName::from_static("x-api-key");

        assert_eq!(transport.auth_header.as_deref(), Some("connection-token"));
        assert_eq!(transport.custom_headers.len(), 2);
        assert_eq!(
            transport
                .custom_headers
                .get(&event_auth_header)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer registration-token")
        );
        assert_eq!(
            transport
                .custom_headers
                .get(&api_key_header)
                .and_then(|value| value.to_str().ok()),
            Some("secret")
        );

        let mut legacy = config;
        legacy.auth_header = Some("Bearer legacy-token".to_string());
        assert_eq!(
            mcp_transport_config(&legacy).auth_header.as_deref(),
            Some("legacy-token")
        );
    }
}
