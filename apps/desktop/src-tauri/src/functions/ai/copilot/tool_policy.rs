//! Request intent, specialist access, and SDK tool construction.

use super::agent_surface::FlowPilotAgentSurface;
use crate::functions::ai::{
    copilot_sdk_tools::SideEffectCommandQueue, frontend_tool_bridge::FrontendToolContext,
};
use flow_like::{
    copilot::{CopilotScope, FlowIrCommitToken},
    flow::{
        board::Board,
        copilot::{BoardCommand, memory::AssistantMemory},
    },
};
use flow_like_types::channel::InProcessChannel;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex as StdMutex},
};
use tauri::AppHandle;

#[cfg(test)]
pub(super) fn is_workflow_edit_request(prompt: &str) -> bool {
    let prompt = prompt.to_lowercase();
    if is_read_only_workflow_request(&prompt) {
        return false;
    }

    let edit_verbs = [
        "add",
        "apply",
        "automate",
        "build",
        "connect",
        "configur",
        "create",
        "draft",
        "embed",
        "fetch",
        "fix",
        "generate",
        "insert",
        "make",
        "modify",
        "repair",
        "schedule",
        "set up",
        "store",
        "translate",
        "update",
        "wire",
        // German UI prompts are common in FlowPilot. Use stems so natural inflections such as
        // "Bau", "baue", "erstelle" and "automatisiere" enter the same guarded edit loop.
        "bau",
        "erstell",
        "hinzuf",
        "füge",
        "automatisier",
        "änder",
        "anpass",
        "reparier",
        "verbind",
        "implementier",
        "konfigurier",
        "plan",
        "speicher",
    ];
    let workflow_terms = [
        "automation",
        "board",
        "cron",
        "database",
        "db",
        "email",
        "flow",
        "flowscript",
        "gmail",
        "imap",
        "lancedb",
        "mail",
        "node",
        "nodes",
        "open database",
        "pipeline",
        "smtp",
        "vector",
        "workflow",
        "api call",
        "edge",
        "edges",
        "execution",
        "event",
        "pin",
        "pins",
        "schedule",
        "scheduler",
        "trigger",
        "automatisierung",
        "auslöser",
        "datenbank",
        "ereignis",
        "knoten",
        "schnittstelle",
        "zeitplan",
        "success output",
        "error output",
    ];

    edit_verbs.iter().any(|verb| prompt.contains(verb))
        && workflow_terms.iter().any(|term| prompt.contains(term))
}

pub(super) fn is_read_only_workflow_request(prompt: &str) -> bool {
    let trimmed = prompt.trim_start();
    let scheduled_check_imperative = (trimmed.starts_with("check ")
        && [
            " every ",
            " each ",
            " hourly",
            " daily",
            " then ",
            " and notify",
            " and send",
            " and store",
        ]
        .iter()
        .any(|signal| trimmed.contains(signal)))
        || ((trimmed.starts_with("prüf ") || trimmed.starts_with("prüfe "))
            && [" jede", " stünd", " täglich", " und sende", " und speicher"]
                .iter()
                .any(|signal| trimmed.contains(signal)));
    if scheduled_check_imperative {
        return false;
    }
    let read_only_terms = [
        "are these",
        "can this",
        "check",
        "debug",
        "diagnose",
        "does this",
        "error",
        "explain",
        "how does",
        "inspect",
        "is this",
        "issue",
        "not working",
        "problem",
        "review",
        "show me",
        "tell me",
        "what does",
        "what is",
        "what's wrong",
        "where",
        "which",
        "why",
        "erklär",
        "warum",
        "wie funktioniert",
        "prüf",
        "untersuch",
        "fehler",
        "problem",
        "zeige",
        "welche",
        "wo ",
    ];
    if !read_only_terms.iter().any(|term| prompt.contains(term)) {
        return false;
    }

    let mutation_terms = [
        "add",
        "apply",
        "automate",
        "build",
        "change",
        "create",
        "delete",
        "draft",
        "fix",
        "generate",
        "insert",
        "make",
        "modify",
        "remove",
        "repair",
        "store",
        "translate",
        "update",
        "bau",
        "erstell",
        "hinzuf",
        "füge",
        "automatisier",
        "änder",
        "anpass",
        "reparier",
        "verbind",
        "implementier",
    ];

    !mutation_terms.iter().any(|term| prompt.contains(term))
}

