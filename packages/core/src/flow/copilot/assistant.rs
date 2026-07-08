//! Shared entry point for the global ("platform-level") FlowPilot assistant.
//!
//! The desktop Tauri command and the server HTTP endpoint both drive the *same* Bits-backed agent
//! loop ([`PlatformCopilot::chat`]). Everything platform-neutral lives here so neither host owns a
//! private copy: the system prompt, the self-awareness context rendering, the open-board section, and
//! a thin [`run_platform_chat`] wrapper that assembles the prompt and runs the loop. Each host only
//! supplies its own hooks — a [`PlatformToolBridge`], a token sink, the `FlowLikeState`, and the
//! resolved `Profile` — so the actual orchestration is never duplicated per platform.

use std::sync::Arc;

use serde::Deserialize;

use super::memory::AssistantMemory;
use super::platform::{PlatformCopilot, PlatformToolBridge};
use super::types::{ChatImage, ChatMessage};
use crate::profile::Profile;
use crate::state::FlowLikeState;

/// The board the user currently has open on screen, forwarded by the frontend so the global
/// assistant knows which board "this workflow / these nodes" refers to and can route board work to
/// `flowpilot_board` without asking which app/board. Mirrors the live `AssistantBoardSurface`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct GlobalOpenBoardContext {
    pub app_id: String,
    #[serde(default)]
    pub board_id: Option<String>,
    #[serde(default)]
    pub board_name: Option<String>,
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub current_layer: Option<String>,
    #[serde(default)]
    pub selected_node_ids: Vec<String>,
    #[serde(default)]
    pub node_count: Option<usize>,
}

/// Inputs for [`build_platform_context`]. Each host gathers these from its own environment (Tauri
/// settings on desktop, the authenticated request on the server) and hands them over as plain data,
/// keeping the rendered wording — which the model relies on for routing — in one place.
#[derive(Debug, Default)]
pub struct PlatformContextInput<'a> {
    /// A human label for the signed-in user (name/email), if known.
    pub user_context: Option<&'a str>,
    /// The active profile as `(name, id)`.
    pub active_profile: Option<(&'a str, &'a str)>,
    /// Names of the other profiles the user can switch to.
    pub switchable_profiles: &'a [String],
    /// The board the user currently has open, if any.
    pub open_board: Option<&'a GlobalOpenBoardContext>,
}

