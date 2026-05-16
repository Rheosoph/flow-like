use std::sync::Arc;

use rig::{completion::ToolDefinition, tool::Tool};
use serde::Deserialize;
use serde_json::json;

use super::provider::CatalogProvider;
use super::search::score_catalog_metadata;
use super::types::{BoardCommand, RunContext, TemplateInfo};
use crate::state::FlowLikeState;

// ============================================================================
// Tool Error Types
// ============================================================================

#[derive(Debug, thiserror::Error)]
#[error("Catalog tool error")]
pub struct CatalogToolError;

#[derive(Debug, thiserror::Error)]
#[error("Template tool error")]
pub struct TemplateToolError;

#[derive(Debug, thiserror::Error)]
#[error("Get node details tool error: {0}")]
pub struct GetNodeDetailsToolError(pub String);

#[derive(Debug, thiserror::Error)]
#[error("Board inspection tool error: {0}")]
pub struct BoardInspectionToolError(pub String);

#[derive(Debug, thiserror::Error)]
#[error("Emit commands tool error")]
pub struct EmitCommandsToolError;

#[derive(Debug, thiserror::Error)]
#[error("Query logs tool error: {0}")]
pub struct QueryLogsToolError(pub String);

// ============================================================================
// Tool Argument Types
// ============================================================================

#[derive(Deserialize)]
pub struct SearchArgs {
    pub query: String,
}

#[derive(Deserialize)]
pub struct SearchByPinArgs {
    pub pin_type: String,
    pub is_input: bool,
}

#[derive(Deserialize)]
pub struct FilterCategoryArgs {
    pub category_prefix: String,
}

#[derive(Deserialize)]
pub struct SearchTemplatesArgs {
    pub query: String,
}

#[derive(Deserialize)]
pub struct ThinkingArgs {
    pub thought: String,
}

#[derive(Deserialize)]
pub struct GetNodeDetailsArgs {
    pub node_id: String,
}

#[derive(Deserialize)]
pub struct FindConnectableNodesArgs {
    pub node_id: String,
    pub pin_name: String,
    pub intent: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct EmitCommandsArgs {
    pub commands: Vec<BoardCommand>,
    pub explanation: String,
}

#[derive(Deserialize, Debug)]
pub struct QueryLogsArgs {
    /// Optional filter query (e.g., "log_level = 4" for errors, "node_id = 'abc123'")
    pub filter: Option<String>,
    /// Maximum number of logs to return
    pub limit: Option<usize>,
}

// ============================================================================
// Catalog Search Tool
// ============================================================================

pub struct CatalogTool {
    pub provider: Arc<dyn CatalogProvider>,
}

impl Tool for CatalogTool {
    const NAME: &'static str = "catalog_search";

    type Error = CatalogToolError;
    type Args = SearchArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "catalog_search".to_string(),
            description: r#"Search the node catalog by functionality or name. Returns matching nodes with their node_type (needed for AddNode).

WHEN TO USE: Before adding any node - to get the exact node_type
EXAMPLE QUERIES: "http request", "parse json", "loop array", "condition if""#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural language search. Examples: 'http request', 'json parse', 'loop array'"
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let matches = self.provider.search(&args.query).await;
        // Use compact format for token efficiency
        let compact: Vec<String> = matches.iter().map(|m| m.to_compact()).collect();
        Ok(compact.join("\n"))
    }
}

// ============================================================================
// Search By Pin Tool
// ============================================================================

pub struct SearchByPinTool {
    pub provider: Arc<dyn CatalogProvider>,
}

impl Tool for SearchByPinTool {
    const NAME: &'static str = "search_by_pin";

    type Error = CatalogToolError;
    type Args = SearchByPinArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "search_by_pin".to_string(),
            description: r#"Find nodes compatible with a specific pin type. Use this to find nodes that can connect to an existing node's pin.

WHEN TO USE: Finding what can connect to/from a specific pin type
EXAMPLES: search_by_pin("String", true) finds nodes with String input pins"#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pin_type": {
                        "type": "string",
                        "description": "Data type: String, Integer, Float, Boolean, Struct, Generic, Execution"
                    },
                    "is_input": {
                        "type": "boolean",
                        "description": "true = find nodes with this INPUT pin type, false = find nodes with this OUTPUT pin type"
                    }
                },
                "required": ["pin_type", "is_input"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let matches = self
            .provider
            .search_by_pin_type(&args.pin_type, args.is_input)
            .await;
        // Use compact format for token efficiency
        let compact: Vec<String> = matches.iter().map(|m| m.to_compact()).collect();
        Ok(compact.join("\n"))
    }
}

