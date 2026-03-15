//! Copilot SDK Tool Adapters
//!
//! This module provides adapters that bridge the existing rig-based tools
//! to the Copilot SDK's tool system. The core logic is reused from
//! `flow_like::flow::copilot::tools`.

use std::sync::Arc;

pub use copilot_sdk::ToolHandler;
use copilot_sdk::{Tool, ToolResultObject};
use flow_like::flow::copilot::{BoardCommand, GraphContext};
use flow_like_catalog::get_catalog;
use serde_json::{Value, json};

/// Create all Copilot SDK tools for board context
pub fn create_board_tools(graph_context: Option<Arc<GraphContext>>) -> Vec<(Tool, ToolHandler)> {
    let mut tools = vec![create_catalog_search_tool(), create_emit_commands_tool()];

    if let Some(ctx) = graph_context.clone() {
        tools.push(create_get_node_details_tool(ctx));
    }

    if let Some(ctx) = graph_context.clone() {
        tools.push(create_get_unconfigured_nodes_tool(ctx));
    }

    if let Some(ctx) = graph_context {
        tools.push(create_list_board_nodes_tool(ctx));
    }

    tools
}

/// Catalog search tool - find nodes by functionality (fully synchronous)
fn create_catalog_search_tool() -> (Tool, ToolHandler) {
    let tool = Tool::new("catalog_search")
        .description(
            r#"Search the node catalog by functionality or name. Returns matching nodes with their node_type (needed for AddNode).

WHEN TO USE: Before adding any node - to get the exact node_type
EXAMPLE QUERIES: "http request", "parse json", "loop array", "condition if""#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language search. Examples: 'http request', 'json parse', 'loop array'"
                }
            },
            "required": ["query"]
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let query_tokens: Vec<&str> = query.split_whitespace().collect();

        // Sync catalog search - no async needed
        let catalog = get_catalog();
        let mut scored_matches: Vec<(i32, String)> = Vec::new();

        for logic in &catalog {
            let node = logic.get_node();
            let name_lower = node.name.to_lowercase();
            let friendly_lower = node.friendly_name.to_lowercase();
            let desc_lower = node.description.to_lowercase();
            let category = name_lower.split("::").nth(1).unwrap_or("");

            let mut score = 0i32;

            if name_lower.contains(&query) {
                score += 100;
            }
            if friendly_lower.contains(&query) {
                score += 90;
            }

            for token in &query_tokens {
                if name_lower.contains(token) {
                    score += 30;
                }
                if friendly_lower.contains(token) {
                    score += 25;
                }
                if category.contains(token) {
                    score += 20;
                }
                if desc_lower.contains(token) {
                    score += 10;
                }
            }

            // Exact part match bonus
            let name_parts: Vec<&str> = name_lower.split([':', '_']).collect();
            for token in &query_tokens {
                if name_parts.iter().any(|part| part == token) {
                    score += 15;
                }
            }

            if score > 0 {
                // Compact format: node_type: friendly_name - truncated description
                let desc_short = if node.description.chars().count() > 50 {
                    format!(
                        "{}...",
                        node.description.chars().take(47).collect::<String>()
                    )
                } else {
                    node.description.clone()
                };
                let compact = format!("{}: {} - {}", node.name, node.friendly_name, desc_short);
                scored_matches.push((score, compact));
            }
        }

        scored_matches.sort_by(|a, b| b.0.cmp(&a.0));
        let results: Vec<String> = scored_matches
            .into_iter()
            .take(10)
            .map(|(_, s)| s)
            .collect();

        if results.is_empty() {
            ToolResultObject::text("No nodes found matching your query. Try different keywords.")
        } else {
            ToolResultObject::text(results.join("\n"))
        }
    });

    (tool, handler)
}

