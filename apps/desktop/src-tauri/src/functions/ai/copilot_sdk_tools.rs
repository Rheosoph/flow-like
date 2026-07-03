//! Copilot SDK Tool Adapters
//!
//! This module provides adapters that bridge the existing rig-based tools
//! to the Copilot SDK's tool system. The core logic is reused from
//! `flow_like::flow::copilot::tools`.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

use super::frontend_tool_bridge::{FrontendToolApproval, FrontendToolBridge};
pub use copilot_sdk::ToolHandler;
use copilot_sdk::{Tool, ToolResultObject};
use flow_like::flow::ast::{
    RenderOptions, blocked_destructive_flowscript_message, board_to_flowscript,
    destructive_flowscript_command_summaries, reconcile_text_with_catalog,
};
use flow_like::flow::board::Board;
use flow_like::flow::copilot::{
    BoardCommand, CatalogProvider, GraphContext, NodeMetadata, search_result_hint_lines,
};
use flow_like::flow::pin::PinType;
use flow_like_catalog::get_catalog;
use serde_json::{Value, json};

/// Create all Copilot SDK tools for board context.
///
/// When a live `board` is supplied the FlowScript transpile surface is enabled: `get_declarations`
/// (signature lookup) and `edit_flowscript` (apply edited FlowScript via reconcile) are registered
/// in addition to the structural `emit_commands` path.
pub fn create_board_tools(
    graph_context: Option<Arc<GraphContext>>,
    board: Option<Arc<Board>>,
    catalog_provider: Option<Arc<dyn CatalogProvider>>,
    side_effect_commands: Option<Arc<Mutex<Vec<BoardCommand>>>>,
) -> Vec<(Tool, ToolHandler)> {
    let mut tools = vec![
        create_catalog_search_tool(catalog_provider.clone()),
        create_validate_commands_tool(graph_context.clone()),
        create_emit_commands_tool(graph_context.clone()),
    ];

    if let Some(provider) = catalog_provider.clone() {
        tools.push(create_get_declarations_tool(provider.clone()));
    }

    if let Some(board) = board {
        tools.push(create_get_current_flowscript_tool(board.clone()));
        tools.push(create_edit_flowscript_tool(
            board,
            catalog_provider,
            side_effect_commands,
        ));
    }

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

/// Create runtime tools that execute through the frontend bridge.
///
/// These tools need browser/app context such as the active backend state, storage provider,
/// approval dialogs, and execution service. The Rust SDK tool blocks until the frontend replies.
pub fn create_runtime_tools(bridge: FrontendToolBridge) -> Vec<(Tool, ToolHandler)> {
    vec![
        create_internet_search_tool(),
        create_database_tool(bridge.clone()),
        create_storage_tool(bridge.clone()),
        create_ui_inspect_tool(bridge.clone()),
        create_execute_event_tool(bridge.clone()),
        create_ask_user_tool(bridge),
    ]
}

fn frontend_tool_result(
    bridge: &FrontendToolBridge,
    tool_name: &'static str,
    args: Value,
    approval: FrontendToolApproval,
) -> ToolResultObject {
    frontend_tool_result_with_timeout(bridge, tool_name, args, approval, Duration::from_secs(120))
}

fn frontend_tool_result_with_timeout(
    bridge: &FrontendToolBridge,
    tool_name: &'static str,
    args: Value,
    approval: FrontendToolApproval,
    timeout: Duration,
) -> ToolResultObject {
    ToolResultObject::text(
        serde_json::to_string_pretty(&bridge.call_with_timeout(tool_name, args, approval, timeout))
            .unwrap_or_else(|_| "{\"status\":\"error\"}".to_string()),
    )
}

fn arg_string(args: &Value, snake: &str, camel: &str) -> String {
    args.get(snake)
        .or_else(|| args.get(camel))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn database_operation_requires_approval(operation: &str) -> bool {
    matches!(
        operation,
        "insert"
            | "add_items"
            | "delete"
            | "remove_items"
            | "update"
            | "build_index"
            | "drop_index"
            | "optimize"
            | "add_column"
            | "drop_columns"
            | "alter_column"
    )
}

fn flowscript_validation_message(diagnostics: &[String]) -> &'static str {
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.contains("labelled branch requires a call condition"))
    {
        return "FlowScript validation failed: labelled branch syntax (`if (...) { // label ... }`) requires the condition to be a catalog/control-node call. For ordinary boolean checks, remove the trailing branch labels/comments and use plain `if (condition) { ... } else { ... }`, or use exact control-node declarations from get_declarations.";
    }

    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.contains("expected `Colon`, found `Assign`"))
    {
        return "FlowScript validation failed: object and call-argument fields use colon syntax, for example `{ host: \"imap.gmail.com\" }`, not assignment syntax like `{ host = \"imap.gmail.com\" }`.";
    }

    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.contains("`const` binding requires a call expression"))
    {
        return "FlowScript validation failed: inside a function/event block, `const name = ...` must bind a catalog/node call. Use `let` for local literal aliases or pass literals/objects directly into node calls.";
    }

    "FlowScript validation failed. Fix the listed issues and call edit_flowscript again."
}

fn flowscript_summary(flowscript: &str) -> Value {
    json!({
        "lines": if flowscript.is_empty() { 0 } else { flowscript.lines().count() },
        "chars": flowscript.chars().count(),
    })
}

fn create_internet_search_tool() -> (Tool, ToolHandler) {
    let tool = Tool::new("internet_search")
        .description(
            r#"Search the public web through Flow-Like's SearXNG instance at search.flow-like.com.

Use this when current public information, documentation, examples, or external references would
help build or debug the workflow. Prefer official docs and primary sources in your follow-up
reasoning. Returns compact title/url/snippet/date results; fetch pages through workflow nodes only
when the actual workflow needs fetching."#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query." },
                "language": { "type": "string", "description": "SearXNG language code, default en-US." },
                "page": { "type": "integer", "description": "1-based page number, default 1." },
                "limit": { "type": "integer", "description": "Maximum results to return, default 8, max 20." }
            },
            "required": ["query"]
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        ToolResultObject::text(
            serde_json::to_string_pretty(&run_internet_search_tool(args))
                .unwrap_or_else(|_| "{\"status\":\"error\"}".to_string()),
        )
    });

    (tool, handler)
}

static SEARCH_CLIENT: LazyLock<Result<reqwest::blocking::Client, reqwest::Error>> =
    LazyLock::new(|| {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("FlowPilot/1.0")
            .build()
    });

fn run_internet_search_tool(args: &Value) -> Value {
    let query = arg_string(args, "query", "query");
    if query.trim().is_empty() {
        return json!({
            "status": "error",
            "tool": "internet_search",
            "error": "internet_search requires a non-empty query."
        });
    }

    let language = arg_string(args, "language", "language");
    let language = if language.trim().is_empty() {
        "en-US".to_string()
    } else {
        language
    };
    let page = args
        .get("page")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .clamp(1, 100);
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(8)
        .clamp(1, 20) as usize;
    let url = format!(
        "https://search.flow-like.com/search?q={}&format=json&pageno={}&language={}",
        urlencoding::encode(&query),
        page,
        urlencoding::encode(&language)
    );

    let client = match SEARCH_CLIENT.as_ref() {
        Ok(client) => client,
        Err(error) => {
            return json!({
                "status": "error",
                "tool": "internet_search",
                "error": format!("Failed to create search client: {error}")
            });
        }
    };

    let response = match client.get(&url).send() {
        Ok(response) => response,
        Err(error) => {
            return json!({
                "status": "error",
                "tool": "internet_search",
                "query": query,
                "error": format!("Search request failed: {error}")
            });
        }
    };

    let status = response.status();
    if !status.is_success() {
        return json!({
            "status": "error",
            "tool": "internet_search",
            "query": query,
            "http_status": status.as_u16(),
            "error": format!("Search request failed with HTTP {status}")
        });
    }

    let payload = match response.json::<Value>() {
        Ok(payload) => payload,
        Err(error) => {
            return json!({
                "status": "error",
                "tool": "internet_search",
                "query": query,
                "error": format!("Search response was not valid JSON: {error}")
            });
        }
    };

    let results = payload
        .get("results")
        .and_then(Value::as_array)
        .map(|results| {
            results
                .iter()
                .take(limit)
                .map(compact_search_result)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    json!({
        "status": "ok",
        "query": query,
        "page": page,
        "results": results
    })
}

fn compact_search_result(result: &Value) -> Value {
    let object = result.as_object();
    json!({
        "title": object.and_then(|item| item.get("title")).cloned().unwrap_or(Value::Null),
        "url": object.and_then(|item| item.get("url")).cloned().unwrap_or(Value::Null),
        "content": object.and_then(|item| item.get("content")).cloned().unwrap_or(Value::Null),
        "publishedDate": object.and_then(|item| item.get("publishedDate")).cloned().unwrap_or(Value::Null),
        "engine": object.and_then(|item| item.get("engine")).cloned().unwrap_or(Value::Null),
        "category": object.and_then(|item| item.get("category")).cloned().unwrap_or(Value::Null),
        "score": object.and_then(|item| item.get("score")).cloned().unwrap_or(Value::Null),
    })
}

fn create_database_tool(bridge: FrontendToolBridge) -> (Tool, ToolHandler) {
    let tool = Tool::new("database_tool")
        .description(
            r#"Inspect or modify the app's built-in LanceDB/Open Database tables through the frontend backend state.

Use this to understand existing local/user databases before generating DataFusion, Lance, vector,
full-text, or hybrid search workflows.

Read operations do not ask for approval. Mutating operations show an approval dialog with a
"don't ask again this session" option.

Operations:
- list_tables: return project and user-scoped tables.
- describe_table: schema, indices, row count, and sample rows.
- query: SQL/filter/vector/FTS query via the existing database query API.
- insert/add_items, delete/remove_items, update.
- build_index, drop_index, optimize, add_column, drop_columns, alter_column."#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": [
                        "list_tables", "describe_table", "query",
                        "insert", "add_items", "delete", "remove_items", "update",
                        "build_index", "drop_index", "optimize",
                        "add_column", "drop_columns", "alter_column"
                    ]
                },
                "app_id": { "type": "string", "description": "App id. Optional when FlowPilot knows the current app." },
                "table_name": { "type": "string", "description": "Table name for table operations." },
                "user_scoped": { "type": "boolean", "description": "Use user-scoped storage/database tables." },
                "query": { "type": "object", "description": "Query payload: {sql, filter, fts_term, vector_query, rerank}." },
                "offset": { "type": "integer" },
                "limit": { "type": "integer" },
                "items": { "type": "array", "items": { "type": "object" } },
                "filter": { "type": "string", "description": "Delete/update filter expression." },
                "updates": { "type": "object" },
                "column": { "type": "string" },
                "columns": { "type": "array", "items": { "type": "string" } },
                "index_type": {
                    "type": "string",
                    "enum": ["FullText", "BTree", "Bitmap", "LabelList", "Auto", "full_text", "btree", "bitmap", "label_list", "auto"]
                },
                "index_name": { "type": "string" },
                "optimize": { "type": "boolean" },
                "keep_versions": { "type": "boolean" },
                "nullable": { "type": "boolean" },
                "column_definition": { "type": "object", "description": "For add_column: {name, sql_expression}." }
            },
            "required": ["operation"]
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let operation = arg_string(args, "operation", "operation");
        let approval = if database_operation_requires_approval(&operation) {
            let table_name = arg_string(args, "table_name", "tableName");
            FrontendToolApproval::mutating(
                "Approve database change",
                format!(
                    "FlowPilot wants to run database operation '{}'{}.",
                    operation,
                    if table_name.is_empty() {
                        String::new()
                    } else {
                        format!(" on table '{table_name}'")
                    }
                ),
                format!("database:{operation}"),
            )
        } else {
            FrontendToolApproval::none()
        };
        frontend_tool_result(&bridge, "database_tool", args.clone(), approval)
    });

    (tool, handler)
}

