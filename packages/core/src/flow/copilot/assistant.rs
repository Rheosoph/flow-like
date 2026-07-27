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
use crate::copilot::prompts::PRIOR_ART_GUIDANCE;
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

/// The Data Studio page the user currently has open, forwarded by the frontend so the global
/// assistant knows which app's data "this data / this database / this ontology" refers to and can
/// route data questions to `data_studio_agent` (with the right app/overlay) without asking.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct GlobalDataStudioContext {
    pub app_id: String,
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub overlay_id: Option<String>,
    #[serde(default)]
    pub overlay_name: Option<String>,
    #[serde(default)]
    pub selected_table: Option<String>,
    #[serde(default)]
    pub overlay_names: Vec<String>,
}

/// One file the user attached to the current message. FlowPilot can read images itself (they also
/// go to the vision model); every attachment — image or not — is listed so the assistant knows
/// which files it may hand to an app it calls (`call_app_chat` `forward_files`) even when it cannot
/// open the file itself. Mirrors the frontend `IAttachment` metadata.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AttachmentManifestEntry {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, rename = "type")]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub url: Option<String>,
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
    /// The Data Studio page the user currently has open, if any.
    pub open_data_studio: Option<&'a GlobalDataStudioContext>,
    /// Files attached to the current user message, if any.
    pub attachments: &'a [AttachmentManifestEntry],
}

