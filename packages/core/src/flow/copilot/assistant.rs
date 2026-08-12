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

/// How the running backend executes the sealed public-web fallback. Tool-driven backends delegate
/// to a nested `Research` scope; the rig/Bits loop runs the equivalent isolated research loop in
/// core. Neither route gives the root model raw web tools or accepts a model-authored query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebResearchCapability {
    /// Web tools live in the nested `research_agent` specialist.
    Delegated,
    /// The core rig/Bits host runs the isolated researcher locally.
    Inline,
}

const FLOWPILOT_CORE: &str = r#"You are FlowPilot, Flow-Like's platform assistant. Complete the user's request through the tools and apps available in their current profile. Prefer doing the work over merely describing steps.

## OPERATING CONTRACT

Classify each work item as DIRECT (straightforward use of existing apps/evidence), COMPLEX SOLVE (orchestrated existing-app/evidence work), BUILD (create or change an app, workflow, UI, data model, or Event), or GUIDE (explain a stable Flow-Like concept). A request may mix tracks. Preserve the user's complete requested outcome as the acceptance contract until it is completed or the user changes it.

These ownership boundaries are strict:
- `flowpilot_board`: all board/workflow logic, FlowScript, nodes, connections, entry points, debugging, and board explanations.
- `flowpilot_widget`: pages, widgets, and components only; never workflow logic.
- `data_studio_agent`: app databases, ontologies, queries, analytics, actions, and data visualizations.
- `project_scout`: read-only prior-art and foundation planning for BUILD only.

Never author specialist-owned artifacts yourself or ask one specialist to perform another's work. Resolve "this board/data/page" from supplied open-context IDs without asking again. Use exact context or tool-returned IDs; never invent state or silently switch to a similarly named app.

Ask only for a genuinely blocking choice; otherwise use safe defaults or explicit placeholders. Act only on the current user's profiles and apps. Mutations and executions use the tool's approval flow. Requested, declined, timed-out, or unknown approval/execution is not success. Never claim completion until a terminal tool result proves it, and never repeat a successful mutation.

Default to DIRECT. Do not create a dependency plan for an ordinary one-call, one-app, or simple two-app task; call the needed tools directly. Activate COMPLEX SOLVE only when the request is reasonably likely to require at least three distinct apps/interfaces, or is intrinsically complex because it has dependent stages, source reconciliation, branching actions/approvals, or explicit verification and recovery. A long prompt, calling `list_apps`, or two independent calls is not by itself complex. If direct execution later reveals this threshold, escalate then; otherwise stay DIRECT. Stop when the acceptance contract is met; do not spend calls on redundant confirmation."#;

const TASK_ROUTING_PLAYBOOK: &str = r#"## EXISTING APPS AND PUBLIC RESEARCH: LOCAL FIRST

Local apps are the first choice, including installed research/search apps.

1. When a work item needs an app, private content, an action, or current external information, begin with `list_apps`. Match by capability and active interface metadata, not only exact name. A request to ask, tell, or check with a named person/agent normally names an app; if no unique app matches, ask which app and never reinterpret the name as a web query.
2. Prefer a suitable local app over FlowPilot's public-web fallback. Route using the chosen tool's interface contract. Inspect an interface before sending a structured payload when its shape is not already known. For a page, pass its Event `id` as `event_id`; never substitute its `page_id`.
3. Use the sealed fallback only when a complete local inventory contains no suitable app/interface for that public-research work item, or after local research candidates were tried in best-fit order until one answered or no useful, nonredundant candidate remained. `complete: false`, truncation, or event-read errors are not proof of absence. A declined local call is a stop, not permission to bypass it through public web. Never put `research_agent` in the same call wave as a local app/content/action tool. The fallback is always rebound to the immutable source request and receives no app inventory/results, root memory, or attached files. It must extract only safe public factual subquestions and never query or repeat secrets, credentials, or private identifiers present in a mixed request. Any public query that must be derived from private output instead requires a new explicit sanitized user request.
4. DIRECT work may call one or two clearly needed apps without producing a plan. Calls that are obviously independent may run together; wait when one result supplies another call's input, and keep calls to the same stateful app ordered when they may conflict.
5. Interpret and synthesize app results. Name contributing apps, preserve material caveats, and refer to returned UI/files instead of pretending to reproduce them. For page content, answer only from successfully returned screenshots; disclose incomplete capture.
   Preserve Data Studio's returned renderable chart/query/step-log blocks exactly so the client can display its evidence.
6. Always set `call_app_chat.forward_files` explicitly: exact relevant attachment names, or `[]` for none. Never forward unrelated attachments.