pub(super) fn drain_streamable_side_effect_commands(
    store: &Arc<StdMutex<SideEffectCommandQueue>>,
) -> Vec<BoardCommand> {
    match store.lock() {
        Ok(mut queue) => queue.drain_streamable(),
        Err(poisoned) => {
            let mut queue = poisoned.into_inner();
            queue.abandon();
            Vec::new()
        }
    }
}

pub(super) fn take_side_effect_delivery(
    store: &Arc<StdMutex<SideEffectCommandQueue>>,
) -> (Vec<BoardCommand>, Option<FlowIrCommitToken>) {
    match store.lock() {
        Ok(mut queue) => queue.take_delivery(),
        Err(poisoned) => {
            let mut queue = poisoned.into_inner();
            queue.abandon();
            (Vec::new(), None)
        }
    }
}

pub(super) fn abandon_side_effect_commands(store: &Arc<StdMutex<SideEffectCommandQueue>>) {
    match store.lock() {
        // The response/delivery channel for this run is gone, but a checked+committed batch stays
        // pending in the retained draft store so the next same-request run redelivers its exact
        // Apply/Dismiss token instead of burning a full rebuild cycle on identical commands.
        Ok(mut queue) => queue.abandon_preserving_retained_review(),
        // A poisoned queue cannot vouch for its claim state; fail closed and reopen the revision.
        Err(poisoned) => poisoned.into_inner().abandon(),
    }
}

/// Every early-return path (provider error, cancellation, closed stream, or host teardown) must
/// abandon a batch that was never transferred into the response. Successfully drained queues are
/// empty, so normal completion makes this cleanup a no-op.
pub(super) struct SideEffectCommandQueueCleanup(pub(super) Arc<StdMutex<SideEffectCommandQueue>>);

impl Drop for SideEffectCommandQueueCleanup {
    fn drop(&mut self) {
        abandon_side_effect_commands(&self.0);
    }
}