fn create_storage_tool(bridge: FrontendToolBridge) -> (Tool, ToolHandler) {
    let tool = Tool::new("storage_tool")
        .description(
            r#"List, read, create, or delete app storage files through the frontend storage state.

Read/list operations are silent. create_file and delete_files show an approval dialog with a
"don't ask again this session" option. Use this when a workflow needs to reference existing files
or create a small helper/config artifact in app/user storage."#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "operation": { "type": "string", "enum": ["list_files", "read_file", "create_file", "delete_files"] },
                "app_id": { "type": "string", "description": "App id. Optional when FlowPilot knows the current app." },
                "prefix": { "type": "string", "description": "Folder/prefix to list." },
                "path": { "type": "string", "description": "File path for read/create." },
                "paths": { "type": "array", "items": { "type": "string" }, "description": "File paths/prefixes for deletion." },
                "content": { "type": "string", "description": "Text content for create_file." },
                "mime_type": { "type": "string", "description": "Content type for create_file, default text/plain." },
                "user_scoped": { "type": "boolean", "description": "Use user storage instead of app storage." },
                "max_chars": { "type": "integer", "description": "Maximum characters to return for read_file." }
            },
            "required": ["operation"]
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let operation = arg_string(args, "operation", "operation");
        let approval = if matches!(operation.as_str(), "create_file" | "delete_files") {
            FrontendToolApproval::mutating(
                "Approve storage change",
                format!("FlowPilot wants to run storage operation '{operation}'."),
                format!("storage:{operation}"),
            )
        } else {
            FrontendToolApproval::none()
        };
        frontend_tool_result(&bridge, "storage_tool", args.clone(), approval)
    });

    (tool, handler)
}

fn create_ui_inspect_tool(bridge: FrontendToolBridge) -> (Tool, ToolHandler) {
    let tool = Tool::new("ui_inspect")
        .description(
            r#"Inspect the app's A2UI pages and widgets so `a2ui*` workflow calls target real elements.

This is a READ-ONLY tool and never asks for approval. Call it BEFORE writing or editing any
`a2ui*` call (set/get element, instantiate widget, push/clear container, navigate) so element
references and widget selectors are never guessed.

Operations:
- list (default): every page (id, name, route, onLoad event) and every widget (selector, description).
- page: full element reference list for one page. An `elementRef` used by `a2uiSetElementText`,
  `a2uiGetElement`, `a2uiGetElementValue`, `a2uiPushToContainer`, etc. is `"<page_id>/<component_id>"`.
- widget: instantiation surface for one widget — the `widgetSelector` plus the `dynPath*`/`dynProp*`
  (camelCase) input pins `a2uiInstantiateWidget` exposes for its bound data paths and exposed props,
  and the action names usable for `fnRefs`."#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "operation": { "type": "string", "enum": ["list", "page", "widget"] },
                "app_id": { "type": "string", "description": "App id. Optional when FlowPilot knows the current app." },
                "board_id": { "type": "string", "description": "Restrict pages to this board. Optional." },
                "page_id": { "type": "string", "description": "Page id for operation 'page'." },
                "widget_selector": { "type": "string", "description": "Widget id or name for operation 'widget'." }
            }
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        frontend_tool_result(
            &bridge,
            "ui_inspect",
            args.clone(),
            FrontendToolApproval::none(),
        )
    });

    (tool, handler)
}

fn create_execute_event_tool(bridge: FrontendToolBridge) -> (Tool, ToolHandler) {
    let tool = Tool::new("execute_event")
        .description(
            r#"Execute a workflow event through the frontend execution service and return bounded logs.

Use this after creating or updating an event-backed workflow to validate behavior with real runtime
logs. This is side-effecting and always asks for approval unless the user selected "don't ask again
this session"."#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "app_id": { "type": "string", "description": "App id. Optional when FlowPilot knows the current app." },
                "event_id": { "type": "string" },
                "payload": { "type": "object", "description": "Run payload. If omitted, {id:event_id,payload:{}} is used." },
                "stream_state": { "type": "boolean", "description": "Stream state/log events, default true." },
                "skip_consent_check": { "type": "boolean" }
            },
            "required": ["event_id"]
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let event_id = arg_string(args, "event_id", "eventId");
        frontend_tool_result_with_timeout(
            &bridge,
            "execute_event",
            args.clone(),
            FrontendToolApproval::execute(
                "Approve workflow execution",
                format!("FlowPilot wants to execute event '{event_id}' and inspect the logs."),
                "execute_event".to_string(),
            ),
            Duration::from_secs(600),
        )
    });

    (tool, handler)
}

fn create_ask_user_tool(bridge: FrontendToolBridge) -> (Tool, ToolHandler) {
    let tool = Tool::new("ask_user")
        .description(
            r#"Ask the user for one targeted input when placeholders/defaults are not enough.

Prefer defaults and placeholder variables. Use this only for genuinely blocking choices. Supports
freeform, single_choice, and multiple_choice modes. Include a recommended default whenever
possible."#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "question": { "type": "string" },
                "mode": { "type": "string", "enum": ["freeform", "single_choice", "multiple_choice"] },
                "choices": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": { "type": "string" },
                            "value": {},
                            "description": { "type": "string" }
                        },
                        "required": ["label"]
                    }
                },
                "default_value": { "description": "Recommended default value/choice." },
                "placeholder": { "type": "string" }
            },
            "required": ["question"]
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        frontend_tool_result_with_timeout(
            &bridge,
            "ask_user",
            args.clone(),
            FrontendToolApproval::none(),
            Duration::from_secs(600),
        )
    });

    (tool, handler)
}

