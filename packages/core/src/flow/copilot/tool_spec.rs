//! Single source of truth for the global FlowPilot assistant's platform tools.
//!
//! Every backend (profile "Bits" models via the rig loop, GitHub Copilot via the Copilot SDK,
//! Codex/Claude Code via the MCP bridge) advertises the SAME tools from these specs: name,
//! description, JSON schema, approval requirement, and dispatch timeout. Adapters convert a spec
//! into the backend-native tool type; execution always funnels through the desktop
//! `PlatformToolBridge`/`FrontendToolBridge`, except for the host-local tools listed below.
//!
//! Host-local tools (dispatched by name, not through the frontend):
//! - `internet_search` runs an in-process web search on the desktop side.
//! - `_memory_store` / `_memory_search` run against the core `AssistantMemory`.

use rig::completion::ToolDefinition;
use serde_json::{Value, json};

pub const INTERNET_SEARCH_TOOL: &str = "internet_search";
pub const MEMORY_STORE_TOOL: &str = "_memory_store";
pub const MEMORY_SEARCH_TOOL: &str = "_memory_search";

/// Approval the host must obtain before executing a tool call. The approval "action" key equals
/// the tool name; messages are built from the call arguments so dialogs can name the target.
#[derive(Clone, Copy)]
pub enum ToolApprovalSpec {
    None,
    Mutating {
        title: &'static str,
        message: fn(&Value) -> String,
    },
    Execute {
        title: &'static str,
        message: fn(&Value) -> String,
    },
}

/// Backend-independent description of one platform tool.
#[derive(Clone, Copy)]
pub struct PlatformToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: fn() -> Value,
    pub approval: ToolApprovalSpec,
    pub timeout_secs: u64,
}

impl PlatformToolSpec {
    pub fn to_tool_definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.to_string(),
            description: self.description.to_string(),
            parameters: (self.schema)(),
        }
    }
}

