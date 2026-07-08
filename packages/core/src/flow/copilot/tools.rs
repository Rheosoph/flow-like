use std::sync::Arc;

use rig::{completion::ToolDefinition, tool::Tool};
use serde::Deserialize;
use serde_json::json;

use super::provider::CatalogProvider;
use super::search::score_catalog_metadata;
use super::types::{BoardCommand, RunContext, TemplateInfo};
use crate::flow::ast::{
    ReconcileResult, RenderOptions, blocked_destructive_flowscript_message, board_to_flowscript,
    destructive_flowscript_command_summaries, reconcile_text_with_catalog,
};
use crate::flow::board::Board;
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

#[derive(Debug, thiserror::Error)]
#[error("FlowScript tool error: {0}")]
pub struct FlowScriptToolError(pub String);

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

#[derive(Deserialize)]
pub struct GetDeclarationsArgs {
    /// Free-text search for the kinds of nodes you want to call in FlowScript
    /// (e.g. "http request", "parse json", "invoke agent").
    pub query: String,
}

#[derive(Deserialize)]
pub struct GetCurrentFlowScriptArgs {}

#[derive(Deserialize)]
pub struct EditFlowScriptArgs {
    /// The full edited FlowScript source for the board. Preserve the `//@n:<id>` anchor comments
    /// on existing statements so identities are matched; literal argument changes become pin
    /// updates. Removed anchored statements are blocked unless `allow_deletions` is true.
    #[serde(alias = "script", alias = "source", alias = "content")]
    pub flowscript: String,
    /// Explicit opt-in for destructive FlowScript edits. Leave false unless the user asked to
    /// remove existing board items.
    #[serde(default)]
    pub allow_deletions: bool,
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
            description: r#"Search the node catalog by functionality or name. Returns matching nodes with their node_type for legacy/manual AddNode commands.

WHEN TO USE: Only for manual command JSON, layout/modeling operations, or debugging catalog metadata.
FOR WORKFLOW EDITS: Prefer get_declarations, write FlowScript, then call edit_flowscript. get_declarations is backed by embedded .flow.d files and returns exact camelCase function signatures.
EXAMPLE QUERIES: "http request", "parse json", "loop array", "condition if", "open database""#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural language catalog search for manual AddNode use. For FlowScript workflows, use get_declarations instead."
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let matches = self.provider.search(&args.query).await;
        Ok(super::search::render_catalog_search_results(&matches))
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
            description: r#"List all nodes and layers in the current workflow with their IDs and positions.

USE THIS FIRST to understand the workflow before making changes.

RETURNS:
- node_id: Use in get_node_details, ConnectPins, UpdateNodePin
- node_type: The node's catalog type
- friendly_name: Human-readable name
- position: {x, y} - use to place new nodes nearby

WORKFLOW:
1. list_board_nodes → see all nodes and positions
2. get_node_details on relevant node → get pin names
3. get_declarations → find signatures, then edit_flowscript (or catalog_search + emit_commands for manual edits)"#.to_string(),
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
            description: r#"Find nodes that need configuration - inputs with no value and no incoming connection.

WHEN TO USE:
- Check what needs to be configured in the workflow
- Find nodes that aren't fully set up
- Identify missing connections
- After planning or after a failed emit_commands validation

RETURNS: List of nodes with their unconfigured non-execution input pins"#.to_string(),
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
            description:
                r#"Get full details about a node including position, all pins, and connections.

CRITICAL: Use this BEFORE connecting to existing nodes!

RETURNS:
- position: {x, y} - use this to position new nodes nearby
- inputs/outputs: Array of pins with {name, type, value}
- incoming/outgoing: Current connections

EXAMPLE USE:
1. Call get_node_details on existing node
2. Note its position (e.g., {x: 500, y: 200})
3. Place new connected node at {x: 750, y: 200} (250px right)
4. Use exact pin names from outputs/inputs in ConnectPins"#
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "node_id": {
                        "type": "string",
                        "description": "The node ID to inspect (from list_board_nodes or context)"
                    }
                },
                "required": ["node_id"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(build_node_details_output(
            &args.node_id,
            &self.graph_context,
        ))
    }
}