/// Exact model-facing tool policy for each specialist. The same policy drives real tool retention
/// and the advertised capability metadata so a prompt/status change cannot broaden authority.
pub(super) fn specialist_tool_policy(
    scope: CopilotScope,
    has_board: bool,
    has_graph_context: bool,
) -> HashSet<&'static str> {
    let mut names = HashSet::new();

    if matches!(scope, CopilotScope::Board | CopilotScope::Both) {
        names.extend(["catalog_search", "emit_commands", "get_declarations"]);
        if has_board {
            names.extend([
                "get_current_flowscript",
                "plan_board_scope",
                "extend_time_budget",
                "write_flowscript",
                "patch_flowscript",
                "check_flowscript",
                "test_flowscript",
                "commit_flowscript",
            ]);
        }
        if has_graph_context {
            names.extend([
                "get_node_details",
                "get_unconfigured_nodes",
                "list_board_nodes",
            ]);
        }
        names.extend([
            // Cross-domain context is read-only in the schemas/handlers created for board scope.
            "database_tool",
            "storage_tool",
            "ui_inspect",
            // Executing and diagnosing a persisted workflow is board-owned. Explain mode removes
            // the execution calls through the exact read-only policy below.
            "execute_event",
            "execute_node",
            "query_execution_logs",
            "run_board_tests",
            // End-to-end verification of persisted work: drive a live page's inputs/buttons and
            // invoke the app's chat event, observing the runs they start.
            "interact_app_page",
            "call_app_chat",
            // Lets a board specialist pull the FlowScript a Scout plan pointed it at, instead of
            // that fragment travelling through the orchestrator's context as inlined text.
            "read_flowscript_source",
            "search_workspace",
            "read_symbol",
        ]);
    }

    if matches!(scope, CopilotScope::Frontend | CopilotScope::Both) {
        names.extend(["emit_ui", "get_component_schema"]);
        // The UI specialist verifies its own work at runtime: inspect pages, drive the live page
        // (fill inputs, press buttons), execute the page's Events, talk to the app's chat, and
        // read run logs. Node-level execution stays board-owned.
        names.extend([
            "ui_inspect",
            "execute_event",
            "query_execution_logs",
            "interact_app_page",
            "call_app_chat",
        ]);
    }

    if matches!(scope, CopilotScope::DataStudio) {
        names.extend([
            "database_tool",
            "graph_overlay_tool",
            "forecast_tool",
            "graph_query_tool",
            "graph_element_tool",
            "ontology_action_tool",
            "list_apps",
            "describe_app_interface",
            "search_workspace",
            "read_symbol",
        ]);
    }

    // The Research specialist holds the ONLY public-web tools in the system. It gets
    // nothing else: no app, data, storage or memory access, so untrusted page text and
    // private data never share a context.
    if matches!(scope, CopilotScope::Research) {
        names.extend([
            flow_like::flow::copilot::tool_spec::INTERNET_SEARCH_TOOL,
            flow_like::flow::copilot::tool_spec::OPEN_URL_TOOL,
            flow_like::flow::copilot::tool_spec::ARCHIVE_LOOKUP_TOOL,
        ]);
    }

    // The Scout only ever reads. The mutating counterparts of what it recommends
    // (`fork_app`, `acquire_app`, `create_app`) stay on the orchestrator so their
    // approval prompts surface where the user sees them.
    if matches!(scope, CopilotScope::Scout) {
        names.extend([
            "search_apps",
            "get_app_detail",
            "inspect_app",
            "search_templates",
            "get_template_preview",
            "fork_preview",
            "search_workspace",
            "read_symbol",
            "list_apps",
            "describe_app_interface",
        ]);
    }

    if matches!(scope, CopilotScope::Home) {
        names.extend([
            "get_home_context",
            "get_home_widget_catalog",
            "list_home_data_sources",
            "validate_home_layout",
            "apply_home_layout",
            "list_apps",
            "describe_app_interface",
        ]);
    }

    names
}

pub(super) fn is_flowpilot_read_only_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "catalog_search"
            | "get_declarations"
            | "get_current_flowscript"
            | "get_node_details"
            | "get_unconfigured_nodes"
            | "list_board_nodes"
            | "database_tool"
            | "storage_tool"
            | "ui_inspect"
            | "query_execution_logs"
            | "read_flowscript_source"
            | "search_workspace"
            | "read_symbol"
            // The whole Scout tool set is read-only, so explain mode keeps all of it.
            | "search_apps"
            | "get_app_detail"
            | "inspect_app"
            | "search_templates"
            | "get_template_preview"
            | "fork_preview"
            | "list_apps"
            | "describe_app_interface"
            // Home inspection and validation do not persist a layout.
            | "get_home_context"
            | "get_home_widget_catalog"
            | "list_home_data_sources"
            | "validate_home_layout"
            // The Research scope is read-only in full: reading public pages changes
            // nothing, so explain mode keeps its whole tool set.
            | "internet_search"
            | "open_url"
            | "archive_lookup"
    )
}