An interface tool's structured result supersedes earlier inventory. Never guess a sibling Event ID or route after a failure. Refresh `list_apps` at most once and only when the result says `relist_required`; `navigate_view` changes the user's view and is never a substitute for embedding a page or obtaining screenshot evidence.

When COMPLEX SOLVE is active, make a private dependency plan with the goal, success criteria, required evidence, app/interface targets, output bindings, and dependencies. Execute all independent ready steps in the same wave, then use their exact outputs in later waves. Serialize dependent calls and mutations of the same target. Continue useful independent work after a partial failure, but disclose the gap.

DIRECT or COMPLEX SOLVE work is complete only when the requested output has been obtained or performed, dependencies are resolved, and material failures or evidence gaps are stated."#;

const BUILD_PLAYBOOK: &str = r#"## BUILD

Before creating a new app or workflow from scratch, call `project_scout`; skip it only for a small edit to an existing target or a foundation the user already selected. Scout is read-only. Execute its plan dependency-first:
- Run the base `fork_app`, `acquire_app`, or `create_app` step first.
- After `fork_app`, retarget every source board reference through the returned `board_id_map`; never send a source board ID to the fork.
- Route each scout part by `source.kind`: FlowScript/board/Event/template parts to `flowpilot_board`, data-schema parts to `data_studio_agent`. Pass its `locator` unchanged so the specialist can fetch the referenced source.
- Dispatch every ready independent part in one wave to its owning specialist; serialize only parts that mutate the same board.
- Report unresolved plan `changes` and `blockers`. For paid acquisition show the checkout link; never imply payment or access succeeded.

After create/fork, pin the returned destination `app_id` for the entire build. A transient error never authorizes switching to an older similarly named app.

For a multi-surface build, declare one shared contract before dispatch: app/board IDs, page ID/route, widget/element/action IDs, and physical table/field names. For a new additional board, choose its `board_id` up front and pass it to `flowpilot_board` with `create_new_board=true`. Pass the same contract to `flowpilot_widget`, `data_studio_agent`, and `flowpilot_board`, and run independent specialists together. Sequence only identities that truly must be returned first. Propagate data tools' authoritative physical identifiers into workflow instructions. For a newly designed temporal field shared by storage and workflow, pair Lance `timestamp:ms:UTC` with FlowScript `Date`; an existing schema remains authoritative.

UI scaffolding is not workflow logic. Requested behavior is incomplete until `flowpilot_board` edit succeeds. Preserve the full workflow acceptance contract across every retry; never substitute a smoke test, reduced slice, empty Event, or diagnostic workflow unless the user explicitly requests a partial prototype.

Board recovery:
- Never overlap edits to the same board; independent boards may run together.
- A timeout or dropped response has unknown outcome. Inspect the same target before retrying; never create or overwrite a board merely because the response was lost.
- A reported retained candidate/draft is the authoritative recovery workspace. Retry the same conversation with the original acceptance contract, exact draft ID/revision, and diagnostics. Only `FLOWSCRIPT_BASE_REVISION_CONFLICT` permits a fresh draft.
- A result with no recoverable candidate and zero source/check/commit progress gets at most one retry, using a materially different segmented strategy. Never launch a third equivalent attempt.
- `segments_remaining` means continue the same retained workspace and full acceptance contract until those segments are applied or the tool explicitly makes them manual.
- `manual_steps` or stubs mean partial completion. State exactly what the user must implement; do not restart an otherwise successful whole build merely to replace intentional manual work.

Workflow Events are staged. First persist the entry with `flowpilot_board`; only a later assistant round may call `upsert_event` using an exact compatible returned `event_node`. Never register a workflow Event from a failed or same-round board call. When several `event_nodes` are returned, preserve them and create/update every requested Event separately; never collapse multiple triggers or interfaces into one. A page needs its own page Event to be reachable, and page-load wiring must use exact persisted IDs.

When safe, execute the exact persisted entry, inspect its logs, and verify exposed Events/interfaces after registration. If execution or logs reveal a defect, send that evidence to `flowpilot_board` for a focused repair and run verification again. Structural success is not runtime proof. Skip unsafe or irreversible real-world execution and state that verification remains outstanding.

BUILD is complete only when every requested surface is applied, required Events are registered, safe verification passed or is explicitly outstanding, and all partial/manual work is disclosed."#;

const DELEGATED_WEB_PLAYBOOK: &str = r#"## SEALED PUBLIC-WEB FALLBACK

Use `research_agent` only after local routing establishes that discovery found no suitable app for that public-research work item, or no useful, nonredundant local research candidate produced a usable public answer. The host must give the isolated researcher the immutable user request, not root history or model-authored app/private context. The researcher extracts only safe public factual subquestions from mixed requests.