/// Full JSON details of one node (pins, values, connections) or a not-found message. Single
/// source for the `get_node_details` tool across every backend executor.
pub fn build_node_details_output(node_id: &str, graph_context: &GraphContext) -> String {
    let Some(node_ctx) = graph_context.nodes.iter().find(|n| n.id == node_id) else {
        return format!("Node with ID '{}' not found in the current graph", node_id);
    };

    let incoming_edges: Vec<_> = graph_context
        .edges
        .iter()
        .filter(|e| e.to_node_id == node_id)
        .map(|e| {
            json!({
                "from_node": e.from_node_id,
                "from_pin": e.from_pin_name,
                "to_pin": e.to_pin_name
            })
        })
        .collect();

    let outgoing_edges: Vec<_> = graph_context
        .edges
        .iter()
        .filter(|e| e.from_node_id == node_id)
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
        "is_selected": graph_context.selected_nodes.contains(&node_id.to_string())
    });

    serde_json::to_string_pretty(&details).unwrap_or_default()
}

/// The `(name, description, parameters)` triple of a rig tool definition, so non-rig adapters
/// (Copilot SDK, MCP) can advertise exactly the same definition as the rig loop.
pub async fn tool_definition_parts<T: Tool>(tool: &T) -> (String, String, serde_json::Value) {
    let definition = tool.definition(String::new()).await;
    (
        definition.name,
        definition.description,
        definition.parameters,
    )
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
            description: r#"Execute low-level graph modifications. Commands are batched and applied atomically with undo support.

PRIMARY WORKFLOW EDIT PATH:
Use get_declarations to search embedded .flow.d signatures, write the workflow as FlowScript, then call edit_flowscript so the text is reconciled into commands.

LOW-LEVEL FALLBACK WORKFLOW:
1. Use catalog_search to get exact node_type
2. Use get_node_details for pin names
3. Emit commands with ref_ids to chain operations

Use this directly only for layout-only MoveNode changes, placeholders/comments/layers, variables, or changes that cannot be represented as FlowScript.

COMMAND TYPES:
- AddNode: Add a node (requires node_type from catalog)
- AddPlaceholder: Add a placeholder with custom pins
- ConnectPins: Connect two pins (use pin NAME, not ID)
- UpdateNodePin: Set a pin's value
- RemoveNode: Delete a node
- CreateVariable/UpdateVariable/DeleteVariable
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
                                        "variable_id": { "type": "string", "description": "Optional variable ID. Omit to let the frontend generate one." },
                                        "name": { "type": "string", "description": "Variable name" },
                                        "data_type": { "type": "string", "description": "Data type: String, Integer, Float, Boolean, Struct, etc." },
                                        "value_type": { "type": "string", "description": "Value type: Normal, Array, HashMap, HashSet" },
                                        "default_value": { "description": "Optional default value" },
                                        "description": { "type": "string", "description": "Optional description" },
                                        "category": { "type": "string", "description": "Optional UI category" },
                                        "schema": { "type": "string", "description": "Optional JSON Schema for Struct variables" },
                                        "exposed": { "type": "boolean" },
                                        "secret": { "type": "boolean" },
                                        "editable": { "type": "boolean" },
                                        "runtime_configured": { "type": "boolean" },
                                        "target_layer": { "type": "string", "description": "Optional layer ID for local variables" },
                                        "summary": { "type": "string", "description": "Human-readable summary" }
                                    },
                                    "required": ["command_type", "name", "data_type", "value_type", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "UpdateVariable" },
                                        "variable_id": { "type": "string", "description": "Variable ID from context" },
                                        "name": { "type": "string", "description": "Optional new name" },
                                        "data_type": { "type": "string", "description": "Optional new data type" },
                                        "value_type": { "type": "string", "description": "Optional new value type" },
                                        "default_value": { "description": "Optional new default value" },
                                        "clear_default_value": { "type": "boolean", "description": "Set true to remove the default value" },
                                        "description": { "type": "string", "description": "Optional new description" },
                                        "clear_description": { "type": "boolean", "description": "Set true to remove the description" },
                                        "category": { "type": "string", "description": "Optional new category" },
                                        "clear_category": { "type": "boolean", "description": "Set true to remove the category" },
                                        "schema": { "type": "string", "description": "Optional new JSON Schema" },
                                        "clear_schema": { "type": "boolean", "description": "Set true to remove the schema" },
                                        "exposed": { "type": "boolean" },
                                        "secret": { "type": "boolean" },
                                        "editable": { "type": "boolean" },
                                        "runtime_configured": { "type": "boolean" },
                                        "summary": { "type": "string", "description": "Human-readable summary" }
                                    },
                                    "required": ["command_type", "variable_id", "summary"]
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
                BoardCommand::CreateVariable { name, .. } => {
                    format!("  - CreateVariable: {}", name)
                }
                BoardCommand::UpdateVariable { variable_id, .. } => {
                    format!("  - UpdateVariable: {}", variable_id)
                }
                BoardCommand::RemoveVariable { variable_id, .. } => {
                    format!("  - DeleteVariable: {}", variable_id)
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
// FlowScript Tools
// ============================================================================

/// Return the live board rendered as anchored FlowScript.
///
/// This is intentionally a tool, even though the system prompt also includes the board source,
/// because long multi-step agents can lose the inline copy. Calling this immediately before
/// `edit_flowscript` gives the model the exact current document to edit.
pub struct GetCurrentFlowScriptTool {
    pub board: Arc<Board>,
}

impl Tool for GetCurrentFlowScriptTool {
    const NAME: &'static str = "get_current_flowscript";

    type Error = FlowScriptToolError;
    type Args = GetCurrentFlowScriptArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "get_current_flowscript".to_string(),
            description: r#"Return the current live board as anchored FlowScript.

Use this before editing an existing board, especially after prior tool calls or after validation
failed. The returned document is the source you must edit and submit in full to
`edit_flowscript`; preserve all `//@n:<id>` anchors on statements you keep."#
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {}
            }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(board_to_flowscript(
            &self.board,
            &RenderOptions {
                anchors: true,
                ..Default::default()
            },
        ))
    }
}

