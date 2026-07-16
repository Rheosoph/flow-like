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
use serde::Serialize;
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

/// Host-neutral approval payload the client must satisfy before a tool runs. Serializes to the same
/// camelCase shape every FlowPilot frontend already handles (`{kind, title, description, sessionKey}`),
/// so the desktop (Tauri event) and the browser (SSE `tool_request` frame) send an identical object.
/// `kind` is one of `"none" | "mutating" | "execute"`; `session_key` is the tool name (the
/// "don't ask again this session" key).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedToolApproval {
    pub kind: String,
    pub title: String,
    pub description: String,
    pub session_key: String,
}

impl ResolvedToolApproval {
    pub fn none() -> Self {
        Self {
            kind: "none".to_string(),
            title: String::new(),
            description: String::new(),
            session_key: String::new(),
        }
    }
}

/// Resolve the approval a tool call requires from its spec + arguments. Single source of truth for
/// approval policy across every backend and host, so a `flowpilot_board` explain call never prompts
/// while a mutating/execute call always does.
pub fn resolve_tool_approval(spec: &PlatformToolSpec, args: &Value) -> ResolvedToolApproval {
    // A read-only board explanation (flowpilot_board mode="explain") changes nothing, so it must not
    // surface the "Approve board edit" prompt — that would make asking about a board feel like
    // authorizing a mutation.
    if spec.name == "flowpilot_board" && spec_arg_str(args, "mode", "mode") == "explain" {
        return ResolvedToolApproval::none();
    }
    match spec.approval {
        ToolApprovalSpec::None => ResolvedToolApproval::none(),
        ToolApprovalSpec::Mutating { title, message } => ResolvedToolApproval {
            kind: "mutating".to_string(),
            title: title.to_string(),
            description: message(args),
            session_key: spec.name.to_string(),
        },
        ToolApprovalSpec::Execute { title, message } => ResolvedToolApproval {
            kind: "execute".to_string(),
            title: title.to_string(),
            description: message(args),
            session_key: spec.name.to_string(),
        },
    }
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

fn execute_event_message(args: &Value) -> String {
    let event_id = spec_arg_str(args, "event_id", "eventId");
    if event_id.is_empty() {
        "FlowPilot wants to execute a workflow event and inspect its logs.".to_string()
    } else {
        format!("FlowPilot wants to execute workflow event '{event_id}' and inspect its logs.")
    }
}

fn execute_node_message(args: &Value) -> String {
    let node_id = spec_arg_str(args, "node_id", "nodeId");
    if node_id.is_empty() {
        "FlowPilot wants to execute a workflow from a board node and inspect its logs.".to_string()
    } else {
        format!(
            "FlowPilot wants to execute the workflow from node '{node_id}' and inspect its logs."
        )
    }
}

fn upsert_event_message(args: &Value) -> String {
    let name = spec_arg_str(args, "name", "name");
    if name.is_empty() {
        "FlowPilot wants to create or update an event.".to_string()
    } else {
        format!("FlowPilot wants to create or update the event '{name}'.")
    }
}

fn delete_event_message(args: &Value) -> String {
    let event_id = spec_arg_str(args, "event_id", "eventId");
    if event_id.is_empty() {
        "FlowPilot wants to delete an event.".to_string()
    } else {
        format!("FlowPilot wants to delete event '{event_id}'.")
    }
}

fn set_page_load_event_message(_args: &Value) -> String {
    "FlowPilot wants to set the page's onLoad event.".to_string()
}

fn execute_event_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_id": { "type": "string", "description": "App id. Optional only when the current board runtime already supplies it." },
            "event_id": { "type": "string", "description": "Persisted app Event id to execute." },
            "payload": { "type": "object", "description": "JSON payload passed to the Event. Optional for payload-free Simple Events." },
            "stream_state": { "type": "boolean", "description": "Collect state/log events while the run executes. Defaults to true." }
        },
        "required": ["event_id"]
    })
}