// ============================================================================
// Filter Category Tool
// ============================================================================

pub struct FilterCategoryTool {
    pub provider: Arc<dyn CatalogProvider>,
}

impl Tool for FilterCategoryTool {
    const NAME: &'static str = "filter_category";

    type Error = CatalogToolError;
    type Args = FilterCategoryArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "filter_category".to_string(),
            description: r#"Browse nodes by category. Categories are hierarchical (e.g., "flow/control", "data/transform").

WHEN TO USE: Exploring what nodes exist in a domain
COMMON CATEGORIES: flow, data, http, math, logic, string, array"#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "category_prefix": {
                        "type": "string",
                        "description": "Category prefix like 'flow', 'data', 'http', 'math'. Use '/' for subcategories: 'flow/control'"
                    }
                },
                "required": ["category_prefix"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let matches = self
            .provider
            .filter_by_category(&args.category_prefix)
            .await;
        // Use compact format for token efficiency
        let compact: Vec<String> = matches.iter().map(|m| m.to_compact()).collect();
        Ok(compact.join("\n"))
    }
}

// ============================================================================
// Search Templates Tool
// ============================================================================

pub struct SearchTemplatesTool {
    pub templates: Vec<TemplateInfo>,
    pub current_template_id: Option<String>,
}

impl Tool for SearchTemplatesTool {
    const NAME: &'static str = "search_templates";

    type Error = TemplateToolError;
    type Args = SearchTemplatesArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "search_templates".to_string(),
            description: r#"Search for workflow templates - reusable patterns that can be instantiated. Templates contain pre-built node configurations.

WHEN TO USE: User asks for a "template", "example", or common workflow pattern
RETURNS: Template info with node_types used (helpful for understanding structure)"#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search by name, description, tags, or node types used in the template"
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let query_lower = args.query.to_lowercase();

        // Filter matching templates, excluding current template being edited
        let mut matches: Vec<&TemplateInfo> = self
            .templates
            .iter()
            .filter(|t| {
                // Skip the current template being edited
                if let Some(ref current_id) = self.current_template_id
                    && &t.id == current_id
                {
                    return false;
                }
                t.name.to_lowercase().contains(&query_lower)
                    || t.description.to_lowercase().contains(&query_lower)
                    || t.tags
                        .iter()
                        .any(|tag| tag.to_lowercase().contains(&query_lower))
                    || t.node_types
                        .iter()
                        .any(|nt| nt.to_lowercase().contains(&query_lower))
            })
            .take(5) // Limit results to reduce context
            .collect();

        // Sort by relevance: exact name match first, then description match
        matches.sort_by(|a, b| {
            let a_name_match = a.name.to_lowercase().contains(&query_lower);
            let b_name_match = b.name.to_lowercase().contains(&query_lower);
            b_name_match.cmp(&a_name_match)
        });

        Ok(serde_json::to_string(&matches).unwrap_or_default())
    }
}

// ============================================================================
// Get Node Details Tool
// ============================================================================

use super::context::GraphContext;

pub struct GetNodeDetailsTool {
    pub graph_context: Arc<GraphContext>,
}

// ============================================================================
// List Board Nodes Tool
// ============================================================================

pub struct ListBoardNodesTool {
    pub graph_context: Arc<GraphContext>,
}

impl Tool for ListBoardNodesTool {
    const NAME: &'static str = "list_board_nodes";

    type Error = BoardInspectionToolError;
    type Args = serde_json::Value;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "list_board_nodes".to_string(),
            description: r#"List all nodes and layers in the current workflow with IDs and positions. Use this first when modifying an existing graph."#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(build_list_board_nodes_output(&self.graph_context))
    }
}

// ============================================================================
// Get Unconfigured Nodes Tool
// ============================================================================

pub struct GetUnconfiguredNodesTool {
    pub graph_context: Arc<GraphContext>,
}

impl Tool for GetUnconfiguredNodesTool {
    const NAME: &'static str = "get_unconfigured_nodes";

    type Error = BoardInspectionToolError;
    type Args = serde_json::Value;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "get_unconfigured_nodes".to_string(),
            description: r#"Find nodes that still have missing non-execution inputs. Use this after planning or after a failed emit_commands validation to see what the graph is missing."#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(build_unconfigured_nodes_output(&self.graph_context))
    }
}

// ============================================================================
// Find Connectable Nodes Tool
// ============================================================================

pub struct FindConnectableNodesTool {
    pub provider: Arc<dyn CatalogProvider>,
    pub graph_context: Arc<GraphContext>,
}

impl Tool for FindConnectableNodesTool {
    const NAME: &'static str = "find_connectable_nodes";