/// Retrieve `.flow.d`-style FlowScript declarations for nodes matching a query.
///
/// This is the FlowScript counterpart to `catalog_search`/`get_node_details`: instead of
/// per-pin JSON, it returns the exact `declare function …` signatures the agent should call when
/// writing FlowScript, including third-party package nodes injected into the catalog.
pub struct GetDeclarationsTool {
    pub provider: Arc<dyn CatalogProvider>,
}

impl Tool for GetDeclarationsTool {
    const NAME: &'static str = "get_declarations";

    type Error = FlowScriptToolError;
    type Args = GetDeclarationsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "get_declarations".to_string(),
            description: r#"Look up FlowScript node declarations (.flow.d) by intent.

Returns a compact ranked list of exact `declare function <camelCaseNodeType>({ pin: type, ... })`
signatures for nodes matching your focused query, plus an `// impure` marker for side-effecting /
control-flow nodes. Empty queries intentionally return guidance only, not the full catalog.

Use this BEFORE writing FlowScript so you call nodes by their exact camelCase name with correctly
typed arguments. This covers every package in the project's catalog, including third-party ones."#
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Focused declaration search. Do not leave blank. Good examples: 'gmail imap fetch mail', 'smtp send email', 'open local database batch insert', 'datafusion sql register lance', 'hybrid vector search build index'."
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(self.provider.get_declarations(&args.query).await)
    }
}

/// Apply an edited FlowScript document to the board via reconcile.
///
/// The agent edits the board's FlowScript (obtained from the system context) and submits the full
/// document here. Reconcile diffs it against the live board — keyed on `//@n:<id>` anchors — and
/// catalog declarations, then emits the minimal `BoardCommand`s. Anchored edits become pin
/// updates/removals; new unanchored catalog calls become AddNode/ConnectPins/UpdateNodePin.
/// The commands are surfaced in the same `<commands>…</commands>` envelope the `emit_commands`
/// path consumes, so they flow through the existing validation/apply/undo pipeline.
pub struct EditFlowScriptTool {
    pub board: Arc<Board>,
    pub provider: Arc<dyn CatalogProvider>,
}

pub fn board_has_no_nodes(board: &Board) -> bool {
    board.nodes.is_empty() && board.layers.values().all(|layer| layer.nodes.is_empty())
}

pub fn flowscript_workspace_tag(flowscript: &str, status: &str) -> String {
    let payload = json!({
        "source": flowscript,
        "status": status,
    });
    format!(
        "<flowscript_workspace>{}</flowscript_workspace>",
        serde_json::to_string(&payload).unwrap_or_default()
    )
}