/// Catalog search tool - find nodes by functionality.
fn create_catalog_search_tool(provider: Option<Arc<dyn CatalogProvider>>) -> (Tool, ToolHandler) {
    let tool = Tool::new("catalog_search")
        .description(
            r#"Search the node catalog by functionality or name. Returns matching nodes with their node_type for legacy/manual AddNode commands.

WHEN TO USE: Only for manual command JSON, layout/modeling operations, or debugging catalog metadata.
FOR WORKFLOW EDITS: Prefer get_declarations, write FlowScript, then call edit_flowscript. get_declarations is backed by embedded .flow.d files and returns exact camelCase function signatures.
EXAMPLE QUERIES: "http request", "parse json", "loop array", "condition if", "open database""#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language catalog search for manual AddNode use. For FlowScript workflows, use get_declarations instead."
                }
            },
            "required": ["query"]
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let provider = provider.clone();
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let results: Vec<NodeMetadata> = if let Some(provider) = provider {
            futures::executor::block_on(provider.search(&query))
        } else {
            Vec::new()
        };

        if results.is_empty() {
            ToolResultObject::text("No nodes found matching your query. Try different keywords.")
        } else {
            let lines: Vec<String> = results
                .iter()
                .map(|meta| {
                    let hints = search_result_hint_lines(meta);
                    if hints.is_empty() {
                        meta.to_compact()
                    } else {
                        format!("{} [{}]", meta.to_compact(), hints.join("; "))
                    }
                })
                .collect();
            ToolResultObject::text(lines.join("\n"))
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

const MAX_EMIT_COMMANDS: usize = 20;

#[derive(Clone, Default)]
struct KnownPins {
    inputs: HashSet<String>,
    outputs: HashSet<String>,
    is_layer: bool,
}

fn create_validate_commands_tool(graph_context: Option<Arc<GraphContext>>) -> (Tool, ToolHandler) {
    let tool = Tool::new("validate_commands")
        .description(
            r#"Validate a planned workflow command batch without queueing it for the user.

Use this immediately before emit_commands when building or modifying workflows. If validation
returns errors, fix the batch and validate again before calling emit_commands."#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "commands": {
                    "type": "array",
                    "maxItems": MAX_EMIT_COMMANDS,
                    "description": "Array of workflow commands in the same format used by emit_commands.",
                    "items": { "type": "object" }
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
            .unwrap_or("Command validation");

        let parsed_commands: Vec<BoardCommand> = match serde_json::from_value(commands.clone()) {
            Ok(cmds) => cmds,
            Err(e) => {
                let result = json!({
                    "status": "validation_errors",
                    "errors": [format!("Error parsing commands: {}", e)],
                    "commands": commands,
                    "explanation": explanation
                });
                return ToolResultObject::text(
                    serde_json::to_string_pretty(&result).unwrap_or_default(),
                );
            }
        };

        let validation_errors =
            validate_sdk_emit_commands(&parsed_commands, graph_context.as_deref());
        let result = if validation_errors.is_empty() {
            json!({
                "status": "valid",
                "commands": commands,
                "explanation": explanation,
                "message": "Command batch is valid. Call emit_commands with the same batch to queue it for review."
            })
        } else {
            json!({
                "status": "validation_errors",
                "errors": validation_errors,
                "commands": commands,
                "explanation": explanation,
                "message": "Command batch is invalid. Fix the listed errors and call validate_commands again."
            })
        };

        ToolResultObject::text(serde_json::to_string_pretty(&result).unwrap_or_default())
    });

    (tool, handler)
}

/// Emit commands tool - execute graph modifications
fn create_emit_commands_tool(graph_context: Option<Arc<GraphContext>>) -> (Tool, ToolHandler) {
    let tool = Tool::new("emit_commands")
        .description(
            r#"Execute graph modifications. Commands are batched and applied atomically.

Prefer calling validate_commands first for non-trivial batches. emit_commands also validates
and will not queue invalid commands.

CRITICAL ORDER:
1. AddNode commands FIRST (create nodes)
2. UpdateNodePin commands NEXT (set literals/configuration that may create dynamic pins)
3. ConnectPins commands LAST (wire execution + data after all pins exist)

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
    {"command_type": "UpdateNodePin", "node_id": "$0", "pin_id": "url", "value": "https://api.example.com", "summary": "Set URL"},
    {"command_type": "UpdateNodePin", "node_id": "$0", "pin_id": "method", "value": "GET", "summary": "Set method"},
    {"command_type": "ConnectPins", "from_node": "$0", "from_pin": "exec_out", "to_node": "$1", "to_pin": "exec_in", "summary": "Execution flow"},
    {"command_type": "ConnectPins", "from_node": "$0", "from_pin": "response_body", "to_node": "$1", "to_pin": "json_string", "summary": "Pass body"}
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

        let validation_errors =
            validate_sdk_emit_commands(&parsed_commands, graph_context.as_deref());
        if !validation_errors.is_empty() {
            let result = json!({
                "status": "validation_errors",
                "errors": validation_errors,
                "commands": commands,
                "explanation": explanation,
                "message": format!(
                    "Validation failed. Fix these issues and call emit_commands again:\n- {}",
                    validation_errors.join("\n- ")
                )
            });
            return ToolResultObject::text(
                serde_json::to_string_pretty(&result).unwrap_or_default(),
            );
        }

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

/// get_declarations tool - look up FlowScript `.flow.d` signatures by intent.
fn create_get_declarations_tool(provider: Arc<dyn CatalogProvider>) -> (Tool, ToolHandler) {
    let tool = Tool::new("get_declarations")
        .description(
            r#"Look up FlowScript node declarations (.flow.d) by intent.

Returns a compact ranked list of exact `declare function <camelCaseNodeType>({ pin: type, ... })`
signatures for nodes matching your focused query, plus an `// impure` marker for side-effecting /
control-flow nodes. Empty queries intentionally return guidance only, not the full catalog.

Use this BEFORE writing FlowScript so you call nodes by their exact camelCase name with correctly
typed arguments. Covers every package in the project's catalog."#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Focused declaration search. Do not leave blank. Good examples: 'gmail imap fetch mail', 'smtp send email', 'open local database batch insert', 'datafusion sql register lance', 'hybrid vector search build index'."
                }
            },
            "required": ["query"]
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let provider = provider.clone();
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let declarations = futures::executor::block_on(provider.get_declarations(&query));
        ToolResultObject::text(declarations)
    });

    (tool, handler)
}

/// edit_flowscript tool - apply an edited FlowScript document to the board via reconcile.
///
/// Always validates first: parse errors and reconcile diagnostics are reported back to the agent
/// and NOTHING is queued. Only a clean parse that yields commands queues them (status "queued"),
/// where the main chat loop turns them into a reviewable `<commands>` envelope.
fn create_get_current_flowscript_tool(board: Arc<Board>) -> (Tool, ToolHandler) {
    let tool = Tool::new("get_current_flowscript")
        .description(
            r#"Return the current live board as anchored FlowScript.

Use this before editing an existing board, especially after prior tool calls or validation errors.
The returned document is the source you must edit and submit in full to `edit_flowscript`; preserve
all `//@n:<id>` anchors on statements you keep."#,
        )
        .schema(json!({
            "type": "object",
            "properties": {}
        }));

    let handler: ToolHandler = Arc::new(move |_name, _args| {
        let flowscript = board_to_flowscript(
            &board,
            &RenderOptions {
                anchors: true,
                ..Default::default()
            },
        );
        let payload = json!({
            "status": "ok",
            "flowscript": flowscript,
            "message": "Edit this exact FlowScript document and submit the full edited source to edit_flowscript."
        });
        ToolResultObject::text(serde_json::to_string_pretty(&payload).unwrap_or_default())
    });

    (tool, handler)
}