    type Error = BoardInspectionToolError;
    type Args = FindConnectableNodesArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "find_connectable_nodes".to_string(),
            description: r#"Find catalog nodes that can connect to a specific existing pin, then rerank them by intent. Use this instead of guessing follow-up nodes for complex workflows."#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "node_id": {
                        "type": "string",
                        "description": "Existing node or layer ID from the current graph"
                    },
                    "pin_name": {
                        "type": "string",
                        "description": "Pin name on that node/layer"
                    },
                    "intent": {
                        "type": "string",
                        "description": "Optional desired outcome for reranking, e.g. 'send email' or 'read unread inbox messages'"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum candidates to return (default 8, max 20)"
                    }
                },
                "required": ["node_id", "pin_name"],
                "additionalProperties": false
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        build_find_connectable_nodes_output(&self.graph_context, self.provider.as_ref(), args).await
    }
}

impl Tool for GetNodeDetailsTool {
    const NAME: &'static str = "get_node_details";

    type Error = GetNodeDetailsToolError;
    type Args = GetNodeDetailsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "get_node_details".to_string(),
            description: r#"Get full details about a node: all pins, connections, current values. Use when context summary is insufficient.

WHEN TO USE:
- Need exact pin names for ConnectPins/UpdateNodePin
- See what values are configured
- Understand node's connections

RETURNS: Full pin list with names, types, current values, and all connections"#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "node_id": {
                        "type": "string",
                        "description": "The node ID from context (e.g., 'abc123')"
                    }
                },
                "required": ["node_id"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // Find the node in the context
        let node = self
            .graph_context
            .nodes
            .iter()
            .find(|n| n.id == args.node_id);

        match node {
            Some(node_ctx) => {
                // Build detailed output including all connections
                let incoming_edges: Vec<_> = self
                    .graph_context
                    .edges
                    .iter()
                    .filter(|e| e.to_node_id == args.node_id)
                    .map(|e| {
                        json!({
                            "from_node": e.from_node_id,
                            "from_pin": e.from_pin_name,
                            "to_pin": e.to_pin_name
                        })
                    })
                    .collect();

                let outgoing_edges: Vec<_> = self
                    .graph_context
                    .edges
                    .iter()
                    .filter(|e| e.from_node_id == args.node_id)
                    .map(|e| {
                        json!({
                            "from_pin": e.from_pin_name,
                            "to_node": e.to_node_id,
                            "to_pin": e.to_pin_name
                        })
                    })
                    .collect();

                let details = json!({
                    "id": node_ctx.id,
                    "node_type": node_ctx.node_type,
                    "friendly_name": node_ctx.friendly_name,
                    "position": { "x": node_ctx.position.0, "y": node_ctx.position.1 },
                    "size": { "width": node_ctx.estimated_size.0, "height": node_ctx.estimated_size.1 },
                    "inputs": node_ctx.inputs.iter().map(|p| {
                        json!({
                            "name": p.name,
                            "type": p.type_name,
                            "default_value": p.default_value
                        })
                    }).collect::<Vec<_>>(),
                    "outputs": node_ctx.outputs.iter().map(|p| {
                        json!({
                            "name": p.name,
                            "type": p.type_name
                        })
                    }).collect::<Vec<_>>(),
                    "incoming_connections": incoming_edges,
                    "outgoing_connections": outgoing_edges,
                    "is_selected": self.graph_context.selected_nodes.contains(&args.node_id)
                });

                Ok(serde_json::to_string_pretty(&details).unwrap_or_default())
            }
            None => Err(GetNodeDetailsToolError(format!(
                "Node with ID '{}' not found in the current graph",
                args.node_id
            ))),
        }
    }
}

// ============================================================================
// Emit Commands Tool
// ============================================================================

pub struct EmitCommandsTool;

impl Tool for EmitCommandsTool {
    const NAME: &'static str = "emit_commands";