fn edit_flowscript_actionability_feedback(
    flowscript: &str,
    board_is_empty: bool,
    diagnostics: &[String],
) -> Option<String> {
    let lower = flowscript.to_lowercase();
    let stub_markers = [
        "implementation plan",
        "implementation notes",
        "implementation should be wired",
        "function stubs",
        "fetcher stub",
        "enricher stub",
        "todo",
        "replace with",
        "when implemented",
        "wire with",
        "wire using",
        "catalog nodes:",
        "flowscript contains stubs",
        "automated nodes added",
        "clear wiring plan",
    ];

    if stub_markers.iter().any(|marker| lower.contains(marker)) {
        return Some(
            "This edit looks like a plan/stub, not actionable FlowScript. `edit_flowscript` only creates board changes from real catalog calls. Do not submit TODOs, stub comments, lists of node names, or \"replace with\" instructions; call `get_declarations` for the missing signatures and submit concrete calls inside a function/event block."
                .to_string(),
        );
    }

    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.contains("expected `Colon`, found `Assign`"))
    {
        return Some(
            "The submitted FlowScript used `=` where FlowScript expected an object/call-argument field separator. In FlowScript call arguments and object literals use colon syntax, e.g. `{ host: \"imap.gmail.com\", port: 993 }`, not `{ host = \"imap.gmail.com\" }`."
                .to_string(),
        );
    }

    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.contains("`const` binding requires a call expression"))
    {
        return Some(
            "Inside a function/event block, `const name = ...` can only bind the output of a node call. Do not bind literals, object literals, arrays, field access, or arithmetic with `const`; use local alias syntax like `let rows = []`, pass literals directly into a node call, or bind a real utility/catalog call."
                .to_string(),
        );
    }

    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.contains("labelled branch requires a call condition"))
    {
        return Some(
            "The submitted FlowScript used labelled branch syntax (`if (...) { // label ... }`) with a non-call condition. In FlowScript, labels after branch braces are reserved for call-based control nodes, so the condition must be a catalog/control-node call. For ordinary boolean checks, remove the trailing branch labels/comments and use plain `if (condition) { ... } else { ... }`, or use exact control-node declarations from `get_declarations`."
                .to_string(),
        );
    }

    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.contains("FlowScript parse error"))
    {
        return Some(
            "The submitted FlowScript did not parse. A common cause is putting node calls at the top level: top-level `const name: type = ...` declarations can only hold literal defaults and do not create nodes. Put catalog calls inside a function/event block, for example `run() { const db = openLocalDb({ name: \"email_vectors\" }) }`, using exact signatures from `get_declarations`."
                .to_string(),
        );
    }

    if board_is_empty && !contains_probable_node_call(flowscript) {
        return Some(
            "The board is empty and this FlowScript contains no executable catalog calls, so there is nothing to translate into nodes. Placeholder variables/comments are fine as supporting context, but the draft must include at least one real node call inside a function/event block."
                .to_string(),
        );
    }

    None
}

fn contains_probable_node_call(flowscript: &str) -> bool {
    flowscript.lines().any(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with('@')
            || trimmed.starts_with("if ")
            || trimmed.starts_with("for ")
            || trimmed.starts_with("return ")
            || trimmed.contains(") {")
        {
            return false;
        }

        if let Some(rest) = trimmed.strip_prefix("const ") {
            return rest
                .split_once('=')
                .is_some_and(|(_, rhs)| starts_with_call_expr(rhs));
        }

        starts_with_call_expr(trimmed)
    })
}