fn create_edit_flowscript_tool(
    board: Arc<Board>,
    provider: Option<Arc<dyn CatalogProvider>>,
    side_effect_commands: Option<Arc<Mutex<Vec<BoardCommand>>>>,
) -> (Tool, ToolHandler) {
    let tool = Tool::new("edit_flowscript")
        .description(
            r#"Apply an edited FlowScript document to the board (PRIMARY way to modify a workflow).

For existing-board edits, call `get_current_flowscript` first, edit that exact returned document,
and submit the FULL edited FlowScript source. Reconcile compares it to the live board using the
`//@n:<id>` anchor comments and catalog declarations, then produces minimal changes:
- A changed literal argument on an anchored call → updates that node's pin value.
- An anchored statement you removed → deletes that node only when `allow_deletions` is true.
- A new unanchored FlowScript call → adds that node, configures literal args, and connects
  resolvable FlowScript references/nested calls.
- A new unanchored `function name(...) { ... }` declaration → creates a Function layer, places
  body nodes inside it, creates boundary pins from params/returns, and wires `return` values.

VALIDATION: This tool validates before queueing. If it reports parse errors or diagnostics,
nothing was queued — fix the FlowScript and resubmit. Only a clean parse queues commands.

RULES:
- PRESERVE every `//@n:<id>` anchor comment on statements you keep, exactly as given.
- Leave `allow_deletions` false unless the user explicitly asked to delete existing board items.
- Do NOT invent anchors for brand-new nodes; write normal unanchored calls using declarations
  from `get_declarations`.
- If you use `variableGet({ varRef: "NAME" })` or any `varRef`, `NAME` must resolve to an
  existing variable or a top-level FlowScript variable declaration such as
  `const NAME: string = ""`; missing varRefs are validation errors.
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
- Prefer writing impure/sequential logic INLINE in the event or loop body over extracting it into a
  helper `function`. A called function's body does not yet receive an execution entry from its call
  site, so impure nodes inside a helper (for example `cuid`, `structSet`, `arrayPushRef`) are created
  but left with no incoming execution connection and never run. For per-iteration work inside
  `controlForEach`, write the statements directly in the loop body instead of calling a `buildRow`-style
  helper.
- Charts (`a2uiPushCsvToChart`) read their data from a `format`-specific pin. With `format: "CSV"`, wire
  a DataFusion query's `table` output into the chart's `table` input (both are the same tabular struct)
  and set `chartType` (for example "Bar" / "Line" / "Pie"). The `data` input is ONLY for
  `format: "JSON"`. Wiring a `table` output into `data` with `format: "CSV"` leaves the chart's data
  unset and fails at run time.
- Read a struct field with `structGet({ struct: <structValue>, field: "name" }).value` (its `value`
  output is the field). To target an a2ui element, either pass the element id path string directly to a
  setter's `elementRef`, or fetch a handle with `a2uiGetElement({ elementRef: "surfaceId/element-id" }).element`;
  both are accepted.
- To reposition nodes on the canvas, use `emit_commands` with MoveNode."#,
        )
        .schema(json!({
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
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let flowscript = args
            .get("flowscript")
            .or_else(|| args.get("script"))
            .or_else(|| args.get("source"))
            .or_else(|| args.get("content"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let allow_deletions = args
            .get("allow_deletions")
            .or_else(|| args.get("allowDeletions"))
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if flowscript.trim().is_empty() {
            let payload = json!({
                "status": "validation_errors",
                "errors": ["edit_flowscript requires a non-empty `flowscript` string. The submitted tool arguments did not contain usable FlowScript."],
                "flowscript_workspace_summary": flowscript_summary(flowscript),
                "message": "FlowScript validation failed. Call edit_flowscript again with the edited FlowScript in `flowscript`."
            });
            return ToolResultObject::text(
                serde_json::to_string_pretty(&payload).unwrap_or_default(),
            );
        }

        let catalog = provider
            .clone()
            .map(|provider| futures::executor::block_on(provider.get_all_metadata()))
            .unwrap_or_default();

        let result = reconcile_text_with_catalog(&board, flowscript, &catalog);
        let has_parse_error = result
            .diagnostics
            .iter()
            .any(|d| d.to_lowercase().contains("parse error"));

        // Parse failure (or no derivable change with diagnostics) → report back, queue nothing.
        if has_parse_error || (result.commands.is_empty() && !result.diagnostics.is_empty()) {
            let message = flowscript_validation_message(&result.diagnostics);
            let payload = json!({
                "status": "validation_errors",
                "errors": result.diagnostics,
                "flowscript_workspace_summary": flowscript_summary(flowscript),
                "message": message
            });
            return ToolResultObject::text(
                serde_json::to_string_pretty(&payload).unwrap_or_default(),
            );
        }

        // Clean parse but no changes derived → nothing to do.
        if result.commands.is_empty() {
            let payload = json!({
                "status": "no_changes",
                "flowscript_workspace_summary": flowscript_summary(flowscript),
                "message": "No board changes were derived from the FlowScript. If this was meant to create nodes, use get_declarations for exact function names and submit concrete catalog calls inside a function/event block."
            });
            return ToolResultObject::text(
                serde_json::to_string_pretty(&payload).unwrap_or_default(),
            );
        }

        if !allow_deletions {
            let destructive = destructive_flowscript_command_summaries(&result.commands);
            if !destructive.is_empty() {
                let message = blocked_destructive_flowscript_message(&destructive);
                let payload = json!({
                    "status": "validation_errors",
                    "errors": [message],
                    "diagnostics": result.diagnostics,
                    "flowscript_workspace_summary": flowscript_summary(flowscript),
                    "message": "FlowScript validation failed. Deletions require an explicit allow_deletions=true opt-in."
                });
                return ToolResultObject::text(
                    serde_json::to_string_pretty(&payload).unwrap_or_default(),
                );
            }
        }

        // Clean parse with derived commands → queue them for review.
        let commands_value = serde_json::to_value(&result.commands).unwrap_or(json!([]));
        if let Some(store) = &side_effect_commands
            && let Ok(mut commands) = store.lock()
        {
            commands.extend(result.commands.clone());
        }
        let payload = json!({
            "status": "queued",
            "commands": commands_value,
            "explanation": format!("Reconciled {} change(s) from edited FlowScript.", result.commands.len()),
            "diagnostics": result.diagnostics,
            "flowscript_workspace_summary": flowscript_summary(flowscript),
        });
        ToolResultObject::text(serde_json::to_string_pretty(&payload).unwrap_or_default())
    });

    (tool, handler)
}

fn validate_sdk_emit_commands(
    commands: &[BoardCommand],
    graph_context: Option<&GraphContext>,
) -> Vec<String> {
    let mut errors = Vec::new();
    if commands.is_empty() {
        errors.push("emit_commands requires at least one command".to_string());
        return errors;
    }
    if commands.len() > MAX_EMIT_COMMANDS {
        errors.push(format!(
            "emit_commands is limited to {MAX_EMIT_COMMANDS} commands per turn"
        ));
    }

    let mut known_entities: HashMap<String, KnownPins> = HashMap::new();
    let mut known_layers: HashSet<String> = HashSet::new();
    let mut known_variables: HashSet<String> = HashSet::new();
    let mut proposed_connections: HashSet<(String, String, String, String)> = HashSet::new();
    if let Some(ctx) = graph_context {
        for node in &ctx.nodes {
            known_entities.insert(
                node.id.clone(),
                KnownPins {
                    inputs: node.inputs.iter().map(|pin| pin.name.clone()).collect(),
                    outputs: node.outputs.iter().map(|pin| pin.name.clone()).collect(),
                    is_layer: false,
                },
            );
        }
        for layer in &ctx.layers {
            known_layers.insert(layer.id.clone());
            known_entities.insert(
                layer.id.clone(),
                KnownPins {
                    inputs: layer.inputs.iter().map(|pin| pin.name.clone()).collect(),
                    outputs: layer.outputs.iter().map(|pin| pin.name.clone()).collect(),
                    is_layer: true,
                },
            );
        }
        known_variables.extend(ctx.variables.iter().map(|variable| variable.id.clone()));
        proposed_connections.extend(ctx.edges.iter().map(|edge| {
            (
                edge.from_node_id.clone(),
                edge.from_pin_name.clone(),
                edge.to_node_id.clone(),
                edge.to_pin_name.clone(),
            )
        }));
    }

    let catalog = get_catalog();
    let mut catalog_nodes = HashMap::new();
    for logic in &catalog {
        let node = logic.get_node();
        catalog_nodes.insert(node.name.clone(), node);
    }

    for (index, command) in commands.iter().enumerate() {
        match command {
            BoardCommand::AddNode {
                node_type,
                ref_id,
                position,
                target_layer,
                ..
            } => {
                if position.is_none() {
                    errors.push(format!("Command {index}: AddNode requires a position"));
                }
                if ref_id.as_deref().unwrap_or_default().trim().is_empty() {
                    errors.push(format!(
                        "Command {index}: AddNode requires a ref_id like '$0'"
                    ));
                }
                if let Some(layer_id) = target_layer
                    && !known_layers.contains(layer_id)
                {
                    errors.push(format!(
                        "Command {index}: target_layer '{layer_id}' is unknown"
                    ));
                }
                let Some(node) = catalog_nodes.get(node_type) else {
                    errors.push(format!(
                        "Command {index}: node type '{node_type}' was not found in the catalog"
                    ));
                    continue;
                };
                if let Some(ref_id) = ref_id {
                    if known_entities.contains_key(ref_id) {
                        errors.push(format!(
                            "Command {index}: ref_id '{ref_id}' is already in use"
                        ));
                    } else {
                        known_entities.insert(
                            ref_id.clone(),
                            KnownPins {
                                inputs: node
                                    .pins
                                    .values()
                                    .filter(|pin| pin.pin_type == PinType::Input)
                                    .map(|pin| pin.name.clone())
                                    .collect(),
                                outputs: node
                                    .pins
                                    .values()
                                    .filter(|pin| pin.pin_type == PinType::Output)
                                    .map(|pin| pin.name.clone())
                                    .collect(),
                                is_layer: false,
                            },
                        );
                    }
                }
            }
            BoardCommand::AddPlaceholder {
                ref_id,
                pins,
                position,
                target_layer,
                ..
            } => {
                if position.is_none() {
                    errors.push(format!(
                        "Command {index}: AddPlaceholder requires a position"
                    ));
                }
                if ref_id.as_deref().unwrap_or_default().trim().is_empty() {
                    errors.push(format!(
                        "Command {index}: AddPlaceholder requires a ref_id like '$0'"
                    ));
                }
                if let Some(layer_id) = target_layer
                    && !known_layers.contains(layer_id)
                {
                    errors.push(format!(
                        "Command {index}: target_layer '{layer_id}' is unknown"
                    ));
                }
                if let Some(ref_id) = ref_id {
                    if known_entities.contains_key(ref_id) {
                        errors.push(format!(
                            "Command {index}: ref_id '{ref_id}' is already in use"
                        ));
                    } else {
                        let mut entity = KnownPins {
                            inputs: HashSet::from(["exec_in".to_string()]),
                            outputs: HashSet::from(["exec_out".to_string()]),
                            is_layer: true,
                        };
                        let mut seen_pins = HashSet::new();
                        for pin in pins.as_deref().unwrap_or(&[]) {
                            if pin.name.trim().is_empty() {
                                errors.push(format!(
                                    "Command {index}: placeholder pin names cannot be empty"
                                ));
                                continue;
                            }
                            if !seen_pins.insert(pin.name.clone()) {
                                errors.push(format!(
                                    "Command {index}: duplicate placeholder pin '{}'",
                                    pin.name
                                ));
                            }
                            if pin.pin_type.eq_ignore_ascii_case("input") {
                                entity.inputs.insert(pin.name.clone());
                            } else if pin.pin_type.eq_ignore_ascii_case("output") {
                                entity.outputs.insert(pin.name.clone());
                            } else {
                                errors.push(format!(
                                    "Command {index}: placeholder pin '{}' has invalid pin_type '{}'",
                                    pin.name, pin.pin_type
                                ));
                            }
                        }
                        known_entities.insert(ref_id.clone(), entity);
                        known_layers.insert(ref_id.clone());
                    }
                }
            }
            BoardCommand::ConnectPins {
                from_node,
                from_pin,
                to_node,
                to_pin,
                ..
            } => {
                if from_node == to_node {
                    errors.push(format!(
                        "Command {index}: cannot connect node '{from_node}' to itself"
                    ));
                }
                match known_entities.get(from_node) {
                    Some(entity)
                        if entity.outputs.contains(from_pin)
                            || (entity.is_layer && entity.inputs.contains(from_pin)) => {}
                    Some(entity) => errors.push(pin_lookup_error(
                        index,
                        from_node,
                        from_pin,
                        &entity.outputs,
                        "source output",
                    )),
                    None => errors.push(format!(
                        "Command {index}: source node '{from_node}' is unknown"
                    )),
                }
                match known_entities.get(to_node) {
                    Some(entity)
                        if entity.inputs.contains(to_pin)
                            || (entity.is_layer && entity.outputs.contains(to_pin)) => {}
                    Some(entity) => errors.push(pin_lookup_error(
                        index,
                        to_node,
                        to_pin,
                        &entity.inputs,
                        "target input",
                    )),
                    None => errors.push(format!(
                        "Command {index}: target node '{to_node}' is unknown"
                    )),
                }
                let connection_key = (
                    from_node.clone(),
                    from_pin.clone(),
                    to_node.clone(),
                    to_pin.clone(),
                );
                if !proposed_connections.insert(connection_key) {
                    errors.push(format!(
                        "Command {index}: duplicate connection '{from_node}.{from_pin}' -> '{to_node}.{to_pin}'"
                    ));
                }
            }
            BoardCommand::DisconnectPins {
                from_node, to_node, ..
            } => {
                if !known_entities.contains_key(from_node) {
                    errors.push(format!("Command {index}: node '{from_node}' is unknown"));
                }
                if !known_entities.contains_key(to_node) {
                    errors.push(format!("Command {index}: node '{to_node}' is unknown"));
                }
            }
            BoardCommand::MoveNode {
                node_id,
                target_layer,
                ..
            } => {
                if !known_entities.contains_key(node_id) {
                    errors.push(format!("Command {index}: node '{node_id}' is unknown"));
                }
                if let Some(layer_id) = target_layer
                    && !known_layers.contains(layer_id)
                {
                    errors.push(format!(
                        "Command {index}: target_layer '{layer_id}' is unknown"
                    ));
                }
            }
            BoardCommand::UpdateNodePin {
                node_id, pin_id, ..
            } => match known_entities.get(node_id) {
                Some(entity) if entity.inputs.contains(pin_id) => {}
                Some(entity) => errors.push(pin_lookup_error(
                    index,
                    node_id,
                    pin_id,
                    &entity.inputs,
                    "input",
                )),
                None => errors.push(format!("Command {index}: node '{node_id}' is unknown")),
            },
            BoardCommand::RemoveNode { node_id, .. } => {
                if !known_entities.contains_key(node_id) {
                    errors.push(format!("Command {index}: node '{node_id}' is unknown"));
                }
            }
            BoardCommand::CreateLayer {
                name,
                ref_id,
                pins,
                node_ids,
                position,
                target_layer,
                ..
            } => {
                if node_ids.is_empty() && position.is_none() {
                    errors.push(format!(
                        "Command {index}: CreateLayer needs node_ids or a position"
                    ));
                }
                for node_id in node_ids {
                    if !known_entities.contains_key(node_id) {
                        errors.push(format!(
                            "Command {index}: layer references unknown node '{node_id}'"
                        ));
                    }
                }
                if let Some(layer_id) = target_layer
                    && !known_layers.contains(layer_id)
                {
                    errors.push(format!(
                        "Command {index}: target_layer '{layer_id}' is unknown"
                    ));
                }
                if let Some(pins) = pins {
                    let mut seen_pins = HashSet::new();
                    for pin in pins {
                        if pin.name.trim().is_empty() {
                            errors
                                .push(format!("Command {index}: layer pin names cannot be empty"));
                            continue;
                        }
                        if !seen_pins.insert(pin.name.clone()) {
                            errors.push(format!(
                                "Command {index}: duplicate layer pin '{}'",
                                pin.name
                            ));
                        }
                        if !matches!(pin.pin_type.as_str(), "Input" | "Output") {
                            errors.push(format!(
                                "Command {index}: layer pin '{}' has invalid pin_type '{}'",
                                pin.name, pin.pin_type
                            ));
                        }
                    }
                }
                let key = ref_id
                    .clone()
                    .unwrap_or_else(|| format!("__new_layer_{index}"));
                if !known_entities.contains_key(&key) {
                    known_entities.insert(
                        key.clone(),
                        KnownPins {
                            inputs: pins
                                .as_ref()
                                .map(|pins| {
                                    pins.iter()
                                        .filter(|pin| pin.pin_type == "Input")
                                        .map(|pin| pin.name.clone())
                                        .collect()
                                })
                                .unwrap_or_default(),
                            outputs: pins
                                .as_ref()
                                .map(|pins| {
                                    pins.iter()
                                        .filter(|pin| pin.pin_type == "Output")
                                        .map(|pin| pin.name.clone())
                                        .collect()
                                })
                                .unwrap_or_default(),
                            is_layer: true,
                        },
                    );
                    known_layers.insert(key);
                    known_layers.insert(name.clone());
                }
            }
            BoardCommand::RemoveLayer { layer_id, .. } => {
                if !known_layers.contains(layer_id) {
                    errors.push(format!("Command {index}: layer '{layer_id}' is unknown"));
                }
            }
            BoardCommand::CreateVariable {
                name,
                data_type,
                value_type,
                ..
            } => {
                if name.trim().is_empty()
                    || data_type.trim().is_empty()
                    || value_type.trim().is_empty()
                {
                    errors.push(format!(
                        "Command {index}: CreateVariable requires name, data_type, and value_type"
                    ));
                }
            }
            BoardCommand::UpdateVariable {
                variable_id,
                name,
                data_type,
                value_type,
                ..
            } => {
                if graph_context.is_some() && !known_variables.contains(variable_id) {
                    errors.push(format!(
                        "Command {index}: variable '{variable_id}' is unknown"
                    ));
                }
                if name.as_deref().is_some_and(|name| name.trim().is_empty()) {
                    errors.push(format!(
                        "Command {index}: UpdateVariable cannot set an empty name"
                    ));
                }
                if data_type
                    .as_deref()
                    .is_some_and(|data_type| data_type.trim().is_empty())
                    || value_type
                        .as_deref()
                        .is_some_and(|value_type| value_type.trim().is_empty())
                {
                    errors.push(format!(
                        "Command {index}: UpdateVariable cannot set an empty data_type or value_type"
                    ));
                }
            }
            BoardCommand::AddComment {
                content,
                target_layer,
                ..
            } => {
                if content.trim().is_empty() {
                    errors.push(format!(
                        "Command {index}: CreateComment requires non-empty content"
                    ));
                }
                if let Some(layer_id) = target_layer
                    && !known_layers.contains(layer_id)
                {
                    errors.push(format!(
                        "Command {index}: target_layer '{layer_id}' is unknown"
                    ));
                }
            }
            BoardCommand::RemoveVariable { variable_id, .. } => {
                if graph_context.is_some() && !known_variables.contains(variable_id) {
                    errors.push(format!(
                        "Command {index}: variable '{variable_id}' is unknown"
                    ));
                }
            }
            BoardCommand::RemoveComment { .. } => {}
        }
    }

    errors
}

fn pin_lookup_error(
    command_index: usize,
    node_id: &str,
    requested_pin: &str,
    available_pins: &HashSet<String>,
    role: &str,
) -> String {
    if let Some(exact_pin) = available_pins
        .iter()
        .find(|pin| pin.eq_ignore_ascii_case(requested_pin))
    {
        return format!(
            "Command {command_index}: {role} pin '{node_id}.{requested_pin}' is not exact. Pin names are case-sensitive; use '{exact_pin}'"
        );
    }

    let mut available: Vec<_> = available_pins.iter().map(String::as_str).collect();
    available.sort_unstable();
    if available.is_empty() {
        format!("Command {command_index}: {role} pin '{node_id}.{requested_pin}' is unknown")
    } else {
        format!(
            "Command {command_index}: {role} pin '{node_id}.{requested_pin}' is unknown. Available pins: {}",
            available.join(", ")
        )
    }
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
        create_get_component_schema_tool(),
        create_validate_ui_tool(),
        create_emit_ui_tool(),
    ]
}

fn emit_ui_schema() -> Value {
    json!({
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
    })
}

fn create_validate_ui_tool() -> (Tool, ToolHandler) {
    let tool = Tool::new("validate_ui")
        .description(
            r#"Validate an A2UI component tree without rendering it.

Use this immediately before emit_ui for non-trivial interfaces. If validation returns errors,
repair the full component tree and validate again before calling emit_ui."#,
        )
        .schema(emit_ui_schema());

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let root_id = args
            .get("rootComponentId")
            .and_then(|v| v.as_str())
            .unwrap_or("root");
        let canvas = args.get("canvasSettings").cloned().unwrap_or(json!({}));
        let components = args.get("components").cloned().unwrap_or(json!([]));
        let (validated_components, validation_errors) =
            validate_ui_components(root_id, &canvas, &components);

        let result = if validation_errors.is_empty() {
            json!({
                "status": "valid",
                "rootComponentId": root_id,
                "canvasSettings": canvas,
                "components": validated_components,
                "message": "UI tree is valid. Call emit_ui with the same tree to render it."
            })
        } else {
            json!({
                "status": "validation_errors",
                "errors": validation_errors,
                "rootComponentId": root_id,
                "canvasSettings": canvas,
                "components": validated_components,
                "message": "UI tree is invalid. Fix the listed errors and call validate_ui again."
            })
        };

        ToolResultObject::text(serde_json::to_string(&result).unwrap_or_default())
    });

    (tool, handler)
}