/// Get node details - full info about a specific node
fn create_get_node_details_tool(context: Arc<GraphContext>) -> (Tool, ToolHandler) {
    let tool = Tool::new("get_node_details")
        .description(
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
4. Use exact pin names from outputs/inputs in ConnectPins"#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "node_id": {
                    "type": "string",
                    "description": "The node ID to inspect (from list_board_nodes or context)"
                }
            },
            "required": ["node_id"]
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let ctx = context.clone();
        let node_id = args
            .get("node_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let node = ctx.nodes.iter().find(|n| n.id == node_id);

        match node {
            Some(node_ctx) => {
                let incoming_edges: Vec<Value> = ctx
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

                let outgoing_edges: Vec<Value> = ctx
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
                    "is_selected": ctx.selected_nodes.contains(&node_id)
                });

                ToolResultObject::text(serde_json::to_string_pretty(&details).unwrap_or_default())
            }
            None => ToolResultObject::text(format!(
                "Error: Node with ID '{}' not found in the current graph",
                node_id
            )),
        }
    });

    (tool, handler)
}

/// Emit commands tool - execute graph modifications
fn create_emit_commands_tool() -> (Tool, ToolHandler) {
    let tool = Tool::new("emit_commands")
        .description(
            r#"Execute graph modifications. Commands are batched and applied atomically.

CRITICAL ORDER:
1. AddNode commands FIRST (create nodes)
2. ConnectPins commands (wire execution + data)
3. UpdateNodePin commands LAST (set values)

COMMAND SCHEMAS:
AddNode: {"command_type": "AddNode", "node_type": "category::subcategory::name", "ref_id": "$0", "position": {"x": 300, "y": 200}, "summary": "description"}
ConnectPins: {"command_type": "ConnectPins", "from_node": "$0", "from_pin": "exec_out", "to_node": "$1", "to_pin": "exec_in", "summary": "Connect execution flow"}
UpdateNodePin: {"command_type": "UpdateNodePin", "node_id": "$0", "pin_id": "url", "value": "https://example.com", "summary": "Set URL"}
RemoveNode: {"command_type": "RemoveNode", "node_id": "existing_node_id", "summary": "Remove node"}

POSITIONING:
- Place new nodes NEAR related nodes (within 250px)
- Horizontal flow: x+250 for each subsequent node
- If connecting TO existing node at {x:500, y:200}, place new node at {x:250, y:200}
- If connecting FROM existing node at {x:500, y:200}, place new node at {x:750, y:200}

REF_IDS:
- Use "$0", "$1", "$2" to reference new nodes in same batch
- Can use ref_id as from_node/to_node in ConnectPins
- Can use ref_id as node_id in UpdateNodePin

EXAMPLE - HTTP request with JSON parsing:
{
  "commands": [
    {"command_type": "AddNode", "node_type": "http::request::send_request", "ref_id": "$0", "position": {"x": 300, "y": 200}, "summary": "HTTP request"},
    {"command_type": "AddNode", "node_type": "data::json::parse", "ref_id": "$1", "position": {"x": 550, "y": 200}, "summary": "Parse JSON"},
    {"command_type": "ConnectPins", "from_node": "$0", "from_pin": "exec_out", "to_node": "$1", "to_pin": "exec_in", "summary": "Execution flow"},
    {"command_type": "ConnectPins", "from_node": "$0", "from_pin": "response_body", "to_node": "$1", "to_pin": "json_string", "summary": "Pass body"},
    {"command_type": "UpdateNodePin", "node_id": "$0", "pin_id": "url", "value": "https://api.example.com", "summary": "Set URL"},
    {"command_type": "UpdateNodePin", "node_id": "$0", "pin_id": "method", "value": "GET", "summary": "Set method"}
  ],
  "explanation": "HTTP GET request followed by JSON parsing"
}"#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "commands": {
                    "type": "array",
                    "description": "Array of command objects. Each needs command_type + relevant fields.",
                    "items": {
                        "type": "object"
                    }
                },
                "explanation": {
                    "type": "string",
                    "description": "Brief description of what these commands accomplish"
                }
            },
            "required": ["commands", "explanation"]
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let commands = args.get("commands").cloned().unwrap_or(json!([]));
        let explanation = args
            .get("explanation")
            .and_then(|v| v.as_str())
            .unwrap_or("Commands queued");

        // Parse commands from JSON
        let parsed_commands: Vec<BoardCommand> = match serde_json::from_value(commands.clone()) {
            Ok(cmds) => cmds,
            Err(e) => {
                return ToolResultObject::text(format!("Error parsing commands: {}", e));
            }
        };

        // Build summary
        let mut summary_lines: Vec<String> = Vec::new();
        summary_lines.push(format!("✓ Queued {} commands:", parsed_commands.len()));

        for cmd in &parsed_commands {
            let cmd_summary = match cmd {
                BoardCommand::AddNode {
                    node_type,
                    ref_id,
                    friendly_name,
                    ..
                } => {
                    format!(
                        "  - AddNode: {} (ref: {})",
                        friendly_name.as_deref().unwrap_or(node_type),
                        ref_id.as_deref().unwrap_or("none")
                    )
                }
                BoardCommand::AddPlaceholder { name, ref_id, .. } => {
                    format!(
                        "  - AddPlaceholder: \"{}\" (ref: {})",
                        name,
                        ref_id.as_deref().unwrap_or("none")
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
                    format!("  - Remove node: {}", node_id)
                }
                BoardCommand::UpdateNodePin {
                    node_id, pin_id, ..
                } => {
                    format!("  - Update pin: {}.{}", node_id, pin_id)
                }
                _ => "  - Other command".to_string(),
            };
            summary_lines.push(cmd_summary);
        }

        summary_lines.push(format!("\nExplanation: {}", explanation));

        // Serialize commands to be returned (the frontend will apply them)
        let result = json!({
            "status": "queued",
            "commands": commands,
            "explanation": explanation,
            "summary": summary_lines.join("\n")
        });

        ToolResultObject::text(serde_json::to_string_pretty(&result).unwrap_or_default())
    });

    (tool, handler)
}