    type Error = EmitCommandsToolError;
    type Args = EmitCommandsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "emit_commands".to_string(),
            description: r#"Execute graph modifications. Commands are batched and applied atomically with undo support.

WORKFLOW:
1. Use catalog_search to get exact node_type
2. Use get_node_details for pin names
3. Emit commands with ref_ids to chain operations

COMMAND TYPES:
- AddNode: Add a node (requires node_type from catalog)
- AddPlaceholder: Add a placeholder with custom pins
- ConnectPins: Connect two pins (use pin NAME, not ID)
- UpdateNodePin: Set a pin's value
- RemoveNode: Delete a node
- CreateVariable/DeleteVariable
- CreateComment/DeleteComment
- CreateLayer/RemoveLayer

REF_IDS: Use '$0', '$1', etc. to reference nodes in same batch"#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "commands": {
                        "type": "array",
                        "description": "Commands to execute. Use ref_id ('$0', '$1') for cross-referencing new nodes.",
                        "items": {
                            "type": "object",
                            "oneOf": [
                                {
                                    "properties": {
                                        "command_type": { "const": "AddNode" },
                                        "node_type": { "type": "string", "description": "EXACT node_type from catalog_search (e.g., 'flow_like_catalog_nodes::example::Example')" },
                                        "ref_id": { "type": "string", "description": "Reference ID like '$0', '$1' to use in ConnectPins/UpdateNodePin" },
                                        "position": {
                                            "type": "object",
                                            "properties": { "x": { "type": "number" }, "y": { "type": "number" } }
                                        },
                                        "friendly_name": { "type": "string", "description": "Optional display name" },
                                        "target_layer": { "type": "string", "description": "Layer ID for placement. Omit for root layer." },
                                        "summary": { "type": "string", "description": "Brief description, e.g. 'Add HTTP GET node'" }
                                    },
                                    "required": ["command_type", "node_type", "ref_id", "position", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "AddPlaceholder" },
                                        "name": { "type": "string", "description": "Name for the placeholder node (e.g., 'Process Order', 'Validate Input')" },
                                        "ref_id": { "type": "string", "description": "Reference ID for this placeholder (e.g., '$0', '$1') to use in subsequent commands" },
                                        "position": {
                                            "type": "object",
                                            "properties": { "x": { "type": "number" }, "y": { "type": "number" } }
                                        },
                                        "pins": {
                                            "type": "array",
                                            "description": "Custom pins to add to the placeholder (beyond the default exec_in/exec_out)",
                                            "items": {
                                                "type": "object",
                                                "properties": {
                                                    "name": { "type": "string", "description": "Internal name for the pin (e.g., 'order_data')" },
                                                    "friendly_name": { "type": "string", "description": "Display name (e.g., 'Order Data')" },
                                                    "description": { "type": "string", "description": "Optional description" },
                                                    "pin_type": { "type": "string", "enum": ["Input", "Output"], "description": "Whether this is an input or output pin" },
                                                    "data_type": { "type": "string", "enum": ["String", "Integer", "Float", "Boolean", "Struct", "Generic", "Execution"], "description": "The data type of the pin" },
                                                    "value_type": { "type": "string", "enum": ["Normal", "Array", "HashMap", "HashSet"], "description": "Value type (default: Normal)" }
                                                },
                                                "required": ["name", "friendly_name", "pin_type", "data_type"]
                                            }
                                        },
                                        "target_layer": { "type": "string", "description": "Layer ID to place the placeholder in. Use layer ID from context. Omit for root/current layer." },
                                        "summary": { "type": "string", "description": "Human-readable summary, e.g. 'Add placeholder for order processing'" }
                                    },
                                    "required": ["command_type", "name", "ref_id", "position", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "RemoveNode" },
                                        "node_id": { "type": "string", "description": "The ID of the node to remove" },
                                        "summary": { "type": "string", "description": "Human-readable summary, e.g. 'Remove the unused filter node'" }
                                    },
                                    "required": ["command_type", "node_id", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "ConnectPins" },
                                        "from_node": { "type": "string", "description": "Source node ID or ref_id (e.g., '$0')" },
                                        "from_pin": { "type": "string", "description": "Output pin NAME (not ID)" },
                                        "to_node": { "type": "string", "description": "Target node ID or ref_id (e.g., '$1')" },
                                        "to_pin": { "type": "string", "description": "Input pin NAME (not ID)" },
                                        "summary": { "type": "string", "description": "Human-readable summary, e.g. 'Connect output to input'" }
                                    },
                                    "required": ["command_type", "from_node", "from_pin", "to_node", "to_pin", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "DisconnectPins" },
                                        "from_node": { "type": "string" },
                                        "from_pin": { "type": "string" },
                                        "to_node": { "type": "string" },
                                        "to_pin": { "type": "string" },
                                        "summary": { "type": "string", "description": "Human-readable summary" }
                                    },
                                    "required": ["command_type", "from_node", "from_pin", "to_node", "to_pin", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "UpdateNodePin" },
                                        "node_id": { "type": "string", "description": "Node ID or ref_id (e.g., '$0')" },
                                        "pin_id": { "type": "string", "description": "Pin NAME (use internal name from catalog, not friendly_name)" },
                                        "value": { "description": "The new value for the pin" },
                                        "summary": { "type": "string", "description": "Human-readable summary, e.g. 'Set threshold to 0.5'" }
                                    },
                                    "required": ["command_type", "node_id", "pin_id", "value", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "MoveNode" },
                                        "node_id": { "type": "string" },
                                        "position": {
                                            "type": "object",
                                            "properties": { "x": { "type": "number" }, "y": { "type": "number" } },
                                            "required": ["x", "y"]
                                        },
                                        "target_layer": { "type": "string", "description": "Layer ID to move the node to. Use layer ID from context." },
                                        "summary": { "type": "string", "description": "Human-readable summary" }
                                    },
                                    "required": ["command_type", "node_id", "position", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "CreateVariable" },
                                        "name": { "type": "string", "description": "Variable name" },
                                        "data_type": { "type": "string", "description": "Data type: String, Integer, Float, Boolean, Struct, etc." },
                                        "value_type": { "type": "string", "description": "Value type: Normal, Array, HashMap, HashSet" },
                                        "default_value": { "description": "Optional default value" },
                                        "description": { "type": "string", "description": "Optional description" },
                                        "summary": { "type": "string", "description": "Human-readable summary" }
                                    },
                                    "required": ["command_type", "name", "data_type", "value_type", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "DeleteVariable" },
                                        "variable_id": { "type": "string", "description": "Variable ID from context" },
                                        "summary": { "type": "string", "description": "Human-readable summary" }
                                    },
                                    "required": ["command_type", "variable_id", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "CreateComment" },
                                        "content": { "type": "string", "description": "Comment text" },
                                        "position": {
                                            "type": "object",
                                            "properties": { "x": { "type": "number" }, "y": { "type": "number" } }
                                        },
                                        "width": { "type": "number", "description": "Comment width in pixels (default: 200)" },
                                        "height": { "type": "number", "description": "Comment height in pixels (default: 100)" },
                                        "color": { "type": "string", "description": "Optional hex color (e.g. #FFD700)" },
                                        "target_layer": { "type": "string", "description": "Layer ID to place the comment in. Use layer ID from context. Omit for root/current layer." },
                                        "summary": { "type": "string", "description": "Human-readable summary" }
                                    },
                                    "required": ["command_type", "content", "position", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "DeleteComment" },
                                        "comment_id": { "type": "string", "description": "Comment ID from context" },
                                        "summary": { "type": "string", "description": "Human-readable summary" }
                                    },
                                    "required": ["command_type", "comment_id", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "CreateLayer" },
                                        "name": { "type": "string", "description": "Layer name" },
                                        "color": { "type": "string", "description": "Optional layer color" },
                                        "node_ids": { "type": "array", "items": { "type": "string" }, "description": "Node IDs to include" },
                                        "position": {
                                            "type": "object",
                                            "properties": { "x": { "type": "number" }, "y": { "type": "number" } }
                                        },
                                        "target_layer": { "type": "string", "description": "Parent layer ID for nesting. Use layer ID from context. Omit for root layer." },
                                        "summary": { "type": "string", "description": "Human-readable summary" }
                                    },
                                    "required": ["command_type", "name", "node_ids", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "RemoveLayer" },
                                        "layer_id": { "type": "string", "description": "Layer ID from context" },
                                        "summary": { "type": "string", "description": "Human-readable summary" }
                                    },
                                    "required": ["command_type", "layer_id", "summary"]
                                }
                            ]
                        }
                    },
                    "explanation": {
                        "type": "string",
                        "description": "Overall explanation of what these commands accomplish together"
                    }
                },
                "required": ["commands", "explanation"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // Build a human-readable summary for the model to understand what was done
        let mut summary_lines: Vec<String> = Vec::new();
        summary_lines.push(format!("✓ Queued {} commands:", args.commands.len()));

        for cmd in &args.commands {
            let cmd_summary = match cmd {
                BoardCommand::AddNode {
                    node_type,
                    ref_id,
                    friendly_name,
                    ..
                } => {
                    format!(
                        "  - AddNode: {} as {} (ref: {})",
                        friendly_name.as_deref().unwrap_or(node_type),
                        node_type,
                        ref_id.as_deref().unwrap_or("none")
                    )
                }
                BoardCommand::AddPlaceholder {
                    name, ref_id, pins, ..
                } => {
                    let pin_count = pins.as_ref().map(|p| p.len()).unwrap_or(0);
                    format!(
                        "  - AddPlaceholder: \"{}\" (ref: {}, {} custom pins)",
                        name,
                        ref_id.as_deref().unwrap_or("none"),
                        pin_count
                    )
                }
                BoardCommand::ConnectPins {
                    from_node,
                    from_pin,
                    to_node,
                    to_pin,
                    ..
                } => {
                    format!(
                        "  - Connect: {}.{} → {}.{}",
                        from_node, from_pin, to_node, to_pin
                    )
                }
                BoardCommand::RemoveNode { node_id, .. } => {
                    format!("  - RemoveNode: {}", node_id)
                }
                BoardCommand::UpdateNodePin {
                    node_id, pin_id, ..
                } => {
                    format!("  - UpdatePin: {}.{}", node_id, pin_id)
                }
                BoardCommand::AddComment {
                    content,
                    width,
                    height,
                    color,
                    ..
                } => {
                    let preview = if content.chars().count() > 30 {
                        let truncated: String = content.chars().take(30).collect();
                        format!("{}...", truncated)
                    } else {
                        content.clone()
                    };
                    let size_info = match (width, height) {
                        (Some(w), Some(h)) => format!(" ({}x{})", w, h),
                        _ => String::new(),
                    };
                    let color_info = color
                        .as_ref()
                        .map(|c| format!(" [{}]", c))
                        .unwrap_or_default();
                    format!("  - AddComment: \"{}\"{}{}", preview, size_info, color_info)
                }
                _ => format!("  - {:?}", cmd),
            };
            summary_lines.push(cmd_summary);
        }

        summary_lines.push(format!("\nExplanation: {}", args.explanation));
        summary_lines.push(
            "\n⚠️ These commands are now queued. Do NOT emit the same commands again.".to_string(),
        );

        // Return the commands as JSON wrapped in a special tag for parsing, plus the summary
        let commands_json = serde_json::to_string(&args.commands).unwrap_or_default();
        Ok(format!(
            "<commands>{}</commands>\n\n{}",
            commands_json,
            summary_lines.join("\n")
        ))
    }
}