/// Emit UI tool - output A2UI JSON components
fn create_emit_ui_tool() -> (Tool, ToolHandler) {
    let tool = Tool::new("emit_ui")
        .description(
            r#"Output A2UI components to render in the interface. This is NOT file editing - it generates JSON that renders directly in the app.

Prefer calling validate_ui first for non-trivial component trees. emit_ui also validates
and will not render invalid component trees.

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
Display: text, image, icon, badge, avatar, progress, spinner, divider, markdown, diffView
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
        .schema(emit_ui_schema());

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let root_id = args
            .get("rootComponentId")
            .and_then(|v| v.as_str())
            .unwrap_or("root");
        let canvas = args.get("canvasSettings").cloned().unwrap_or(json!({}));
        let components = args.get("components").cloned().unwrap_or(json!([]));

        // Validate components and collect errors
        let (validated_components, validation_errors) =
            validate_ui_components(root_id, &canvas, &components);

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
            return ToolResultObject::text(
                "Please provide at least one component type to look up.",
            );
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
        "text" => Some(&[
            "content", "variant", "size", "weight", "color", "align", "truncate", "maxLines",
        ]),
        "image" => Some(&[
            "src",
            "alt",
            "fit",
            "fallback",
            "fallbackSrc",
            "loading",
            "aspectRatio",
            "width",
            "height",
        ]),
        "icon" => Some(&["name", "size", "color", "strokeWidth"]),
        "video" => Some(&[
            "src", "poster", "autoplay", "autoPlay", "loop", "muted", "controls", "width", "height",
        ]),
        "lottie" => Some(&["src", "autoplay", "loop", "speed", "width", "height"]),
        "markdown" => Some(&["content", "allowHtml"]),
        "diffView" => Some(&[
            "original",
            "modified",
            "mode",
            "kind",
            "language",
            "markdownMode",
            "showLineNumbers",
            "wordWrap",
            "wordLevel",
            "collapseUnchanged",
            "contextLines",
            "showStats",
            "originalLabel",
            "modifiedLabel",
            "ignoreWhitespace",
            "ignoreCase",
            "trimTrailingWhitespace",
            "swapSides",
        ]),
        "divider" => Some(&["orientation", "thickness", "color"]),
        "badge" => Some(&["content", "text", "variant", "color"]),
        "avatar" => Some(&["src", "fallback", "size"]),
        "progress" => Some(&["value", "max", "showLabel", "variant", "color"]),
        "spinner" => Some(&["size", "color"]),
        "skeleton" => Some(&["width", "height", "rounded", "variant"]),
        "iframe" => Some(&[
            "src",
            "srcdoc",
            "width",
            "height",
            "sandbox",
            "allow",
            "title",
            "referrerPolicy",
            "border",
            "loading",
        ]),
        "table" => Some(&[
            "columns",
            "data",
            "caption",
            "striped",
            "bordered",
            "hoverable",
            "compact",
            "stickyHeader",
            "sortable",
            "searchable",
            "paginated",
            "pageSize",
            "selectable",
            "showPagination",
        ]),
        "plotlyChart" => Some(&[
            "chartType",
            "data",
            "title",
            "layout",
            "config",
            "height",
            "width",
        ]),
        "nivoChart" => Some(&[
            "chartType",
            "data",
            "height",
            "width",
            "colors",
            "colorScheme",
            "showLegend",
            "legendPosition",
            "margin",
            "axisBottom",
            "axisLeft",
            "animate",
            "motionConfig",
            "style",
        ]),
        "filePreview" => Some(&[
            "src",
            "url",
            "filename",
            "mimeType",
            "fileType",
            "width",
            "height",
            "fit",
            "showControls",
            "fallbackText",
        ]),
        "boundingBoxOverlay" => Some(&[
            "src",
            "boxes",
            "showLabels",
            "showConfidence",
            "normalized",
            "width",
            "height",
        ]),
        "button" => Some(&[
            "label",
            "variant",
            "size",
            "disabled",
            "loading",
            "icon",
            "iconPosition",
            "tooltip",
        ]),
        "textField" => Some(&[
            "value",
            "placeholder",
            "label",
            "helperText",
            "error",
            "disabled",
            "inputType",
            "type",
            "multiline",
            "rows",
            "maxLength",
            "required",
        ]),
        "select" => Some(&[
            "value",
            "options",
            "placeholder",
            "label",
            "disabled",
            "multiple",
            "searchable",
        ]),
        "slider" => Some(&[
            "value",
            "min",
            "max",
            "step",
            "disabled",
            "showValue",
            "label",
        ]),
        "checkbox" => Some(&["checked", "label", "disabled", "indeterminate"]),
        "switch" => Some(&["checked", "label", "disabled"]),
        "radioGroup" => Some(&["value", "options", "disabled", "orientation", "label"]),
        "dateTimeInput" => Some(&["value", "mode", "min", "max", "disabled", "label"]),
        "fileInput" => Some(&[
            "value",
            "label",
            "helperText",
            "accept",
            "multiple",
            "maxSize",
            "maxFiles",
            "disabled",
            "error",
        ]),
        "imageInput" => Some(&[
            "value",
            "label",
            "helperText",
            "accept",
            "multiple",
            "maxSize",
            "maxFiles",
            "disabled",
            "error",
            "aspectRatio",
            "showPreview",
        ]),
        "imageLabeler" => Some(&["src", "labels", "boxes", "disabled", "width", "height"]),
        "imageHotspot" => Some(&["src", "hotspots", "markerStyle", "width", "height"]),
        "link" => Some(&[
            "href",
            "label",
            "text",
            "route",
            "queryParams",
            "external",
            "target",
            "variant",
            "underline",
            "disabled",
            "openInNewTab",
        ]),
        "card" => Some(&[
            "title",
            "description",
            "footer",
            "hoverable",
            "clickable",
            "variant",
            "padding",
            "headerImage",
            "headerIcon",
        ]),
        "modal" => Some(&[
            "open",
            "title",
            "description",
            "closeOnOverlay",
            "closeOnEscape",
            "showCloseButton",
            "size",
            "centered",
        ]),
        "tabs" => Some(&["value", "tabs", "orientation", "variant", "defaultValue"]),
        "accordion" => Some(&[
            "items",
            "multiple",
            "defaultExpanded",
            "collapsible",
            "type",
        ]),
        "drawer" => Some(&[
            "open",
            "side",
            "title",
            "size",
            "overlay",
            "closable",
            "description",
        ]),
        "tooltip" => Some(&["content", "side", "delayMs", "maxWidth"]),
        "popover" => Some(&[
            "open",
            "contentComponentId",
            "side",
            "trigger",
            "closeOnClickOutside",
            "content",
        ]),
        "canvas2d" => Some(&["width", "height", "backgroundColor", "pixelPerfect"]),
        "sprite" => Some(&[
            "src", "x", "y", "width", "height", "rotation", "scale", "opacity", "flipX", "flipY",
            "zIndex",
        ]),
        "shape" => Some(&[
            "shapeType",
            "x",
            "y",
            "width",
            "height",
            "radius",
            "points",
            "fill",
            "stroke",
            "strokeWidth",
        ]),
        "scene3d" => Some(&[
            "width",
            "height",
            "cameraType",
            "cameraPosition",
            "backgroundColor",
            "controlMode",
            "fixedView",
            "autoRotateSpeed",
            "enableControls",
            "enableZoom",
            "enablePan",
            "fov",
            "near",
            "far",
            "target",
            "ambientLight",
            "directionalLight",
            "showGrid",
            "showAxes",
        ]),
        "model3d" => Some(&[
            "src",
            "position",
            "rotation",
            "scale",
            "castShadow",
            "receiveShadow",
            "animation",
            "autoRotate",
            "rotateSpeed",
            "viewerHeight",
            "backgroundColor",
            "cameraDistance",
            "fov",
            "cameraAngle",
            "cameraPosition",
            "cameraTarget",
            "enableControls",
            "enableZoom",
            "enablePan",
            "autoRotateCamera",
            "cameraRotateSpeed",
            "ambientLight",
            "directionalLight",
            "fillLight",
            "rimLight",
            "lightColor",
            "lightingPreset",
            "showGround",
            "groundColor",
            "enableReflections",
            "environment",
            "environmentSource",
            "useHdrBackground",
            "polyhavenHdri",
            "polyhavenResolution",
        ]),
        "dialogue" => Some(&["text", "speakerName", "typewriter", "speed", "portrait"]),
        "characterPortrait" => {
            Some(&["image", "expression", "position", "width", "height", "flip"])
        }
        "choiceMenu" => Some(&["choices", "title", "layout", "columns"]),
        "inventoryGrid" => Some(&["items", "columns", "rows", "cellSize", "showTooltips"]),
        "healthBar" => Some(&[
            "value",
            "maxValue",
            "label",
            "fillColor",
            "variant",
            "showLabel",
            "size",
            "animated",
        ]),
        "miniMap" => Some(&[
            "mapImage",
            "width",
            "height",
            "markers",
            "playerX",
            "playerY",
            "viewportWidth",
            "viewportHeight",
            "zoom",
        ]),
        "calendar" => Some(&[
            "events",
            "view",
            "date",
            "editable",
            "selectable",
            "firstDayOfWeek",
            "minTime",
            "maxTime",
            "slotDuration",
            "showWeekends",
            "showNowIndicator",
            "showAllDay",
            "locale",
            "height",
            "responsive",
            "compactBreakpoint",
        ]),
        "gantt" => Some(&[
            "tasks",
            "view",
            "editable",
            "draggable",
            "resizable",
            "showDependencies",
            "showProgress",
            "showToday",
            "rowHeight",
            "columns",
            "height",
            "responsive",
            "compactBreakpoint",
        ]),
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
        "diffView" => &["original", "modified"],
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
        "calendar" => &["events"],
        "gantt" => &["tasks"],
        _ => &[],
    }
}