/// Get unconfigured nodes - find nodes with empty/unconnected required inputs
fn create_get_unconfigured_nodes_tool(context: Arc<GraphContext>) -> (Tool, ToolHandler) {
    let tool = Tool::new("get_unconfigured_nodes")
        .description(
            r#"Find nodes that need configuration - inputs with no value and no incoming connection.

WHEN TO USE:
- Check what needs to be configured in the workflow
- Find nodes that aren't fully set up
- Identify missing connections

RETURNS: List of nodes with their unconfigured input pins"#,
        )
        .schema(json!({
            "type": "object",
            "properties": {},
            "required": []
        }));

    let handler: ToolHandler = Arc::new(move |_name, _args| {
        let ctx = context.clone();

        // Build set of connected input pins
        let connected_pins: std::collections::HashSet<(String, String)> = ctx
            .edges
            .iter()
            .map(|e| (e.to_node_id.clone(), e.to_pin_name.clone()))
            .collect();

        let mut unconfigured: Vec<Value> = Vec::new();

        for node in &ctx.nodes {
            let mut missing_inputs: Vec<Value> = Vec::new();

            for input in &node.inputs {
                // Skip execution pins - they're optional flow control
                if input.type_name == "Execution" {
                    continue;
                }

                let has_connection =
                    connected_pins.contains(&(node.id.clone(), input.name.clone()));
                let has_value = input.default_value.is_some();

                if !has_connection && !has_value {
                    missing_inputs.push(json!({
                        "pin": input.name,
                        "type": input.type_name
                    }));
                }
            }

            if !missing_inputs.is_empty() {
                unconfigured.push(json!({
                    "node_id": node.id,
                    "name": node.friendly_name,
                    "type": node.node_type,
                    "missing_inputs": missing_inputs
                }));
            }
        }

        if unconfigured.is_empty() {
            ToolResultObject::text("All nodes are configured - no missing inputs found.")
        } else {
            ToolResultObject::text(serde_json::to_string_pretty(&unconfigured).unwrap_or_default())
        }
    });

    (tool, handler)
}