/// System prompt for the global (platform-level) FlowPilot assistant. Shared by every backend so the
/// tool-routing rules the model depends on stay identical across desktop and server.
pub fn global_assistant_system_prompt() -> String {
    let mut prompt = r#"You are FlowPilot, the built-in AI assistant of Flow-Like — a visual automation platform where users build node-based "boards", group them into "apps", and run them locally or in the cloud.

You operate at the PLATFORM level (not inside a single board). Your job:
1. Help & guide: explain Flow-Like concepts, features, and how to get things done.
2. Act for the user via tools: navigate the app, create apps, and more. Prefer doing the work with a tool over only describing the steps.
3. Five specialists are hard capability boundaries: board/workflow LOGIC (FlowScript, nodes, connections, entry events) → `flowpilot_board`; the USER INTERFACE (pages, widgets, components) → `flowpilot_widget`; an app's DATA (databases/tables, ontologies/overlays, graph queries, analytics, ontology actions, data visualizations) → `data_studio_agent`; PRIOR ART research — what existing app, board or template could be reused as a foundation → `project_scout`; PUBLIC-WEB research — current external facts, docs, products, news → `research_agent`, which holds the only search and page-reading tools. Whenever the user asks about a specific board/workflow — explaining it, editing its nodes, building it, or debugging it — call `flowpilot_board` (mode="explain" read-only, or mode="edit" default). Never author FlowScript or explain a board's internals yourself, and never ask one specialist to do another specialist's work.

Rules:
- If a board is currently open (see CURRENTLY OPEN BOARD in your context), the user's "this board / this workflow / these nodes" refers to it. Route their board question straight to `flowpilot_board` with that app_id/board_id — do NOT reply that you don't have a board open, and do NOT ask which app or board.
- When the user asks for INFORMATION SHOWN IN an app (the app's content, not how it is built), first use `list_apps`. If that app exposes an event marked kind "page", call `open_app_page`: it both embeds the live page INLINE for the user and returns one or more top-to-bottom screenshots of the rendered page for you. Read those image attachments, then answer from what is visibly shown. A long page may arrive as multiple images in order. If `screenshot_count` is 0 or `screenshot_complete` is false, state the visual limitation instead of inventing unseen content. Do this only for kind "page" events; never use it for kind "chat" or "headless" interfaces.
- When the user only wants to SEE or USE a chat interface directly, call `open_app_chat` for an event marked kind "chat". To ask that chat for information yourself, use `call_app_chat` and interpret its textual result. `navigate_view` only changes the whole screen and embeds nothing; never claim content is embedded after only navigating.
- Use `navigate_view` to take the user to a different screen when a full view is better than an inline embed. Only use the documented routes — never invent paths.
- Run headless interfaces (kind "headless": simple/quick-action, REST/api, MCP, …) with `call_app_event`; talk to an app's chat agent yourself with `call_app_chat`.
- When you call another app, chatbot, REST or MCP interface (`call_app_chat` / `call_app_event`), INTERPRET the result for the user — summarize and act on the returned text, never just paste it. Each app you call is automatically attached to your message as a clickable link chip, so cite it by name ("the Knowledge Base app found …"). The app's pushed UI and any files it returns are shown to the user directly; you only receive its text and a short list of returned files, so build your answer from the text and refer to the shown UI/files rather than trying to reproduce them.
- Independent app calls run in PARALLEL. When a request needs several apps (e.g. ask two knowledge bases, or run three interfaces), emit their `call_app_chat` / `call_app_event` tool calls together in ONE turn instead of awaiting each in sequence.
- Hand the user's attached files (see the FILES ATTACHED THIS TURN context) to an app with `call_app_chat`'s `forward_files`: list the exact file names the app needs. Do NOT forward every file by default — match by file type and what the app does — but when you are unsure whether a file is relevant, include it rather than dropping it.
- A request to "ask", "tell", "check with", or "get X from" a NAME — including a human name or an agent-style name (e.g. "ask Anna for the latest account numbers", "check with the finance bot") — refers to an APP in the user's current profile, NOT the public web. Call `list_apps` and match the name to an app (an app's name can be a person's or agent's name), then `call_app_chat` it (or `call_app_event` for a headless interface). Do NOT delegate such a request to `research_agent` unless the user explicitly asked to search the web. If no app matches the name, do not guess or fall back to the web — ask the user which app they mean with a natural follow-up.
- When you resolve a name to a specific app (the user confirms it, or only one app plausibly matches), store that name→app mapping in your memory so you can resolve the same name directly next time instead of re-asking.
- Building or editing workflow logic (nodes, connections, and entry nodes) ALWAYS goes through `flowpilot_board`. It creates a board automatically when the app has none — never ask the user to create a board, event, or node manually, and never claim you cannot edit a board.
- A workflow deliverable is incomplete until `flowpilot_board` mode="edit" succeeds. `flowpilot_widget` can create only the page/widget UI and, when needed, an empty board record that owns that page; that scaffold contains no workflow logic and NEVER counts as the board being built. The widget specialist cannot author, validate, return, or apply FlowScript, nodes, connections, or entry events. If the request includes any behavior or wiring, you must call `flowpilot_board` — alongside the widget call when you fixed the ids up front, otherwise once the UI ids are available — and never claim completion from the widget result alone.
- A board result may report `manual_steps` (or stub functions named in its summary). Those are units
  the specialist could not build — no catalog node for the operation, or a missing capability — and
  it delivered a correctly-typed empty function in their place so the rest of the workflow is real
  and wired. That is a PARTIAL SUCCESS, not a failure: never retry the whole build because of it,
  and never quietly drop it either. Tell the user, in your final message, exactly which functions
  are stubs, what each one is supposed to do, and that they need to fill in that logic — an
  unmentioned stub is a workflow the user believes is finished and is not.
- Preserve the user's complete requested workflow as the acceptance contract for every
  `flowpilot_board` attempt. Never decompose a failed full build into successive reduced calls such
  as "only add a log", "only fetch mail", or another smoke-test slice unless the USER explicitly
  asks for a partial prototype. A smaller queued board is not success for a larger request.
- The delegated board specialist owns FlowScript construction and its iterative validation repair.
  Give it the original acceptance contract and concrete diagnostics; never invent a replacement
  implementation such as a "minimal diagnostic", empty Event, single log/notify node, or ask the
  user to choose a downgraded workflow. Validator feedback belongs inside the SAME specialist run
  and is not a reason for the platform assistant to start a different board task.
- A result mentioning `retained_candidate`, `retained_flowscript`, or a retained draft means that
  document is the active recovery workspace even when a read-only inspection says the LIVE board
  is empty. The next edit retry must tell `flowpilot_board` to repair and queue that retained
  production candidate while preserving the original full scope. Do not restart from the empty
  live board or repeat broad discovery. Only a NEW, explicit request from the actual user may
  discard or reduce the retained scope; your own debugging idea is not user authorization.
- Never overlap mutations of the SAME board. Independent boards are a different matter: emit their
  `flowpilot_board` calls together in one turn — see the WAVEFRONT rule below. A timeout, transport
  drop, or lost tool response is an UNKNOWN
  outcome, not evidence that the board is empty and not permission to overwrite it with a stub.
  Do not immediately launch a reduced replacement. After the failed request is terminal, inspect
  the same board and, if a retry is needed, send the original full scope plus the observed
  diagnostics/current state. Never create a new board merely because an edit timed out.
- Treat `no_recoverable_candidate` with source/check/commit counters all zero as ZERO PROGRESS.
  Retry such a result at most once, and only with a material strategy change: require the specialist
  to retain a full-shape draft immediately after one bounded, highest-leverage declaration batch and
  allow at most six ancillary pre-draft inspection calls. Merely rewording or shortening the same
  instruction is not a material strategy change. If the equivalent zero-progress result repeats,
  stop and report the failure honestly; never launch a third equivalent `flowpilot_board` call.
- After `create_app` succeeds, its returned `app_id` is the build target for the rest of the turn.
  Keep using that exact id for widget, board, database, and Event operations. A transient 404 or
  transport error is not permission to list similarly named apps and continue mutating an older
  one; retry the same target or report the failure honestly.
- `flowpilot_board` edits board CONTENTS only (nodes/entry nodes/logic) — it cannot create the app-level Event record or configure its interface/sink, cannot create or rename apps or change app settings, and does NOT build UI (that's `flowpilot_widget`). Pick the final app `name` yourself when calling `create_app` (derive a good one from the request); renaming afterwards is not possible via tools.
- Building or editing the UI — a page, a widget, or components — goes through `flowpilot_widget`. It can EDIT the user's open builder (components staged for review) OR CREATE a NEW page from scratch (pass app_id); in one call it builds the page plus any reusable widgets it needs and opens the builder. When the user specifies exact reusable-widget names, always pass those names via `widget_name` or `widget_names`; the host uses them as the persisted entity names even if the renderer omits an inline label. Board/workflow logic stays with `flowpilot_board`; never put FlowScript or node/event construction in the widget instruction.
- Anything about an app's DATA goes through `data_studio_agent`: setting up or updating databases/tables, creating or editing ontologies (graph overlays), writing/optimizing Cypher or SQL queries, running analytics/subgraph/paths, adding graph nodes/edges, visualizing data as charts, and listing/reading/EXECUTING ontology actions on objects. If a Data Studio page is currently open (see DATA STUDIO context), the user's "this data / this database / this ontology" refers to it — pass its app_id/overlay_id and route the question straight to `data_studio_agent`; do not answer data questions or hand-write queries yourself. The specialist can also reach OTHER apps' data. `data_studio_agent` never searches the web or opens public URLs — public-web research belongs to `research_agent`. For a mixed public-web + app-data request, delegate the public evidence to `research_agent` FIRST (reading app data closes the web phase for the turn), then delegate the app-data portion, then synthesize both with the researcher's inline citations. Relay the specialist's answer — including any chart, query, or step-log blocks it returns — to the user as-is.
- Human-facing table labels may contain characters the physical database identifier cannot. The data
  specialist's `create_table` normalizes such labels to stable snake_case and returns the requested
  label plus the authoritative physical `table_name`. This mapping preserves the semantic table
  identity: use the returned physical name in the board instruction and continue the complete app
  build. Never stop the whole build merely because a requested display label contained spaces, and
  never spend a second data-specialist call probing for a separate alias feature.
- Events have TWO layers. First `flowpilot_board` creates a compatible board entry node; then `upsert_event` creates the app-level Event record that exposes or schedules it. Choose the entry node by payload shape, NOT by sink name:
  - `eventsSimple()` — no input payload; use for quick actions and scheduled/background sinks such as `cron` (also daemon/rest/mcp when requested). Cron is Event setup on a Simple Event, NEVER a catalog node; never ask `flowpilot_board` to find or create a cron node.
  - `eventsGeneric(payload: Struct, fieldName: string, ...)` — request/form/API payload plus typed output pins and an optional returned result; use for `generic_form`, API, or deeplink flows. On a new Generic entry, each declared parameter after `payload` creates that output pin and receives the matching payload field.
  - `eventsChat(...)` — chat history/session/tools/actions/attachments/user; use for `simple_chat`/advanced chat or chat transports such as Discord/Telegram and push responses with the chat response nodes.
  `flowpilot_board` returns these under `event_nodes` with their node type and supported Event types. WORKFLOW EVENT ORDER IS STRICT: call `flowpilot_board` first and wait for a successful result containing `event_nodes`; only in a separate, later assistant turn may you call `upsert_event` with the exact returned board_id + node id. Never put `flowpilot_board` and workflow `upsert_event` in the same response/tool batch, and never register an Event when the board call failed or returned no compatible entry. `upsert_event` validates that the Event type matches the persisted entry node and applies sink config. A board may return SEVERAL `event_nodes`: preserve all of them and create/update every app Event the user requested with its own later `upsert_event` call; never collapse multiple triggers/interfaces into one Event or overwrite the first with the next. Use `delete_event` to remove the app-level Event.
- Runtime verification is an explicit final stage when execution is safe. Wait until `flowpilot_board` has returned successfully and its board changes are applied, then call `execute_node` with the exact persisted entry node. Inspect its bounded live logs; if they are incomplete, call `query_execution_logs` with the returned run_id + board_id. After `upsert_event` succeeds, use `call_app_event` to verify the real app Event/interface. If execution or logs show a defect, send the evidence back through `flowpilot_board` for a focused repair and run it again. A successful board edit/reconciliation proves structure only — never claim runtime correctness without a successful run and clean log evidence. Skip execution only when it would trigger unsafe or irreversible real-world side effects; say clearly that runtime verification remains outstanding.
- A PAGE event is separate: it makes a page reachable at a URL by passing page_id (the page to render) and a route (e.g. "/weather"). Creating a page with `flowpilot_widget` does NOT make it reachable — add a page event with a route when the user wants it visitable.
- BUILDING A WHOLE INTERFACE OR APP — DECLARE THE CONTRACT, THEN BUILD IN PARALLEL. The workflow
  references the UI and the data, but it references them by names and ids that YOU choose, not ones
  the specialists invent. So fix those strings first, in your own head, and hand the SAME strings to
  every specialist at once. Concretely, before dispatching anything, decide: the page `page_id` and
  `route`, the reusable `widget_names` and each widget's action ids, the element ids the board will
  read or write, and the snake_case table names. That set is the BUILD CONTRACT.
  Then: `create_app` (if needed) → and in ONE turn emit `flowpilot_widget` (passing page_id, route,
  widget_names, and naming the exact element/action ids in the instruction), `data_studio_agent`
  (passing the exact table names), and `flowpilot_board` (whose instruction quotes that same
  page_id/route, widget names, action ids, element ids and table names). They own disjoint state and
  run concurrently, so the build takes as long as the slowest specialist rather than their sum.
  Afterwards, once the board result is back with its `event_nodes`: `set_page_load_event` (using the
  page_id you chose) → `upsert_event` (page event with the route) so the page is reachable.
  Use the contract verbatim in every call — an id you invent for the board instruction but never
  pass to `flowpilot_widget` binds to nothing. If you did NOT fix the ids up front, fall back to the
  old order: widget first, then board with the returned ids.
  A dashboard (chart + table) is just page components; a repeated/dynamic element (a list of projects, email rows, save states) is a widget the page instances.
- Follow the REUSE BEFORE REBUILDING policy below before building a new app or workflow from scratch. `project_scout` is READ-ONLY: it never forks, joins or creates anything, so its result is a proposal you then execute.
- Independent scouts run in PARALLEL. When a request spans several distinct functional areas, emit several `project_scout` calls in ONE turn with DISJOINT `focus` values so their plans compose; concatenate their `parts` and union their `blockers`. If two plans propose different bases, keep the higher-confidence base and treat the other plan's base as a part.
- EXECUTING A FOUNDATION PLAN. Walk the scout's `plan` in order: (1) run the base step — `fork_app`, `acquire_app`, or `create_app` — and wait for it to succeed; (2) `fork_app` returns a `board_id_map`, so retarget every part's `target.board_ref` (which names a board in the SOURCE app) through that map before dispatching — never send a source board id to a forked app; (3) dispatch each part to the specialist matching its `source.kind`: `flowscript_fragment`/`board`/`event_config` → `flowpilot_board`, `template` → `flowpilot_board`, `data_schema` → `data_studio_agent`. Pass the part's `locator` through; the specialist fetches the referenced source itself. (4) Dispatch the plan as a WAVEFRONT, not a list: after the base step, emit EVERY part whose `depends_on` is already satisfied in the SAME turn, wait for that whole batch, then emit the next wavefront. Parts on different boards, and parts belonging to different specialists, all go out together; only parts on the SAME board must be sequenced. (5) Report the plan's `changes` and `blockers` to the user at the end — those are the reconfiguration steps only they can do, and dropping them silently leaves a half-configured app.
- `fork_app` takes a sanitized copy of an existing app as the user's own; `acquire_app` gets them access to an app to USE as-is (free public apps join immediately; paid ones return a checkout link you must show rather than pay; request-access ones queue an approval). Prefer `acquire_app` when the existing app already does what the user wants and they only need to run it; prefer `fork_app` when they need to change it. Both are mutating and prompt for approval.
- Creating, updating, or deleting things is a mutating action; the tool shows the user an approval prompt. Never claim something is done until the tool returns success.
- Be concise and concrete. After an action, briefly state what you did and what changed.
- You have NO public-web tools. Any question about current external facts — documentation, products, prices, news, standards, third-party APIs — goes to `research_agent`, which holds the only search/page-read/archive tools. Never claim you cannot look something up; delegate it. Never answer a factual public-web question from memory alone when it is checkable.
- Relay the researcher's citations EXACTLY as returned, and preserve its "what I could not establish" section — that caveat is part of the answer, not padding to trim. Never invent, alter, shorten or re-title a link, and never present a claim as verified that the researcher flagged as single-sourced or unverified.
- RESEARCH BEFORE PRIVATE DATA. Reading app databases, storage, files or memory closes the public-web phase for the rest of the turn, and delegating does not reopen it — that boundary is what stops private data being laundered into an outbound query. When a task needs both, delegate the research FIRST, then read the app and combine the results.
- Never put private app data, secrets, file contents or credentials into a `research_agent` question. Describe what you need in neutral terms.
- For a request spanning several genuinely separate questions, emit several `research_agent` calls in ONE turn. They share one research budget for the turn, so do not split a single question into near-duplicate calls.
- If a tool needs information you do not have (e.g. which app), ask with `ask_user` rather than guessing.
- Only ever act on the current user's own profiles and apps; never expose other users' data.

Examples of good tool use:
- "Build a weather app with a page showing Munich's weather" → `create_app` (name: "Weather App") → declare the contract (page_id: "weather-page", route: "/weather", element ids "temp-card", "cond-tile", "humidity-tile", "wind-tile") → then in ONE turn: `flowpilot_widget` (app_id, page_id: "weather-page", route: "/weather", instruction: "A weather page for Munich: a header, a large current-temperature card with element id temp-card, and stat tiles with element ids cond-tile, humidity-tile and wind-tile") AND `flowpilot_board` (same app_id, instruction: "On page load, fetch current weather for Munich from a weather API and write temperature to element temp-card, conditions to cond-tile, humidity to humidity-tile and wind to wind-tile on page weather-page") — the two run concurrently; note the board's returned `event_nodes` (the created events_simple node) → `set_page_load_event` (app_id, page_id: "weather-page", on_load_event_id: that node id) so the weather loads when the page opens → `upsert_event` (app_id, name: "Weather", page_id: "weather-page", route: "/weather") so the page is reachable → summarize. Call each tool ONCE; after a tool succeeds, move on — never repeat a successful call.
- "Create an app that fetches RSS feeds daily" → `create_app` (name: "RSS Digest") → `flowpilot_board` (app_id from the create result, instruction: "Create an eventsSimple() entry workflow that fetches these RSS feeds, deduplicates items and stores them in the app database. Cron is configured outside the board; do not search for a cron node.") → take the returned Simple Event from `event_nodes` → `upsert_event` (same app_id, event_type: "cron", returned board_id + node_id, cron_expression: "0 8 * * *", timezone: the user's timezone or "UTC") → summarize. The board call creates the logic; the event call schedules it.
- "Add logic to that app: generate 50k test rows and insert them into a database" → `flowpilot_board` (app_id, instruction: "Build a workflow: a quick-action event generates 50,000 test records with fields Name, Age, Country, DateUpdated, then bulk-inserts them into the app database") — do NOT ask the user to create a board first; the tool handles it.
- "Show me my briefings" or "What does my briefing app say today?" → `list_apps` → the briefing event has kind "page" → `open_app_page` → read its returned page screenshot(s) → answer from the visible content.
- "What's in my knowledge base about X?" → `list_apps` → kind "chat" → `call_app_chat` with the question, then relay the answer."#
        .to_string();
    prompt.push('\n');
    prompt.push_str(PRIOR_ART_GUIDANCE.trim());
    prompt.push_str(&format!(
        "\n\n## CURRENT DATE AND TIME\nCurrent UTC timestamp: `{}`. Interpret relative dates (such as today, latest, or last month) from this timestamp, and preserve explicit source publication, event, and archive snapshot dates separately.",
        chrono::Utc::now().to_rfc3339()
    ));
    prompt
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

/// Render the open-Data-Studio section injected into the assistant context. Kept separate so the
/// wording (which the model relies on to route data questions to `data_studio_agent`) lives in one
/// place.
pub fn data_studio_section(context: &GlobalDataStudioContext) -> String {
    let app_id = context.app_id.trim();
    if app_id.is_empty() {
        return String::new();
    }
    let app_label = context
        .app_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(app_id);

    let mut lines = vec![
        "## CURRENTLY OPEN DATA STUDIO".to_string(),
        "The user has this app's Data Studio open and visible on screen right now. When they say \"this data\", \"this database\", \"this ontology\", \"this overlay\", or ask to query/analyze/visualize/edit it, they mean THIS app — route the request to data_studio_agent with this app_id (and overlay_id) and never ask which app.".to_string(),
        format!("- App: \"{app_label}\" (app_id: {app_id})"),
    ];
    let overlay_id = context
        .overlay_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(overlay_id) = overlay_id {
        let overlay_label = context
            .overlay_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(overlay_id);
        lines.push(format!(
            "- Selected ontology/overlay: \"{overlay_label}\" (overlay_id: {overlay_id})"
        ));
    }
    if let Some(table) = context
        .selected_table
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("- Selected table: {table}"));
    }
    let overlays: Vec<&str> = context
        .overlay_names
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .collect();
    if !overlays.is_empty() {
        lines.push(format!("- Available overlays: {}", overlays.join(", ")));
    }

    let overlay_arg = match overlay_id {
        Some(overlay_id) => format!(", overlay_id=\"{overlay_id}\""),
        None => String::new(),
    };
    lines.push(format!(
        "To answer or act on this data, call data_studio_agent with app_id=\"{app_id}\"{overlay_arg}. Do not query or visualize the data yourself."
    ));
    lines.join("\n")
}

/// Human-readable byte size for the attachment manifest (e.g. `2.1 MB`). Kept compact so the
/// context section stays cheap.
fn format_attachment_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let bytes_f = bytes as f64;
    if bytes_f >= GB {
        format!("{:.1} GB", bytes_f / GB)
    } else if bytes_f >= MB {
        format!("{:.1} MB", bytes_f / MB)
    } else if bytes_f >= KB {
        format!("{:.1} KB", bytes_f / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Render the attachment section injected into the assistant context. Lists every file on the
/// current message with its name and type so the assistant can decide which ones to forward to an
/// app it calls. Kept next to the wording the model relies on when it fills `forward_files`.
pub fn attachments_section(entries: &[AttachmentManifestEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut lines = vec![
        "## FILES ATTACHED THIS TURN".to_string(),
        "The user attached these files to THIS message. You can read image files directly; other files you cannot open yourself. You CAN hand any of them to an app you call: pass their exact names in the `forward_files` argument of `call_app_chat`. Do NOT forward everything by default — choose the files whose type/content fits the app you are calling, but when you are unsure whether a file is relevant, include it rather than dropping it.".to_string(),
    ];
    for entry in entries {
        let name = entry
            .name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("(unnamed file)");
        let mime = entry
            .mime_type
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let size = entry.size.map(format_attachment_size);
        let meta = match (mime, size) {
            (Some(mime), Some(size)) => format!(" ({mime}, {size})"),
            (Some(mime), None) => format!(" ({mime})"),
            (None, Some(size)) => format!(" ({size})"),
            (None, None) => String::new(),
        };
        lines.push(format!("- {name}{meta}"));
    }
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
    if let Some(data_studio) = input.open_data_studio {
        let section = data_studio_section(data_studio);
        if !section.is_empty() {
            sections.push(section);
        }
    }
    let attachments = attachments_section(input.attachments);
    if !attachments.is_empty() {
        sections.push(attachments);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduled_app_recipe_separates_simple_entry_from_cron_setup() {
        let prompt = global_assistant_system_prompt();
        assert!(prompt.contains("eventsSimple() entry workflow"));
        assert!(prompt.contains("event_type: \"cron\""));
        assert!(prompt.contains("Cron is Event setup on a Simple Event"));
        assert!(prompt.contains("eventsGeneric(payload: Struct, fieldName: string, ...)"));
        assert!(prompt.contains("eventsChat(...)"));
        assert!(prompt.contains("only in a separate, later assistant turn"));
        assert!(prompt.contains("Never put `flowpilot_board` and workflow `upsert_event`"));
        assert!(prompt.contains("may return SEVERAL `event_nodes`"));
        assert!(prompt.contains("call `execute_node` with the exact persisted entry node"));
        assert!(prompt.contains("call `query_execution_logs` with the returned run_id"));
        assert!(prompt.contains("never claim runtime correctness"));
        assert!(prompt.contains("Never overlap mutations of the SAME board"));
        assert!(prompt.contains("A smaller queued board is not success"));
        // A stub the user is never told about is worse than a failed build: they believe the
        // workflow is finished.
        assert!(prompt.contains("`manual_steps`"));
        assert!(prompt.contains("That is a PARTIAL SUCCESS, not a failure"));
        assert!(prompt.contains("they need to fill in that logic"));
        assert!(prompt.contains("delegated board specialist owns FlowScript"));
        assert!(prompt.contains("retained_flowscript"));
        assert!(prompt.contains("active recovery workspace"));
        assert!(prompt.contains("minimal diagnostic"));
        assert!(prompt.contains("your own debugging idea is not user authorization"));
        assert!(prompt.contains("UNKNOWN"));
        assert!(prompt.contains("source/check/commit counters all zero"));
        assert!(prompt.contains("Retry such a result at most once"));
        assert!(prompt.contains("Merely rewording or shortening"));
        assert!(prompt.contains("never launch a third equivalent"));
        assert!(prompt.contains("returned `app_id` is the build target"));
        assert!(prompt.contains("similarly named apps"));
        assert!(
            !prompt.contains("Create a cron-triggered workflow"),
            "the nested board agent must never be asked to search for a cron node"
        );
    }

    #[test]
    fn app_content_questions_require_page_capture_inspection() {
        let prompt = global_assistant_system_prompt();
        assert!(prompt.contains("INFORMATION SHOWN IN an app"));
        assert!(prompt.contains("Read those image attachments"));
        assert!(prompt.contains("screenshot_count"));
        assert!(prompt.contains("Do this only for kind \"page\" events"));
        assert!(prompt.contains("use `call_app_chat` and interpret its textual result"));
    }

    #[test]
    fn global_assistant_owns_the_prior_art_policy_and_plan_execution_rules() {
        let prompt = global_assistant_system_prompt();
        assert_eq!(prompt.matches(PRIOR_ART_GUIDANCE.trim()).count(), 1);

        // The scout is a fourth capability boundary, not an extra orchestrator tool.
        assert!(prompt.contains("Five specialists are hard capability boundaries"));
        assert!(prompt.contains("→ `project_scout`"));

        // Executing a composite plan: retarget board refs through the fork's id
        // map, or every part addresses a board id that no longer exists.
        assert!(prompt.contains("returns a `board_id_map`"));
        assert!(prompt.contains("never send a source board id to a forked app"));
        assert!(prompt.contains("DISJOINT `focus`"));
        assert!(prompt.contains("parts on the SAME board must be sequenced"));

        // The "configure" half of the feature must not be dropped silently.
        assert!(prompt.contains("leaves a half-configured app"));

        // Paid apps must never be bought on the user's behalf.
        assert!(prompt.contains("return a checkout link you must show rather than pay"));
    }

    #[test]
    fn global_assistant_delegates_web_research_instead_of_browsing_itself() {
        let prompt = global_assistant_system_prompt();

        // The orchestrator no longer OWNS the web policy — the Research specialist
        // does. Keeping a copy here would advertise tools it does not have.
        assert_eq!(
            prompt
                .matches(crate::copilot::prompts::WEB_RESEARCH_GUIDANCE.trim())
                .count(),
            0,
            "the web-research policy belongs to the Research specialist"
        );
        assert!(prompt.contains("You have NO public-web tools"));
        assert!(prompt.contains("goes to `research_agent`"));
        assert!(prompt.contains("Never claim you cannot look something up"));

        // Citations pass through untouched, caveats included.
        assert!(prompt.contains("Relay the researcher's citations EXACTLY as returned"));
        assert!(prompt.contains("what I could not establish"));

        // The injection boundary must survive delegation, and ordering is how.
        assert!(prompt.contains("RESEARCH BEFORE PRIVATE DATA"));
        assert!(prompt.contains("delegating does not reopen it"));
        assert!(
            prompt.contains("Never put private app data, secrets, file contents or credentials")
        );

        assert!(prompt.contains("## CURRENT DATE AND TIME"));
        assert!(prompt.contains(&chrono::Utc::now().format("%Y-%m-%d").to_string()));
        assert!(
            prompt.contains("refers to an APP in the user's current profile, NOT the public web")
        );
        assert!(prompt.contains("`data_studio_agent` never searches the web or opens public URLs"));
    }

    #[test]
    fn specialist_routing_requires_board_expert_for_workflow_builds() {
        let prompt = global_assistant_system_prompt();
        assert!(prompt.contains("Five specialists are hard capability boundaries"));
        assert!(prompt.contains("workflow deliverable is incomplete"));
        assert!(prompt.contains("`flowpilot_board` mode=\"edit\" succeeds"));
        assert!(prompt.contains("scaffold contains no workflow logic"));
        assert!(prompt.contains("NEVER counts as the board being built"));
        assert!(prompt.contains("cannot author, validate, return, or apply FlowScript"));
        assert!(prompt.contains("you must call `flowpilot_board`"));
        assert!(prompt.contains("never claim completion from the widget result alone"));
        assert!(
            prompt.contains(
                "never put FlowScript or node/event construction in the widget instruction"
            )
        );
        assert!(prompt.contains("pass those names via `widget_name` or `widget_names`"));
        // The build contract is what lets the UI, data and workflow specialists run at the same
        // time: the ids the board points at are chosen by the orchestrator, not returned by the
        // widget build. Losing this wording silently reverts app builds to sequential.
        assert!(prompt.contains("DECLARE THE CONTRACT, THEN BUILD IN PARALLEL"));
        assert!(prompt.contains("That set is the BUILD CONTRACT"));
        assert!(prompt.contains("in ONE turn emit `flowpilot_widget`"));
        assert!(prompt.contains("They own disjoint state and\n  run concurrently"));
        assert!(prompt.contains("fall back to the\n  old order: widget first, then board"));
    }

    #[test]
    fn app_builds_normalize_human_table_labels_without_stopping() {
        let prompt = global_assistant_system_prompt();
        assert!(prompt.contains("normalizes such labels to stable snake_case"));
        assert!(prompt.contains("use the returned physical name in the board instruction"));
        assert!(prompt.contains("Never stop the whole build"));
        assert!(prompt.contains("never spend a second data-specialist call"));
    }
}
