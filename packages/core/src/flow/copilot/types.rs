use serde::{Deserialize, Serialize};

fn is_false(value: &bool) -> bool {
    !*value
}

/// Metadata about a node in the catalog
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMetadata {
    pub name: String,
    pub friendly_name: String,
    pub description: String,
    pub inputs: Vec<PinMetadata>,
    pub outputs: Vec<PinMetadata>,
    pub category: Option<String>,
    #[serde(default)]
    pub required_inputs: Vec<String>,
    #[serde(default)]
    pub companion_nodes: Vec<String>,
    #[serde(default)]
    pub capability_tags: Vec<String>,
}

impl NodeMetadata {
    /// Convert to a minimal string format for token efficiency
    /// Format: "node_type: friendly_name - description (truncated)"
    pub fn to_compact(&self) -> String {
        // Truncate description to first ~50 chars
        let desc = if self.description.chars().count() > 50 {
            let truncated: String = self.description.chars().take(47).collect();
            format!("{}...", truncated)
        } else {
            self.description.clone()
        };

        format!("{}: {} - {}", self.name, self.friendly_name, desc)
    }

    /// Get detailed pin information (only call when needed)
    pub fn to_detailed(&self) -> String {
        let inputs: Vec<String> = self
            .inputs
            .iter()
            .filter(|p| p.data_type != "Execution")
            .map(|p| {
                if p.description.is_empty() {
                    format!("  - {} ({})", p.name, p.data_type)
                } else {
                    format!("  - {} ({}): {}", p.name, p.data_type, p.description)
                }
            })
            .collect();

        let outputs: Vec<String> = self
            .outputs
            .iter()
            .filter(|p| p.data_type != "Execution")
            .map(|p| format!("  - {} ({})", p.name, p.data_type))
            .collect();

        let mut result = format!(
            "Node: {}\nName: {}\nDescription: {}\n",
            self.name, self.friendly_name, self.description
        );

        if !inputs.is_empty() {
            result.push_str(&format!("Inputs:\n{}\n", inputs.join("\n")));
        }
        if !outputs.is_empty() {
            result.push_str(&format!("Outputs:\n{}\n", outputs.join("\n")));
        }
        result
    }
}

/// Metadata about a pin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinMetadata {
    pub name: String, // INTERNAL name - use this in commands! (e.g., "body_text")
    pub friendly_name: String, // Display name (e.g., "Body (text)")
    pub description: String, // Pin description
    pub data_type: String, // e.g., "String", "Integer", "Struct", "Generic", "Execution"
    pub value_type: String, // e.g., "Normal", "Array", "HashMap", "HashSet"
    #[serde(default)]
    pub default_value: Option<String>,
    pub schema: Option<String>,            // JSON schema for Struct types
    pub is_generic: bool,                  // Generic pins can connect to any type
    pub valid_values: Option<Vec<String>>, // For enum-like pins
    pub enforce_schema: bool,              // If true, schema must match exactly
}

/// Pin definition for placeholder nodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaceholderPinDef {
    pub name: String,          // Internal name for the pin
    pub friendly_name: String, // Display name for the pin
    pub description: Option<String>,
    pub pin_type: String,           // "Input" or "Output"
    pub data_type: String, // "String", "Integer", "Float", "Boolean", "Struct", "Generic", "Execution"
    pub value_type: Option<String>, // "Normal", "Array", "HashMap", "HashSet" (default: "Normal")
}

/// Edge in the graph
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Edge {
    pub id: String,
    pub from: String,
    pub from_pin: String,
    pub to: String,
    pub to_pin: String,
}

/// A suggestion for a node to add
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Suggestion {
    pub node_type: String,
    pub reason: String,
    pub connection_description: String,
    pub position: Option<NodePosition>,
    pub connections: Vec<Connection>,
}

/// Position of a node
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NodePosition {
    pub x: f64,
    pub y: f64,
}

/// A connection between nodes
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Connection {
    pub from_node_id: String,
    pub from_pin: String,
    pub to_pin: String,
}

/// A step in the execution plan
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PlanStep {
    pub id: String,
    pub description: String,
    pub status: PlanStepStatus,
    pub tool_name: Option<String>,
}

/// Status of a plan step
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// Events that can be streamed from the copilot
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum StreamEvent {
    Token(String),
    PlanStep(PlanStep),
    ToolCall {
        name: String,
        args: String,
    },
    ToolResult {
        name: String,
        result: String,
    },
    Thinking(String),
    FocusNode {
        node_id: String,
        description: String,
    },
}

/// Represents the type of response from the copilot
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AgentType {
    /// Response that primarily explains
    Explain,
    /// Response that includes modifications
    Edit,
}

/// An image attachment in a chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatImage {
    /// Base64-encoded image data (without data URL prefix)
    pub data: String,
    /// MIME type (e.g., "image/png", "image/jpeg")
    pub media_type: String,
}

/// A message in the chat history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    /// Optional images attached to this message (for vision models)
    #[serde(default)]
    pub images: Option<Vec<ChatImage>>,
}

/// Role in the chat conversation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChatRole {
    User,
    Assistant,
}