fn starts_with_call_expr(source: &str) -> bool {
    let source = source.trim_start();
    let Some(paren_idx) = source.find('(') else {
        return false;
    };
    let name = source[..paren_idx].trim();
    !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

pub fn render_edit_flowscript_result(
    flowscript: &str,
    result: &ReconcileResult,
    board_is_empty: bool,
    allow_deletions: bool,
) -> String {
    let blocking_diagnostics: Vec<&String> = result
        .diagnostics
        .iter()
        .filter(|diagnostic| is_blocking_flowscript_diagnostic(diagnostic))
        .collect();

    if result.commands.is_empty() {
        let actionability =
            edit_flowscript_actionability_feedback(flowscript, board_is_empty, &result.diagnostics);
        let status = if actionability.is_some() || !result.diagnostics.is_empty() {
            "validation_errors"
        } else {
            "no_changes"
        };

        let mut msg = match actionability {
            Some(feedback) => {
                format!("{feedback}\n\nNo board changes were derived from the FlowScript.")
            }
            None => "No board changes were derived from the FlowScript.".to_string(),
        };
        if !result.diagnostics.is_empty() {
            msg.push_str("\nDiagnostics:\n");
            for d in &result.diagnostics {
                msg.push_str("- ");
                msg.push_str(d);
                msg.push('\n');
            }
        }
        return format!("{}\n{}", flowscript_workspace_tag(flowscript, status), msg);
    }

    if !blocking_diagnostics.is_empty() {
        let mut msg = String::from(
            "FlowScript validation failed before queueing board changes. The script produced partial commands, but at least one construct cannot be translated safely yet.",
        );
        msg.push_str("\nDiagnostics:\n");
        for d in blocking_diagnostics {
            msg.push_str("- ");
            msg.push_str(d);
            msg.push('\n');
        }
        msg.push_str(
            "\nRewrite new control flow as concrete catalog/control-node calls, or use straight-line SSA-style node calls without mutable branch/loop side effects.",
        );
        return format!(
            "{}\n{}",
            flowscript_workspace_tag(flowscript, "validation_errors"),
            msg
        );
    }

    if !allow_deletions {
        let destructive = destructive_flowscript_command_summaries(&result.commands);
        if !destructive.is_empty() {
            let mut msg = blocked_destructive_flowscript_message(&destructive);
            if !result.diagnostics.is_empty() {
                msg.push_str("\nDiagnostics:\n");
                for d in &result.diagnostics {
                    msg.push_str("- ");
                    msg.push_str(d);
                    msg.push('\n');
                }
            }
            return format!(
                "{}\n{}",
                flowscript_workspace_tag(flowscript, "validation_errors"),
                msg
            );
        }
    }

    let commands_json = serde_json::to_string(&result.commands).unwrap_or_default();
    let mut lines = vec![format!(
        "✓ Reconciled {} change(s) from FlowScript:",
        result.commands.len()
    )];
    for cmd in &result.commands {
        match cmd {
            BoardCommand::UpdateNodePin {
                node_id, pin_id, ..
            } => lines.push(format!("  - UpdatePin: {}.{}", node_id, pin_id)),
            BoardCommand::RemoveNode { node_id, .. } => {
                lines.push(format!("  - RemoveNode: {}", node_id))
            }
            BoardCommand::CreateVariable { name, .. } => {
                lines.push(format!("  - CreateVariable: {}", name))
            }
            BoardCommand::UpdateVariable { variable_id, .. } => {
                lines.push(format!("  - UpdateVariable: {}", variable_id))
            }
            BoardCommand::RemoveVariable { variable_id, .. } => {
                lines.push(format!("  - DeleteVariable: {}", variable_id))
            }
            _ => lines.push("  - (change)".to_string()),
        }
    }
    for d in &result.diagnostics {
        lines.push(format!("  - Note: {}", d));
    }
    lines.push(
        "\n⚠️ These changes are now queued. Do NOT submit the same FlowScript again.".to_string(),
    );

    format!(
        "{}\n<commands>{}</commands>\n\n{}",
        flowscript_workspace_tag(flowscript, "queued"),
        commands_json,
        lines.join("\n")
    )
}

fn is_blocking_flowscript_diagnostic(diagnostic: &str) -> bool {
    diagnostic.contains("not yet converted automatically")
        || diagnostic.contains("skipped local alias")
        || diagnostic.contains("skipped connection")
        || diagnostic.contains("could not choose an output pin")
        || diagnostic.contains("does not match a catalog declaration")
        || diagnostic.contains("is ambiguous")
}

impl Tool for EditFlowScriptTool {
    const NAME: &'static str = "edit_flowscript";

    type Error = FlowScriptToolError;
    type Args = EditFlowScriptArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "edit_flowscript".to_string(),
            description: r#"Apply an edited FlowScript document to the board.

This is the PRIMARY way to modify a workflow. For existing-board edits, call
`get_current_flowscript` first, edit that exact returned document, and submit the FULL edited
FlowScript source. Reconcile compares it to the live board using the `//@n:<id>` anchor comments
and catalog declarations, then produces minimal changes:
- A changed literal argument on an anchored call → updates that node's pin value.
- An anchored statement you removed → deletes that node only when `allow_deletions` is true.
- A new unanchored FlowScript call → adds that node, configures literal args, and connects
  resolvable FlowScript references/nested calls.
- A new unanchored `function name(...) { ... }` declaration → creates a Function layer, places
  body nodes inside it, creates boundary pins from params/returns, and wires `return` values.

RULES:
- PRESERVE every `//@n:<id>` anchor comment on statements you keep, exactly as given.
- Leave `allow_deletions` false unless the user explicitly asked to delete existing board items.
- Do NOT invent anchors for brand-new nodes; write normal unanchored calls using declarations
  from `get_declarations`.
- New catalog calls must be inside a function/event block, e.g.
  `run() { const db = openLocalDb({ name: "email_vectors" }) }`.
- Top-level `const name: Type = literal` declarations are variables/defaults only; they must use
  literal defaults and do not create node calls.
- If you use `variableGet({ varRef: "NAME" })` or any `varRef`, `NAME` must resolve to an
  existing variable or a top-level FlowScript variable declaration such as
  `const NAME: string = ""`; missing varRefs are validation errors.
- Inside a function/event block, `const name = ...` must bind a node-call expression. Use
  local alias syntax like `let rows = []` / `rows = arrayPush(...)`, typed `let name: Type =
  literal`, or direct literals for non-call values.
- Do not rely on mutable assignments inside brand-new `if`/`for` blocks; new control-flow body
  lowering is limited and unsafe partial graph edits are rejected.
- FlowScript statement order maps to the normal execution path only when the previous node has one
  execution output, a `done` / `exec_done` output, or an explicit continuation policy in the
  reconciler. Multi-output nodes are not guessed by pin order; API Call/httpFetch continues from
  `exec_success`, never `exec_error`. If no policy exists, validation reports a diagnostic instead
  of queueing an unsafe edge.
- Existing multi-output execution graphs render back to FlowScript as labelled branch blocks, so
  board -> FlowScript -> board preserves those branches rather than flattening them.
- Streaming calls with `on_stream` plus `exec_done` may place `.chunk` consumers immediately after
  the call; those consumers wire from `on_stream`, while later `.response` / `.stats` consumers
  continue from `exec_done`.
- For loops, the body is the `exec_out` path and the next statement continues from `done` /
  `exec_done`; make sure the loop's `array` input receives the array being iterated.
- Object/call-argument fields use colon syntax (`{ host: "imap.gmail.com" }`), never assignment
  syntax (`{ host = "imap.gmail.com" }`).
- Do NOT submit implementation plans, TODOs, function stubs, comments-only FlowScript, or lists of
  node names. If a signature is missing, call `get_declarations` again and submit concrete calls.
- Always provide the complete edited document in the `flowscript` argument; never call this tool
  with an empty string or only a summary.
- To reposition nodes on the canvas, use `emit_commands` with MoveNode. Positions are visual and
  not represented in FlowScript text."#
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "flowscript": {
                        "type": "string",
                        "description": "The full edited FlowScript source for the board, with anchors preserved."
                    },
                    "allow_deletions": {
                        "type": "boolean",
                        "description": "Set true only when the user explicitly requested deletion of existing board items. Defaults false to prevent incomplete FlowScript from deleting nodes."
                    }
                },
                "required": ["flowscript"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        if args.flowscript.trim().is_empty() {
            return Ok(format!(
                "{}\n{}",
                flowscript_workspace_tag(&args.flowscript, "validation_errors"),
                "FlowScript validation failed: edit_flowscript requires a non-empty `flowscript` string."
            ));
        }

        let catalog = self.provider.get_all_metadata().await;
        let result = reconcile_text_with_catalog(&self.board, &args.flowscript, &catalog);

        Ok(render_edit_flowscript_result(
            &args.flowscript,
            &result,
            board_has_no_nodes(&self.board),
            args.allow_deletions,
        ))
    }
}