/// Read a string argument, accepting both snake_case and camelCase keys (agent backends differ).
pub fn spec_arg_str<'a>(args: &'a Value, snake: &str, camel: &str) -> &'a str {
    args.get(snake)
        .or_else(|| args.get(camel))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn snake_to_camel(snake: &str) -> String {
    let mut out = String::with_capacity(snake.len());
    let mut upper_next = false;
    for ch in snake.chars() {
        if ch == '_' {
            upper_next = true;
        } else if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// Check `args` against the spec schema's top-level `required` list (accepting camelCase key
/// variants). Returns an actionable error message when a required argument is missing or an
/// empty string, so the model retries with complete arguments instead of the host executing a
/// broken call (e.g. `create_app` without a name) or showing a pointless approval dialog.
pub fn missing_required_args(spec: &PlatformToolSpec, args: &Value) -> Option<String> {
    let schema = (spec.schema)();
    let required = schema.get("required")?.as_array()?;

    let missing: Vec<&str> = required
        .iter()
        .filter_map(Value::as_str)
        .filter(|key| {
            let value = args
                .get(*key)
                .or_else(|| args.get(snake_to_camel(key).as_str()));
            match value {
                None | Some(Value::Null) => true,
                Some(Value::String(text)) => text.trim().is_empty(),
                Some(_) => false,
            }
        })
        .collect();

    if missing.is_empty() {
        None
    } else {
        Some(format!(
            "{} requires non-empty argument(s): {}. Call the tool again with all required arguments set.",
            spec.name,
            missing.join(", ")
        ))
    }
}

fn create_app_message(args: &Value) -> String {
    let name = spec_arg_str(args, "name", "name");
    if name.is_empty() {
        "FlowPilot wants to create a new app.".to_string()
    } else {
        format!("FlowPilot wants to create a new app named '{name}'.")
    }
}

fn flowpilot_board_message(args: &Value) -> String {
    let instruction = spec_arg_str(args, "instruction", "instruction");
    if instruction.is_empty() {
        "FlowPilot wants to run the board copilot on this app.".to_string()
    } else {
        format!("FlowPilot wants to run the board copilot: {instruction}")
    }
}

fn flowpilot_widget_message(args: &Value) -> String {
    let instruction = spec_arg_str(args, "instruction", "instruction");
    if instruction.is_empty() {
        "FlowPilot wants to design UI on the open widget surface.".to_string()
    } else {
        format!("FlowPilot wants to design UI on the open widget surface: {instruction}")
    }
}

fn call_app_chat_message(args: &Value) -> String {
    let app_id = spec_arg_str(args, "app_id", "appId");
    if app_id.is_empty() {
        "FlowPilot wants to message an app's chat.".to_string()
    } else {
        format!("FlowPilot wants to message the chat of app '{app_id}'.")
    }
}

fn call_app_event_message(args: &Value) -> String {
    let app_id = spec_arg_str(args, "app_id", "appId");
    if app_id.is_empty() {
        "FlowPilot wants to execute an app event.".to_string()
    } else {
        format!("FlowPilot wants to execute an event of app '{app_id}'.")
    }
}

/// The complete tool set of the global FlowPilot assistant. `memory_enabled` appends the
/// `_memory_store`/`_memory_search` tools (only offered when the user selected an embedding model).
pub fn global_assistant_tool_specs(memory_enabled: bool) -> Vec<PlatformToolSpec> {
    let mut specs = vec![
        PlatformToolSpec {
            name: "list_apps",
            description: r#"List the apps visible in the user's CURRENT profile, with the callable interfaces each
one exposes. Every event carries a `kind` that tells you which tool consumes it: "chat" →
`open_app_chat`/`call_app_chat`, "page" → `open_app_page` (embed the app's UI inline), "headless"
(simple/REST/MCP/…) → `call_app_event`. Use this before acting on any app. Only apps in the current
profile are returned."#,
            schema: || json!({ "type": "object", "properties": {} }),
            approval: ToolApprovalSpec::None,
            timeout_secs: 120,
        },
        PlatformToolSpec {
            name: "describe_app_interface",
            description: r#"Read the full, user-readable configuration of one app event/interface (chat, MCP, REST,
simple chat, …). Use after `list_apps` to understand HOW to call an interface: its inputs, routes,
tools, or chat settings. Read-only."#,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "app_id": { "type": "string", "description": "App id (from list_apps)." },
                        "event_id": { "type": "string", "description": "Event id (from list_apps)." }
                    },
                    "required": ["app_id", "event_id"]
                })
            },
            approval: ToolApprovalSpec::None,
            timeout_secs: 120,
        },
        PlatformToolSpec {
            name: "open_app_chat",
            description: r#"Open an app's chat event as an inline chat card in the user's current view, so the USER
can talk to that app directly. Prefer this over `call_app_chat` when the user should take over the
conversation. Non-destructive UI change."#,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "app_id": { "type": "string", "description": "App id (from list_apps)." },
                        "event_id": { "type": "string", "description": "Chat event id (from list_apps). Optional; defaults to the app's first chat event." }
                    },
                    "required": ["app_id"]
                })
            },
            approval: ToolApprovalSpec::None,
            timeout_secs: 120,
        },
        PlatformToolSpec {
            name: "open_app_page",
            description: r#"Embed an app's UI page/interface inline in the conversation (like an artifact), so the
USER can see and use the app's frontend without leaving the chat. This is THE tool when the user asks
to "show", "embed" or "display" an app's content in the chat. Works for events with kind "page" in
`list_apps` — NOT for "chat" events (use `open_app_chat`) or "headless" events (use `call_app_event`).
Non-destructive UI change."#,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "app_id": { "type": "string", "description": "App id (from list_apps)." },
                        "event_id": { "type": "string", "description": "Page event id (kind \"page\" in list_apps). Optional; defaults to the app's first page-capable event." }
                    },
                    "required": ["app_id"]
                })
            },
            approval: ToolApprovalSpec::None,
            timeout_secs: 120,
        },
        PlatformToolSpec {
            name: "call_app_event",
            description: r#"Execute a headless event/interface of a Flow-Like app (kind "headless" in `list_apps`:
simple/quick-action, REST or api routes, MCP tools, …) with a JSON payload and return the run's
outputs and bounded logs.