// ============================================================================
// Query Logs Tool
// ============================================================================

pub struct QueryLogsTool {
    pub state: Arc<FlowLikeState>,
    pub run_context: Option<RunContext>,
}

impl Tool for QueryLogsTool {
    const NAME: &'static str = "query_logs";

    type Error = QueryLogsToolError;
    type Args = QueryLogsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "query_logs".to_string(),
            description: r#"Query execution logs from a flow run. Useful for debugging errors and tracing execution.

LOG LEVELS: Debug(0), Info(1), Warn(2), Error(3), Fatal(4)

FILTER EXAMPLES:
- 'log_level >= 3' → Errors and fatal only
- 'node_id = "abc123"' → Logs from specific node
- 'message LIKE "%timeout%"' → Search in messages

RETURNS: Logs with level, message, node_id (use node_id with get_node_details)"#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "filter": {
                        "type": "string",
                        "description": "SQL-like filter: 'log_level >= 3', 'node_id = \"id\"', 'message LIKE \"%error%\"'"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max logs to return (default: 50, max: 100)"
                    }
                },
                "required": []
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        println!("[QueryLogsTool] call() invoked with args: {:?}", args);

        let run_context = self.run_context.as_ref().ok_or_else(|| {
            println!("[QueryLogsTool] ERROR: No run context available");
            QueryLogsToolError(
                "No run context available. User must select a run first.".to_string(),
            )
        })?;

        println!(
            "[QueryLogsTool] run_context: app_id={}, run_id={}, board_id={}",
            run_context.app_id, run_context.run_id, run_context.board_id
        );

        let limit = args.limit.unwrap_or(50).min(100);
        let filter = args.filter.clone().unwrap_or_default();

        println!("[QueryLogsTool] Using limit={}, filter='{}'", limit, filter);

        // Build LogMeta from RunContext
        let log_meta = crate::flow::execution::LogMeta {
            app_id: run_context.app_id.clone(),
            run_id: run_context.run_id.clone(),
            board_id: run_context.board_id.clone(),
            start: 0,
            end: 0,
            log_level: 0,
            version: String::new(),
            nodes: None,
            logs: None,
            node_id: String::new(),
            event_version: None,
            event_id: String::new(),
            payload: vec![],
            is_remote: false,
        };

        #[cfg(feature = "flow-runtime")]
        {
            println!("[QueryLogsTool] Calling state.query_run()...");
            let logs = self
                .state
                .query_run(&log_meta, &filter, Some(limit), Some(0))
                .await
                .map_err(|e| {
                    println!("[QueryLogsTool] ERROR querying logs: {}", e);
                    QueryLogsToolError(format!("Failed to query logs: {}", e))
                })?;

            println!("[QueryLogsTool] Got {} logs", logs.len());

            if logs.is_empty() {
                let msg = if filter.is_empty() {
                    "No logs found for this run. The execution may have completed without producing any log output, or logs may have been cleared."
                } else {
                    "No logs matching your filter criteria. Try a broader search or check if the filter syntax is correct."
                };
                println!("[QueryLogsTool] Returning empty message: {}", msg);
                return Ok(msg.to_string());
            }

            // Format logs for the AI
            let formatted_logs: Vec<serde_json::Value> = logs
                .iter()
                .map(|log| {
                    json!({
                        "level": match log.log_level {
                            crate::flow::execution::LogLevel::Debug => "Debug",
                            crate::flow::execution::LogLevel::Info => "Info",
                            crate::flow::execution::LogLevel::Warn => "Warn",
                            crate::flow::execution::LogLevel::Error => "Error",
                            crate::flow::execution::LogLevel::Fatal => "Fatal",
                        },
                        "message": log.message,
                        "node_id": log.node_id,
                    })
                })
                .collect();

            let result = serde_json::to_string_pretty(&formatted_logs).unwrap_or_default();
            println!(
                "[QueryLogsTool] Returning {} bytes of formatted logs",
                result.len()
            );
            println!(
                "[QueryLogsTool] First 500 chars: {}",
                &result[..result.len().min(500)]
            );
            Ok(result)
        }

        #[cfg(not(feature = "flow-runtime"))]
        {
            println!("[QueryLogsTool] flow-runtime feature not enabled");
            Ok("Log querying is not available in this build.".to_string())
        }
    }
}