// ============================================================================
// Tool Execution Helpers
// ============================================================================

pub fn build_list_board_nodes_output(graph_context: &GraphContext) -> String {
    if graph_context.nodes.is_empty() && graph_context.layers.is_empty() {
        return "The board is empty - no nodes found. Use get_declarations to find FlowScript signatures, then call edit_flowscript with the new workflow."
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
        "get_declarations" => {
            if let Some(query) = arguments.get("query").and_then(|v| v.as_str()) {
                format!("Looking up FlowScript declarations for \"{}\"", query)
            } else {
                "Looking up FlowScript declarations...".to_string()
            }
        }
        "get_current_flowscript" => "Reading current board FlowScript...".to_string(),
        "edit_flowscript" => "Applying FlowScript edits to the board...".to_string(),
        _ => format!("Running {}...", name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_flowscript_args_accept_common_source_aliases() {
        for key in ["flowscript", "script", "source", "content"] {
            let args: EditFlowScriptArgs =
                serde_json::from_value(json!({ key: "const db = openLocalDb({ name: \"x\" });" }))
                    .expect("alias should deserialize");
            assert!(args.flowscript.contains("openLocalDb"));
        }
    }

    #[test]
    fn edit_flowscript_result_flags_comment_only_empty_board_drafts() {
        let result = ReconcileResult::default();
        let output = render_edit_flowscript_result(
            "// Implementation plan: call openLocalDb later",
            &result,
            true,
            false,
        );

        assert!(output.contains("\"status\":\"validation_errors\""));
        assert!(output.contains("plan/stub"));
        assert!(output.contains("No board changes were derived"));
    }

    #[test]
    fn edit_flowscript_result_includes_workspace_tag_for_preview() {
        let result = ReconcileResult::default();
        let output = render_edit_flowscript_result(
            "run() {\n    const db = openLocalDb({ name: \"gmail_vectors\" })\n}",
            &result,
            false,
            false,
        );

        assert!(output.starts_with("<flowscript_workspace>"));
        assert!(output.contains("\"source\""));
        assert!(output.contains("openLocalDb"));
    }

    #[test]
    fn edit_flowscript_result_flags_empty_function_shells() {
        let result = ReconcileResult::default();
        let output = render_edit_flowscript_result("run() {\n}", &result, true, false);

        assert!(output.contains("\"status\":\"validation_errors\""));
        assert!(output.contains("no executable catalog calls"));
    }

    #[test]
    fn edit_flowscript_result_explains_colon_parse_errors() {
        let result = ReconcileResult {
            commands: Vec::new(),
            diagnostics: vec![
                "FlowScript parse error at line 31, col 21: expected `Colon`, found `Assign`"
                    .to_string(),
            ],
        };
        let output = render_edit_flowscript_result(
            "run() {\n    emailImapConnect({ host = \"imap.gmail.com\" })\n}",
            &result,
            true,
            false,
        );

        assert!(output.contains("\"status\":\"validation_errors\""));
        assert!(output.contains("colon syntax"));
        assert!(output.contains("not `{ host ="));
    }

    #[test]
    fn edit_flowscript_result_explains_const_binding_parse_errors() {
        let result = ReconcileResult {
            commands: Vec::new(),
            diagnostics: vec![
                "FlowScript parse error at line 45, col 9: `const` binding requires a call expression"
                    .to_string(),
            ],
        };
        let output = render_edit_flowscript_result(
            "run() {\n    const row = { id: \"x\" }\n}",
            &result,
            true,
            false,
        );

        assert!(output.contains("\"status\":\"validation_errors\""));
        assert!(output.contains("can only bind the output of a node call"));
        assert!(output.contains("local alias syntax like `let rows = []`"));
    }

    #[test]
    fn edit_flowscript_result_blocks_partial_control_flow_commands() {
        let result = ReconcileResult {
            commands: vec![BoardCommand::AddNode {
                node_type: "control_for_each".to_string(),
                ref_id: Some("$0".to_string()),
                position: None,
                friendly_name: None,
                target_layer: None,
                summary: None,
            }],
            diagnostics: vec![
                "new FlowScript loop statements are not yet converted automatically; use emit_commands for loop body wiring if needed"
                    .to_string(),
            ],
        };
        let output = render_edit_flowscript_result(
            "run() {\n    for (const item of controlForEach({ array: rows })) {\n        log({ text: item.value })\n    }\n}",
            &result,
            true,
            false,
        );

        assert!(output.contains("\"status\":\"validation_errors\""));
        assert!(output.contains("partial commands"));
        assert!(!output.contains("<commands>"));
    }

    #[test]
    fn edit_flowscript_result_blocks_deletions_by_default() {
        let result = ReconcileResult {
            commands: vec![BoardCommand::RemoveNode {
                node_id: "old_node".to_string(),
                summary: None,
            }],
            diagnostics: Vec::new(),
        };
        let output = render_edit_flowscript_result("run() {\n}", &result, false, false);

        assert!(output.contains("\"status\":\"validation_errors\""));
        assert!(output.contains("Deletions are blocked by default"));
        assert!(!output.contains("<commands>"));
    }

    #[test]
    fn edit_flowscript_result_keeps_true_no_changes_non_error() {
        let result = ReconcileResult::default();
        let output = render_edit_flowscript_result("run() {\n}", &result, false, false);

        assert!(output.contains("\"status\":\"no_changes\""));
    }
}