Use `list_apps` to find the app + event, and `describe_app_interface` to learn the expected payload
shape first. For "chat" interfaces use `call_app_chat`; for "page" interfaces use `open_app_page`.
Executing an app event is side-effecting, so it asks for approval unless the user selected "don't ask
again this session"."#,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "app_id": { "type": "string", "description": "Id of the app whose event to execute (from list_apps)." },
                        "event_id": { "type": "string", "description": "Id of the event to execute (from list_apps)." },
                        "payload": { "type": "object", "description": "JSON payload passed to the event (shape from describe_app_interface). Optional." }
                    },
                    "required": ["app_id", "event_id"]
                })
            },
            approval: ToolApprovalSpec::Execute {
                title: "Approve app event execution",
                message: call_app_event_message,
            },
            timeout_secs: 600,
        },
        PlatformToolSpec {
            name: "navigate_view",
            description: r#"Navigate the Flow-Like desktop app to a different screen. This changes the WHOLE view —
it does NOT embed anything in the conversation (use `open_app_page` / `open_app_chat` for that; never
claim content is embedded after navigating).

Navigation is a non-destructive UI change and runs without an approval dialog. Prefer a logical
`view` (+ `app_id`); only pass `route` for the documented routes — never invent paths."#,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "view": { "type": "string", "description": "Logical view id: 'home', 'apps', 'store', 'settings', 'profile', 'learn', or 'app' (an app's use surface, requires app_id)." },
                        "route": { "type": "string", "description": "Explicit router path — ONLY these exist: '/', '/library', '/store', '/settings', '/learn', '/chat', '/use?id=<app>' (an app's pages) or '/flow?id=<board>&app=<app>' (a board). Anything else is invalid." },
                        "app_id": { "type": "string", "description": "App id, when the target view is app-scoped." },
                        "page_route": { "type": "string", "description": "Route path of the app page to open inside the app's use surface (from its route mapping), e.g. '/briefing'. Only with app_id." }
                    },
                    "required": ["view"]
                })
            },
            approval: ToolApprovalSpec::None,
            timeout_secs: 120,
        },
        PlatformToolSpec {
            name: "create_app",
            description: r#"Create a new Flow-Like app (project) in the current profile.

Use this when the user wants to start a new automation/app. By default the app is created online
(synced to the user's Flow-Like account) when they are signed in, and local-only otherwise. Set
`online` to false to force a local-only app. Creating an app is a mutating action and shows an
approval dialog with a "don't ask again this session" option before it runs."#,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Human-readable app name." },
                        "description": { "type": "string", "description": "Short description of what the app does." },
                        "online": { "type": "boolean", "description": "Create an online/cloud app synced to the user's account (the default when signed in) or a local-only app when false. Forced to local when the user is not signed in." }
                    },
                    "required": ["name"]
                })
            },
            approval: ToolApprovalSpec::Mutating {
                title: "Approve app creation",
                message: create_app_message,
            },
            timeout_secs: 120,
        },
        PlatformToolSpec {
            name: "flowpilot_board",
            description: r#"The single entry point for ANYTHING about a specific board/workflow/page — explaining it, editing it, or debugging it. Delegates to the board FlowPilot, which has full access to the board's nodes, connections and layers.

Two modes (set `mode`):
- mode="explain" (read-only): answer the user's question about the board — "explain this workflow", "what does this do", "why is this failing". Nothing is modified and no approval is asked. Relay the returned answer to the user.
- mode="edit" (default): build or modify the board (add/connect/configure nodes) or a UI page. If the app has no board yet, one is created automatically — never ask the user to create a board manually. Give a complete, self-contained instruction (trigger/event, the processing steps, and where results go). Side-effecting; asks for approval unless the user selected "don't ask again this session".

When a board is already open (see CURRENTLY OPEN BOARD in your context), pass its app_id/board_id and route the user's board question here directly — do NOT ask which app or board, and do NOT answer board questions yourself.

SCOPE: it reads/edits board/page CONTENTS only. It cannot create apps (use create_app), rename apps, or change app metadata/settings — do not put such requests in the instruction."#,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "instruction": { "type": "string", "description": "Complete natural-language instruction or question for the board copilot. For mode=edit: trigger, steps, outputs. For mode=explain: the user's question about the board." },
                        "mode": { "type": "string", "enum": ["edit", "explain"], "description": "\"explain\" to answer a question about the board (read-only, no changes, no approval); \"edit\" to build/modify it. Defaults to \"edit\"." },
                        "app_id": { "type": "string", "description": "App id (from list_apps, create_app, or the CURRENTLY OPEN BOARD context)." },
                        "board_id": { "type": "string", "description": "Target board id within the app. Optional; defaults to the app's first board (or the open board), creating one if none exists." },
                        "board_name": { "type": "string", "description": "Name for the board if one has to be created. Optional." }
                    },
                    "required": ["instruction", "app_id"]
                })
            },
            approval: ToolApprovalSpec::Execute {
                title: "Approve board edit",
                message: flowpilot_board_message,
            },
            timeout_secs: 600,
        },
        PlatformToolSpec {
            name: "flowpilot_widget",
            description: r#"Design or modify UI components on the user's CURRENTLY OPEN widget/page surface (A2UI).
Only available when the user has a widget/page builder open — check list_apps/context or just call it;
it errors cleanly when no surface is open. Give a complete instruction of what the UI should look
like/do. Side-effecting; asks for approval."#,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "instruction": { "type": "string", "description": "Complete natural-language instruction describing the UI to design or modify on the open widget/page surface." }
                    },
                    "required": ["instruction"]
                })
            },
            approval: ToolApprovalSpec::Execute {
                title: "Approve UI edit",
                message: flowpilot_widget_message,
            },
            timeout_secs: 600,
        },
        PlatformToolSpec {
            name: "ask_user",
            description: r#"Ask the user for one targeted input when placeholders/defaults are not enough.

Prefer defaults and placeholder variables. Use this only for genuinely blocking choices. Supports
freeform, single_choice, and multiple_choice modes. Include a recommended default whenever
possible."#,
            schema: || {
                json!({
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
                })
            },
            approval: ToolApprovalSpec::None,
            timeout_secs: 600,
        },
        PlatformToolSpec {
            name: "call_app_chat",
            description: r#"Talk to a Flow-Like app that exposes a chat event: send it a message and get its reply.

Use this to interact with an app's own chat agent on the user's behalf (e.g. ask a knowledge-base app
a question). Running the app's chat is side-effecting, so it asks for approval unless the user selected
"don't ask again this session". Returns the app chat's text response."#,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "app_id": { "type": "string", "description": "Id of the app whose chat event to call (from list_apps)." },
                        "event_id": { "type": "string", "description": "Id of the specific chat event to call (from list_apps). Optional; defaults to the app's first chat event." },
                        "message": { "type": "string", "description": "Message to send to the app's chat." }
                    },
                    "required": ["app_id", "message"]
                })
            },
            approval: ToolApprovalSpec::Execute {
                title: "Approve app chat call",
                message: call_app_chat_message,
            },
            // Longer than the other tools: the app chat can raise interactive dialogs
            // (single/multiple choice, form) that a human must answer, and a workflow may chain
            // several. The frontend bridge blocks for this whole window, so it has to comfortably
            // exceed the interactions' TTLs plus human response time.
            timeout_secs: 1800,
        },
        PlatformToolSpec {
            name: INTERNET_SEARCH_TOOL,
            description: r#"Search the public web through Flow-Like's SearXNG instance at search.flow-like.com.

Use this when current public information, documentation, examples, or external references would
help. Prefer official docs and primary sources in your follow-up reasoning. Returns compact
title/url/snippet/date results."#,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query." },
                        "language": { "type": "string", "description": "SearXNG language code, default en-US." },
                        "page": { "type": "integer", "description": "1-based page number, default 1." },
                        "limit": { "type": "integer", "description": "Maximum results to return, default 8, max 20." }
                    },
                    "required": ["query"]
                })
            },
            approval: ToolApprovalSpec::None,
            timeout_secs: 120,
        },
    ];

    if memory_enabled {
        specs.push(PlatformToolSpec {
            name: MEMORY_STORE_TOOL,
            description: "Store an important fact, user preference, decision, or context in your persistent profile-scoped memory. Call this immediately when you learn something worth remembering — do not merely say you will remember.",
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "content": { "type": "string", "description": "The fact/observation to remember." },
                        "role": { "type": "string", "description": "One of: user, assistant, observation, summary. Default: observation." }
                    },
                    "required": ["content"]
                })
            },
            approval: ToolApprovalSpec::None,
            timeout_secs: 120,
        });
        specs.push(PlatformToolSpec {
            name: MEMORY_SEARCH_TOOL,
            description: "Search your persistent profile-scoped memory for relevant facts and context. Search at the start of a conversation and whenever prior context would help.",
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "What to recall." }
                    },
                    "required": ["query"]
                })
            },
            approval: ToolApprovalSpec::None,
            timeout_secs: 120,
        });
    }

    specs
}

/// Look up one global-assistant tool spec by name (memory tools included).
pub fn find_global_tool_spec(name: &str) -> Option<PlatformToolSpec> {
    global_assistant_tool_specs(true)
        .into_iter()
        .find(|spec| spec.name == name)
}