// ============================================================================
// Tool Execution Helpers
// ============================================================================

pub fn build_list_board_nodes_output(graph_context: &GraphContext) -> String {
    if graph_context.nodes.is_empty() && graph_context.layers.is_empty() {
        return "The board is empty - no nodes found. Use catalog_search to find nodes to add."
            .to_string();
    }

    let mut lines = Vec::new();
    lines.push(format!("Board has {} nodes:", graph_context.nodes.len()));

    for node in &graph_context.nodes {
        let selected = if graph_context.selected_nodes.contains(&node.id) {
            " [SELECTED]"
        } else {
            ""
        };
        lines.push(format!(
            "- {} | {} | {} | pos:({},{}){}",
            node.id, node.node_type, node.friendly_name, node.position.0, node.position.1, selected
        ));
    }

    if !graph_context.layers.is_empty() {
        lines.push(format!("\nLayers ({}):", graph_context.layers.len()));
        for layer in &graph_context.layers {
            let parent = layer.parent_id.as_deref().unwrap_or("root");
            lines.push(format!(
                "- {} | {} | parent:{} | nodes:{} | pos:({},{})",
                layer.id,
                layer.name,
                parent,
                layer.node_ids.len(),
                layer.position.0,
                layer.position.1,
            ));
        }
    }

    if !graph_context.variables.is_empty() {
        lines.push(format!("\nVariables ({}):", graph_context.variables.len()));
        for variable in &graph_context.variables {
            lines.push(format!(
                "- {}: {} ({}/{})",
                variable.id, variable.name, variable.data_type, variable.value_type
            ));
        }
    }

    lines.push("\n→ Use get_node_details(node_id) to inspect exact pin names".to_string());
    lines.join("\n")
}