const BASE_PROPS: &[&str] = &["type", "id", "style", "children", "actions"];
const MAX_UI_COMPONENTS: usize = 120;
const MAX_UI_COMPONENT_ID_CHARS: usize = 120;
const MAX_UI_CUSTOM_CSS_CHARS: usize = 12_000;
const MAX_UI_STYLE_STRING_CHARS: usize = 1_000;
const MAX_UI_ACTIONS: usize = 20;

/// Validate an array of components and return (validated_components, errors)
fn validate_ui_components(
    root_id: &str,
    canvas: &Value,
    components: &Value,
) -> (Value, Vec<String>) {
    let mut errors = Vec::new();
    validate_canvas_settings(canvas, &mut errors);

    let arr = match components.as_array() {
        Some(a) => a,
        None => {
            errors.push("'components' must be an array".to_string());
            return (json!([]), errors);
        }
    };

    if arr.len() > MAX_UI_COMPONENTS {
        errors.push(format!(
            "'components' is limited to {MAX_UI_COMPONENTS} components per response"
        ));
    }

    let mut all_ids = HashSet::new();
    let mut duplicate_ids = HashSet::new();
    for comp in arr {
        if let Some(id) = comp.get("id").and_then(|v| v.as_str())
            && !all_ids.insert(id.to_string())
        {
            duplicate_ids.insert(id.to_string());
        }
    }
    for id in &duplicate_ids {
        errors.push(format!("Duplicate component id '{}'", id));
    }
    if !root_id.is_empty() && !all_ids.contains(root_id) {
        errors.push(format!(
            "rootComponentId '{}' does not exist in the components array",
            root_id
        ));
    }

    let mut validated = Vec::new();
    let mut child_graph: HashMap<String, Vec<String>> = HashMap::new();

    for comp in arr {
        let id = match comp.get("id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                errors.push(
                    "Component missing 'id' field - every component needs a unique id".to_string(),
                );
                continue;
            }
        };
        if id.trim().is_empty() {
            errors.push("Component ids cannot be empty".to_string());
            continue;
        }
        if id.chars().count() > MAX_UI_COMPONENT_ID_CHARS {
            errors.push(format!(
                "{}: component id is too long; maximum is {MAX_UI_COMPONENT_ID_CHARS} characters",
                id
            ));
            continue;
        }

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

        let known = known_props_for_type(comp_type);
        if known.is_none() {
            errors.push(format!(
                "{}: unknown component type '{}'. Use get_component_schema to look up available types.",
                id, comp_type
            ));
            continue;
        }
        let known_set = known.unwrap();

        if comp_type == "markdown"
            && component
                .get("allowHtml")
                .and_then(|value| value.get("literalBool"))
                .and_then(|value| value.as_bool())
                == Some(true)
        {
            errors.push(format!(
                "{}: markdown.allowHtml must be false for generated UI",
                id
            ));
        }

        if comp_type == "iframe"
            && let Some(sandbox) = component
                .get("sandbox")
                .and_then(|value| value.get("literalString"))
                .and_then(|value| value.as_str())
        {
            for token in ["allow-same-origin", "allow-popups-to-escape-sandbox"] {
                if sandbox.split_whitespace().any(|part| part == token) {
                    errors.push(format!(
                        "{}: iframe sandbox token '{}' is not allowed in generated UI",
                        id, token
                    ));
                }
            }
        }

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

        for required in required_props_for_type(comp_type) {
            if component.get(*required).is_none() {
                errors.push(format!(
                    "{}: missing required prop '{}' on '{}'. This prop is mandatory.",
                    id, required, comp_type
                ));
            }
        }

        if let Some(obj) = component.as_object() {
            for (key, value) in obj {
                let k = key.as_str();
                if BASE_PROPS.contains(&k) {
                    continue;
                }
                if matches!(
                    k,
                    "tabs"
                        | "items"
                        | "overlays"
                        | "columns"
                        | "data"
                        | "boxes"
                        | "hotspots"
                        | "markers"
                        | "choices"
                ) {
                    continue;
                }
                if (value.is_string() || value.is_number() || value.is_boolean())
                    && known_set.contains(&k)
                    && k != "type"
                {
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

        if let Some(style) = comp.get("style") {
            validate_style_value(id, "style", style, &mut errors);
        }
        if let Some(style) = component.get("style") {
            validate_style_value(id, "component.style", style, &mut errors);
        }
        if let Some(actions) = component.get("actions") {
            validate_actions_value(id, actions, &mut errors);
        }

        let mut component_refs = Vec::new();
        if let Some(children) = component.get("children") {
            component_refs.extend(collect_child_refs(id, children, &all_ids, &mut errors));
        }

        if let Some(content_component_id) = component
            .get("contentComponentId")
            .and_then(bound_or_plain_string)
        {
            push_component_ref(
                id,
                content_component_id,
                "contentComponentId",
                &all_ids,
                &mut errors,
                &mut component_refs,
            );
        }

        if let Some(base_component_id) = component
            .get("baseComponentId")
            .and_then(bound_or_plain_string)
        {
            push_component_ref(
                id,
                base_component_id,
                "baseComponentId",
                &all_ids,
                &mut errors,
                &mut component_refs,
            );
        }

        if let Some(overlays) = component.get("overlays").and_then(|value| value.as_array()) {
            for overlay in overlays {
                if let Some(overlay_id) = overlay
                    .get("componentId")
                    .or_else(|| overlay.get("id"))
                    .and_then(bound_or_plain_string)
                {
                    push_component_ref(
                        id,
                        overlay_id,
                        "overlays[].componentId",
                        &all_ids,
                        &mut errors,
                        &mut component_refs,
                    );
                }
            }
        }

        for (array_prop, ref_prop) in [
            ("tabs", "contentComponentId"),
            ("items", "contentComponentId"),
        ] {
            if let Some(items) = component.get(array_prop).and_then(|value| value.as_array()) {
                for item in items {
                    if let Some(content_component_id) =
                        item.get(ref_prop).and_then(bound_or_plain_string)
                    {
                        push_component_ref(
                            id,
                            content_component_id,
                            &format!("{array_prop}[].{ref_prop}"),
                            &all_ids,
                            &mut errors,
                            &mut component_refs,
                        );
                    }
                }
            }
        }

        if !component_refs.is_empty() {
            child_graph.insert(id.to_string(), component_refs);
        }

        validated.push(comp.clone());
    }

    if let Some(cycle) = find_child_cycle(&child_graph) {
        errors.push(format!(
            "Component references contain a cycle: {}",
            cycle.join(" -> ")
        ));
    }

    (json!(validated), errors)
}

fn bound_or_plain_string(value: &Value) -> Option<&str> {
    value
        .get("literalString")
        .and_then(Value::as_str)
        .or_else(|| value.as_str())
}

fn push_component_ref(
    parent_id: &str,
    target_id: &str,
    field: &str,
    all_ids: &HashSet<String>,
    errors: &mut Vec<String>,
    refs: &mut Vec<String>,
) {
    if target_id == parent_id {
        errors.push(format!("{}: {} cannot reference itself", parent_id, field));
    }
    if !all_ids.contains(target_id) {
        errors.push(format!(
            "{}: {} references '{}' which doesn't exist",
            parent_id, field, target_id
        ));
    }
    refs.push(target_id.to_string());
}

fn is_known_style_prop(key: &str) -> bool {
    matches!(
        key,
        "className"
            | "background"
            | "border"
            | "shadow"
            | "position"
            | "transform"
            | "overflow"
            | "responsiveOverrides"
            | "margin"
            | "padding"
            | "gap"
            | "width"
            | "height"
            | "minWidth"
            | "minHeight"
            | "maxWidth"
            | "maxHeight"
            | "flex"
            | "flexGrow"
            | "flexShrink"
            | "flexBasis"
            | "alignSelf"
            | "gridColumn"
            | "gridRow"
            | "gridArea"
            | "justifySelf"
            | "color"
            | "fontSize"
            | "fontWeight"
            | "fontFamily"
            | "lineHeight"
            | "letterSpacing"
            | "textAlign"
            | "textDecoration"
            | "textTransform"
            | "whiteSpace"
            | "wordBreak"
            | "opacity"
            | "visibility"
            | "cursor"
            | "userSelect"
            | "pointerEvents"
            | "zIndex"
            | "transition"
            | "animation"
            | "display"
            | "outline"
            | "outlineOffset"
            | "filter"
            | "backdropFilter"
            | "aspectRatio"
    )
}

fn validate_style_value(component_id: &str, path: &str, style: &Value, errors: &mut Vec<String>) {
    let Some(style_obj) = style.as_object() else {
        errors.push(format!("{}: {} must be an object", component_id, path));
        return;
    };

    for (key, value) in style_obj {
        if !is_known_style_prop(key) {
            errors.push(format!(
                "{}: unknown style prop '{}.{}'",
                component_id, path, key
            ));
        }
        validate_style_strings(component_id, &format!("{path}.{key}"), value, errors);
    }
}

fn validate_style_strings(component_id: &str, path: &str, value: &Value, errors: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            if text.len() > MAX_UI_STYLE_STRING_CHARS {
                errors.push(format!(
                    "{}: {} is too long; maximum is {MAX_UI_STYLE_STRING_CHARS} bytes",
                    component_id, path
                ));
            }

            let lowered = text.to_ascii_lowercase();
            let compact: String = lowered.chars().filter(|ch| !ch.is_whitespace()).collect();
            if compact.contains("javascript:")
                || compact.contains("vbscript:")
                || compact.contains("data:text/html")
                || compact.contains("-moz-binding")
            {
                errors.push(format!(
                    "{}: {} contains an unsafe CSS value",
                    component_id, path
                ));
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                validate_style_strings(component_id, &format!("{path}[{index}]"), item, errors);
            }
        }
        Value::Object(obj) => {
            for (key, item) in obj {
                validate_style_strings(component_id, &format!("{path}.{key}"), item, errors);
            }
        }
        _ => {}
    }
}