/// List board nodes - get a compact overview of all nodes in the workflow
fn create_list_board_nodes_tool(context: Arc<GraphContext>) -> (Tool, ToolHandler) {
    let tool = Tool::new("list_board_nodes")
        .description(
            r#"List all nodes in the current workflow with their IDs and positions.

USE THIS FIRST to understand the workflow before making changes.

RETURNS:
- node_id: Use in get_node_details, ConnectPins, UpdateNodePin
- node_type: The node's catalog type
- friendly_name: Human-readable name
- position: {x, y} - use to place new nodes nearby

WORKFLOW:
1. list_board_nodes → see all nodes and positions
2. get_node_details on relevant node → get pin names
3. catalog_search → find new node types to add
4. emit_commands → create nodes near existing ones + connect"#,
        )
        .schema(json!({
            "type": "object",
            "properties": {},
            "required": []
        }));

    let handler: ToolHandler = Arc::new(move |_name, _args| {
        let ctx = context.clone();

        if ctx.nodes.is_empty() {
            return ToolResultObject::text(
                "The board is empty - no nodes found. Use catalog_search to find nodes to add.",
            );
        }

        let mut lines: Vec<String> = Vec::new();
        lines.push(format!("Board has {} nodes:", ctx.nodes.len()));

        for node in &ctx.nodes {
            let selected = if ctx.selected_nodes.contains(&node.id) {
                " [SELECTED]"
            } else {
                ""
            };
            let pos_str = format!("pos:({},{})", node.position.0, node.position.1);
            lines.push(format!(
                "- {} | {} | {} | {}{}",
                node.id, node.node_type, node.friendly_name, pos_str, selected
            ));
        }

        if !ctx.variables.is_empty() {
            lines.push(format!("\nVariables ({}):", ctx.variables.len()));
            for var in &ctx.variables {
                lines.push(format!("- {}: {} ({})", var.id, var.name, var.data_type));
            }
        }

        lines
            .push("\n→ Use get_node_details(node_id) to get pin names for connections".to_string());

        ToolResultObject::text(lines.join("\n"))
    });

    (tool, handler)
}

// =============================================================================
// FRONTEND (A2UI) TOOLS
// =============================================================================

/// Create all Copilot SDK tools for frontend/A2UI context
pub fn create_frontend_tools() -> Vec<(Tool, ToolHandler)> {
    vec![
        create_emit_ui_tool(),
        create_get_component_schema_tool(),
    ]
}

/// Emit UI tool - output A2UI JSON components
fn create_emit_ui_tool() -> (Tool, ToolHandler) {
    let tool = Tool::new("emit_ui")
        .description(
            r#"Output A2UI components to render in the interface. This is NOT file editing - it generates JSON that renders directly in the app.

OUTPUT FORMAT:
{
  "rootComponentId": "root",
  "canvasSettings": { "backgroundColor": "bg-background", "padding": "1rem" },
  "components": [...]
}

COMPONENT FORMAT:
{
  "id": "unique-kebab-case-id",
  "style": { "className": "tailwind classes" },
  "component": { "type": "componentType", ...props }
}

BOUNDVALUE FORMAT (ALL props use this):
- String: {"literalString": "text"}
- Number: {"literalNumber": 42}
- Boolean: {"literalBool": true}
- Options: {"literalOptions": [{"value": "v", "label": "L"}]}
- Data binding: {"path": "$.data.field", "defaultValue": "fallback"}

CHILDREN FORMAT:
"children": {"explicitList": ["child-id-1", "child-id-2"]}

AVAILABLE COMPONENTS:
Layout: column, row, grid, stack, scrollArea, box, center, spacer
Display: text, image, icon, badge, avatar, progress, spinner, divider, markdown
Interactive: button, textField, select, slider, checkbox, switch, link
Container: card, modal, tabs, accordion, drawer, tooltip

THEME COLORS (use these, not hardcoded):
bg-background, bg-muted, bg-card, bg-primary, bg-secondary
text-foreground, text-muted-foreground, text-primary-foreground
border-border

CUSTOM CSS (for advanced effects):
Use canvasSettings.customCss for animations/effects not achievable with Tailwind:
{"canvasSettings": {"backgroundColor": "bg-background", "customCss": ".animated { animation: fade 1s; } @keyframes fade { from{opacity:0} to{opacity:1} }"}}

EXAMPLE - Simple card:
{
  "rootComponentId": "card-1",
  "canvasSettings": {"backgroundColor": "bg-background"},
  "components": [
    {
      "id": "card-1",
      "style": {"className": "p-4"},
      "component": {
        "type": "card",
        "children": {"explicitList": ["title", "content"]}
      }
    },
    {
      "id": "title",
      "component": {
        "type": "text",
        "content": {"literalString": "Hello"},
        "variant": {"literalString": "h2"}
      }
    },
    {
      "id": "content",
      "component": {
        "type": "text",
        "content": {"literalString": "World"}
      }
    }
  ]
}"#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "rootComponentId": {
                    "type": "string",
                    "description": "ID of the root component"
                },
                "canvasSettings": {
                    "type": "object",
                    "description": "Canvas settings (backgroundColor, padding, customCss)"
                },
                "components": {
                    "type": "array",
                    "description": "Array of SurfaceComponent objects",
                    "items": { "type": "object" }
                }
            },
            "required": ["rootComponentId", "components"]
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let root_id = args
            .get("rootComponentId")
            .and_then(|v| v.as_str())
            .unwrap_or("root");
        let canvas = args.get("canvasSettings").cloned().unwrap_or(json!({}));
        let components = args.get("components").cloned().unwrap_or(json!([]));

        // Validate components and collect errors
        let (validated_components, validation_errors) = validate_ui_components(&components);

        if !validation_errors.is_empty() {
            let error_list = validation_errors.join("\n- ");
            let result = json!({
                "status": "validation_errors",
                "errors": validation_errors,
                "rootComponentId": root_id,
                "canvasSettings": canvas,
                "components": validated_components,
                "message": format!(
                    "UI rendered with {} validation error(s). Fix these and call emit_ui again:\n- {}",
                    validation_errors.len(),
                    error_list
                )
            });
            return ToolResultObject::text(serde_json::to_string(&result).unwrap_or_default());
        }

        let result = json!({
            "status": "rendered",
            "rootComponentId": root_id,
            "canvasSettings": canvas,
            "components": validated_components,
            "message": "UI components have been rendered successfully"
        });

        ToolResultObject::text(serde_json::to_string(&result).unwrap_or_default())
    });

    (tool, handler)
}

