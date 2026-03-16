use flow_like::bit::Bit;
use flow_like_model_provider::history::{History, Tool};
use flow_like_types::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub mod add_datafusion;
pub mod from_model;
pub mod helpers;
pub mod invoke;
pub mod memory;
pub mod register_mcp_tools;
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
            return None;
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

        Some(base_prompt)
    }

    /// Check if this agent has any DataFusion contexts
    pub fn has_datafusion_contexts(&self) -> bool {
        !self.datafusion_contexts.is_empty()
    }
}