fn validate_actions_value(component_id: &str, actions: &Value, errors: &mut Vec<String>) {
    let Some(actions) = actions.as_array() else {
        errors.push(format!("{}: actions must be an array", component_id));
        return;
    };

    if actions.len() > MAX_UI_ACTIONS {
        errors.push(format!(
            "{}: actions is limited to {MAX_UI_ACTIONS} entries",
            component_id
        ));
    }

    for (index, action) in actions.iter().enumerate() {
        let Some(action_obj) = action.as_object() else {
            errors.push(format!(
                "{}: actions[{index}] must be an object",
                component_id
            ));
            continue;
        };

        for key in action_obj.keys() {
            if !matches!(key.as_str(), "name" | "context") {
                errors.push(format!(
                    "{}: unknown action prop 'actions[{index}].{}'",
                    component_id, key
                ));
            }
        }

        match action_obj.get("name").and_then(Value::as_str) {
            Some(name) if !name.trim().is_empty() => {}
            _ => errors.push(format!(
                "{}: actions[{index}].name must be a non-empty string",
                component_id
            )),
        }

        match action_obj.get("context") {
            Some(Value::Object(_)) => {}
            Some(_) => errors.push(format!(
                "{}: actions[{index}].context must be an object",
                component_id
            )),
            None => errors.push(format!(
                "{}: actions[{index}].context is required",
                component_id
            )),
        }
    }
}