/// Component schema lookup tool
fn create_get_component_schema_tool() -> (Tool, ToolHandler) {
    let tool = Tool::new("get_component_schema")
        .description(
            r#"Look up the detailed schema for one or more A2UI component types. Call this BEFORE generating components you haven't used before.

Returns: Full property list with types, required fields, BoundValue format, and a working example.

AVAILABLE TYPES:
Layout: column, row, grid, stack, scrollArea, box, center, spacer, absolute, aspectRatio, overlay
Display: text, image, icon, video, lottie, markdown, badge, avatar, progress, spinner, divider, skeleton, iframe
Interactive: button, textField, select, slider, checkbox, switch, radioGroup, dateTimeInput, fileInput, imageInput, link
Container: card, modal, tabs, accordion, drawer, tooltip, popover
Data: table, nivoChart, plotlyChart, filePreview
Vision: boundingBoxOverlay, imageLabeler, imageHotspot
Game: canvas2d, sprite, shape, scene3d, model3d, dialogue, characterPortrait, choiceMenu, inventoryGrid, healthBar, miniMap"#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "component_types": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Array of component type names to look up (e.g., [\"card\", \"text\", \"button\"])"
                }
            },
            "required": ["component_types"]
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let types = args
            .get("component_types")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if types.is_empty() {
            return ToolResultObject::text("Please provide at least one component type to look up.");
        }

        let mut docs = Vec::new();
        for comp_type in &types {
            docs.push(format!(
                "## {}\n{}",
                comp_type,
                get_component_schema_doc(comp_type)
            ));
        }

        ToolResultObject::text(docs.join("\n\n---\n\n"))
    });

    (tool, handler)
}

// =============================================================================
// VALIDATION HELPERS
// =============================================================================