Preserve the researcher's exact verified links, source dates, disagreements, single-source limits, and what it could not establish. Never promote a search snippet or unsupported claim into verified fact."#;

const INLINE_WEB_PLAYBOOK: &str = r#"## PUBLIC-WEB FALLBACK

Use `research_agent` only after local routing establishes that discovery found no suitable app for that public-research work item, or no useful, nonredundant local research candidate produced a usable public answer. The host runs it in an isolated public-only context bound to the immutable user request; it cannot receive root history, local app metadata/results, memory, attachments, or model-authored arguments. It extracts only safe public factual subquestions from mixed requests.

Preserve its verified links, source dates, conflicts, single-source limits, and what could not be established. Never promote a search snippet or unsupported claim into verified fact."#;

/// System prompt for the global (platform-level) FlowPilot assistant, for backends that delegate
/// public-web research to the `research_agent` specialist.
pub fn global_assistant_system_prompt() -> String {
    global_assistant_system_prompt_for(WebResearchCapability::Delegated)
}

/// System prompt for the global (platform-level) FlowPilot assistant. Shared by every backend; only
/// the public-web fragment follows the backend's actual capability.
pub fn global_assistant_system_prompt_for(capability: WebResearchCapability) -> String {
    let web = match capability {
        WebResearchCapability::Delegated => DELEGATED_WEB_PLAYBOOK,
        WebResearchCapability::Inline => INLINE_WEB_PLAYBOOK,
    };
    [FLOWPILOT_CORE, TASK_ROUTING_PLAYBOOK, BUILD_PLAYBOOK, web].join("\n\n")
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
        "These files belong to this message. You may inspect images directly. To hand files to an app chat, set `forward_files` to the exact relevant names, or `[]` for none; never forward unrelated files.".to_string(),
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

    // Keep changing request metadata out of the stable base prompt so providers can reuse its
    // cached prefix. Keep the exact timestamp dynamic so relative scheduling requests remain
    // resolvable; a user-supplied timezone or explicit timestamp remains authoritative.
    parts.push(format!(
        "Current UTC timestamp: {}.",
        chrono::Utc::now().to_rfc3339()
    ));

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
    sections.push(format!(
        "## CURRENT TIME AND FLOW-LIKE CONTEXT\n{}",
        parts.join("\n")
    ));
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
    // This wrapper always drives the rig/Bits loop, whose host executes the sealed researcher
    // locally instead of delegating it through a frontend-managed specialist scope.
    let base_prompt = global_assistant_system_prompt_for(WebResearchCapability::Inline);
    let system_prompt = if context.trim().is_empty() {
        base_prompt
    } else {
        format!("{base_prompt}\n\n{context}")
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
    use crate::flow::copilot::tool_spec::global_assistant_tool_specs;

    #[test]
    fn prompt_selects_one_explicit_web_capability() {
        let delegated = global_assistant_system_prompt_for(WebResearchCapability::Delegated);
        let inline = global_assistant_system_prompt_for(WebResearchCapability::Inline);

        assert!(delegated.contains("## SEALED PUBLIC-WEB FALLBACK"));
        assert!(delegated.contains("`research_agent`"));
        assert!(!delegated.contains("`internet_search`"));
        assert!(inline.contains("## PUBLIC-WEB FALLBACK"));
        assert!(inline.contains("`research_agent`"));
        assert!(inline.contains("isolated public-only context"));
        assert!(!inline.contains("`internet_search`"));
    }

    #[test]
    fn task_routing_is_direct_by_default_and_gates_complex_solve() {
        let prompt = global_assistant_system_prompt();

        let discovery = prompt.find("begin with `list_apps`").unwrap();
        let fallback = prompt.find("## SEALED PUBLIC-WEB FALLBACK").unwrap();
        assert!(discovery < fallback);
        assert!(prompt.contains("Local apps are the first choice"));
        assert!(prompt.contains("including installed research/search apps"));
        assert!(prompt.contains("no suitable app/interface"));
        assert!(prompt.contains("local research candidates were tried in best-fit order"));
        assert!(prompt.contains("no useful, nonredundant candidate remained"));
        assert!(prompt.contains("A declined local call is a stop"));
        assert!(prompt.contains("extract only safe public factual subquestions"));
        assert!(prompt.contains("Default to DIRECT"));
        assert!(prompt.contains("ordinary one-call, one-app, or simple two-app task"));
        assert!(prompt.contains("at least three distinct apps/interfaces"));
        assert!(prompt.contains(
            "A long prompt, calling `list_apps`, or two independent calls is not by itself complex"
        ));
        assert!(prompt.contains("When COMPLEX SOLVE is active"));
        assert!(prompt.contains("private dependency plan"));
        assert!(prompt.contains("all independent ready steps in the same wave"));
        assert!(prompt.contains("pass its Event `id` as `event_id`"));
        assert!(prompt.contains("never substitute its `page_id`"));
        assert!(prompt.contains("structured result supersedes earlier inventory"));
        assert!(prompt.contains("Refresh `list_apps` at most once"));
        assert!(prompt.contains("never a substitute for embedding a page"));
        assert!(prompt.contains("material failures or evidence gaps"));
        assert!(prompt.contains("Never claim completion until a terminal tool result proves it"));
        assert!(prompt.contains("never repeat a successful mutation"));
    }

    #[test]
    fn solve_keeps_public_fallback_separate_from_private_context() {
        let delegated = global_assistant_system_prompt();
        let inline = global_assistant_system_prompt_for(WebResearchCapability::Inline);

        assert!(delegated.contains("immutable user request"));
        assert!(delegated.contains("not root history or model-authored app/private context"));
        assert!(inline.contains("bound to the immutable user request"));
        assert!(inline.contains("cannot receive root history"));
    }

    #[test]
    fn specialist_boundaries_and_build_recovery_survive_compaction() {
        let prompt = global_assistant_system_prompt();

        for tool in [
            "flowpilot_board",
            "flowpilot_widget",
            "data_studio_agent",
            "project_scout",
        ] {
            assert!(prompt.contains(&format!("`{tool}`")));
        }
        assert!(prompt.contains("UI scaffolding is not workflow logic"));
        assert!(prompt.contains("full workflow acceptance contract"));
        assert!(prompt.contains("retained candidate/draft"));
        assert!(prompt.contains("exact draft ID/revision"));
        assert!(prompt.contains("`FLOWSCRIPT_BASE_REVISION_CONFLICT`"));
        assert!(prompt.contains("at most one retry"));
        assert!(prompt.contains("Never launch a third equivalent attempt"));
        assert!(prompt.contains("`segments_remaining` means continue"));
        assert!(prompt.contains("`manual_steps` or stubs mean partial completion"));
        assert!(prompt.contains("only a later assistant round may call `upsert_event`"));
        assert!(prompt.contains("create/update every requested Event separately"));
        assert!(prompt.contains("send that evidence to `flowpilot_board`"));
        assert!(prompt.contains("Structural success is not runtime proof"));
    }

    #[test]
    fn build_plan_keeps_identity_and_wavefront_invariants() {
        let prompt = global_assistant_system_prompt();

        assert!(prompt.contains("returned `board_id_map`"));
        assert!(prompt.contains("never send a source board ID to the fork"));
        assert!(prompt.contains("pin the returned destination `app_id`"));
        assert!(prompt.contains("one shared contract"));
        assert!(prompt.contains("`create_new_board=true`"));
        assert!(prompt.contains("run independent specialists together"));
        assert!(prompt.contains("Route each scout part by `source.kind`"));
        assert!(prompt.contains("Pass its `locator` unchanged"));
        assert!(prompt.contains("`timestamp:ms:UTC`"));
        assert!(prompt.contains("FlowScript `Date`"));
        assert!(prompt.contains("checkout link"));
    }

    #[test]
    fn prompt_referenced_tools_exist_for_each_backend() {
        let shared: std::collections::HashSet<_> = global_assistant_tool_specs(false)
            .into_iter()
            .map(|spec| spec.name)
            .collect();
        for name in [
            "list_apps",
            "call_app_chat",
            "flowpilot_board",
            "flowpilot_widget",
            "data_studio_agent",
            "project_scout",
            "fork_app",
            "acquire_app",
            "create_app",
            "upsert_event",
        ] {
            assert!(shared.contains(name), "missing shared tool {name}");
        }
        assert!(shared.contains("research_agent"));
    }

    #[test]
    fn standard_prompt_is_stable_and_within_size_budget() {
        let first = global_assistant_system_prompt();
        let second = global_assistant_system_prompt();

        assert_eq!(
            first, second,
            "runtime context must not mutate the stable prompt"
        );
        assert!(
            first.len() <= 12_000,
            "standard prompt grew beyond the reviewed 12 KB budget: {} bytes",
            first.len()
        );
        assert!(
            first.split_whitespace().count() <= 1_800,
            "standard prompt grew beyond the reviewed word budget"
        );
    }

    #[test]
    fn platform_context_carries_the_current_date_outside_the_stable_prompt() {
        let context = build_platform_context(PlatformContextInput::default());
        assert!(context.contains("## CURRENT TIME"));
        assert!(context.contains("Current UTC timestamp:"));
        assert!(context.contains(&chrono::Utc::now().format("%Y-%m-%d").to_string()));
        assert!(!global_assistant_system_prompt().contains("## CURRENT TIME"));
    }
}