fn validate_canvas_settings(canvas: &Value, errors: &mut Vec<String>) {
    if !canvas.is_object() {
        if !canvas.is_null() {
            errors.push("canvasSettings must be an object".to_string());
        }
        return;
    }
    if let Some(custom_css) = canvas.get("customCss").and_then(|value| value.as_str())
        && custom_css.len() > MAX_UI_CUSTOM_CSS_CHARS
    {
        errors.push(format!(
            "canvasSettings.customCss is too large; maximum is {MAX_UI_CUSTOM_CSS_CHARS} bytes"
        ));
    }
    if let Some(background_image) = canvas
        .get("backgroundImage")
        .and_then(|value| value.as_str())
    {
        let allowed = background_image.starts_with("http://")
            || background_image.starts_with("https://")
            || background_image.starts_with("data:image/png;base64,")
            || background_image.starts_with("data:image/jpeg;base64,")
            || background_image.starts_with("data:image/webp;base64,")
            || background_image.starts_with("data:image/gif;base64,");
        if !allowed {
            errors.push(
                "canvasSettings.backgroundImage must be http(s) or a safe data:image URL"
                    .to_string(),
            );
        }
    }
}

fn collect_child_refs(
    parent_id: &str,
    children: &Value,
    all_ids: &HashSet<String>,
    errors: &mut Vec<String>,
) -> Vec<String> {
    let Some(children_obj) = children.as_object() else {
        errors.push(format!("{}: children must be an object", parent_id));
        return Vec::new();
    };

    if let Some(explicit_list) = children_obj.get("explicitList") {
        let Some(explicit_list) = explicit_list.as_array() else {
            errors.push(format!(
                "{}: children.explicitList must be an array of component ids",
                parent_id
            ));
            return Vec::new();
        };

        let mut refs = Vec::new();
        for child_ref in explicit_list {
            let Some(child_id) = child_ref.as_str() else {
                errors.push(format!(
                    "{}: children.explicitList can only contain strings",
                    parent_id
                ));
                continue;
            };
            if child_id == parent_id {
                errors.push(format!("{}: component cannot be its own child", parent_id));
            }
            if !all_ids.contains(child_id) {
                errors.push(format!(
                    "{}: children references '{}' which doesn't exist in the components array",
                    parent_id, child_id
                ));
            }
            refs.push(child_id.to_string());
        }
        return refs;
    }

    if let Some(template) = children_obj.get("template") {
        let template_component_id = template
            .get("templateComponentId")
            .and_then(|value| value.as_str());
        let data_path = template.get("dataPath").and_then(|value| value.as_str());
        match (template_component_id, data_path) {
            (Some(component_id), Some(_)) if all_ids.contains(component_id) => {
                return vec![component_id.to_string()];
            }
            (Some(component_id), Some(_)) => {
                errors.push(format!(
                    "{}: templateComponentId '{}' does not exist",
                    parent_id, component_id
                ));
            }
            _ => {
                errors.push(format!(
                    "{}: children.template requires templateComponentId and dataPath",
                    parent_id
                ));
            }
        }
        return Vec::new();
    }

    errors.push(format!(
        "{}: children must contain explicitList or template",
        parent_id
    ));
    Vec::new()
}

fn find_child_cycle(graph: &HashMap<String, Vec<String>>) -> Option<Vec<String>> {
    fn visit(
        node: &str,
        graph: &HashMap<String, Vec<String>>,
        visiting: &mut HashSet<String>,
        visited: &mut HashSet<String>,
        stack: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        if visited.contains(node) {
            return None;
        }
        if !visiting.insert(node.to_string()) {
            if let Some(start) = stack.iter().position(|item| item == node) {
                let mut cycle = stack[start..].to_vec();
                cycle.push(node.to_string());
                return Some(cycle);
            }
            return Some(vec![node.to_string(), node.to_string()]);
        }

        stack.push(node.to_string());
        if let Some(children) = graph.get(node) {
            for child in children {
                if let Some(cycle) = visit(child, graph, visiting, visited, stack) {
                    return Some(cycle);
                }
            }
        }
        stack.pop();
        visiting.remove(node);
        visited.insert(node.to_string());
        None
    }

    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    let mut stack = Vec::new();
    for node in graph.keys() {
        if let Some(cycle) = visit(node, graph, &mut visiting, &mut visited, &mut stack) {
            return Some(cycle);
        }
    }
    None
}

/// Get detailed schema documentation for a component type
fn get_component_schema_doc(component_type: &str) -> String {
    use flow_like::a2ui::copilot::get_component_schema;

    let base_doc = get_component_schema(component_type);

    // Add BoundValue reminder and known props list
    if let Some(props) = known_props_for_type(component_type) {
        let required = required_props_for_type(component_type);
        let prop_list: Vec<String> = props
            .iter()
            .map(|p| {
                if required.contains(p) {
                    format!("- {} (REQUIRED)", p)
                } else {
                    format!("- {}", p)
                }
            })
            .collect();

        format!(
            "{}\n\n### Valid Props\n{}\n\n### BoundValue Reminder\nAll props must use BoundValue format:\n- String: {{\"literalString\": \"text\"}}\n- Number: {{\"literalNumber\": 42}}\n- Boolean: {{\"literalBool\": true}}\n- Options: {{\"literalOptions\": [{{\"value\": \"v\", \"label\": \"L\"}}]}}\n- Children: {{\"explicitList\": [\"child-id-1\"]}}",
            base_doc,
            prop_list.join("\n")
        )
    } else {
        base_doc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internet_search_requires_non_empty_query_without_network() {
        let result = run_internet_search_tool(&json!({ "query": "   " }));

        assert_eq!(result.get("status").and_then(Value::as_str), Some("error"));
        assert_eq!(
            result.get("tool").and_then(Value::as_str),
            Some("internet_search")
        );
    }

    #[test]
    fn compact_search_result_keeps_only_model_relevant_fields() {
        let result = compact_search_result(&json!({
            "title": "Flow Like",
            "url": "https://flow-like.com",
            "content": "Workflow automation",
            "publishedDate": "2026-06-04",
            "engine": "test",
            "category": "general",
            "score": 1.25,
            "huge": "drop me"
        }));

        assert_eq!(
            result.get("title").and_then(Value::as_str),
            Some("Flow Like")
        );
        assert!(result.get("huge").is_none());
    }
}