/// Response from the copilot that may include commands for the frontend to execute
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotResponse {
    pub agent_type: AgentType,
    pub message: String,
    pub commands: Vec<BoardCommand>,
    pub suggestions: Vec<Suggestion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flowscript_workspace: Option<String>,
    /// Exact typed-IR command batch retained by the host for atomic Apply/Dismiss review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_ir_commit: Option<FlowIrCommitToken>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowIrCommitToken {
    pub board_id: String,
    pub draft_id: String,
    pub revision: u64,
    pub base_fingerprint: String,
    pub claim_id: String,
    /// Host-derived review policy. This is a UI hint only: native Apply derives the same policy
    /// again from the retained draft and fails closed unless the host passes explicit approval.
    #[serde(default, skip_serializing_if = "is_false")]
    pub requires_destructive_approval: bool,
}

/// Context for a specific run (for log queries)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunContext {
    pub run_id: String,
    pub app_id: String,
    pub board_id: String,
}

/// Compact template info for model context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateInfo {
    pub id: String,
    pub app_id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub node_count: usize,
    pub node_types: Vec<String>,
}

/// Commands that can be executed on the board
/// These are sent to the frontend which executes them to maintain undo/redo history
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command_type")]
pub enum BoardCommand {
    AddNode {
        node_type: String,
        ref_id: Option<String>,
        position: Option<NodePosition>,
        #[serde(default)]
        friendly_name: Option<String>,
        /// Additional pins to append to the catalog node when it is created. FlowScript uses
        /// this for user-declared outputs on a new `events_generic` entry node.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        additional_pins: Option<Vec<PlaceholderPinDef>>,
        /// Target layer ID to place the node in. If None, uses current layer.
        #[serde(default)]
        target_layer: Option<String>,
        #[serde(default)]
        summary: Option<String>,
    },
    /// Add a placeholder node (layer) for process modeling
    /// Placeholders allow sketching workflows before implementing with real nodes
    AddPlaceholder {
        name: String,
        ref_id: Option<String>,
        position: Option<NodePosition>,
        #[serde(default)]
        pins: Option<Vec<PlaceholderPinDef>>,
        /// Target layer ID to place the placeholder in. If None, uses current layer.
        #[serde(default)]
        target_layer: Option<String>,
        #[serde(default)]
        summary: Option<String>,
    },
    RemoveNode {
        node_id: String,
        #[serde(default)]
        summary: Option<String>,
    },
    ConnectPins {
        from_node: String,
        from_pin: String,
        to_node: String,
        to_pin: String,
        #[serde(default)]
        summary: Option<String>,
    },
    DisconnectPins {
        from_node: String,
        from_pin: String,
        to_node: String,
        to_pin: String,
        #[serde(default)]
        summary: Option<String>,
    },
    UpdateNodePin {
        node_id: String,
        pin_id: String,
        value: serde_json::Value,
        #[serde(default)]
        summary: Option<String>,
    },
    /// Set a node's function references (e.g. an agent's registered tool functions). `fn_refs`
    /// carries the referenced targets as ref tokens (`$N` ref-ids, board node/layer anchors, or
    /// names) which the applier resolves to concrete node ids.
    SetNodeFunctionRefs {
        node_id: String,
        fn_refs: Vec<String>,
        #[serde(default)]
        summary: Option<String>,
    },
    MoveNode {
        node_id: String,
        position: NodePosition,
        /// Target layer ID to move the node to. If None, moves within current layer.
        #[serde(default)]
        target_layer: Option<String>,
        #[serde(default)]
        summary: Option<String>,
    },
    // Variable management
    CreateVariable {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variable_id: Option<String>,
        name: String,
        data_type: String,  // "String", "Integer", "Float", "Boolean", "Struct"
        value_type: String, // "Normal", "Array", "HashMap", "HashSet"
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_value: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        category: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exposed: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        secret: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        editable: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        runtime_configured: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_layer: Option<String>,
        #[serde(default)]
        summary: Option<String>,
    },
    UpdateVariable {
        variable_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_value: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "is_false")]
        clear_default_value: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        clear_description: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        category: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        clear_category: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        clear_schema: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exposed: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        secret: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        editable: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        runtime_configured: Option<bool>,
        /// Backward-compatible alias used by older UI/model commands for "set default value".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<serde_json::Value>,
        #[serde(default)]
        summary: Option<String>,
    },
    #[serde(rename = "DeleteVariable")]
    RemoveVariable {
        variable_id: String,
        #[serde(default)]
        summary: Option<String>,
    },
    // Comment management
    #[serde(rename = "CreateComment")]
    AddComment {
        content: String,
        position: NodePosition,
        width: Option<f64>,
        height: Option<f64>,
        color: Option<String>,
        /// Target layer ID to place the comment in. If None, uses current layer.
        #[serde(default)]
        target_layer: Option<String>,
        #[serde(default)]
        summary: Option<String>,
    },
    #[serde(rename = "DeleteComment")]
    RemoveComment {
        comment_id: String,
        #[serde(default)]
        summary: Option<String>,
    },
    // Layer/grouping management
    CreateLayer {
        name: String,
        /// Reference ID like "$0" for same-batch references. FlowScript reconcile uses this when
        /// creating a new function layer and placing its body nodes inside it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ref_id: Option<String>,
        /// Layer kind. Omitted/default means "Collapsed"; FlowScript functions use "Function".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layer_type: Option<String>,
        #[serde(default)]
        node_ids: Vec<String>, // Nodes to include in the layer
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pins: Option<Vec<PlaceholderPinDef>>,
        position: Option<NodePosition>,
        color: Option<String>,
        /// Parent layer ID. If None, creates at root or current layer.
        #[serde(default)]
        target_layer: Option<String>,
        #[serde(default)]
        summary: Option<String>,
    },
    RemoveLayer {
        layer_id: String,
        #[serde(default)]
        summary: Option<String>,
    },
}