/// System prompt for the global (platform-level) FlowPilot assistant. Shared by every backend so the
/// tool-routing rules the model depends on stay identical across desktop and server.
pub fn global_assistant_system_prompt() -> String {
    r#"You are FlowPilot, the built-in AI assistant of Flow-Like — a visual automation platform where users build node-based "boards", group them into "apps", and run them locally or in the cloud.

You operate at the PLATFORM level (not inside a single board). Your job:
1. Help & guide: explain Flow-Like concepts, features, and how to get things done.
2. Act for the user via tools: navigate the app, create apps, and more. Prefer doing the work with a tool over only describing the steps.
3. Two specialists, clear split: board/workflow LOGIC (nodes, connections, events, data) → `flowpilot_board`; the USER INTERFACE (pages, widgets, components) → `flowpilot_widget`. Whenever the user asks about a specific board/workflow — explaining it, editing its nodes, or debugging it — call `flowpilot_board` (mode="explain" read-only, or mode="edit" default). Never author FlowScript or explain a board's internals yourself.

Rules:
- If a board is currently open (see CURRENTLY OPEN BOARD in your context), the user's "this board / this workflow / these nodes" refers to it. Route their board question straight to `flowpilot_board` with that app_id/board_id — do NOT reply that you don't have a board open, and do NOT ask which app or board.
- When the user wants to SEE or USE an app's content/results in the conversation ("show me", "embed", "display here"), call `open_app_page` (for events marked kind "page" in `list_apps`) or `open_app_chat` (kind "chat") — these embed the app INLINE in the chat. `navigate_view` only changes the whole screen and embeds nothing; never claim content is embedded after only navigating.
- Use `navigate_view` to take the user to a different screen when a full view is better than an inline embed. Only use the documented routes — never invent paths.
- Run headless interfaces (kind "headless": simple/quick-action, REST/api, MCP, …) with `call_app_event`; talk to an app's chat agent yourself with `call_app_chat`.
- Building or editing workflow logic (nodes, connections, events) ALWAYS goes through `flowpilot_board`. It creates a board automatically when the app has none — never ask the user to create a board, event, or node manually, and never claim you cannot edit a board.
- `flowpilot_board` edits board CONTENTS only (nodes/events/logic) — it cannot create or rename apps or change app settings, and it does NOT build UI (that's `flowpilot_widget`). Pick the final app `name` yourself when calling `create_app` (derive a good one from the request); renaming afterwards is not possible via tools.
- Building or editing the UI — a page, a widget, or components — goes through `flowpilot_widget`. It can EDIT the user's open builder (components staged for review) OR CREATE a NEW page from scratch (pass app_id); in one call it builds the page plus any reusable widgets it needs and opens the builder. Board/workflow logic stays with `flowpilot_board`.
- Events are a DELIBERATE step you choose — never auto-created by other tools. Use `upsert_event` to create/update one and `delete_event` to remove it. A PAGE event makes a page reachable at a URL: pass page_id (the page) and a route (e.g. "/weather"). A NORMAL event is a workflow trigger: pass board_id + node_id (an events_* node). Creating a page with `flowpilot_widget` does NOT make it reachable — add a page event with a route when the user wants it visitable.
- To build a whole interface or app, ORDER MATTERS: `create_app` (if needed) → `flowpilot_widget` to create the page and its widgets FIRST → then `flowpilot_board` to wire the logic (it returns `event_nodes` — the events_simple nodes it created) → `set_page_load_event` to run one of those when the page opens (e.g. to load data) → `upsert_event` (page event with a route) so the page is reachable. Create the UI first because the workflow references it: nodes like widget-action events reference a widget's action, and navigation/onLoad reference a page — so the widgets and pages must exist before the board can point at them. When you then call `flowpilot_board`, include the created page name/route and the widget names + their action ids in the instruction so it wires the logic to the right targets. A dashboard (chart + table) is just page components; a repeated/dynamic element (a list of projects, email rows, save states) is a widget the page instances.
- Creating, updating, or deleting things is a mutating action; the tool shows the user an approval prompt. Never claim something is done until the tool returns success.
- Be concise and concrete. After an action, briefly state what you did and what changed.
- Use `internet_search` for general/public-web questions.
- If a tool needs information you do not have (e.g. which app), ask with `ask_user` rather than guessing.
- Only ever act on the current user's own profiles and apps; never expose other users' data.

Examples of good tool use:
- "Build a weather app with a page showing Munich's weather" → `create_app` (name: "Weather App") → `flowpilot_widget` (app_id from the result, instruction: "A weather page for Munich: a header, a large current-temperature card, and stat tiles for conditions, humidity and wind") → `flowpilot_board` (same app_id, instruction: "On page load, fetch current weather for Munich from a weather API and output temperature, conditions, humidity and wind for the page to display") — note the returned `event_nodes` (the created events_simple node) → `set_page_load_event` (app_id, page_id from flowpilot_widget, on_load_event_id: that node id) so the weather loads when the page opens → `upsert_event` (app_id, name: "Weather", page_id, route: "/weather") so the page is reachable → summarize. Call each tool ONCE, in this order; after a tool succeeds, move to the next step — never repeat a successful call.
- "Create an app that fetches RSS feeds daily" → `create_app` (name: "RSS Digest") → `flowpilot_board` (app_id from the create result, instruction: "Create a cron-triggered workflow that fetches these RSS feeds daily, deduplicates items and stores them in the app database") → summarize what was built.
- "Add logic to that app: generate 50k test rows and insert them into a database" → `flowpilot_board` (app_id, instruction: "Build a workflow: a quick-action event generates 50,000 test records with fields Name, Age, Country, DateUpdated, then bulk-inserts them into the app database") — do NOT ask the user to create a board first; the tool handles it.
- "Show me my briefings" → `list_apps` → the briefing event has kind "page" → `open_app_page`.
- "What's in my knowledge base about X?" → `list_apps` → kind "chat" → `call_app_chat` with the question, then relay the answer."#
        .to_string()
}

/// Render the open-board section injected into the assistant context. Kept separate so the wording
/// (which the model relies on to route board questions to `flowpilot_board`) lives in one place.
pub fn open_board_section(board: &GlobalOpenBoardContext) -> String {
    let app_id = board.app_id.trim();
    if app_id.is_empty() {
        return String::new();
    }
    let app_label = board
        .app_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(app_id);
    let board_label = board
        .board_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Untitled board");

    let mut lines = vec![
        "## CURRENTLY OPEN BOARD".to_string(),
        "The user has this board open and visible on screen right now. When they say \"this board\", \"this workflow\", \"this flow\", \"these nodes\", or ask to explain / edit / debug it, they mean THIS board — never ask which app or board.".to_string(),
        format!("- App: \"{app_label}\" (app_id: {app_id})"),
    ];
    match board.board_id.as_deref().map(str::trim) {
        Some(board_id) if !board_id.is_empty() => {
            lines.push(format!("- Board: \"{board_label}\" (board_id: {board_id})"));
        }
        _ => lines.push(format!("- Board: \"{board_label}\"")),
    }
    if let Some(layer) = board
        .current_layer
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("- Editing layer: {layer}"));
    }
    if let Some(count) = board.node_count {
        let selected = board.selected_node_ids.len();
        lines.push(if selected > 0 {
            format!("- {count} nodes ({selected} selected)")
        } else {
            format!("- {count} nodes")
        });
    } else if !board.selected_node_ids.is_empty() {
        lines.push(format!(
            "- {} nodes selected",
            board.selected_node_ids.len()
        ));
    }

    let board_arg = match board.board_id.as_deref().map(str::trim) {
        Some(board_id) if !board_id.is_empty() => format!(", board_id=\"{board_id}\""),
        _ => String::new(),
    };
    lines.push(format!(
        "To explain OR change this board, call flowpilot_board with app_id=\"{app_id}\"{board_arg} — use mode=\"explain\" to answer a question about it (read-only) and mode=\"edit\" to modify it. Do not answer board questions yourself."
    ));
    lines.join("\n")
}