pub fn build_unconfigured_nodes_output(graph_context: &GraphContext) -> String {
    let connected_pins: std::collections::HashSet<(String, String)> = graph_context
        .edges
        .iter()
        .map(|edge| (edge.to_node_id.clone(), edge.to_pin_name.clone()))
        .collect();

    let mut unconfigured = Vec::new();

    for node in &graph_context.nodes {
        let missing_inputs: Vec<_> = node
            .inputs
            .iter()
            .filter(|input| input.type_name != "Execution")
            .filter(|input| {
                !connected_pins.contains(&(node.id.clone(), input.name.clone()))
                    && input.default_value.is_none()
            })
            .map(|input| {
                json!({
                    "pin": input.name,
                    "type": input.type_name,
                })
            })
            .collect();

        if !missing_inputs.is_empty() {
            unconfigured.push(json!({
                "node_id": node.id,
                "node_type": node.node_type,
                "name": node.friendly_name,
                "missing_inputs": missing_inputs,
            }));
        }
    }

    if unconfigured.is_empty() {
        "All nodes are configured - no missing non-execution inputs found.".to_string()
    } else {
        serde_json::to_string_pretty(&unconfigured).unwrap_or_default()
    }
}

pub async fn build_find_connectable_nodes_output(
    graph_context: &GraphContext,
    provider: &dyn CatalogProvider,
    args: FindConnectableNodesArgs,
) -> Result<String, BoardInspectionToolError> {
    let limit = args.limit.unwrap_or(8).clamp(1, 20);

    let mut pin_direction = None;
    let mut pin_type = None;

    if let Some(node) = graph_context
        .nodes
        .iter()
        .find(|node| node.id == args.node_id)
    {
        if let Some(pin) = node.inputs.iter().find(|pin| pin.name == args.pin_name) {
            pin_direction = Some("input");
            pin_type = Some(pin.type_name.clone());
        } else if let Some(pin) = node.outputs.iter().find(|pin| pin.name == args.pin_name) {
            pin_direction = Some("output");
            pin_type = Some(pin.type_name.clone());
        }
    }

    if pin_type.is_none()
        && let Some(layer) = graph_context
            .layers
            .iter()
            .find(|layer| layer.id == args.node_id)
    {
        if let Some(pin) = layer.inputs.iter().find(|pin| pin.name == args.pin_name) {
            pin_direction = Some("input");
            pin_type = Some(pin.type_name.clone());
        } else if let Some(pin) = layer.outputs.iter().find(|pin| pin.name == args.pin_name) {
            pin_direction = Some("output");
            pin_type = Some(pin.type_name.clone());
        }
    }

    let pin_type = pin_type.ok_or_else(|| {
        BoardInspectionToolError(format!(
            "Pin '{}' not found on node/layer '{}'",
            args.pin_name, args.node_id
        ))
    })?;

    let search_for_inputs = pin_direction == Some("output");
    let mut matches = provider
        .search_by_pin_type(&pin_type, search_for_inputs)
        .await;

    matches.retain(|metadata| metadata.name != args.node_id);

    if let Some(intent) = args.intent.as_ref() {
        matches.sort_by(|left, right| {
            score_catalog_metadata(right, intent).cmp(&score_catalog_metadata(left, intent))
        });
    }

    let payload = json!({
        "source": {
            "node_id": args.node_id,
            "pin_name": args.pin_name,
            "pin_type": pin_type,
            "pin_direction": pin_direction.unwrap_or("unknown"),
            "searching_for": if search_for_inputs { "input pins" } else { "output pins" },
        },
        "candidates": matches.into_iter().take(limit).collect::<Vec<_>>(),
    });

    Ok(serde_json::to_string_pretty(&payload).unwrap_or_default())
}