/// Known props per component type (mirrors validateComponents.ts)
fn known_props_for_type(component_type: &str) -> Option<&'static [&'static str]> {
    match component_type {
        "row" => Some(&["gap", "align", "justify", "wrap", "reverse"]),
        "column" => Some(&["gap", "align", "justify", "reverse", "wrap"]),
        "stack" => Some(&["align", "width", "height"]),
        "grid" => Some(&["columns", "rows", "gap", "columnGap", "rowGap", "autoFlow"]),
        "scrollArea" => Some(&["direction"]),
        "aspectRatio" => Some(&["ratio"]),
        "absolute" => Some(&["width", "height"]),
        "box" => Some(&["as", "semanticRole"]),
        "center" => Some(&["inline"]),
        "spacer" => Some(&["size", "flex", "direction", "flexible"]),
        "overlay" => Some(&["baseComponentId", "overlays"]),
        "widgetInstance" => Some(&["widgetId", "widgetInputs", "bindOutputs"]),
        "text" => Some(&["content", "variant", "size", "weight", "color", "align", "truncate", "maxLines"]),
        "image" => Some(&["src", "alt", "fit", "fallback", "fallbackSrc", "loading", "aspectRatio", "width", "height"]),
        "icon" => Some(&["name", "size", "color", "strokeWidth"]),
        "video" => Some(&["src", "poster", "autoplay", "autoPlay", "loop", "muted", "controls", "width", "height"]),
        "lottie" => Some(&["src", "autoplay", "loop", "speed", "width", "height"]),
        "markdown" => Some(&["content", "allowHtml"]),
        "divider" => Some(&["orientation", "thickness", "color"]),
        "badge" => Some(&["content", "text", "variant", "color"]),
        "avatar" => Some(&["src", "fallback", "size"]),
        "progress" => Some(&["value", "max", "showLabel", "variant", "color"]),
        "spinner" => Some(&["size", "color"]),
        "skeleton" => Some(&["width", "height", "rounded", "variant"]),
        "iframe" => Some(&["src", "srcdoc", "width", "height", "sandbox", "allow", "title", "referrerPolicy", "border", "loading"]),
        "table" => Some(&["columns", "data", "caption", "striped", "bordered", "hoverable", "compact", "stickyHeader", "sortable", "searchable", "paginated", "pageSize", "selectable", "showPagination"]),
        "plotlyChart" => Some(&["chartType", "data", "title", "layout", "config", "height", "width"]),
        "nivoChart" => Some(&["chartType", "data", "height", "width", "colors", "colorScheme", "showLegend", "legendPosition", "margin", "axisBottom", "axisLeft", "animate", "motionConfig", "style"]),
        "filePreview" => Some(&["url", "mimeType", "width", "height"]),
        "boundingBoxOverlay" => Some(&["src", "boxes", "showLabels", "showConfidence", "normalized", "width", "height"]),
        "button" => Some(&["label", "variant", "size", "disabled", "loading", "icon", "iconPosition", "tooltip"]),
        "textField" => Some(&["value", "placeholder", "label", "helperText", "error", "disabled", "inputType", "type", "multiline", "rows", "maxLength", "required"]),
        "select" => Some(&["value", "options", "placeholder", "label", "disabled", "multiple", "searchable"]),
        "slider" => Some(&["value", "min", "max", "step", "disabled", "showValue", "label"]),
        "checkbox" => Some(&["checked", "label", "disabled", "indeterminate"]),
        "switch" => Some(&["checked", "label", "disabled"]),
        "radioGroup" => Some(&["value", "options", "disabled", "orientation", "label"]),
        "dateTimeInput" => Some(&["value", "mode", "min", "max", "disabled", "label"]),
        "fileInput" => Some(&["value", "label", "helperText", "accept", "multiple", "maxSize", "maxFiles", "disabled", "error"]),
        "imageInput" => Some(&["value", "label", "helperText", "accept", "multiple", "maxSize", "maxFiles", "disabled", "error", "aspectRatio", "showPreview"]),
        "imageLabeler" => Some(&["src", "labels", "boxes", "disabled", "width", "height"]),
        "imageHotspot" => Some(&["src", "hotspots", "markerStyle", "width", "height"]),
        "link" => Some(&["href", "label", "text", "route", "queryParams", "external", "target", "variant", "underline", "disabled", "openInNewTab"]),
        "card" => Some(&["title", "description", "footer", "hoverable", "clickable", "variant", "padding", "headerImage", "headerIcon"]),
        "modal" => Some(&["open", "title", "description", "closeOnOverlay", "closeOnEscape", "showCloseButton", "size", "centered"]),
        "tabs" => Some(&["value", "tabs", "orientation", "variant", "defaultValue"]),
        "accordion" => Some(&["items", "multiple", "defaultExpanded", "collapsible", "type"]),
        "drawer" => Some(&["open", "side", "title", "size", "overlay", "closable", "description"]),
        "tooltip" => Some(&["content", "side", "delayMs", "maxWidth"]),
        "popover" => Some(&["open", "contentComponentId", "side", "trigger", "closeOnClickOutside", "content"]),
        "canvas2d" => Some(&["width", "height", "backgroundColor", "pixelPerfect"]),
        "sprite" => Some(&["src", "x", "y", "width", "height", "rotation", "scale", "opacity", "flipX", "flipY", "zIndex"]),
        "shape" => Some(&["shapeType", "x", "y", "width", "height", "radius", "points", "fill", "stroke", "strokeWidth"]),
        "scene3d" => Some(&["width", "height", "cameraType", "cameraPosition", "backgroundColor", "controlMode", "fixedView", "autoRotateSpeed", "enableControls", "enableZoom", "enablePan", "fov", "near", "far", "target", "ambientLight", "directionalLight", "showGrid", "showAxes"]),
        "model3d" => Some(&["src", "position", "rotation", "scale", "castShadow", "receiveShadow", "animation", "autoRotate", "rotateSpeed", "viewerHeight", "backgroundColor", "cameraDistance", "fov", "cameraAngle", "cameraPosition", "cameraTarget", "enableControls", "enableZoom", "enablePan", "autoRotateCamera", "cameraRotateSpeed", "ambientLight", "directionalLight", "fillLight", "rimLight", "lightColor", "lightingPreset", "showGround", "groundColor", "enableReflections", "environment", "environmentSource", "useHdrBackground", "polyhavenHdri", "polyhavenResolution"]),
        "dialogue" => Some(&["text", "speakerName", "typewriter", "speed", "portrait"]),
        "characterPortrait" => Some(&["image", "expression", "position", "width", "height", "flip"]),
        "choiceMenu" => Some(&["choices", "title", "layout", "columns"]),
        "inventoryGrid" => Some(&["items", "columns", "rows", "cellSize", "showTooltips"]),
        "healthBar" => Some(&["value", "maxValue", "label", "fillColor", "variant", "showLabel", "size", "animated"]),
        "miniMap" => Some(&["mapImage", "width", "height", "markers", "playerX", "playerY", "viewportWidth", "viewportHeight", "zoom"]),
        _ => None,
    }
}