/// Collect the self-awareness context for the global assistant: the signed-in user, the active
/// profile, the names of the user's other profiles, and — when a board is open — that board's
/// identity. Injected into the system prompt so the assistant knows where it is operating and which
/// board "board work" refers to. Host-neutral: callers supply the values as plain data.
pub fn build_platform_context(input: PlatformContextInput) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(user) = input.user_context.map(str::trim).filter(|v| !v.is_empty()) {
        parts.push(format!("Signed-in user: {user}."));
    }

    if let Some((name, id)) = input.active_profile {
        let name = name.trim();
        let name = if name.is_empty() {
            "Unnamed profile"
        } else {
            name
        };
        parts.push(format!("Active profile: \"{name}\" (id: {id})."));
    }

    let mut names: Vec<String> = input
        .switchable_profiles
        .iter()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();
    names.sort();
    if !names.is_empty() {
        parts.push(format!(
            "Profiles the user can switch to (by name): {}.",
            names.join(", ")
        ));
    }

    let mut sections: Vec<String> = Vec::new();
    if !parts.is_empty() {
        sections.push(format!(
            "## CURRENT FLOW-LIKE CONTEXT\n{}",
            parts.join("\n")
        ));
    }
    if let Some(board) = input.open_board {
        let section = open_board_section(board);
        if !section.is_empty() {
            sections.push(section);
        }
    }
    sections.join("\n\n")
}

/// Assemble the global assistant system prompt (base prompt + self-awareness `context`) and run the
/// Bits-backed [`PlatformCopilot`] loop. This is the single shared entry point both the desktop Tauri
/// command and the server HTTP endpoint call; the host supplies its own tool `bridge`, token sink
/// (`on_token`), `state`, and resolved `profile`. Returns the final assistant message.
#[allow(clippy::too_many_arguments)]
pub async fn run_platform_chat<F>(
    state: Arc<FlowLikeState>,
    profile: Option<Arc<Profile>>,
    context: String,
    user_prompt: String,
    current_images: Option<Vec<ChatImage>>,
    history: Vec<ChatMessage>,
    model_id: Option<String>,
    token: Option<String>,
    bridge: Arc<dyn PlatformToolBridge>,
    memory: Option<Arc<AssistantMemory>>,
    on_token: Option<F>,
) -> flow_like_types::Result<String>
where
    F: Fn(String) + Send + Sync + 'static,
{
    let system_prompt = if context.trim().is_empty() {
        global_assistant_system_prompt()
    } else {
        format!("{}\n\n{}", global_assistant_system_prompt(), context)
    };

    let assistant = PlatformCopilot::new(state, profile);
    assistant
        .chat(
            system_prompt,
            user_prompt,
            current_images,
            history,
            model_id,
            token,
            bridge,
            memory,
            on_token,
        )
        .await
}