/// Get a human-readable description for a tool call
pub fn get_tool_description(name: &str, arguments: &serde_json::Value) -> String {
    match name {
        "think" => {
            if let Some(thought) = arguments.get("thought").and_then(|v| v.as_str()) {
                thought.to_string()
            } else {
                "Reasoning through the problem...".to_string()
            }
        }
        "get_node_details" => {
            if let Some(node_id) = arguments.get("node_id").and_then(|v| v.as_str()) {
                format!("Getting details for node {}", node_id)
            } else {
                "Getting node details...".to_string()
            }
        }
        "emit_commands" => {
            if let Some(commands) = arguments.get("commands").and_then(|v| v.as_array()) {
                format!("Preparing {} change(s)...", commands.len())
            } else {
                "Preparing changes...".to_string()
            }
        }
        "catalog_search" => {
            if let Some(query) = arguments.get("query").and_then(|v| v.as_str()) {
                format!("Searching catalog for \"{}\"", query)
            } else {
                "Searching the catalog...".to_string()
            }
        }
        "search_by_pin" => {
            if let Some(pin_type) = arguments.get("pin_type").and_then(|v| v.as_str()) {
                format!("Finding nodes with {} pins", pin_type)
            } else {
                "Finding compatible nodes...".to_string()
            }
        }
        "find_connectable_nodes" => {
            let node_id = arguments
                .get("node_id")
                .and_then(|v| v.as_str())
                .unwrap_or("node");
            let pin_name = arguments
                .get("pin_name")
                .and_then(|v| v.as_str())
                .unwrap_or("pin");
            format!("Finding connectable nodes for {}.{}", node_id, pin_name)
        }
        "list_board_nodes" => "Listing nodes in the current workflow...".to_string(),
        "get_unconfigured_nodes" => "Checking which nodes still need configuration...".to_string(),
        "filter_category" => {
            if let Some(category) = arguments.get("category_prefix").and_then(|v| v.as_str()) {
                format!("Browsing {} category", category)
            } else {
                "Browsing categories...".to_string()
            }
        }
        "search_templates" => {
            if let Some(query) = arguments.get("query").and_then(|v| v.as_str()) {
                format!("Searching templates for \"{}\"", query)
            } else {
                "Searching templates...".to_string()
            }
        }
        "query_logs" => {
            if let Some(query) = arguments.get("query").and_then(|v| v.as_str()) {
                format!("Searching logs for \"{}\"", query)
            } else {
                "Querying execution logs...".to_string()
            }
        }
        _ => format!("Running {}...", name),
    }
}