fn scoped_execute_node_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_id": { "type": "string", "description": "App id. Optional when the current board runtime already supplies it." },
            "board_id": { "type": "string", "description": "Persisted board id containing the node." },
            "node_id": { "type": "string", "description": "Persisted node id to use as the execution entry. The run follows its connected downstream graph." },
            "payload": { "type": "object", "description": "Optional payload supplied to the node execution." },
            "stream_state": { "type": "boolean", "description": "Collect state/log events while the run executes. Defaults to true." }
        },
        "required": ["board_id", "node_id"]
    })
}

fn global_execute_node_schema() -> Value {
    let mut schema = scoped_execute_node_schema();
    schema["required"] = json!(["app_id", "board_id", "node_id"]);
    schema
}

fn scoped_query_execution_logs_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_id": { "type": "string", "description": "App id. Optional when the current board runtime already supplies it." },
            "board_id": { "type": "string", "description": "Board id that produced the run." },
            "run_id": { "type": "string", "description": "Exact run id returned by execute_node/execute_event or the run inspector." },
            "filter": { "type": "string", "description": "Optional SQL-like log filter, for example `log_level >= 3` or `node_id = \"...\"`." },
            "limit": { "type": "integer", "minimum": 1, "maximum": 100, "description": "Maximum logs to return. Defaults to 100 and is capped at 100." },
            "offset": { "type": "integer", "minimum": 0, "description": "Pagination offset. Defaults to 0." },
            "run_metadata": { "type": "object", "description": "Optional run metadata returned by execution/list-runs. Supplying it avoids an extra run lookup and preserves local/remote routing." }
        },
        "required": ["board_id", "run_id"]
    })
}

fn global_query_execution_logs_schema() -> Value {
    let mut schema = scoped_query_execution_logs_schema();
    schema["required"] = json!(["app_id", "board_id", "run_id"]);
    schema
}

/// Runtime verification tools offered inside a board-scoped FlowPilot session. The host supplies
/// the current app, but callers must still identify the persisted board/node or Event they want to
/// run. These definitions are shared by every desktop SDK/MCP provider.
pub fn runtime_execution_tool_specs() -> Vec<PlatformToolSpec> {
    vec![
        PlatformToolSpec {
            name: "execute_event",
            description: r#"Execute a persisted app Event through Flow-Like's normal execution service and return the
run id, outputs/metadata, and bounded live logs. Use this to verify an already persisted Event-backed
workflow. A FlowScript edit with status `queued` is not persisted until the current board-agent turn
finishes; do not execute it in the same board-agent turn and claim the new draft was tested."#,
            schema: execute_event_schema,
            approval: ToolApprovalSpec::Execute {
                title: "Approve workflow execution",
                message: execute_event_message,
            },
            timeout_secs: 600,
        },
        PlatformToolSpec {
            name: "execute_node",
            description: r#"Execute a persisted board starting at one exact node and return the run id,
outputs/metadata, and bounded live logs. Execution follows the node's connected downstream graph;
this is not an isolated catalog-node sandbox. Use it after a board edit has actually been applied, or
to reproduce/debug an existing graph. A merely `queued` FlowScript draft is not yet executable."#,
            schema: scoped_execute_node_schema,
            approval: ToolApprovalSpec::Execute {
                title: "Approve node execution",
                message: execute_node_message,
            },
            timeout_secs: 600,
        },
        PlatformToolSpec {
            name: "query_execution_logs",
            description: r#"Read persisted logs for one exact workflow run. Pass the run_id returned by
execute_node/execute_event plus its board_id. Filter by log level, message, or node id and paginate
with limit/offset. Read-only. Use this after execution when the bounded live events are incomplete or
when diagnosing a prior run. Successful reconciliation alone is structural evidence, not proof that
the workflow ran correctly."#,
            schema: scoped_query_execution_logs_schema,
            approval: ToolApprovalSpec::None,
            timeout_secs: 120,
        },
    ]
}