pub(super) fn build_flowpilot_sdk_tools(
    app_handle: AppHandle,
    scope: CopilotScope,
    surface: &FlowPilotAgentSurface,
    global: bool,
    nested: bool,
    tool_context: Option<FrontendToolContext>,
    memory: Option<Arc<AssistantMemory>>,
    user_prompt: &str,
    channel: Arc<InProcessChannel>,
) -> Vec<(copilot_sdk::Tool, copilot_sdk::ToolHandler)> {
    use crate::functions::ai::{
        copilot_sdk_tools::{
            create_board_support_tools, create_board_tools, create_data_studio_tools,
            create_frontend_support_tools, create_frontend_tools, create_global_assistant_tools,
            create_home_tools, create_research_tools, create_scout_tools,
            delegated_research_session_for_turn,
        },
        frontend_tool_bridge::{FrontendToolBridge, GLOBAL_FRONTEND_TOOL_EVENT},
    };

    let run_scope_id = tool_context
        .as_ref()
        .and_then(|context| context.run_id.clone());
    // The explicit brief is model input. Original user text remains the provenance seed and
    // session identity, including when a follow-up brief changes the research question.
    let source_user_prompt = tool_context
        .as_ref()
        .and_then(|context| context.source_user_prompt.as_deref())
        .filter(|prompt| !prompt.trim().is_empty())
        .unwrap_or(user_prompt)
        .to_owned();
    let research_session = (scope == CopilotScope::Research).then(|| {
        crate::functions::ai::native_tool_bridge::research_session(
            tool_context.as_ref(),
            &source_user_prompt,
        )
        .unwrap_or_else(|| {
            delegated_research_session_for_turn(&source_user_prompt, run_scope_id.as_deref())
        })
    });

    // Build the runtime bridge once so every path carries the owning run context. Global and
    // nested tools share the global event listener; ordinary board tools keep the board listener.
    let runtime_bridge = if global || nested {
        FrontendToolBridge::new_with_event(app_handle, GLOBAL_FRONTEND_TOOL_EVENT, channel)
    } else {
        FrontendToolBridge::new(app_handle, channel)
    }
    .with_context(tool_context);

    // The global assistant is not bound to a board/surface: it gets the curated global tool set on
    // its own bridge event so its tool requests reach the global listener, not the board copilot's.
    if global {
        return create_global_assistant_tools(
            runtime_bridge,
            memory,
            &source_user_prompt,
            run_scope_id.as_deref(),
        );
    }

    let mut tools = match scope {
        CopilotScope::Board => create_board_tools(
            surface.graph_context.clone(),
            surface.board_arc.clone(),
            surface.live_board.clone(),
            surface.request_acceptance_prompt.as_deref(),
            surface.catalog_provider.clone(),
            Some(surface.side_effect_commands.clone()),
            Some(surface.queued_flowscript.clone()),
        ),
        CopilotScope::Frontend => create_frontend_tools(Some(surface.emitted_surfaces.clone())),
        CopilotScope::Both => {
            let mut all_tools = create_board_tools(
                surface.graph_context.clone(),
                surface.board_arc.clone(),
                surface.live_board.clone(),
                surface.request_acceptance_prompt.as_deref(),
                surface.catalog_provider.clone(),
                Some(surface.side_effect_commands.clone()),
                Some(surface.queued_flowscript.clone()),
            );
            all_tools.extend(create_frontend_tools(Some(
                surface.emitted_surfaces.clone(),
            )));
            all_tools
        }
        // These scopes are not board/UI specialists. They get only their own tool sets from the
        // runtime bridge below.
        CopilotScope::DataStudio
        | CopilotScope::Scout
        | CopilotScope::Home
        | CopilotScope::Research => Vec::new(),
    };
    match scope {
        CopilotScope::Board | CopilotScope::Both => {
            tools.extend(create_board_support_tools(runtime_bridge));
        }
        CopilotScope::Frontend => {
            tools.extend(create_frontend_support_tools(runtime_bridge));
        }
        CopilotScope::DataStudio => {
            tools.extend(create_data_studio_tools(runtime_bridge));
        }
        CopilotScope::Scout => {
            tools.extend(create_scout_tools(runtime_bridge));
        }
        CopilotScope::Home => {
            tools.extend(create_home_tools(runtime_bridge));
        }
        CopilotScope::Research => {
            tools.extend(create_research_tools(
                runtime_bridge,
                research_session.expect("Research scope has a shared session"),
            ));
        }
    }
    let allowed = specialist_tool_policy(
        scope,
        surface.board_arc.is_some(),
        surface.graph_context.is_some(),
    );
    tools.retain(|(tool, _)| allowed.contains(tool.name.as_str()));
    tools
}