/// Required props per component type
fn required_props_for_type(component_type: &str) -> &'static [&'static str] {
    match component_type {
        "text" => &["content"],
        "image" => &["src"],
        "icon" => &["name"],
        "video" => &["src"],
        "lottie" => &["src"],
        "markdown" => &["content"],
        "badge" => &["content"],
        "progress" => &["value"],
        "button" => &["label"],
        "textField" => &["value"],
        "select" => &["value", "options"],
        "slider" => &["value"],
        "checkbox" => &["checked"],
        "switch" => &["checked"],
        "radioGroup" => &["value", "options"],
        "dateTimeInput" => &["value"],
        "fileInput" => &["value"],
        "imageInput" => &["value"],
        "link" => &["href"],
        "modal" => &["open"],
        "tabs" => &["value"],
        "canvas2d" => &["width", "height"],
        "sprite" => &["src", "x", "y"],
        "shape" => &["shapeType", "x", "y"],
        "scene3d" => &["width", "height"],
        "model3d" => &["src"],
        "aspectRatio" => &["ratio"],
        "boundingBoxOverlay" => &["src"],
        _ => &[],
    }
}

const BASE_PROPS: &[&str] = &["type", "id", "style", "children", "actions"];

/// Validate an array of components and return (validated_components, errors)
fn validate_ui_components(components: &Value) -> (Value, Vec<String>) {
    let mut errors = Vec::new();

    let arr = match components.as_array() {
        Some(a) => a,
        None => {
            errors.push("'components' must be an array".to_string());
            return (json!([]), errors);
        }
    };

    let all_ids: std::collections::HashSet<String> = arr
        .iter()
        .filter_map(|c| c.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();

    let mut validated = Vec::new();

    for comp in arr {
        let id = match comp.get("id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                errors.push("Component missing 'id' field - every component needs a unique id".to_string());
                continue;
            }
        };

        let component = match comp.get("component") {
            Some(c) if c.is_object() => c,
            _ => {
                errors.push(format!("{}: missing 'component' object", id));
                continue;
            }
        };

        let comp_type = match component.get("type").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                errors.push(format!("{}: missing 'component.type' field", id));
                continue;
            }
        };

        // Check if component type is known
        let known = known_props_for_type(comp_type);
        if known.is_none() {
            errors.push(format!(
                "{}: unknown component type '{}'. Use get_component_schema to look up available types.",
                id, comp_type
            ));
            continue;
        }
        let known_set = known.unwrap();

        // Check for unknown props
        if let Some(obj) = component.as_object() {
            for key in obj.keys() {
                let k = key.as_str();
                if !BASE_PROPS.contains(&k) && !known_set.contains(&k) {
                    errors.push(format!(
                        "{}: unknown prop '{}' on '{}'. Use get_component_schema(\"{}\") to see valid props.",
                        id, k, comp_type, comp_type
                    ));
                }
            }
        }

        // Check required props
        for required in required_props_for_type(comp_type) {
            if component.get(*required).is_none() {
                errors.push(format!(
                    "{}: missing required prop '{}' on '{}'. This prop is mandatory.",
                    id, required, comp_type
                ));
            }
        }

        // Validate BoundValue format for non-structural props
        if let Some(obj) = component.as_object() {
            for (key, value) in obj {
                let k = key.as_str();
                if BASE_PROPS.contains(&k) || k == "type" {
                    continue;
                }
                // Structural props (arrays/objects holding non-BoundValue data) - skip
                if matches!(
                    k,
                    "tabs" | "items" | "overlays" | "columns" | "data" | "boxes"
                        | "hotspots" | "markers" | "choices"
                ) {
                    continue;
                }
                // Check if value should be BoundValue but is a bare primitive
                if value.is_string() || value.is_number() || value.is_boolean() {
                    if known_set.contains(&k) && k != "type" {
                        errors.push(format!(
                            "{}: prop '{}' uses a bare value. Wrap it as BoundValue: string→{{\"literalString\": \"{}\"}}, number→{{\"literalNumber\": {}}}, bool→{{\"literalBool\": {}}}",
                            id, k,
                            value.as_str().unwrap_or("..."),
                            value.as_f64().map(|n| n.to_string()).unwrap_or_else(|| "...".to_string()),
                            value.as_bool().map(|b| b.to_string()).unwrap_or_else(|| "...".to_string()),
                        ));
                    }
                }
            }
        }

        // Validate children references
        if let Some(children) = component.get("children") {
            if let Some(explicit_list) = children.get("explicitList").and_then(|v| v.as_array()) {
                for child_ref in explicit_list {
                    if let Some(child_id) = child_ref.as_str() {
                        if !all_ids.contains(child_id) {
                            errors.push(format!(
                                "{}: children references '{}' which doesn't exist in the components array",
                                id, child_id
                            ));
                        }
                    }
                }
            }
        }

        validated.push(comp.clone());
    }

    (json!(validated), errors)
}

/// Get detailed schema documentation for a component type
fn get_component_schema_doc(component_type: &str) -> String {
    use flow_like::a2ui::copilot::get_component_schema;

    let base_doc = get_component_schema(component_type);

    // Add BoundValue reminder and known props list
    if let Some(props) = known_props_for_type(component_type) {
        let required = required_props_for_type(component_type);
        let prop_list: Vec<String> = props.iter().map(|p| {
            if required.contains(p) {
                format!("- {} (REQUIRED)", p)
            } else {
                format!("- {}", p)
            }
        }).collect();

        format!(
            "{}\n\n### Valid Props\n{}\n\n### BoundValue Reminder\nAll props must use BoundValue format:\n- String: {{\"literalString\": \"text\"}}\n- Number: {{\"literalNumber\": 42}}\n- Boolean: {{\"literalBool\": true}}\n- Options: {{\"literalOptions\": [{{\"value\": \"v\", \"label\": \"L\"}}]}}\n- Children: {{\"explicitList\": [\"child-id-1\"]}}",
            base_doc,
            prop_list.join("\n")
        )
    } else {
        base_doc
    }
}