/// Look up one board-scoped runtime execution tool spec by name.
pub fn find_runtime_execution_tool_spec(name: &str) -> Option<PlatformToolSpec> {
    runtime_execution_tool_specs()
        .into_iter()
        .find(|spec| spec.name == name)
}

fn global_runtime_verification_tool_specs() -> Vec<PlatformToolSpec> {
    vec![
        PlatformToolSpec {
            name: "execute_node",
            description: r#"Execute a persisted board starting at one exact node and return the run id,
outputs/metadata, and bounded live logs. Execution follows the connected downstream graph. Use the
exact app_id, board_id and node_id returned by board inspection/edit results, and run only after the
board edit has been applied."#,
            schema: global_execute_node_schema,
            approval: ToolApprovalSpec::Execute {
                title: "Approve node execution",
                message: execute_node_message,
            },
            timeout_secs: 600,
        },
        PlatformToolSpec {
            name: "query_execution_logs",
            description: r#"Read persisted logs for one exact workflow run. Pass app_id, board_id and
the run_id returned by call_app_event/execute_node. Optional filter, limit, offset and run_metadata
narrow or paginate the result. Read-only. Use the evidence to verify or repair a workflow; never
claim runtime correctness from a successful board edit alone."#,
            schema: global_query_execution_logs_schema,
            approval: ToolApprovalSpec::None,
            timeout_secs: 120,
        },
    ]
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
            description: r#"Create a new Flow-Like app (project) in the current profile. ALWAYS pass a `name` — derive a short one from the request (e.g. "Weather App"). Call this ONCE per app; if it succeeds, move on — do NOT call it again with empty arguments.

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
- mode="edit" (default): build or modify the board's WORKFLOW LOGIC (add/connect/configure nodes and events). This is NOT for UI — pages, widgets and components go to flowpilot_widget. If the app has no board yet, one is created automatically — never ask the user to create a board manually. Give a complete, self-contained instruction (trigger/event, the processing steps, and where results go). Side-effecting; asks for approval unless the user selected "don't ask again this session".

For edit mode, the complete user-requested behavior is the acceptance contract. Do not replace a
failed/timeout full build with a reduced smoke test or a sequence of partial board calls unless the
user explicitly asked for a partial prototype. Never overlap mutations. A timeout or transport
drop is an unknown outcome, not proof that the board is empty; inspect the same board after the
request is terminal, then retry the full scope with diagnostics if necessary.

The delegated board specialist owns the FlowScript draft and its edit/validate/repair loop. The
platform caller must not turn a validation problem into a new implementation request such as a
"minimal diagnostic", empty Event, one-node log/notify test, or ask_user choice to downgrade the
workflow. If any result reports `retained_candidate`, `retained_flowscript`, or a retained draft,
that document remains the active recovery workspace even if the persisted board is still empty.
Retained drafts are bound to THIS conversation plus the ORIGINAL user request: a follow-up
repair call resumes them only within the same conversation, never from a different one. Include
that original user request text verbatim in the instruction, name the retained draft_id and its
expected_revision, and direct the specialist to repair that draft in place — same draft_id, same
revision chain, never "start a new draft" or a from-scratch rewrite. Retry on the SAME app/board
with the original acceptance contract and observed diagnostics, and explicitly instruct the
specialist to repair and queue the retained production candidate. Only an
explicit NEW end-user request may discard or reduce it.

When a board is already open (see CURRENTLY OPEN BOARD in your context), pass its app_id/board_id and route the user's board question here directly — do NOT ask which app or board, and do NOT answer board questions yourself.

SCOPE: it reads/edits board/page CONTENTS only. It cannot create apps (use create_app), rename apps, or change app metadata/settings — do not put such requests in the instruction."#,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "instruction": { "type": "string", "description": "Complete natural-language instruction or question for the board copilot. For mode=edit: preserve the original full acceptance contract across retries; when a prior result retained a draft, include the original user request text verbatim, name the retained draft_id + expected_revision, and request repair of that same retained production candidate with its diagnostics — never a minimal replacement or a new draft id. For mode=explain: the user's question about the board." },
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
            // Full production FlowScripts can require several validator-driven repair passes in
            // the nested specialist. Keep the frontend dispatch bound aligned with the external
            // MCP call bound (30 minutes); explicit cancellation and request-ownership fences
            // still stop abandoned runs and reject late mutations.
            timeout_secs: 1800,
        },
        PlatformToolSpec {
            name: "flowpilot_widget",
            description: r#"The UI specialist — design and build interfaces (A2UI). Two modes:
- EDIT the user's currently OPEN widget/page builder (generated components are staged for review), OR
- CREATE a NEW page from scratch in an app (pass app_id). A page is board-scoped, so a board is created automatically if the app has none.
It builds the page AND any reusable widgets it needs — repeated or dynamic elements like list/grid cards, project or save-state rows, email-list items — in ONE call, then navigates the user to the page builder. A simple one-off layout (e.g. a dashboard with a chart and a table) needs no widget. Give a complete instruction of what the UI should look like and do. Side-effecting; asks for approval.
SCOPE: UI only — pages, widgets, components. Board/workflow LOGIC (nodes, events, data wiring) goes through flowpilot_board."#,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "instruction": { "type": "string", "description": "Complete natural-language description of the UI to build or modify (layout, content, and any reusable/repeated widgets)." },
                        "app_id": { "type": "string", "description": "App to create a NEW page in (from list_apps/create_app). Omit when editing the currently open builder surface." },
                        "page_name": { "type": "string", "description": "Name for the new page. Optional; a generic name is used if omitted." },
                        "route": { "type": "string", "description": "URL route for the new page, e.g. \"/dashboard\". Optional; derived from the page name." },
                        "board_id": { "type": "string", "description": "Board the new page binds to. Optional; defaults to the app's first board, creating one if none exists." }
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
        PlatformToolSpec {
            name: "upsert_event",
            description: r#"Create or update an app-level EVENT — the interface/sink setup attached to a board entry node. A board entry node and an Event type are separate layers:
- events_simple entry: quick_action (default), api, cron, daemon, deeplink, rest, mcp. Cron is configured HERE on an events_simple node; it is not a catalog node.
- events_generic entry: generic_form (default), api, deeplink. Its payload and typed output pins carry request/form values; a new FlowScript `eventsGeneric(payload: Struct, field: type, ...)` entry materializes those field pins.
- events_chat entry: simple_chat (default), advanced_chat, discord, telegram.

`flowpilot_board` returns compatible entries under `event_nodes`. For a WORKFLOW event, this tool must run in a separate, later assistant turn: first wait for `flowpilot_board` to succeed and persist the board, then pass the exact returned board_id and node id here. Never call `flowpilot_board` and workflow `upsert_event` in the same response/tool batch, and do not call this tool when the board result failed or contained no compatible `event_nodes`. This tool checks node/Event compatibility and fills the Event type's default config. Pass `config` for sink/interface-specific overrides. For cron pass `cron_expression` (recurring) OR `scheduled_for` (one-time), plus an explicit IANA `timezone` when known.

Two target forms:
- PAGE event (shows a page at a URL): pass page_id (the page to render) and route (e.g. "/weather"). board_id/node_id are not needed.
- WORKFLOW event: pass board_id and node_id (an events_simple/events_generic/events_chat entry node), plus a compatible event_type and optional route.
Omit event_id to create; pass it to update. Side-effecting; asks for approval."#,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "app_id": { "type": "string", "description": "App id." },
                        "event_id": { "type": "string", "description": "Existing event id to UPDATE. Omit to create a new event." },
                        "name": { "type": "string", "description": "Event name." },
                        "event_type": { "type": "string", "description": "Interface/sink type compatible with the referenced entry node: simple -> quick_action/api/cron/daemon/deeplink/rest/mcp; generic -> generic_form/api/deeplink; chat -> simple_chat/advanced_chat/discord/telegram. Omit to use that node kind's default." },
                        "page_id": { "type": "string", "description": "PAGE event: the page id to render (sets default_page_id)." },
                        "route": { "type": "string", "description": "URL path the event/page is reachable at, e.g. \"/weather\". Optional." },
                        "board_id": { "type": "string", "description": "NORMAL event: the board holding the entry node." },
                        "node_id": { "type": "string", "description": "WORKFLOW event: an events_simple/events_generic/events_chat entry-node id, normally from flowpilot_board.event_nodes." },
                        "config": { "type": "object", "description": "Optional sink/interface config overrides merged over the selected Event type's defaults." },
                        "cron_expression": { "type": "string", "description": "Recurring cron setup for an events_simple entry, using a 5- or 6-field expression. Mutually exclusive with scheduled_for." },
                        "scheduled_for": {
                            "type": "object",
                            "description": "One-time cron setup in local wall time. Mutually exclusive with cron_expression.",
                            "properties": {
                                "date": { "type": "string", "description": "YYYY-MM-DD" },
                                "time": { "type": "string", "description": "HH:mm" }
                            },
                            "required": ["date", "time"]
                        },
                        "timezone": { "type": "string", "description": "IANA timezone for cron/scheduled setup, e.g. Europe/Berlin. Defaults to UTC when omitted." },
                        "execution_mode": { "type": "string", "enum": ["Local", "Remote"], "description": "Where this Event runs. A Local/Remote board forces the matching mode; Hybrid boards use this choice." },
                        "description": { "type": "string", "description": "Short description." },
                        "active": { "type": "boolean", "description": "Whether the event is active. Defaults to true." }
                    },
                    "required": ["app_id", "name"]
                })
            },
            approval: ToolApprovalSpec::Mutating {
                title: "Approve event change",
                message: upsert_event_message,
            },
            timeout_secs: 120,
        },
        PlatformToolSpec {
            name: "delete_event",
            description: r#"Delete an event from an app (and its URL route mapping). Side-effecting; asks for approval."#,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "app_id": { "type": "string", "description": "App id." },
                        "event_id": { "type": "string", "description": "Event id to delete." }
                    },
                    "required": ["app_id", "event_id"]
                })
            },
            approval: ToolApprovalSpec::Mutating {
                title: "Approve event deletion",
                message: delete_event_message,
            },
            timeout_secs: 120,
        },
        PlatformToolSpec {
            name: "set_page_load_event",
            description: r#"Wire a page's onLoad behavior — the workflow that runs when the page opens (e.g. to fetch data to display). Pass the page_id and on_load_event_id (an events_* NODE id in the page's board; a flowpilot_board result lists new ones under `event_nodes`). Side-effecting; asks for approval."#,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "app_id": { "type": "string", "description": "App id." },
                        "page_id": { "type": "string", "description": "The page whose onLoad event to set (from flowpilot_widget)." },
                        "on_load_event_id": { "type": "string", "description": "Board NODE id (events_simple) to run when the page opens. From a flowpilot_board result's `event_nodes`." },
                        "on_interval_event_id": { "type": "string", "description": "Optional: node id to run on a timer." },
                        "on_interval_seconds": { "type": "number", "description": "Optional: interval in seconds for on_interval_event_id." },
                        "board_id": { "type": "string", "description": "The page's board id (optional)." }
                    },
                    "required": ["app_id", "page_id", "on_load_event_id"]
                })
            },
            approval: ToolApprovalSpec::Mutating {
                title: "Approve page event",
                message: set_page_load_event_message,
            },
            timeout_secs: 120,
        },
    ];

    // Global FlowPilot can execute a persisted board node directly and inspect any resulting run.
    // Event execution remains `call_app_event` at platform scope because that tool also performs
    // interface discovery/validation; board-scoped agents use `execute_event` from the runtime set.
    specs.extend(global_runtime_verification_tool_specs());

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_event_schema_exposes_typed_cron_setup() {
        let spec = find_global_tool_spec("upsert_event").expect("upsert_event spec");
        let schema = (spec.schema)();
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("upsert_event properties");

        for field in [
            "config",
            "cron_expression",
            "scheduled_for",
            "timezone",
            "execution_mode",
        ] {
            assert!(properties.contains_key(field), "missing {field}: {schema}");
        }
        assert!(spec.description.contains("events_simple"));
        assert!(spec.description.contains("events_generic"));
        assert!(spec.description.contains("events_chat"));
        assert!(spec.description.contains("not a catalog node"));
        assert!(spec.description.contains("separate, later assistant turn"));
        assert!(
            spec.description
                .contains("Never call `flowpilot_board` and workflow `upsert_event`")
        );
    }

    #[test]
    fn board_edit_spec_forbids_timeout_scope_regressions() {
        let spec = find_global_tool_spec("flowpilot_board").expect("flowpilot_board spec");
        assert!(
            spec.description
                .contains("complete user-requested behavior")
        );
        assert!(spec.description.contains("reduced smoke test"));
        assert!(spec.description.contains("Never overlap mutations"));
        assert!(spec.description.contains("unknown outcome"));
        assert!(
            spec.description
                .contains("specialist owns the FlowScript draft")
        );
        assert!(spec.description.contains("retained_flowscript"));
        assert!(spec.description.contains("active recovery workspace"));
        assert!(spec.description.contains("minimal diagnostic"));
        assert!(spec.description.contains("explicit NEW end-user request"));
        assert!(
            spec.description
                .contains("original user request text verbatim")
        );
        assert!(
            spec.description
                .contains("only within the same conversation")
        );
        assert!(
            spec.description
                .contains("retained draft_id and its\nexpected_revision")
        );
        assert!(spec.description.contains("never \"start a new draft\""));
        assert_eq!(spec.timeout_secs, 1800);

        let schema = (spec.schema)();
        let instruction = schema["properties"]["instruction"]["description"]
            .as_str()
            .expect("flowpilot_board instruction description");
        assert!(instruction.contains("same retained production candidate"));
        assert!(instruction.contains("original user request text verbatim"));
        assert!(instruction.contains("retained draft_id + expected_revision"));
        assert!(instruction.contains("never a minimal replacement or a new draft id"));
    }

    #[test]
    fn scoped_runtime_specs_require_persisted_execution_targets() {
        let specs = runtime_execution_tool_specs();
        let names = specs.iter().map(|spec| spec.name).collect::<Vec<_>>();
        assert_eq!(
            names,
            vec!["execute_event", "execute_node", "query_execution_logs"]
        );

        let execute_node = find_runtime_execution_tool_spec("execute_node").unwrap();
        assert_eq!(
            (execute_node.schema)()["required"],
            json!(["board_id", "node_id"])
        );
        assert_eq!(
            resolve_tool_approval(
                &execute_node,
                &json!({"board_id":"board", "node_id":"node"})
            )
            .kind,
            "execute"
        );

        let query = find_runtime_execution_tool_spec("query_execution_logs").unwrap();
        assert_eq!((query.schema)()["required"], json!(["board_id", "run_id"]));
        assert_eq!(
            resolve_tool_approval(&query, &json!({"board_id":"board", "run_id":"run"})).kind,
            "none"
        );
    }

    #[test]
    fn global_runtime_specs_require_explicit_app_scope() {
        let execute_node = find_global_tool_spec("execute_node").unwrap();
        assert_eq!(
            (execute_node.schema)()["required"],
            json!(["app_id", "board_id", "node_id"])
        );

        let query = find_global_tool_spec("query_execution_logs").unwrap();
        let schema = (query.schema)();
        assert_eq!(schema["required"], json!(["app_id", "board_id", "run_id"]));
        for field in ["filter", "limit", "offset", "run_metadata"] {
            assert!(schema["properties"].get(field).is_some(), "missing {field}");
        }
    }
}
