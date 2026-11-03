//! Bot runtime v1: Assistant, Flux and every custom bot answer in the global chat engine with
//! their own persona, the chat's rendering instructions, their tools and their model.
//!
//! A bot holds one of two toolsets. [`BotToolset::Capabilities`] is the shared
//! [`PlatformCopilot`] loop on the orchestrator surface with three changes: the system prompt comes
//! from [`bot_system_prompt`], the advertised tools from [`BotRunContext::tool_specs`], and every
//! tool call passes the bot's app guard before the host bridge sees it. [`BotToolset::FlowPilot`]
//! (Flux) is today's FlowPilot run, prompt and tools unchanged, with the bot's persona and
//! instructions on top. The bot acts on behalf of its owner: every tool runs with the owner's
//! account.

use std::{
    collections::{BTreeSet, HashSet},
    future::Future,
    sync::Arc,
};

use async_trait::async_trait;
use flow_like_model_provider::history::HistoryThinking;
use flow_like_types::bot::{BotCapability, BotModelPolicy, BotSystemKind};
use flow_like_types::data_classification::{DataClassification, DataSensitivity};
use serde_json::{Value, json};

use super::assistant::{
    AttachmentManifestEntry, CHAT_REPLY_FORMATTING, GlobalDataStudioContext, PlatformContextInput,
    WebResearchCapability, attachment_lines, build_platform_context,
    global_assistant_system_prompt_for,
};
use super::bot_talk::{BotTalk, BotTalkBridge, ask_bot_tool_spec};
use super::capability_tools::{HostTools, bot_tool_spec, linked_tool_names, tool_capability};
use super::memory::{AssistantMemory, MemoryScope};
use super::memory_tools::{MemoryBridge, memory_tool_specs};
use super::platform::{PlatformCopilot, PlatformSurface, PlatformToolBridge};
use super::tool_spec::{
    MEMORY_SEARCH_TOOL, MEMORY_STORE_TOOL, PlatformToolSpec, READ_ONLY_DATABASE_OPERATIONS,
    READ_WRITE_DATABASE_OPERATIONS, RESEARCH_AGENT_TOOL, bot_workspace_database_tool_spec,
    find_data_studio_tool_spec, find_global_tool_spec, find_scout_tool_spec,
    find_workflow_context_tool_spec, global_assistant_tool_specs, spec_arg_str,
};
use super::types::{ChatImage, ChatMessage};
use crate::a2ui::authoring::chat_ui_prompt_section;
use crate::bit::{Bit, BitModelPreference, BitTypes};
use crate::models::device::Interaction;
use crate::profile::Profile;
use crate::state::FlowLikeState;

/// `bot_id` of Assistant when the person has no account row for it (signed out, or the list
/// failed).
pub const GENERAL_BOT_ID: &str = "general";
/// Assistant's default name and description, shown until its owner renames it.
pub const GENERAL_BOT_NAME: &str = "Assistant";
pub const GENERAL_BOT_DESCRIPTION: &str = "Your everyday helper";
/// `bot_id` a client may send for Flux. Without an account row it is today's FlowPilot run.
pub const FLUX_BOT_ID: &str = "flux";
/// Flux's default name, shown until its owner renames it.
pub const FLUX_BOT_NAME: &str = "Flux";

const APP_TOOLS: [&str; 8] = [
    "list_apps",
    "describe_app_interface",
    "get_app_detail",
    "open_app_page",
    "open_app_chat",
    "call_app_event",
    "call_app_chat",
    "interact_app_page",
];
const DATABASE_TOOL: &str = "database_tool";
const GRAPH_QUERY_TOOL: &str = "graph_query_tool";
const STORAGE_TOOL: &str = "storage_tool";
const GRAPH_QUERY_OPERATIONS: [&str; 8] = [
    "cypher",
    "sql",
    "neighbors",
    "subgraph",
    "paths",
    "analytics",
    "search_nodes",
    "sample",
];
const STORAGE_READ_OPERATIONS: [&str; 2] = ["list_files", "read_file"];
/// Tools that need a person at the open chat: questions, and app pages or chats shown inline.
const CHAT_ONLY_TOOLS: [&str; 4] = [
    "ask_user",
    "open_app_page",
    "open_app_chat",
    "interact_app_page",
];
/// Tools core runs itself on every host.
const CORE_TOOLS: [&str; 3] = [RESEARCH_AGENT_TOOL, MEMORY_STORE_TOOL, MEMORY_SEARCH_TOOL];

const DEFAULT_TOOL_ROUNDS: usize = 8;
const HEAVY_TOOL_ROUNDS: usize = 12;

const BOT_DATABASE_DESCRIPTION: &str = r#"Read the tables of an app you may use: list_tables, describe_table (include_sample=false for schema only), or a read-only query. Pass the app_id. Change an app's data through its Events instead."#;
const BOT_STORAGE_DESCRIPTION: &str = r#"List and read files in the storage of an app you may use: list_files (optional prefix) and read_file (bounded text). Read-only. Pass the app_id."#;

/// What executes a bot turn's tools, which decides the tools the turn is offered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BotToolHost {
    /// An open chat: it asks the owner and shows app pages and chats inline; the host runs the
    /// rest.
    #[default]
    Chat,
    /// No one is watching; the host runs app, data and file tools natively (the desktop, iOS and
    /// Android apps).
    Native,
    /// No one is watching and the host has no native app, data or file tools (a server worker
    /// today): only the tools core runs itself, web research and memory.
    CoreOnly,
}

impl BotToolHost {
    /// Whether a turn on this host can run `tool_name`.
    pub fn runs(self, tool_name: &str) -> bool {
        match self {
            Self::Chat => true,
            Self::Native => !CHAT_ONLY_TOOLS.contains(&tool_name),
            Self::CoreOnly => CORE_TOOLS.contains(&tool_name),
        }
    }
}

/// Which tools and base prompt a bot runs with.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BotToolset {
    /// The tool groups its capabilities switch on, behind its app guard, with the bot prompt.
    #[default]
    Capabilities,
    /// FlowPilot's complete toolset and prompt (Flux and its clones), with the bot's persona and
    /// instructions on top.
    FlowPilot,
}

impl BotToolset {
    /// `FlowPilot` for a bot that holds `flowpilot` (Flux and its duplicates), else `Capabilities`.
    pub fn of(capabilities: &[BotCapability]) -> Self {
        if capabilities.contains(&BotCapability::Flowpilot) {
            Self::FlowPilot
        } else {
            Self::Capabilities
        }
    }
}

/// Who a bot run is: its persona, what its owner switched on, and the apps it may use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BotRunContext {
    pub bot_id: String,
    /// `general` (Assistant) or `flux` for the built-in bots; `None` for bots the owner created.
    pub system: Option<BotSystemKind>,
    pub toolset: BotToolset,
    pub name: String,
    pub description: Option<String>,
    /// How the bot should behave, in the owner's words.
    pub instructions: Option<String>,
    pub capabilities: Vec<BotCapability>,
    pub classification: DataSensitivity,
    /// Apps attached to the bot; they count while `selected_apps` is on.
    pub selected_app_ids: BTreeSet<String>,
    /// Apps of the bot's profile; they count while `profile_apps` is on.
    pub profile_app_ids: BTreeSet<String>,
    /// The bot's own workspace app (its files and tables). Always in scope; storage and data tools
    /// act on it. `None` until the workspace is provisioned.
    pub workspace_app_id: Option<String>,
    /// Bit id of the embedding model the bot's memory uses; `None` uses the chat's.
    pub embedding_model: Option<String>,
    /// What runs this turn's tools; a turn without an open chat gets only what its host runs.
    pub tool_host: BotToolHost,
    /// The bot-tools tools this host links and runs; a bot is offered one only while it holds the
    /// tool's capability too.
    pub host_tools: HostTools,
}

impl BotRunContext {
    /// Assistant as it is without an account row: its default name, `internet` and `memory` on.
    pub fn general() -> Self {
        Self {
            bot_id: GENERAL_BOT_ID.to_string(),
            system: Some(BotSystemKind::General),
            name: GENERAL_BOT_NAME.to_string(),
            description: Some(GENERAL_BOT_DESCRIPTION.to_string()),
            capabilities: vec![BotCapability::Internet, BotCapability::Memory],
            ..Default::default()
        }
    }

    pub fn has(&self, capability: BotCapability) -> bool {
        self.capabilities.contains(&capability)
    }

    /// Tool rounds before the closing answer: 12 for bots that hold `code_execution`,
    /// `data_studio` or `documents`, 8 otherwise.
    pub fn tool_rounds(&self) -> usize {
        let heavy = [
            BotCapability::CodeExecution,
            BotCapability::DataStudio,
            BotCapability::Documents,
        ];
        if heavy.into_iter().any(|capability| self.has(capability)) {
            HEAVY_TOOL_ROUNDS
        } else {
            DEFAULT_TOOL_ROUNDS
        }
    }

    /// The bot-tools tools offered: the bot holds the tool's capability, has a workspace to act
    /// in, and the host runs the tool.
    fn capability_tool_specs(&self) -> Vec<PlatformToolSpec> {
        if self.workspace_app_id.is_none() {
            return Vec::new();
        }
        linked_tool_names()
            .into_iter()
            .filter(|name| self.host_tools.runs(name))
            .filter(|name| tool_capability(name).is_some_and(|capability| self.has(capability)))
            .filter_map(bot_tool_spec)
            .collect()
    }

    /// Whether the bot recalls and keeps memories: with `memory` on, and always for FlowPilot bots.
    pub fn remembers(&self) -> bool {
        self.toolset == BotToolset::FlowPilot || self.has(BotCapability::Memory)
    }

    /// The embedding model a host opens the bot's memory with: the bot's own, else the chat's;
    /// `None` when the bot doesn't remember, so the host opens no memory at all.
    pub fn memory_model<'a>(&'a self, chat_model: Option<&'a str>) -> Option<&'a str> {
        if !self.remembers() {
            return None;
        }
        trimmed(self.embedding_model.as_deref()).or_else(|| trimmed(chat_model))
    }

    /// The bot's own memory store, never another bot's. Flux keeps FlowPilot's memory of the
    /// profile it runs in until bot memory gets its own phase.
    pub fn memory_scope(&self, profile_id: &str) -> MemoryScope {
        match self.system {
            Some(BotSystemKind::Flux) => MemoryScope::Profile(profile_id.to_string()),
            _ => MemoryScope::Bot(self.bot_id.clone()),
        }
    }

    /// `Inline` sealed research when `internet` is on, otherwise no web access at all.
    pub fn web_research(&self) -> WebResearchCapability {
        if self.has(BotCapability::Internet) {
            WebResearchCapability::Inline
        } else {
            WebResearchCapability::Disabled
        }
    }

    fn app_tools_on(&self) -> bool {
        self.has(BotCapability::SelectedApps) || self.has(BotCapability::ProfileApps)
    }

    /// Whether any app can be in scope, so data and file tools have something to act on.
    fn reaches_apps(&self) -> bool {
        self.app_tools_on() || self.workspace_app_id.is_some()
    }

    fn is_workspace(&self, app_id: &str) -> bool {
        self.workspace_app_id.as_deref() == Some(app_id)
    }

    /// Whether a tool call may name `app_id`: an attached app under `selected_apps`, a profile
    /// app under `profile_apps`, or the bot's workspace.
    pub fn allows_app(&self, app_id: &str) -> bool {
        self.is_workspace(app_id)
            || (self.has(BotCapability::SelectedApps) && self.selected_app_ids.contains(app_id))
            || (self.has(BotCapability::ProfileApps) && self.profile_app_ids.contains(app_id))
    }

    fn data_tools_on(&self) -> bool {
        self.has(BotCapability::DataStudio) && self.reaches_apps()
    }

    fn storage_tools_on(&self) -> bool {
        self.has(BotCapability::Storage) && self.reaches_apps()
    }

    /// The tools this run advertises, each only where the tool host runs it, memory tools only
    /// while the bot [remembers](Self::remembers). `FlowPilot`: today's complete orchestrator set.
    /// `Capabilities`: `ask_user`, memory, and one group per capability; FlowPilot's build, debug
    /// and management tools never appear.
    pub fn tool_specs(&self, memory_enabled: bool) -> Vec<PlatformToolSpec> {
        let mut global = global_assistant_tool_specs(memory_enabled && self.remembers());
        if self.toolset == BotToolset::FlowPilot {
            global.retain(|spec| self.tool_host.runs(spec.name));
            global.extend(self.memory_engine_tool_specs());
            return global;
        }
        let pick = |name: &str| global.iter().find(|spec| spec.name == name).copied();
        let mut specs: Vec<PlatformToolSpec> = pick("ask_user").into_iter().collect();
        if self.web_research() == WebResearchCapability::Inline {
            specs.extend(pick(RESEARCH_AGENT_TOOL));
        }
        if self.app_tools_on() {
            specs.extend(
                APP_TOOLS
                    .iter()
                    .filter_map(|name| pick(name).or_else(|| find_scout_tool_spec(name))),
            );
        }
        if self.data_tools_on() {
            specs.extend(self.database_tool_spec());
            specs.extend(find_data_studio_tool_spec(GRAPH_QUERY_TOOL));
        }
        let capability_specs = self.capability_tool_specs();
        let has_file_tools = ["file_list", "file_read"]
            .iter()
            .all(|name| capability_specs.iter().any(|spec| spec.name == *name));
        if self.storage_tools_on() && !has_file_tools {
            specs.extend(find_workflow_context_tool_spec(STORAGE_TOOL).map(|spec| {
                PlatformToolSpec {
                    description: BOT_STORAGE_DESCRIPTION,
                    ..spec
                }
            }));
        }
        specs.extend(pick(MEMORY_STORE_TOOL));
        specs.extend(pick(MEMORY_SEARCH_TOOL));
        specs.retain(|spec| self.tool_host.runs(spec.name));
        specs.extend(capability_specs);
        specs.extend(self.memory_engine_tool_specs());
        specs
    }

    /// The memory engine's tools, run by the host's toolbox: only for a bot that remembers and a
    /// host that opened its memory for this turn.
    fn memory_engine_tool_specs(&self) -> Vec<PlatformToolSpec> {
        if self.remembers() && self.host_tools.memory().is_some() {
            memory_tool_specs()
        } else {
            Vec::new()
        }
    }

    fn offered_tools(&self) -> Vec<&'static str> {
        self.tool_specs(false)
            .iter()
            .map(|spec| spec.name)
            .collect()
    }

    /// With a workspace the bot may change its own tables, so it sees every operation; without
    /// one it only reads.
    fn database_tool_spec(&self) -> Option<PlatformToolSpec> {
        if self.workspace_app_id.is_some() {
            return Some(bot_workspace_database_tool_spec());
        }
        find_workflow_context_tool_spec(DATABASE_TOOL).map(|spec| PlatformToolSpec {
            description: BOT_DATABASE_DESCRIPTION,
            ..spec
        })
    }

    fn offers(&self, tool_name: &str) -> bool {
        self.offered_tools().contains(&tool_name)
    }

    /// Checks one tool call against the bot's limits before any host runs it. Returns the
    /// arguments to run with (a data or file call naming no app acts on the workspace), or the
    /// refusal the model receives instead of a result.
    fn prepare_tool_call(&self, tool_name: &str, mut arguments: Value) -> Result<Value, String> {
        if !self.offers(tool_name) {
            return Err(refusal(
                tool_name,
                "bot_tool_unavailable",
                false,
                format!(
                    "{} has no tool named {tool_name}; nothing was run.",
                    self.label()
                ),
            ));
        }
        let apps = app_references(&arguments)
            .map_err(|message| refusal(tool_name, "bot_app_reference_invalid", true, message))?;
        if let Some(app_id) = apps.iter().find(|app_id| !self.allows_app(app_id)) {
            return Err(refusal(
                tool_name,
                "bot_app_out_of_scope",
                false,
                format!(
                    "{} may not use app '{app_id}'. It works only with the apps its owner gave it; call list_apps to see them. Nothing was run.",
                    self.label()
                ),
            ));
        }
        if let Some(app_id) = nested_app_ids(&arguments)
            .into_iter()
            .find(|app_id| !self.allows_app(app_id))
        {
            return Err(refusal(
                tool_name,
                "bot_app_out_of_scope",
                false,
                format!(
                    "{} may not use app '{app_id}'. It works only with the apps its owner gave it, and the call names that app inside its arguments. Nothing was run.",
                    self.label()
                ),
            ));
        }
        let acts_on_data = matches!(tool_name, DATABASE_TOOL | GRAPH_QUERY_TOOL | STORAGE_TOOL);
        let acts_on_app =
            acts_on_data || (tool_name != "list_apps" && APP_TOOLS.contains(&tool_name));
        let target = match apps.as_slice() {
            [] if acts_on_data && self.workspace_app_id.is_some() => {
                Some(self.workspace_app_id.clone().unwrap_or_default())
            }
            [] if acts_on_app => {
                return Err(refusal(
                    tool_name,
                    "bot_app_required",
                    true,
                    "Name the app with app_id (from list_apps). Nothing was run.".to_string(),
                ));
            }
            [] => None,
            [app_id] => Some(app_id.clone()),
            _ => {
                return Err(refusal(
                    tool_name,
                    "bot_app_reference_conflict",
                    true,
                    "The call names more than one app; name exactly one app_id. Nothing was run."
                        .to_string(),
                ));
            }
        };
        pin_app_target(&mut arguments, target.as_deref());
        let in_workspace = target
            .as_deref()
            .is_some_and(|app_id| self.is_workspace(app_id));
        if let Some(message) = self.operation_refusal(tool_name, &arguments, in_workspace) {
            return Err(refusal(
                tool_name,
                "bot_operation_not_allowed",
                false,
                message,
            ));
        }
        Ok(arguments)
    }

    /// Data and file tools read every allowed app; only the bot's workspace tables may change.
    fn operation_refusal(
        &self,
        tool_name: &str,
        arguments: &Value,
        in_workspace: bool,
    ) -> Option<String> {
        let operation = spec_arg_str(arguments, "operation", "operation").trim();
        let allowed: &[&str] = match tool_name {
            DATABASE_TOOL if in_workspace => READ_WRITE_DATABASE_OPERATIONS,
            DATABASE_TOOL => READ_ONLY_DATABASE_OPERATIONS,
            GRAPH_QUERY_TOOL => &GRAPH_QUERY_OPERATIONS,
            STORAGE_TOOL => &STORAGE_READ_OPERATIONS,
            _ => return None,
        };
        if allowed.contains(&operation) {
            return None;
        }
        Some(format!(
            "{} may run only these {tool_name} operations here: {}. Operation '{operation}' was not run{}.",
            self.label(),
            allowed.join(", "),
            if tool_name == DATABASE_TOOL && !in_workspace {
                "; change an app's data through its Events"
            } else {
                ""
            }
        ))
    }

    /// `list_apps` shows only the apps this bot may use, rebuilt from the fields it may see so
    /// nothing counts or describes the owner's other apps.
    fn scope_tool_output(&self, tool_name: &str, output: String) -> String {
        if tool_name != "list_apps" {
            return output;
        }
        let Ok(value) = serde_json::from_str::<Value>(&output) else {
            return app_inventory_unreadable();
        };
        match value.get("status").and_then(Value::as_str) {
            Some("ok" | "partial") => {}
            Some("error") => return app_inventory_unreadable_with(&value),
            _ => return app_inventory_unreadable(),
        }
        let Some(listed) = value.get("apps").and_then(Value::as_array) else {
            return app_inventory_unreadable();
        };
        let apps: Vec<Value> = listed
            .iter()
            .filter(|app| {
                app.get("app_id")
                    .or_else(|| app.get("id"))
                    .and_then(Value::as_str)
                    .is_some_and(|app_id| self.allows_app(app_id))
            })
            .map(visible_app)
            .collect();
        let events_read = apps
            .iter()
            .all(|app| app.get("events_status").and_then(Value::as_str) == Some("ok"));
        let missing_here = value
            .get("missing_profile_app_ids")
            .and_then(Value::as_array)
            .is_some_and(|missing| {
                missing
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|app_id| self.allows_app(app_id))
            });
        let cut_short = value.get("truncated").and_then(Value::as_bool) == Some(true);
        let complete = events_read && !missing_here && !cut_short;
        let mut result = json!({
            "status": if complete { "ok" } else { "partial" },
            "complete": complete,
            "total": apps.len(),
            "returned": apps.len(),
            "apps": apps,
            "bot_scope": "Only the apps this bot may use are listed.",
        });
        if let Some(query) = value.get("query").and_then(Value::as_str)
            && let Some(object) = result.as_object_mut()
        {
            object.insert("query".to_string(), json!(query));
        }
        if !complete && let Some(object) = result.as_object_mut() {
            object.insert(
                "note".to_string(),
                json!("The listing is incomplete: some Events could not be read or apps were cut off. App absence is unproven; refine the query or retry before concluding that an app does not exist."),
            );
        }
        result.to_string()
    }

    fn label(&self) -> String {
        format!("Bot '{}'", self.name)
    }
}

/// The tool result a refused call returns to the model.
fn refusal(tool_name: &str, code: &str, retryable: bool, message: String) -> String {
    json!({
        "status": "error",
        "tool": tool_name,
        "code": code,
        "retryable": retryable,
        "message": message,
    })
    .to_string()
}

/// Every app a call names at its top level: `app_id`/`appId` and the lists `app_ids`/`appIds`.
fn app_references(arguments: &Value) -> Result<Vec<String>, String> {
    let mut apps: Vec<String> = Vec::new();
    let mut push = |value: &Value| -> Result<(), String> {
        let app_id = value
            .as_str()
            .ok_or("App ids must be strings. Nothing was run.")?
            .trim();
        if !app_id.is_empty() && !apps.iter().any(|known| known == app_id) {
            apps.push(app_id.to_string());
        }
        Ok(())
    };
    for key in ["app_id", "appId"] {
        if let Some(value) = arguments.get(key).filter(|value| !value.is_null()) {
            push(value)?;
        }
    }
    for key in ["app_ids", "appIds"] {
        match arguments.get(key) {
            None | Some(Value::Null) => {}
            Some(Value::Array(values)) => values.iter().try_for_each(&mut push)?,
            Some(_) => {
                return Err("App id lists must be arrays of strings. Nothing was run.".into());
            }
        }
    }
    Ok(apps)
}

const APP_ID_KEYS: [&str; 4] = ["app_id", "appId", "app_ids", "appIds"];
const APP_FIELDS: [&str; 4] = ["app_id", "name", "description", "events_status"];
const EVENT_FIELDS: [&str; 9] = [
    "id",
    "name",
    "description",
    "event_type",
    "kind",
    "consumer_tool",
    "interface_error",
    "page_id",
    "route",
];

fn copy_fields(source: &Value, fields: &[&str]) -> serde_json::Map<String, Value> {
    fields
        .iter()
        .filter_map(|field| Some((field.to_string(), source.get(*field)?.clone())))
        .collect()
}

/// One listed app as a bot may see it: the allowlisted fields of the app and of its Events.
fn visible_app(app: &Value) -> Value {
    let mut visible = copy_fields(app, &APP_FIELDS);
    if visible.get("app_id").is_none()
        && let Some(id) = app.get("id")
    {
        visible.insert("app_id".to_string(), id.clone());
    }
    let events = app
        .get("events")
        .and_then(Value::as_array)
        .map(|events| {
            events
                .iter()
                .map(|event| Value::Object(copy_fields(event, &EVENT_FIELDS)))
                .collect()
        })
        .unwrap_or_default();
    visible.insert("events".to_string(), Value::Array(events));
    Value::Object(visible)
}

const APP_INVENTORY_UNREADABLE: &str = "The app inventory could not be read. App absence is unproven; retry before concluding that an app does not exist.";

fn app_inventory_unreadable() -> String {
    json!({
        "status": "error",
        "tool": "list_apps",
        "code": "app_inventory_unavailable",
        "retryable": true,
        "message": APP_INVENTORY_UNREADABLE,
    })
    .to_string()
}

fn app_inventory_unreadable_with(failed: &Value) -> String {
    let mut result = serde_json::from_str::<Value>(&app_inventory_unreadable()).unwrap_or_default();
    if let (Some(object), Some(code)) = (
        result.as_object_mut(),
        failed.get("code").and_then(Value::as_str),
    ) {
        object.insert("code".to_string(), json!(code));
    }
    if let (Some(object), Some(retryable)) = (
        result.as_object_mut(),
        failed.get("retryable").and_then(Value::as_bool),
    ) {
        object.insert("retryable".to_string(), json!(retryable));
    }
    result.to_string()
}

/// Every app id named below the top level of a call, such as in an Event's payload.
fn nested_app_ids(arguments: &Value) -> Vec<String> {
    fn walk(value: &Value, top: bool, found: &mut Vec<String>) {
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    if !top && APP_ID_KEYS.contains(&key.as_str()) {
                        match child {
                            Value::String(app_id) => found.push(app_id.trim().to_string()),
                            Value::Array(items) => found.extend(
                                items
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .map(|app_id| app_id.trim().to_string()),
                            ),
                            _ => {}
                        }
                    }
                    walk(child, false, found);
                }
            }
            Value::Array(items) => items.iter().for_each(|item| walk(item, false, found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    walk(arguments, true, &mut found);
    found.retain(|app_id| !app_id.is_empty());
    found
}

/// Hosts read only `app_id`: the guard writes the app it checked there and drops every other
/// way of naming one.
fn pin_app_target(arguments: &mut Value, target: Option<&str>) {
    let Some(object) = arguments.as_object_mut() else {
        return;
    };
    for key in APP_ID_KEYS {
        object.remove(key);
    }
    if let Some(app_id) = target {
        object.insert("app_id".to_string(), Value::String(app_id.to_string()));
    }
}

/// Runs every bot tool call past the bot's limits before the host bridge sees it.
struct BotToolBridge {
    bot: Arc<BotRunContext>,
    inner: Arc<dyn PlatformToolBridge>,
}

#[async_trait]
impl PlatformToolBridge for BotToolBridge {
    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        match self.bot.prepare_tool_call(tool_name, arguments) {
            Ok(arguments) => {
                let output = self.inner.call(tool_name, arguments).await;
                self.bot.scope_tool_output(tool_name, output)
            }
            Err(refusal) => refusal,
        }
    }

    async fn drain_steering(&self) -> Vec<String> {
        self.inner.drain_steering().await
    }

    async fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled().await
    }
}

/// The spec a host resolves for a bot's tool call (approval, timeout, required arguments).
/// `database_tool` resolves to the read-write Data Studio spec, so approval follows the operation.
pub fn find_bot_tool_spec(tool_name: &str) -> Option<PlatformToolSpec> {
    match tool_name {
        DATABASE_TOOL => Some(bot_workspace_database_tool_spec()),
        GRAPH_QUERY_TOOL => find_data_studio_tool_spec(tool_name),
        STORAGE_TOOL => find_workflow_context_tool_spec(tool_name),
        "get_app_detail" => find_scout_tool_spec(tool_name),
        "ask_user" | RESEARCH_AGENT_TOOL | MEMORY_STORE_TOOL | MEMORY_SEARCH_TOOL => {
            find_global_tool_spec(tool_name)
        }
        _ if APP_TOOLS.contains(&tool_name) => find_global_tool_spec(tool_name),
        _ => bot_tool_spec(tool_name),
    }
}

/// The bot's model on this host: its primary, then its fallbacks in order, the first one
/// `can_run` accepts. `None` when it names none or none runs here; the host then falls back to the
/// chat's own model selection.
pub async fn resolve_bot_model<F, Fut>(policy: &BotModelPolicy, mut can_run: F) -> Option<String>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = bool>,
{
    let mut tried = HashSet::new();
    let candidates = policy
        .primary
        .iter()
        .chain(policy.fallbacks.iter())
        .map(|model_id| model_id.trim())
        .filter(|model_id| !model_id.is_empty());
    for model_id in candidates {
        if tried.insert(model_id) && can_run(model_id.to_string()).await {
            return Some(model_id.to_string());
        }
    }
    None
}

/// The completion model `model_id` names, or the profile's best one, resolved the way the chat
/// loop resolves it on this host.
pub async fn resolve_host_completion_model(
    state: &Arc<FlowLikeState>,
    profile: &Profile,
    model_id: Option<&str>,
) -> flow_like_types::Result<Bit> {
    let preference = BitModelPreference {
        reasoning_weight: Some(1.0),
        ..Default::default()
    };
    let capabilities = FlowLikeState::completion_model_capabilities(state).await;
    let devices = state.device_model_probe(Interaction::Allowed { run_label: None });
    profile
        .resolve_completion_model(
            model_id,
            &preference,
            false,
            capabilities,
            devices.as_ref(),
            state.http_client.clone(),
        )
        .await
}

/// The completion model `model_id` names when `profile` can run it on this host right now;
/// `None` when it is unknown, not a language model, or would fall back to another model.
pub async fn runnable_completion_model(
    state: &Arc<FlowLikeState>,
    profile: &Profile,
    model_id: &str,
) -> Option<Bit> {
    let bit = resolve_host_completion_model(state, profile, Some(model_id))
        .await
        .ok()?;
    (bit.id == model_id && matches!(bit.bit_type, BitTypes::Llm | BitTypes::Vlm)).then_some(bit)
}

fn capability_label(capability: BotCapability) -> &'static str {
    match capability {
        BotCapability::Internet => "Internet",
        BotCapability::ProfileApps => "Profile apps",
        BotCapability::SelectedApps => "Selected apps",
        BotCapability::CodeExecution => "Code execution",
        BotCapability::ComputerUse => "Computer use",
        BotCapability::BrowserUse => "Browser use",
        BotCapability::Email => "Email",
        BotCapability::PushNotifications => "Push notifications",
        BotCapability::Scheduling => "Scheduling",
        BotCapability::Storage => "Files",
        BotCapability::DataStudio => "Data Studio",
        BotCapability::Memory => "Memory",
        BotCapability::Documents => "Documents",
        BotCapability::DataConnections => "Data connections",
        BotCapability::Knowledge => "Knowledge",
        BotCapability::Connectors => "Connectors",
        BotCapability::Channels => "Channels",
        BotCapability::Skills => "Skills",
        BotCapability::ImageGeneration => "Image generation",
        BotCapability::Delegation => "Delegation",
        BotCapability::CommandLine => "Command line",
        BotCapability::Encryption => "End-to-end encryption",
        BotCapability::Webhook => "Webhook",
        BotCapability::Flowpilot => "FlowPilot",
    }
}

/// Capabilities the owner switched on that have no tools in this run.
fn pending_capabilities(bot: &BotRunContext, offered: &[&str]) -> Vec<String> {
    let reason = |capability: BotCapability, reaches_apps: bool| {
        let why = if reaches_apps {
            "not available in this run"
        } else {
            "you have no apps to use it with yet"
        };
        Some(format!("{} ({why})", capability_label(capability)))
    };
    let backed = |capability: BotCapability| {
        offered
            .iter()
            .any(|name| tool_capability(name) == Some(capability))
    };
    BotCapability::ALL
        .into_iter()
        .filter(|capability| bot.has(*capability))
        .filter_map(|capability| match capability {
            BotCapability::Internet | BotCapability::Memory => None,
            BotCapability::ProfileApps | BotCapability::SelectedApps
                if offered.contains(&"list_apps") =>
            {
                None
            }
            BotCapability::ProfileApps | BotCapability::SelectedApps => reason(capability, true),
            BotCapability::DataStudio if offered.contains(&DATABASE_TOOL) => None,
            BotCapability::Storage if offered.contains(&STORAGE_TOOL) => None,
            BotCapability::DataStudio | BotCapability::Storage if !backed(capability) => {
                reason(capability, bot.reaches_apps())
            }
            BotCapability::DataStudio | BotCapability::Storage => None,
            BotCapability::Knowledge
            | BotCapability::CodeExecution
            | BotCapability::Documents
            | BotCapability::DataConnections
            | BotCapability::ImageGeneration
                if backed(capability) =>
            {
                None
            }
            BotCapability::Knowledge
            | BotCapability::CodeExecution
            | BotCapability::Documents
            | BotCapability::DataConnections
            | BotCapability::ImageGeneration => Some(format!(
                "{} (not available on this host)",
                capability_label(capability)
            )),
            other => Some(capability_label(other).to_string()),
        })
        .collect()
}

fn app_source(bot: &BotRunContext) -> &'static str {
    match (
        bot.has(BotCapability::SelectedApps),
        bot.has(BotCapability::ProfileApps),
    ) {
        (true, true) => "the apps your owner attached to you and every app in your profile",
        (true, false) => "the apps your owner attached to you",
        _ => "every app in your profile",
    }
}

fn workspace_note(bot: &BotRunContext, subject: &str) -> String {
    bot.workspace_app_id
        .as_deref()
        .map(|app_id| format!(" Your own {subject} live in your workspace app (app_id: {app_id})."))
        .unwrap_or_default()
}

/// Backticked names of the `candidates` this run offers, in order.
fn offered_list(offered: &[&str], candidates: &[&str]) -> String {
    candidates
        .iter()
        .filter(|name| offered.contains(name))
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn ability_lines(bot: &BotRunContext, offered: &[&str]) -> Vec<String> {
    let offers = |name: &str| offered.contains(&name);
    let mut lines = vec![if offers("ask_user") {
        "- Ask: `ask_user` asks your owner when a choice truly blocks you; never for what a tool can look up.".to_string()
    } else {
        "- No one is watching this turn, so you can't ask your owner anything: decide on safe defaults and say which ones you assumed.".to_string()
    }];
    lines.push(if offers(RESEARCH_AGENT_TOOL) {
        format!(
            "- Public web: `research_agent` hands your owner's current request to an isolated researcher and returns cited findings{}. It receives that request only: no history, memory, files or app data. Keep its verified links, dates and gaps; never present a search snippet as fact.",
            if offers("list_apps") {
                ", once `list_apps` showed no app that answers"
            } else {
                ""
            }
        )
    } else {
        "- No internet: you can't search or open web pages. Answer from your tools and the conversation, and say what you could not check.".to_string()
    });
    if offers("list_apps") {
        lines.push(format!(
            "- Apps: `list_apps` lists {} with their active Events. Use an app through the exact `consumer_tool` of its Event ({}); {} explain it.{} Calls that name any other app are refused, and calls that run an app ask your owner first.",
            app_source(bot),
            offered_list(
                offered,
                &[
                    "call_app_chat",
                    "open_app_page",
                    "call_app_event",
                    "open_app_chat",
                    "interact_app_page"
                ]
            ),
            offered_list(offered, &["describe_app_interface", "get_app_detail"]),
            if offers("open_app_page") {
                ""
            } else {
                " Pages and app chats shown inline need the open chat, so they aren't available in this run."
            }
        ));
    }
    if offers(DATABASE_TOOL) {
        lines.push(format!(
            "- Data: `database_tool` reads the tables and `graph_query_tool` queries the ontologies of the apps you may use; pass the app_id. Change an app's data through its Events, never directly.{}",
            bot.workspace_app_id
                .as_deref()
                .map(|app_id| format!(" Your own tables live in your workspace app (app_id: {app_id}); there you may also create and change tables without asking; dropping a table asks your owner first."))
                .unwrap_or_default()
        ));
    }
    if offers(STORAGE_TOOL) {
        lines.push(format!(
            "- Files: `storage_tool` lists and reads files of the apps you may use (read-only); pass the app_id.{}",
            workspace_note(bot, "files")
        ));
    }
    lines.extend(capability_tool_lines(bot, offered));
    lines
}

const CAPABILITY_TOOL_LINES: [(BotCapability, &str, &str); 5] = [
    (
        BotCapability::Storage,
        "Files",
        "read and write files; reads reach the apps you may use, writes and saves go to your workspace only. Large results are saved to your workspace and come back as a handle you can reuse",
    ),
    (
        BotCapability::Knowledge,
        "Knowledge",
        "build named searchable indexes over files and search one, several or all of them; dropping an index asks your owner first",
    ),
    (
        BotCapability::CodeExecution,
        "Code",
        "run code in a sandbox and read its output; files it writes land in your workspace",
    ),
    (
        BotCapability::ImageGeneration,
        "Images",
        "create an image with your image model; it is saved to your workspace",
    ),
    (
        BotCapability::Documents,
        "Documents",
        "read, summarize and extract files, and create Word, Excel, PowerPoint and PDF files in your workspace",
    ),
];

fn capability_tool_lines(bot: &BotRunContext, offered: &[&str]) -> Vec<String> {
    CAPABILITY_TOOL_LINES
        .iter()
        .filter_map(|(capability, label, how)| {
            let names: Vec<&str> = offered
                .iter()
                .copied()
                .filter(|name| tool_capability(name) == Some(*capability))
                .collect();
            if names.is_empty() {
                return None;
            }
            let tools = names
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!(
                "- {label}: {tools} {how}.{}",
                match capability {
                    BotCapability::Storage => workspace_note(bot, "files"),
                    BotCapability::CodeExecution if !bot.host_tools.code_languages().is_empty() =>
                        format!(
                            " Languages here: {}. Network is off unless you name hosts, which asks your owner first.",
                            bot.host_tools.code_languages().join(", ")
                        ),
                    _ => String::new(),
                }
            ))
        })
        .collect()
}

fn classification_note(sensitivity: &DataSensitivity) -> String {
    let mut lines = Vec::new();
    match sensitivity.classification {
        Some(level) => lines.push(format!(
            "Your owner classified the data you handle as {}: {}.",
            level.as_str(),
            match level {
                DataClassification::Public => "it may be shared with anyone",
                DataClassification::Internal => "it stays within your owner's organization",
                DataClassification::Confidential => {
                    "only people who need it may see it, and disclosure causes harm"
                }
                DataClassification::Restricted => {
                    "it is the most sensitive kind, and disclosure causes severe harm"
                }
            }
        )),
        None if !sensitivity.pii => lines.push(
            "Your owner hasn't classified the data you handle; treat app data, files and memories as private to your owner.".to_string(),
        ),
        None => {}
    }
    if sensitivity.pii {
        lines.push(
            "It includes personal data: use only what the request needs and never repeat it to anyone but your owner.".to_string(),
        );
    }
    lines.push(
        "Never put private data into web research, and forward files or data to an app only when the request needs it there.".to_string(),
    );
    lines.join(" ")
}

fn reply_closing(offered: &[&str]) -> String {
    let apps = if offered.contains(&"open_app_page") {
        "an app you may use that already shows this (its page inline via `open_app_page`, its chat via `call_app_chat`); else "
    } else {
        ""
    };
    format!(
        "Visual answers, in order of preference: {apps}an `a2ui` fence built from the data you gathered, whenever it reads better than prose; else Markdown. Inline `a2ui` UI lives in this message only; pages and widgets your owner keeps are Flux's work.\n\nUse supported formats, `a2ui` fences, and ordinary Markdown; never arbitrary HTML/CSS, scripts, or JSX expressions for chat layout."
    )
}

fn trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// What a FlowPilot-toolset bot adds to FlowPilot's prompt: the name and description its owner
/// gave it and their instructions. `None` while Flux is untouched, so its prompt stays today's.
pub fn bot_persona_section(bot: &BotRunContext) -> Option<String> {
    let default_name = match bot.system {
        Some(BotSystemKind::Flux) => FLUX_BOT_NAME,
        _ => "",
    };
    let name = trimmed(Some(bot.name.as_str())).filter(|name| *name != default_name);
    let description = trimmed(bot.description.as_deref());
    let instructions = trimmed(bot.instructions.as_deref());
    let mut sections = Vec::new();
    if name.is_some() || description.is_some() {
        let name = name.unwrap_or(default_name);
        let mut who = format!(
            "## WHO YOU ARE\nIn this chat your owner calls you {name}; answer to that name. Your role and rules above stay the same."
        );
        if let Some(description) = description {
            who.push_str(&format!("\nHow your owner describes you: {description}"));
        }
        sections.push(who);
    }
    if let Some(instructions) = instructions {
        sections.push(format!(
            "## YOUR OWNER'S INSTRUCTIONS\nFollow them unless they conflict with the rules above.\n\n{instructions}"
        ));
    }
    (!sections.is_empty()).then(|| sections.join("\n\n"))
}

/// The bot's system prompt. `FlowPilot`: today's FlowPilot prompt with the persona on top.
/// `Capabilities`: who it is and for whom it acts, its owner's instructions, what it can do and
/// what is switched on but not available yet, the data classification, and then the chat's
/// rendering rules (the same `a2ui` section the "Enable Interactive UI" agent node adds).
pub fn bot_system_prompt(bot: &BotRunContext) -> String {
    if bot.toolset == BotToolset::FlowPilot {
        let flowpilot = global_assistant_system_prompt_for(WebResearchCapability::Inline);
        return match bot_persona_section(bot) {
            Some(persona) => format!("{flowpilot}\n\n{persona}"),
            None => flowpilot,
        };
    }
    let name = trimmed(Some(bot.name.as_str())).unwrap_or("Bot");
    let mut identity = format!("You are {name}, one of your owner's bots in Flow-Like.");
    if let Some(description) = trimmed(bot.description.as_deref()) {
        identity.push(' ');
        identity.push_str(description);
    }
    identity.push_str(&format!(
        "\n\nYou act on behalf of your owner, the person writing to you. Everything you do runs with their account and never with more rights than they have, and you hold only the tools listed below. Speak as {name}, in the first person. Prefer acting over describing steps, and never claim an action succeeded until a tool result proves it."
    ));

    let offered = bot.offered_tools();
    let mut sections = vec![identity];
    if let Some(instructions) = trimmed(bot.instructions.as_deref()) {
        sections.push(format!(
            "## YOUR OWNER'S INSTRUCTIONS\nFollow them unless they conflict with the rules of this prompt.\n\n{instructions}"
        ));
    }
    sections.push(format!(
        "## WHAT YOU CAN DO\n{}",
        ability_lines(bot, &offered).join("\n")
    ));
    let pending = pending_capabilities(bot, &offered);
    if !pending.is_empty() {
        sections.push(format!(
            "## SWITCHED ON BUT NOT AVAILABLE HERE\n{}. Your owner switched these on, but you can't use them yet: say so when they come up and never pretend to use them.",
            pending.join(", ")
        ));
    }
    sections.push(
        "Everything else is switched off. When your owner asks for something you can't do, say what is missing; they can change your capabilities in your settings.".to_string(),
    );
    sections.push(
        "## LEAVE TO FLUX\nBuilding, changing or debugging apps, workflows, boards, pages, widgets, Events, FlowScript or the Home page is Flux's work. When your owner asks for that, say so and suggest asking Flux; never claim you built or changed one.".to_string(),
    );
    sections.push(format!(
        "## DATA\n{}",
        classification_note(&bot.classification)
    ));
    sections.push(CHAT_REPLY_FORMATTING.to_string());
    sections.push(chat_ui_prompt_section());
    sections.push(reply_closing(&offered));
    sections.join("\n\n")
}

fn bot_attachments_section(bot: &BotRunContext, entries: &[AttachmentManifestEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut lines = vec![
        "## FILES ATTACHED THIS TURN".to_string(),
        "These files belong to this message. You may inspect images directly.".to_string(),
    ];
    if bot.offers("call_app_chat") {
        lines.push("To hand files to an app chat, set `call_app_chat` `forward_files` to the exact relevant names, or `[]` for none; never forward unrelated files.".to_string());
    }
    lines.extend(attachment_lines(entries));
    lines.join("\n")
}

fn bot_data_studio_section(bot: &BotRunContext, context: &GlobalDataStudioContext) -> String {
    let app_id = context.app_id.trim();
    if app_id.is_empty() || !bot.offers(DATABASE_TOOL) || !bot.allows_app(app_id) {
        return String::new();
    }
    let app_label = trimmed(context.app_name.as_deref()).unwrap_or(app_id);
    format!(
        "## CURRENTLY OPEN DATA STUDIO\nThe user has the data of \"{app_label}\" (app_id: {app_id}) open on screen. \"This data\" or \"this table\" means that app; pass app_id=\"{app_id}\" to `database_tool` and `graph_query_tool`."
    )
}

/// The per-turn context of a bot run. `FlowPilot`: today's FlowPilot context. `Capabilities`:
/// time, user and profile, the files on this message, and the open Data Studio app when the bot
/// may read it; the open-board and FlowPilot routing sections stay out, that work is Flux's.
pub fn bot_platform_context(bot: &BotRunContext, input: PlatformContextInput) -> String {
    if bot.toolset == BotToolset::FlowPilot {
        return build_platform_context(input);
    }
    let base = build_platform_context(PlatformContextInput {
        user_context: input.user_context,
        active_profile: input.active_profile,
        switchable_profiles: &[],
        open_board: None,
        open_data_studio: None,
        attachments: &[],
    });
    let mut sections = vec![base];
    if let Some(context) = input.open_data_studio {
        sections.push(bot_data_studio_section(bot, context));
    }
    sections.push(bot_attachments_section(bot, input.attachments));
    sections.retain(|section| !section.is_empty());
    sections.join("\n\n")
}

/// A bot run reads and writes only its own memory store.
fn bot_memory_error(
    memory: Option<&MemoryScope>,
    bot: &BotRunContext,
    profile_id: &str,
) -> Option<String> {
    memory
        .filter(|scope| **scope != bot.memory_scope(profile_id))
        .map(|_| format!("Bot '{}' can only use its own memory", bot.name))
}

/// FlowPilot never reads or writes a bot's memory store.
pub(super) fn flowpilot_memory_error(memory: Option<&MemoryScope>) -> Option<String> {
    matches!(memory, Some(MemoryScope::Bot(_)))
        .then(|| "FlowPilot can't use a bot's memory".to_string())
}

/// The memory a bot turn gets: none unless the bot [remembers](BotRunContext::remembers), so a
/// bot without `memory` neither recalls nor stores anything, whatever its host opened.
fn turn_memory<M>(bot: &BotRunContext, memory: Option<M>) -> Option<M> {
    memory.filter(|_| bot.remembers())
}

/// What one bot turn runs with: its system prompt, advertised tools and bridge.
struct BotTurn {
    system_prompt: String,
    tool_specs: Vec<PlatformToolSpec>,
    tool_rounds: usize,
    bridge: Arc<dyn PlatformToolBridge>,
}

/// Puts a turn together. With `talk`, a crew to ask adds `ask_bot` and the crew section, and a
/// turn another bot asked for never runs on a chat tool host: nobody can answer its questions.
async fn prepare_bot_turn(
    bot: Arc<BotRunContext>,
    context: &str,
    memory_enabled: bool,
    bridge: Arc<dyn PlatformToolBridge>,
    talk: Option<&Arc<BotTalk>>,
) -> BotTurn {
    let bot = match talk {
        Some(talk) if talk.is_nested() && bot.tool_host == BotToolHost::Chat => {
            Arc::new(BotRunContext {
                tool_host: BotToolHost::Native,
                ..(*bot).clone()
            })
        }
        _ => bot,
    };
    let talk_turn = match talk {
        Some(talk) => talk.turn().await,
        None => Default::default(),
    };
    let mut sections = vec![bot_system_prompt(&bot)];
    sections.extend(talk_turn.prompt);
    if !context.trim().is_empty() {
        sections.push(context.to_string());
    }
    let mut tool_specs = bot.tool_specs(memory_enabled);
    if talk_turn.asks {
        tool_specs.push(ask_bot_tool_spec());
    }
    let mut bridge: Arc<dyn PlatformToolBridge> = match bot.toolset {
        BotToolset::FlowPilot => bridge,
        BotToolset::Capabilities => Arc::new(BotToolBridge {
            bot: bot.clone(),
            inner: bridge,
        }),
    };
    if let (true, Some(talk)) = (talk_turn.asks, talk) {
        bridge = Arc::new(BotTalkBridge::new(talk.clone(), bridge));
    }
    if let (true, Some(memory)) = (bot.remembers(), bot.host_tools.memory()) {
        bridge = Arc::new(MemoryBridge {
            toolbox: memory.toolbox().clone(),
            inner: bridge,
        });
    }
    BotTurn {
        system_prompt: sections.join("\n\n"),
        tool_specs,
        tool_rounds: bot.tool_rounds(),
        bridge,
    }
}

/// Run one bot turn without bot talk; see [`run_bot_chat_with_talk`].
#[allow(clippy::too_many_arguments)]
pub async fn run_bot_chat<F>(
    state: Arc<FlowLikeState>,
    profile: Option<Arc<Profile>>,
    bot: Arc<BotRunContext>,
    context: String,
    user_prompt: String,
    current_images: Option<Vec<ChatImage>>,
    history: Vec<ChatMessage>,
    model_id: Option<String>,
    thinking: Option<HistoryThinking>,
    token: Option<String>,
    bridge: Arc<dyn PlatformToolBridge>,
    memory: Option<Arc<AssistantMemory>>,
    on_token: Option<F>,
) -> flow_like_types::Result<String>
where
    F: Fn(String) + Send + Sync + 'static,
{
    run_bot_chat_with_talk(
        state,
        profile,
        bot,
        context,
        user_prompt,
        current_images,
        history,
        model_id,
        thinking,
        token,
        bridge,
        memory,
        on_token,
        None,
    )
    .await
}

/// Run one bot turn: the bot's prompt and per-turn `context`, its tools (its capabilities' groups
/// behind its app guard, or FlowPilot's complete set), its own `memory` while it remembers, and
/// `model_id` as the host resolved it (see [`resolve_bot_model`]). Same loop, streaming frames
/// and approvals as FlowPilot; a capabilities bot gets sealed web research only when `internet`
/// is on. With `talk` the bot can ask the owner's other bots (`ask_bot`, see
/// [`super::bot_talk`]). Nothing assumes a UI: the chat commands call it today, and a background
/// worker calls it with its own `bridge`, no `on_token`, and the bot's [`BotToolHost`] set to
/// what that worker runs natively.
#[allow(clippy::too_many_arguments)]
pub async fn run_bot_chat_with_talk<F>(
    state: Arc<FlowLikeState>,
    profile: Option<Arc<Profile>>,
    bot: Arc<BotRunContext>,
    context: String,
    user_prompt: String,
    current_images: Option<Vec<ChatImage>>,
    history: Vec<ChatMessage>,
    model_id: Option<String>,
    thinking: Option<HistoryThinking>,
    token: Option<String>,
    bridge: Arc<dyn PlatformToolBridge>,
    memory: Option<Arc<AssistantMemory>>,
    on_token: Option<F>,
    talk: Option<Arc<BotTalk>>,
) -> flow_like_types::Result<String>
where
    F: Fn(String) + Send + Sync + 'static,
{
    let profile_id = profile.as_deref().map_or("", |profile| profile.id.as_str());
    let memory = turn_memory(&bot, memory);
    if let Some(error) = bot_memory_error(
        memory.as_deref().map(AssistantMemory::scope),
        &bot,
        profile_id,
    ) {
        return Err(flow_like_types::anyhow!(error));
    }
    let turn = prepare_bot_turn(bot, &context, memory.is_some(), bridge, talk.as_ref()).await;
    PlatformCopilot::new(state, profile)
        .with_thinking(thinking)
        .with_tool_specs(turn.tool_specs)
        .with_tool_rounds(turn.tool_rounds)
        .chat(
            PlatformSurface::Orchestrator,
            turn.system_prompt,
            user_prompt,
            current_images,
            history,
            model_id,
            token,
            turn.bridge,
            memory,
            on_token,
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::copilot::bot_talk::{BotTalkHost, CrewMember, NestedBotTurn};
    use crate::flow::copilot::tool_spec::{
        cross_board_source_tool_specs, data_studio_specialist_tool_specs,
        home_specialist_tool_specs, public_web_tool_specs, runtime_execution_tool_specs,
        scout_specialist_tool_specs, workflow_context_tool_specs,
    };
    use flow_like_types::bot::BotLocation;
    use std::sync::Mutex;

    const BOT_TOOLS: [&str; 15] = [
        "ask_user",
        RESEARCH_AGENT_TOOL,
        "list_apps",
        "describe_app_interface",
        "get_app_detail",
        "open_app_page",
        "open_app_chat",
        "call_app_event",
        "call_app_chat",
        "interact_app_page",
        DATABASE_TOOL,
        GRAPH_QUERY_TOOL,
        STORAGE_TOOL,
        MEMORY_STORE_TOOL,
        MEMORY_SEARCH_TOOL,
    ];

    fn bot(capabilities: &[BotCapability]) -> BotRunContext {
        BotRunContext {
            bot_id: "bot_scout".into(),
            name: "Scout".into(),
            description: Some("Finds the right app for every question.".into()),
            instructions: Some("Answer in German and keep replies short.".into()),
            capabilities: capabilities.to_vec(),
            selected_app_ids: ["app-crm".to_string()].into(),
            profile_app_ids: ["app-notes".to_string()].into(),
            ..Default::default()
        }
    }

    fn names(bot: &BotRunContext, memory: bool) -> Vec<&'static str> {
        bot.tool_specs(memory)
            .iter()
            .map(|spec| spec.name)
            .collect()
    }

    fn code(refusal: &str) -> String {
        serde_json::from_str::<Value>(refusal).unwrap()["code"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn prompt_carries_persona_rendering_rules_and_capability_notes() {
        let mut scout = bot(&[
            BotCapability::Internet,
            BotCapability::SelectedApps,
            BotCapability::Email,
            BotCapability::ComputerUse,
            BotCapability::DataStudio,
        ]);
        scout.classification = DataSensitivity {
            classification: Some(DataClassification::Confidential),
            pii: true,
        };
        let prompt = bot_system_prompt(&scout);

        assert!(prompt.starts_with("You are Scout, one of your owner's bots in Flow-Like."));
        assert!(prompt.contains("Finds the right app for every question."));
        assert!(prompt.contains("You act on behalf of your owner"));
        assert!(prompt.contains("## YOUR OWNER'S INSTRUCTIONS"));
        assert!(prompt.contains("Answer in German and keep replies short."));
        assert!(prompt.contains("`research_agent`"));
        assert!(prompt.contains("`list_apps` lists the apps your owner attached to you"));
        assert!(prompt.contains("`database_tool` reads the tables"));
        assert!(prompt.contains("## SWITCHED ON BUT NOT AVAILABLE HERE\nComputer use, Email."));
        assert!(prompt.contains("## LEAVE TO FLUX"));
        assert!(prompt.contains("classified the data you handle as confidential"));
        assert!(prompt.contains("It includes personal data"));
        let chat_ui = chat_ui_prompt_section();
        let formatting = prompt.find(CHAT_REPLY_FORMATTING).unwrap();
        let interactive_ui = prompt.find(chat_ui.as_str()).unwrap();
        assert!(formatting < interactive_ui);
        assert!(prompt.ends_with(&format!(
            "{chat_ui}\n\n{}",
            reply_closing(&scout.offered_tools())
        )));
        assert_eq!(prompt.matches("```a2ui").count(), 1);
        assert!(!prompt.contains("flowpilot_"));
        assert!(!prompt.contains("data_studio_agent"));
    }

    #[test]
    fn a_bot_without_tools_says_so_instead_of_pretending() {
        let mut quiet = bot(&[BotCapability::Storage, BotCapability::Webhook]);
        quiet.instructions = Some("   ".into());
        let prompt = bot_system_prompt(&quiet);

        assert!(prompt.contains("- No internet:"));
        assert!(!prompt.contains("`research_agent`"));
        assert!(!prompt.contains("YOUR OWNER'S INSTRUCTIONS"));
        assert!(!prompt.contains("`list_apps`"));
        assert!(!prompt.contains("`open_app_page`"));
        assert!(prompt.contains("Files (you have no apps to use it with yet), Webhook."));
        assert!(prompt.contains("hasn't classified the data you handle"));
        assert_eq!(names(&quiet, false), ["ask_user"]);
    }

    #[test]
    fn the_flowpilot_capability_picks_flowpilot_s_toolset() {
        assert_eq!(
            BotToolset::of(&[BotCapability::Flowpilot]),
            BotToolset::FlowPilot
        );
        assert_eq!(
            BotToolset::of(&[BotCapability::Internet, BotCapability::Flowpilot]),
            BotToolset::FlowPilot
        );
        assert_eq!(BotToolset::of(&[]), BotToolset::Capabilities);
        let others: Vec<BotCapability> = BotCapability::ALL
            .into_iter()
            .filter(|capability| *capability != BotCapability::Flowpilot)
            .collect();
        assert_eq!(BotToolset::of(&others), BotToolset::Capabilities);
    }

    #[test]
    fn assistant_without_a_row_has_its_defaults_and_sealed_research() {
        let general = BotRunContext::general();
        assert_eq!(general.bot_id, GENERAL_BOT_ID);
        assert_eq!(general.name, "Assistant");
        assert_eq!(general.description.as_deref(), Some("Your everyday helper"));
        assert_eq!(general.web_research(), WebResearchCapability::Inline);
        assert_eq!(names(&general, false), ["ask_user", RESEARCH_AGENT_TOOL]);
        assert_eq!(
            names(&general, true),
            [
                "ask_user",
                RESEARCH_AGENT_TOOL,
                MEMORY_STORE_TOOL,
                MEMORY_SEARCH_TOOL
            ]
        );
        assert!(bot_system_prompt(&general).starts_with("You are Assistant,"));
        assert_eq!(
            general.memory_scope("profile-1"),
            MemoryScope::Bot("general".into())
        );
        assert!(general.has(BotCapability::Memory));
    }

    #[test]
    fn only_bots_with_memory_and_flowpilot_bots_remember() {
        let quiet = bot(&[BotCapability::Internet]);
        let keeper = bot(&[BotCapability::Internet, BotCapability::Memory]);
        let flux = BotRunContext {
            toolset: BotToolset::FlowPilot,
            ..bot(&[])
        };
        assert!(!quiet.remembers() && keeper.remembers() && flux.remembers());
        for name in [MEMORY_STORE_TOOL, MEMORY_SEARCH_TOOL] {
            assert!(!names(&quiet, true).contains(&name), "{name}");
            assert!(names(&keeper, true).contains(&name), "{name}");
            assert!(names(&flux, true).contains(&name), "{name}");
            assert!(!names(&keeper, false).contains(&name), "{name}");
        }
        assert_eq!(turn_memory(&quiet, Some("store")), None);
        assert_eq!(turn_memory(&keeper, Some("store")), Some("store"));
        assert_eq!(turn_memory(&flux, Some("store")), Some("store"));

        assert_eq!(quiet.memory_model(Some("embed-chat")), None);
        assert_eq!(keeper.memory_model(Some("embed-chat")), Some("embed-chat"));
        assert_eq!(keeper.memory_model(None), None);
        assert_eq!(flux.memory_model(Some("embed-chat")), Some("embed-chat"));
        let own = BotRunContext {
            embedding_model: Some(" embed-own ".into()),
            ..keeper.clone()
        };
        assert_eq!(own.memory_model(Some("embed-chat")), Some("embed-own"));
        assert_eq!(own.memory_model(None), Some("embed-own"));
        let blank = BotRunContext {
            embedding_model: Some("  ".into()),
            ..keeper
        };
        assert_eq!(blank.memory_model(Some("embed-chat")), Some("embed-chat"));
        assert!(
            !bot_system_prompt(&own).contains("SWITCHED ON BUT NOT AVAILABLE HERE"),
            "memory is never listed as missing"
        );
    }

    fn hosted(capabilities: &[BotCapability], host_tools: HostTools) -> BotRunContext {
        BotRunContext {
            workspace_app_id: Some("app-workspace".into()),
            host_tools,
            ..bot(capabilities)
        }
    }

    fn capability_names(bot: &BotRunContext) -> Vec<&'static str> {
        names(bot, false)
            .into_iter()
            .filter(|name| tool_capability(name).is_some())
            .collect()
    }

    const FILE_TOOLS: [&str; 4] = ["file_list", "file_read", "file_save", "file_write"];
    const KNOWLEDGE_TOOLS: [&str; 5] = [
        "knowledge_create",
        "knowledge_add",
        "knowledge_search",
        "knowledge_list",
        "knowledge_drop",
    ];

    #[test]
    fn capability_tools_need_the_capability_and_a_host_that_runs_them() {
        let linked = HostTools::linked();
        let held = [BotCapability::Storage, BotCapability::Knowledge];
        let cases: [(&[BotCapability], HostTools, Vec<&str>); 6] = [
            (&[], linked.clone(), vec![]),
            (&held, HostTools::none(), vec![]),
            (
                &[BotCapability::Storage],
                linked.clone(),
                FILE_TOOLS.to_vec(),
            ),
            (
                &[BotCapability::Knowledge],
                linked.clone(),
                KNOWLEDGE_TOOLS.to_vec(),
            ),
            (
                &held,
                HostTools::only(&["file_read", "knowledge_search"]),
                vec!["file_read", "knowledge_search"],
            ),
            (
                &[
                    BotCapability::DataConnections,
                    BotCapability::ImageGeneration,
                ],
                linked.clone(),
                vec![],
            ),
        ];
        for (capabilities, host_tools, expected) in cases {
            let offered = capability_names(&hosted(capabilities, host_tools));
            assert_eq!(offered, expected, "{capabilities:?}");
        }
        let both = capability_names(&hosted(&held, linked));
        assert_eq!(
            both,
            [FILE_TOOLS.to_vec(), KNOWLEDGE_TOOLS.to_vec()].concat()
        );
    }

    #[test]
    fn capability_tools_need_a_workspace() {
        let no_workspace = BotRunContext {
            workspace_app_id: None,
            ..hosted(&[BotCapability::Storage], HostTools::linked())
        };
        assert!(capability_names(&no_workspace).is_empty());
    }

    #[test]
    fn file_tools_replace_storage_tool_and_are_not_run_by_a_tool_host() {
        let both = hosted(&[BotCapability::Storage], HostTools::linked());
        assert!(!names(&both, false).contains(&STORAGE_TOOL));
        let partial = hosted(&[BotCapability::Storage], HostTools::only(&["file_read"]));
        assert!(names(&partial, false).contains(&STORAGE_TOOL));
        let core_only = BotRunContext {
            tool_host: BotToolHost::CoreOnly,
            ..hosted(&[BotCapability::Storage], HostTools::linked())
        };
        assert_eq!(capability_names(&core_only), FILE_TOOLS);
    }

    #[test]
    fn a_call_to_a_tool_the_bot_is_not_offered_is_refused() {
        let storage_only = hosted(&[BotCapability::Storage], HostTools::linked());
        assert!(
            storage_only
                .prepare_tool_call("file_read", json!({ "file": "bot/a.md" }))
                .is_ok()
        );
        let refused = storage_only
            .prepare_tool_call("knowledge_search", json!({ "query": "x" }))
            .unwrap_err();
        assert_eq!(code(&refused), "bot_tool_unavailable");
        let foreign = storage_only
            .prepare_tool_call("file_list", json!({ "app_id": "app-other" }))
            .unwrap_err();
        assert_eq!(code(&foreign), "bot_app_out_of_scope");
    }

    #[test]
    fn switched_on_capabilities_without_a_runnable_tool_are_listed_as_not_available_here() {
        let held = [
            BotCapability::Storage,
            BotCapability::Knowledge,
            BotCapability::Documents,
            BotCapability::CodeExecution,
        ];
        let on_host = hosted(&held, HostTools::only(&["file_read", "file_list"]));
        let prompt = bot_system_prompt(&on_host);
        assert!(
            prompt.contains("## SWITCHED ON BUT NOT AVAILABLE HERE\n"),
            "{prompt}"
        );
        assert!(prompt.contains("Knowledge (not available on this host)"));
        assert!(prompt.contains("Documents (not available on this host)"));
        assert!(prompt.contains("Code execution (not available on this host)"));
        assert!(!prompt.contains("Files (not available"));
        assert!(prompt.contains("- Files: `file_list`, `file_read`"));
        let linked = hosted(&[BotCapability::Knowledge], HostTools::linked());
        let prompt = bot_system_prompt(&linked);
        assert!(!prompt.contains("SWITCHED ON BUT NOT AVAILABLE HERE"));
        assert!(prompt.contains("- Knowledge: `knowledge_create`"));
        assert!(prompt.contains("dropping an index asks your owner first"));
    }

    #[test]
    fn rounds_are_twelve_for_code_data_and_documents_and_eight_otherwise() {
        for capability in [
            BotCapability::CodeExecution,
            BotCapability::DataStudio,
            BotCapability::Documents,
        ] {
            assert_eq!(bot(&[capability]).tool_rounds(), 12, "{capability:?}");
        }
        for capability in [
            BotCapability::Internet,
            BotCapability::Storage,
            BotCapability::Knowledge,
            BotCapability::DataConnections,
            BotCapability::ImageGeneration,
        ] {
            assert_eq!(bot(&[capability]).tool_rounds(), 8, "{capability:?}");
        }
        assert_eq!(bot(&[]).tool_rounds(), 8);
    }

    #[test]
    fn only_dropping_a_workspace_table_and_an_index_ask_for_approval() {
        use crate::flow::copilot::tool_spec::resolve_tool_approval;
        let asks = |name: &str, args: Value| {
            let spec = find_bot_tool_spec(name).expect(name);
            resolve_tool_approval(&spec, &args).kind != "none"
        };
        assert!(!asks(
            DATABASE_TOOL,
            json!({ "operation": "create_table", "table_name": "notes" })
        ));
        assert!(!asks(
            DATABASE_TOOL,
            json!({ "operation": "insert", "table_name": "notes" })
        ));
        assert!(asks(
            DATABASE_TOOL,
            json!({ "operation": "delete_table", "table_name": "notes" })
        ));
        assert!(asks("knowledge_drop", json!({ "index": "papers" })));
        let linked = linked_tool_names();
        let args_spec = |name: &'static str| PlatformToolSpec {
            name,
            description: "",
            schema: || json!({}),
            approval: crate::flow::copilot::tool_spec::bot_args_approval(),
            timeout_secs: 1,
        };
        let spec_asks = |name: &'static str, args: Value| {
            resolve_tool_approval(&args_spec(name), &args).kind != "none"
        };
        let key = |name: &'static str, args: Value| {
            resolve_tool_approval(&args_spec(name), &args).session_key
        };
        if linked.contains(&"data_sources") {
            let attach = |kind: &str, host: &str| {
                json!({
                    "operation": "attach",
                    "attach": { "name": "crm", "kind": kind, "connection": { "host": host, "port": 5432, "database": "crm" } }
                })
            };
            for kind in ["postgres", "mysql", "clickhouse"] {
                assert!(
                    !spec_asks("data_sources", attach(kind, "db.example.com")),
                    "{kind}"
                );
            }
            for args in [
                json!({ "operation": "attach", "attach": { "name": "t", "kind": "csv", "file": "bot/t.csv" } }),
                json!({ "operation": "list" }),
                json!({ "operation": "detach", "name": "crm" }),
            ] {
                assert!(!spec_asks("data_sources", args.clone()), "{args}");
            }
            let real = crate::flow::copilot::capability_tools::bot_tool_spec("data_sources")
                .expect("data_sources spec");
            let network = attach("postgres", "db.example.com");
            assert_eq!(resolve_tool_approval(&real, &network).kind, "none");
        }
        if linked.contains(&"code_run") {
            let run =
                |hosts: Value| json!({ "language": "python", "code": "1", "network_hosts": hosts });
            assert!(spec_asks("code_run", run(json!(["api.example.com"]))));
            assert!(!spec_asks("code_run", run(json!([]))));
            assert!(!spec_asks(
                "code_run",
                json!({ "language": "python", "code": "1" })
            ));
            assert_ne!(
                key("code_run", run(json!(["a.com"]))),
                key("code_run", run(json!(["b.com"])))
            );
            assert_eq!(
                key("code_run", run(json!(["B.com", "a.com"]))),
                key("code_run", run(json!(["a.com", "b.com"])))
            );
        }
        for name in [
            "file_write",
            "file_save",
            "knowledge_add",
            "knowledge_create",
        ] {
            assert!(!asks(name, json!({})), "{name}");
        }
    }

    #[test]
    fn each_capability_switches_on_exactly_its_tool_group() {
        assert_eq!(names(&bot(&[]), false), ["ask_user"]);
        assert_eq!(
            names(&bot(&[BotCapability::Internet]), false),
            ["ask_user", RESEARCH_AGENT_TOOL]
        );
        let app_tools = [
            "ask_user",
            "list_apps",
            "describe_app_interface",
            "get_app_detail",
            "open_app_page",
            "open_app_chat",
            "call_app_event",
            "call_app_chat",
            "interact_app_page",
        ];
        assert_eq!(
            names(&bot(&[BotCapability::SelectedApps]), false),
            app_tools
        );
        assert_eq!(names(&bot(&[BotCapability::ProfileApps]), false), app_tools);
        assert_eq!(
            names(&bot(&[BotCapability::DataStudio]), false),
            ["ask_user"]
        );
        assert_eq!(
            names(
                &bot(&[BotCapability::DataStudio, BotCapability::SelectedApps]),
                false
            )[app_tools.len()..],
            [DATABASE_TOOL, GRAPH_QUERY_TOOL]
        );
        assert_eq!(
            names(
                &bot(&[BotCapability::Storage, BotCapability::ProfileApps]),
                false
            )[app_tools.len()..],
            [STORAGE_TOOL]
        );
        let mut workspace_only = bot(&[BotCapability::DataStudio, BotCapability::Storage]);
        workspace_only.workspace_app_id = Some("app-workspace".into());
        assert_eq!(
            names(&workspace_only, false),
            ["ask_user", DATABASE_TOOL, GRAPH_QUERY_TOOL, STORAGE_TOOL]
        );
        for pending in [
            BotCapability::CodeExecution,
            BotCapability::ComputerUse,
            BotCapability::BrowserUse,
            BotCapability::Email,
            BotCapability::PushNotifications,
            BotCapability::Scheduling,
            BotCapability::CommandLine,
            BotCapability::Encryption,
            BotCapability::Webhook,
        ] {
            assert_eq!(names(&bot(&[pending]), false), ["ask_user"]);
        }
    }

    #[test]
    fn database_tool_reads_everywhere_and_changes_only_the_workspace() {
        let reader = bot(&[BotCapability::DataStudio, BotCapability::SelectedApps]);
        let advertised = reader.database_tool_spec().unwrap();
        assert_eq!(advertised.description, BOT_DATABASE_DESCRIPTION);
        assert_eq!(
            (advertised.schema)()["properties"]["operation"]["enum"],
            json!(READ_ONLY_DATABASE_OPERATIONS)
        );
        let mut owner = reader.clone();
        owner.workspace_app_id = Some("app-workspace".into());
        assert_eq!(
            (owner.database_tool_spec().unwrap().schema)()["properties"]["operation"]["enum"],
            json!(READ_WRITE_DATABASE_OPERATIONS)
        );

        let query =
            json!({ "operation": "query", "app_id": "app-crm", "query": { "sql": "SELECT 1" } });
        assert!(
            reader
                .prepare_tool_call(DATABASE_TOOL, query.clone())
                .is_ok()
        );
        assert!(owner.prepare_tool_call(DATABASE_TOOL, query).is_ok());
        let insert = json!({ "operation": "insert", "app_id": "app-crm", "table_name": "deals", "items": [{}] });
        for context in [&reader, &owner] {
            let refusal = context
                .prepare_tool_call(DATABASE_TOOL, insert.clone())
                .unwrap_err();
            assert_eq!(code(&refusal), "bot_operation_not_allowed");
            assert!(refusal.contains("through its Events"));
        }
        let into_workspace = json!({ "operation": "insert", "table_name": "notes", "items": [{}] });
        assert_eq!(
            owner
                .prepare_tool_call(DATABASE_TOOL, into_workspace.clone())
                .unwrap()["app_id"],
            "app-workspace"
        );
        assert_eq!(
            code(
                &reader
                    .prepare_tool_call(DATABASE_TOOL, into_workspace)
                    .unwrap_err()
            ),
            "bot_app_required"
        );
        assert_eq!(
            code(
                &owner
                    .prepare_tool_call(DATABASE_TOOL, json!({ "app_id": "app-crm" }))
                    .unwrap_err()
            ),
            "bot_operation_not_allowed"
        );

        let files = bot(&[BotCapability::Storage, BotCapability::SelectedApps]);
        assert!(
            files
                .prepare_tool_call(
                    STORAGE_TOOL,
                    json!({ "operation": "read_file", "app_id": "app-crm", "path": "a.md" })
                )
                .is_ok()
        );
        assert_eq!(
            code(
                &files
                    .prepare_tool_call(
                        STORAGE_TOOL,
                        json!({ "operation": "delete_files", "app_id": "app-crm", "paths": ["a.md"] })
                    )
                    .unwrap_err()
            ),
            "bot_operation_not_allowed"
        );
    }

    #[test]
    fn app_guard_refuses_foreign_apps_and_passes_allowed_ones() {
        let both = bot(&[BotCapability::SelectedApps, BotCapability::ProfileApps]);
        let selected = bot(&[BotCapability::SelectedApps]);
        let call = |app: Value| json!({ "app_id": app, "event_id": "event-1", "message": "hi", "forward_files": [] });

        assert!(
            both.prepare_tool_call("call_app_chat", call(json!("app-crm")))
                .is_ok()
        );
        assert!(
            both.prepare_tool_call("call_app_chat", call(json!("app-notes")))
                .is_ok()
        );
        assert!(
            selected
                .prepare_tool_call("call_app_chat", call(json!("app-crm")))
                .is_ok()
        );
        let foreign = selected
            .prepare_tool_call("call_app_chat", call(json!("app-notes")))
            .unwrap_err();
        assert_eq!(code(&foreign), "bot_app_out_of_scope");
        assert!(foreign.contains("Bot 'Scout' may not use app 'app-notes'"));
        assert_eq!(
            code(
                &both
                    .prepare_tool_call("open_app_page", json!({ "appId": "app-elsewhere" }))
                    .unwrap_err()
            ),
            "bot_app_out_of_scope"
        );
        assert_eq!(
            code(
                &both
                    .prepare_tool_call(
                        "open_app_page",
                        json!({ "app_id": "app-crm", "appId": "app-elsewhere" })
                    )
                    .unwrap_err()
            ),
            "bot_app_out_of_scope"
        );
        assert_eq!(
            code(
                &both
                    .prepare_tool_call(
                        "describe_app_interface",
                        json!({ "app_ids": ["app-crm", "app-elsewhere"], "event_id": "e" })
                    )
                    .unwrap_err()
            ),
            "bot_app_out_of_scope"
        );
        assert_eq!(
            code(
                &both
                    .prepare_tool_call("open_app_page", json!({ "app_id": 7 }))
                    .unwrap_err()
            ),
            "bot_app_reference_invalid"
        );
        assert_eq!(
            code(
                &both
                    .prepare_tool_call(
                        "open_app_page",
                        json!({ "app_id": "app-crm", "appId": "app-notes" })
                    )
                    .unwrap_err()
            ),
            "bot_app_reference_conflict"
        );
        assert_eq!(
            code(
                &both
                    .prepare_tool_call("interact_app_page", json!({ "actions": [] }))
                    .unwrap_err()
            ),
            "bot_app_required"
        );
        assert!(both.prepare_tool_call("list_apps", json!({})).is_ok());
        assert_eq!(
            code(
                &both
                    .prepare_tool_call("flowpilot_board", json!({ "app_id": "app-crm" }))
                    .unwrap_err()
            ),
            "bot_tool_unavailable"
        );
        assert_eq!(
            code(
                &bot(&[BotCapability::Internet])
                    .prepare_tool_call("call_app_chat", call(json!("app-crm")))
                    .unwrap_err()
            ),
            "bot_tool_unavailable"
        );
    }

    #[derive(Default)]
    struct RecordingBridge {
        calls: Mutex<Vec<(String, Value)>>,
        output: String,
    }

    #[async_trait]
    impl PlatformToolBridge for RecordingBridge {
        async fn call(&self, tool_name: &str, arguments: Value) -> String {
            self.calls
                .lock()
                .unwrap()
                .push((tool_name.to_string(), arguments));
            self.output.clone()
        }
    }

    #[tokio::test]
    async fn refused_calls_never_reach_the_host_bridge() {
        let host = Arc::new(RecordingBridge {
            output: json!({ "status": "ok" }).to_string(),
            ..Default::default()
        });
        let bridge = BotToolBridge {
            bot: Arc::new(bot(&[BotCapability::SelectedApps])),
            inner: host.clone(),
        };

        let refused = bridge
            .call(
                "call_app_event",
                json!({ "app_id": "app-notes", "event_id": "event-1" }),
            )
            .await;
        assert_eq!(code(&refused), "bot_app_out_of_scope");
        assert!(host.calls.lock().unwrap().is_empty());

        let allowed = bridge
            .call(
                "call_app_event",
                json!({ "app_id": "app-crm", "event_id": "event-1" }),
            )
            .await;
        assert_eq!(
            serde_json::from_str::<Value>(&allowed).unwrap()["status"],
            "ok"
        );
        let calls = host.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "call_app_event");
        assert_eq!(calls[0].1["app_id"], "app-crm");
    }

    #[tokio::test]
    async fn list_apps_shows_only_the_apps_the_bot_may_use() {
        let host = Arc::new(RecordingBridge {
            output: json!({
                "status": "partial",
                "complete": false,
                "total": 3,
                "returned": 3,
                "profile_total": 4,
                "missing_profile_app_count": 2,
                "missing_profile_app_ids": ["app-crm", "app-hidden"],
                "apps": [
                    { "app_id": "app-crm", "name": "CRM", "events": [] },
                    { "app_id": "app-notes", "name": "Notes", "events": [] },
                    { "app_id": "app-payroll", "name": "Payroll", "events": [] }
                ]
            })
            .to_string(),
            ..Default::default()
        });
        let bridge = BotToolBridge {
            bot: Arc::new(bot(&[BotCapability::SelectedApps])),
            inner: host,
        };
        let listed: Value =
            serde_json::from_str(&bridge.call("list_apps", json!({})).await).unwrap();

        assert_eq!(listed["status"], "partial");
        assert_eq!(listed["apps"].as_array().unwrap().len(), 1);
        assert_eq!(listed["apps"][0]["app_id"], "app-crm");
        assert_eq!(listed["total"], 1);
        assert_eq!(listed["returned"], 1);
        assert!(listed.get("profile_total").is_none());
        assert_eq!(listed["complete"], false);
        assert!(listed.get("missing_profile_app_ids").is_none());
        assert!(listed.get("missing_profile_app_count").is_none());
        assert!(!listed.to_string().contains("app-payroll"));
        assert!(!listed.to_string().contains("app-hidden"));
    }

    /// Review finding 2: hosts read only `app_id`, so the app the guard checked is the only one
    /// they may see.
    #[test]
    fn the_host_sees_only_the_app_the_guard_checked() {
        let both = bot(&[BotCapability::SelectedApps, BotCapability::ProfileApps]);
        for arguments in [
            json!({ "app_ids": ["app-crm"], "event_id": "event-1" }),
            json!({ "appIds": [" app-crm "], "event_id": "event-1" }),
            json!({ "appId": "app-crm", "event_id": "event-1" }),
            json!({ "app_id": "app-crm", "appId": "app-crm", "app_ids": ["app-crm"], "event_id": "event-1" }),
        ] {
            let prepared = both
                .prepare_tool_call("call_app_event", arguments.clone())
                .unwrap();
            assert_eq!(prepared["app_id"], "app-crm", "{arguments}");
            assert_eq!(prepared["event_id"], "event-1");
            for alias in ["appId", "app_ids", "appIds"] {
                assert!(
                    prepared.get(alias).is_none(),
                    "{alias} survived in {prepared}"
                );
            }
        }
        let mut workspace = bot(&[BotCapability::DataStudio]);
        workspace.workspace_app_id = Some("app-workspace".into());
        let prepared = workspace
            .prepare_tool_call(
                DATABASE_TOOL,
                json!({ "operation": "list_tables", "appIds": [] }),
            )
            .unwrap();
        assert_eq!(prepared["app_id"], "app-workspace");
        assert!(prepared.get("appIds").is_none());
    }

    /// Review finding 6: an app named anywhere in a call, not only at its top level, must be one
    /// the bot may use.
    #[test]
    fn apps_named_inside_a_call_must_be_in_scope_too() {
        let selected = bot(&[BotCapability::SelectedApps]);
        let event = |payload: Value| json!({ "app_id": "app-crm", "event_id": "event-1", "payload": payload });
        for payload in [
            json!({ "app_id": "app-payroll" }),
            json!({ "rows": [{ "appId": "app-payroll" }] }),
            json!({ "targets": { "app_ids": ["app-crm", "app-payroll"] } }),
            json!([{ "nested": { "appIds": ["app-payroll"] } }]),
        ] {
            let refused = selected
                .prepare_tool_call("call_app_event", event(payload.clone()))
                .unwrap_err();
            assert_eq!(code(&refused), "bot_app_out_of_scope", "{payload}");
            assert!(refused.contains("'app-payroll'"), "{refused}");
        }
        assert!(
            selected
                .prepare_tool_call(
                    "call_app_event",
                    event(
                        json!({ "app_id": "app-crm", "rows": [{ "note": "app_id" }], "count": 7 })
                    )
                )
                .is_ok()
        );
    }

    /// Review finding 5: `list_apps` is rebuilt from the fields a bot may see, so nothing counts
    /// or describes the owner's other apps.
    #[tokio::test]
    async fn list_apps_tells_a_bot_nothing_about_the_owner_s_other_apps() {
        let listing = |output: Value| async move {
            let host = Arc::new(RecordingBridge {
                output: output.to_string(),
                ..Default::default()
            });
            let bridge = BotToolBridge {
                bot: Arc::new(bot(&[BotCapability::SelectedApps])),
                inner: host,
            };
            serde_json::from_str::<Value>(&bridge.call("list_apps", json!({})).await).unwrap()
        };
        let listed = listing(json!({
            "status": "partial",
            "complete": false,
            "total": 312,
            "returned": 250,
            "query": "crm",
            "matched_total": 312,
            "profile_total": 412,
            "missing_profile_app_count": 3,
            "missing_profile_app_ids": ["app-hidden", "app-secret", "app-other"],
            "missing_profile_app_ids_truncated": true,
            "truncated": true,
            "note": "3 current-profile app records are missing. Only the first 250 of 312 matching profile apps are listed.",
            "apps": [
                {
                    "app_id": "app-crm",
                    "name": "CRM",
                    "description": "Deals",
                    "owner_email": "ceo@example.com",
                    "events": [{
                        "id": "event-1",
                        "name": "Ask",
                        "description": "Chat with the CRM",
                        "event_type": "simple_chat",
                        "kind": "chat",
                        "consumer_tool": "call_app_chat",
                        "route": "/ask",
                        "secret_config": { "token": "abc" }
                    }],
                    "events_status": "ok"
                },
                { "app_id": "app-payroll", "name": "Payroll", "events": [], "events_status": "error" }
            ]
        }))
        .await;

        let mut fields: Vec<&str> = listed
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        assert_eq!(
            fields,
            [
                "apps",
                "bot_scope",
                "complete",
                "note",
                "query",
                "returned",
                "status",
                "total"
            ]
        );
        assert_eq!(listed["status"], "partial");
        assert_eq!(
            (listed["total"].clone(), listed["returned"].clone()),
            (json!(1), json!(1))
        );
        let note = listed["note"].as_str().unwrap();
        assert!(!note.chars().any(|c| c.is_ascii_digit()), "{note}");
        assert_eq!(
            listed["apps"],
            json!([{
                "app_id": "app-crm",
                "name": "CRM",
                "description": "Deals",
                "events": [{
                    "id": "event-1",
                    "name": "Ask",
                    "description": "Chat with the CRM",
                    "event_type": "simple_chat",
                    "kind": "chat",
                    "consumer_tool": "call_app_chat",
                    "route": "/ask"
                }],
                "events_status": "ok"
            }])
        );
        for leaked in [
            "312",
            "412",
            "app-payroll",
            "app-hidden",
            "ceo@example.com",
            "abc",
        ] {
            assert!(!listed.to_string().contains(leaked), "{leaked} leaked");
        }

        let complete = listing(json!({
            "status": "partial",
            "complete": false,
            "total": 2,
            "returned": 2,
            "note": "One or more app interface inventories could not be read.",
            "apps": [
                { "app_id": "app-crm", "name": "CRM", "events": [], "events_status": "ok" },
                { "app_id": "app-payroll", "name": "Payroll", "events": [], "events_status": "error" }
            ]
        }))
        .await;
        assert_eq!(complete["status"], "ok");
        assert_eq!(complete["complete"], true);
        assert!(complete.get("note").is_none());

        let failed = listing(json!({
            "status": "error",
            "code": "app_inventory_unavailable",
            "retryable": true,
            "message": "The app inventory came back empty while this profile references 7 apps, so it could not be read."
        }))
        .await;
        assert_eq!(failed["status"], "error");
        assert_eq!(failed["code"], "app_inventory_unavailable");
        assert_eq!(failed["retryable"], true);
        assert!(!failed["message"].as_str().unwrap().contains('7'));

        let host = Arc::new(RecordingBridge {
            output: "Listing failed for 7 apps".to_string(),
            ..Default::default()
        });
        let bridge = BotToolBridge {
            bot: Arc::new(bot(&[BotCapability::SelectedApps])),
            inner: host,
        };
        let unreadable: Value =
            serde_json::from_str(&bridge.call("list_apps", json!({})).await).unwrap();
        assert_eq!(unreadable["status"], "error");
        assert!(!unreadable.to_string().contains('7'));
    }

    #[test]
    fn flux_tools_never_reach_any_bot() {
        let mut everything = bot(&BotCapability::ALL);
        everything.workspace_app_id = Some("app-workspace".into());
        let advertised = names(&everything, true);
        assert_eq!(advertised.len(), BOT_TOOLS.len());
        for name in &advertised {
            assert!(BOT_TOOLS.contains(name), "{name} is not a bot tool");
            assert!(
                find_bot_tool_spec(name).is_some(),
                "hosts can't resolve {name}"
            );
        }

        let flux_only: Vec<&str> = global_assistant_tool_specs(true)
            .into_iter()
            .chain(data_studio_specialist_tool_specs())
            .chain(scout_specialist_tool_specs())
            .chain(home_specialist_tool_specs())
            .chain(runtime_execution_tool_specs())
            .chain(workflow_context_tool_specs())
            .chain(cross_board_source_tool_specs())
            .chain(public_web_tool_specs())
            .map(|spec| spec.name)
            .filter(|name| !BOT_TOOLS.contains(name))
            .collect();
        for name in [
            "create_app",
            "fork_app",
            "acquire_app",
            "flowpilot_board",
            "flowpilot_widget",
            "flowpilot_home",
            "data_studio_agent",
            "project_scout",
            "navigate_view",
            "upsert_event",
            "execute_node",
            "query_execution_logs",
            "run_board_tests",
            "ui_inspect",
            "read_flowscript_source",
            "search_workspace",
            "graph_overlay_tool",
            "graph_element_tool",
            HTTP_REQUEST_TOOL_NAME,
        ] {
            assert!(flux_only.contains(&name), "{name} must stay Flux's");
        }
        for name in &flux_only {
            assert!(!advertised.contains(name));
            assert!(
                find_bot_tool_spec(name).is_none(),
                "hosts resolve Flux's {name}"
            );
            assert_eq!(
                code(&everything.prepare_tool_call(name, json!({})).unwrap_err()),
                "bot_tool_unavailable"
            );
        }
    }

    const HTTP_REQUEST_TOOL_NAME: &str = crate::flow::copilot::tool_spec::HTTP_REQUEST_TOOL;

    #[test]
    fn flowpilot_keeps_its_surface_tools_and_its_own_memory() {
        let flux: Vec<&str> = PlatformSurface::Orchestrator
            .tool_specs(true)
            .iter()
            .map(|spec| spec.name)
            .collect();
        let global: Vec<&str> = global_assistant_tool_specs(true)
            .iter()
            .map(|spec| spec.name)
            .collect();
        assert_eq!(flux, global);
        assert!(flux.contains(&"flowpilot_board"));

        let scout = bot(&[]);
        let own = MemoryScope::Bot("bot_scout".into());
        let profile = MemoryScope::Profile("profile-1".into());
        assert_eq!(bot_memory_error(None, &scout, "profile-1"), None);
        assert_eq!(bot_memory_error(Some(&own), &scout, "profile-1"), None);
        for foreign in [MemoryScope::Bot("bot_other".into()), profile.clone()] {
            assert_eq!(
                bot_memory_error(Some(&foreign), &scout, "profile-1").as_deref(),
                Some("Bot 'Scout' can only use its own memory")
            );
        }
        assert_eq!(flowpilot_memory_error(None), None);
        assert_eq!(flowpilot_memory_error(Some(&profile)), None);
        assert!(flowpilot_memory_error(Some(&own)).is_some());

        let flux = BotRunContext {
            bot_id: "bot_flux".into(),
            system: Some(BotSystemKind::Flux),
            toolset: BotToolset::FlowPilot,
            ..Default::default()
        };
        assert_eq!(flux.memory_scope("profile-1"), profile);
        assert_eq!(bot_memory_error(Some(&profile), &flux, "profile-1"), None);
        for foreign in [
            MemoryScope::Bot("bot_flux".into()),
            MemoryScope::Profile("profile-2".into()),
        ] {
            assert!(bot_memory_error(Some(&foreign), &flux, "profile-1").is_some());
        }
        let clone = BotRunContext {
            bot_id: "bot_builder".into(),
            toolset: BotToolset::FlowPilot,
            ..Default::default()
        };
        assert_eq!(
            clone.memory_scope("profile-1"),
            MemoryScope::Bot("bot_builder".into())
        );
    }

    #[test]
    fn the_flowpilot_toolset_is_todays_flowpilot_with_the_persona_on_top() {
        let untouched = BotRunContext {
            bot_id: "bot_flux".into(),
            system: Some(BotSystemKind::Flux),
            toolset: BotToolset::FlowPilot,
            name: FLUX_BOT_NAME.into(),
            capabilities: vec![BotCapability::Internet],
            ..Default::default()
        };
        let flowpilot = global_assistant_system_prompt_for(WebResearchCapability::Inline);
        assert_eq!(bot_persona_section(&untouched), None);
        assert_eq!(bot_system_prompt(&untouched), flowpilot);
        let global: Vec<&str> = global_assistant_tool_specs(true)
            .iter()
            .map(|spec| spec.name)
            .collect();
        assert_eq!(names(&untouched, true), global);

        let edited = BotRunContext {
            name: "Fluxy".into(),
            description: Some("Builds my CRM".into()),
            instructions: Some("Always add tests.".into()),
            ..untouched.clone()
        };
        let prompt = bot_system_prompt(&edited);
        assert!(prompt.starts_with(&flowpilot));
        let persona = &prompt[flowpilot.len()..];
        assert!(persona.contains("your owner calls you Fluxy"));
        assert!(persona.contains("Builds my CRM"));
        assert!(persona.contains("## YOUR OWNER'S INSTRUCTIONS"));
        assert!(persona.contains("Always add tests."));

        let only_instructions = BotRunContext {
            instructions: Some("Answer in German.".into()),
            ..untouched.clone()
        };
        let persona = bot_persona_section(&only_instructions).unwrap();
        assert!(!persona.contains("WHO YOU ARE"));
        assert!(persona.contains("Answer in German."));

        let board = crate::flow::copilot::GlobalOpenBoardContext {
            app_id: "app-crm".into(),
            board_id: Some("board-1".into()),
            ..Default::default()
        };
        let input = || PlatformContextInput {
            open_board: Some(&board),
            ..Default::default()
        };
        let context = bot_platform_context(&untouched, input());
        assert!(context.contains("flowpilot_board"));
        let today = build_platform_context(input());
        let strip_time = |text: &str| {
            text.lines()
                .filter(|line| !line.starts_with("Current UTC timestamp"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(strip_time(&context), strip_time(&today));
    }

    #[tokio::test]
    async fn model_resolution_tries_primary_then_fallbacks_in_order() {
        let policy = BotModelPolicy {
            primary: Some(" model-a ".into()),
            fallbacks: vec![
                "".into(),
                "model-b".into(),
                "model-a".into(),
                "model-c".into(),
            ],
        };
        let resolve = |runnable: &'static [&'static str]| {
            let policy = policy.clone();
            async move {
                let tried = Arc::new(Mutex::new(Vec::new()));
                let seen = tried.clone();
                let chosen = resolve_bot_model(&policy, move |model_id: String| {
                    seen.lock().unwrap().push(model_id.clone());
                    std::future::ready(runnable.contains(&model_id.as_str()))
                })
                .await;
                let tried = tried.lock().unwrap().clone();
                (chosen, tried)
            }
        };

        assert_eq!(
            resolve(&["model-a", "model-c"]).await,
            (Some("model-a".into()), vec!["model-a".to_string()])
        );
        assert_eq!(
            resolve(&["model-c", "model-b"]).await,
            (
                Some("model-b".into()),
                vec!["model-a".to_string(), "model-b".to_string()]
            )
        );
        assert_eq!(resolve(&["model-c"]).await.0.as_deref(), Some("model-c"));
        assert_eq!(
            resolve(&[]).await,
            (
                None,
                vec![
                    "model-a".to_string(),
                    "model-b".to_string(),
                    "model-c".to_string()
                ]
            )
        );
        assert_eq!(
            resolve_bot_model(&BotModelPolicy::default(), |_| std::future::ready(true)).await,
            None
        );
    }

    #[test]
    fn bot_context_leaves_board_routing_to_flux() {
        let reader = bot(&[BotCapability::DataStudio, BotCapability::SelectedApps]);
        let board = crate::flow::copilot::GlobalOpenBoardContext {
            app_id: "app-crm".into(),
            board_id: Some("board-1".into()),
            ..Default::default()
        };
        let open_data = |app_id: &str| GlobalDataStudioContext {
            app_id: app_id.into(),
            app_name: Some("CRM".into()),
            ..Default::default()
        };
        let attachments = [AttachmentManifestEntry {
            name: Some("deals.csv".into()),
            mime_type: Some("text/csv".into()),
            size: Some(2048),
            url: None,
        }];
        let crm = open_data("app-crm");
        let context = bot_platform_context(
            &reader,
            PlatformContextInput {
                user_context: Some("Ada"),
                active_profile: Some(("Work", "profile-1")),
                switchable_profiles: &["Home".to_string()],
                open_board: Some(&board),
                open_data_studio: Some(&crm),
                attachments: &attachments,
            },
        );
        assert!(context.contains("Signed-in user: Ada."));
        assert!(context.contains("Active profile: \"Work\" (id: profile-1)."));
        assert!(!context.contains("Profiles the user can switch to"));
        assert!(!context.contains("flowpilot_board"));
        assert!(!context.contains("data_studio_agent"));
        assert!(context.contains("## CURRENTLY OPEN DATA STUDIO"));
        assert!(context.contains("app_id=\"app-crm\""));
        assert!(context.contains("- deals.csv (text/csv, 2.0 KB)"));
        assert!(context.contains("`call_app_chat` `forward_files`"));

        let payroll = open_data("app-payroll");
        let elsewhere = bot_platform_context(
            &reader,
            PlatformContextInput {
                open_data_studio: Some(&payroll),
                ..Default::default()
            },
        );
        assert!(!elsewhere.contains("CURRENTLY OPEN DATA STUDIO"));
        assert!(!elsewhere.contains("app-payroll"));
        assert!(!elsewhere.contains("FILES ATTACHED"));
    }

    #[test]
    fn a_bot_without_internet_gets_no_public_web_route() {
        let prompt = crate::flow::copilot::global_assistant_system_prompt_for(
            WebResearchCapability::Disabled,
        );
        assert!(prompt.contains("this run has no public-web access"));
        assert!(!prompt.contains("PUBLIC-WEB FALLBACK"));
        assert!(!prompt.contains("{public_web_route}"));
        assert!(!prompt.contains("\n\n\n\n"));
        assert_eq!(bot(&[]).web_research(), WebResearchCapability::Disabled);
    }

    #[test]
    fn a_turn_without_a_chat_gets_only_what_its_host_runs() {
        let mut everything = bot(&BotCapability::ALL);
        everything.workspace_app_id = Some("app-workspace".into());
        assert_eq!(everything.tool_host, BotToolHost::Chat);
        assert_eq!(names(&everything, true).len(), BOT_TOOLS.len());

        let native = BotRunContext {
            tool_host: BotToolHost::Native,
            ..everything.clone()
        };
        assert_eq!(
            names(&native, true),
            [
                RESEARCH_AGENT_TOOL,
                "list_apps",
                "describe_app_interface",
                "get_app_detail",
                "call_app_event",
                "call_app_chat",
                DATABASE_TOOL,
                GRAPH_QUERY_TOOL,
                STORAGE_TOOL,
                MEMORY_STORE_TOOL,
                MEMORY_SEARCH_TOOL,
            ]
        );
        for tool_name in CHAT_ONLY_TOOLS {
            assert_eq!(
                code(
                    &native
                        .prepare_tool_call(tool_name, json!({ "app_id": "app-crm" }))
                        .unwrap_err()
                ),
                "bot_tool_unavailable"
            );
        }
        let prompt = bot_system_prompt(&native);
        assert!(prompt.contains("No one is watching this turn"));
        assert!(prompt.contains("aren't available in this run"));
        for chat_only in ["`ask_user`", "`open_app_page`", "`open_app_chat`"] {
            assert!(!prompt.contains(chat_only), "{chat_only}");
        }

        let core_only = BotRunContext {
            tool_host: BotToolHost::CoreOnly,
            ..everything
        };
        assert_eq!(
            names(&core_only, true),
            [RESEARCH_AGENT_TOOL, MEMORY_STORE_TOOL, MEMORY_SEARCH_TOOL]
        );
        let prompt = bot_system_prompt(&core_only);
        assert!(prompt.contains("Profile apps (not available in this run)"));
        assert!(prompt.contains("Selected apps (not available in this run)"));
        assert!(prompt.contains("Files (not available in this run)"));
        assert!(prompt.contains("Data Studio (not available in this run)"));
        assert!(!prompt.contains("`list_apps`"));
        assert!(!prompt.contains("`database_tool`"));
    }

    /// Flux and its duplicates get FlowPilot's whole set in a chat, but a turn without one still
    /// gets only what its host runs.
    #[test]
    fn a_flowpilot_turn_without_a_chat_gets_only_what_its_host_runs() {
        let flux = BotRunContext {
            toolset: BotToolset::FlowPilot,
            capabilities: vec![BotCapability::Flowpilot],
            ..bot(&[])
        };
        let in_chat = names(&flux, true);
        assert!(in_chat.contains(&"ask_user") && in_chat.contains(&"open_app_page"));
        for tool_host in [BotToolHost::Native, BotToolHost::CoreOnly] {
            let offered = names(
                &BotRunContext {
                    tool_host,
                    ..flux.clone()
                },
                true,
            );
            for chat_only in CHAT_ONLY_TOOLS {
                assert!(
                    !offered.contains(&chat_only),
                    "{tool_host:?} offers {chat_only}"
                );
            }
            let runnable: Vec<&str> = in_chat
                .iter()
                .copied()
                .filter(|name| tool_host.runs(name))
                .collect();
            assert_eq!(offered, runnable, "{tool_host:?}");
        }
        let core_only = names(
            &BotRunContext {
                tool_host: BotToolHost::CoreOnly,
                ..flux
            },
            true,
        );
        assert!(
            core_only.contains(&MEMORY_STORE_TOOL) && core_only.contains(&MEMORY_SEARCH_TOOL),
            "{core_only:?}"
        );
        assert!(
            core_only.iter().all(|name| CORE_TOOLS.contains(name)),
            "{core_only:?}"
        );
    }

    struct FixedCrew(Vec<CrewMember>);

    #[async_trait]
    impl BotTalkHost for FixedCrew {
        async fn roster(&self, _: &CrewMember) -> flow_like_types::Result<Vec<CrewMember>> {
            Ok(self.0.clone())
        }

        async fn has_grant(&self, _: &CrewMember, _: &CrewMember) -> flow_like_types::Result<bool> {
            Ok(false)
        }

        async fn store_grant(&self, _: &CrewMember, _: &CrewMember) -> flow_like_types::Result<()> {
            Ok(())
        }

        async fn exchange_conversation(
            &self,
            _: &CrewMember,
            _: &CrewMember,
        ) -> flow_like_types::Result<String> {
            Err(flow_like_types::anyhow!("no inbox"))
        }

        async fn record(
            &self,
            _: &str,
            _: &CrewMember,
            _: &str,
        ) -> flow_like_types::Result<String> {
            Err(flow_like_types::anyhow!("no inbox"))
        }

        async fn run_turn(&self, _: NestedBotTurn) -> flow_like_types::Result<String> {
            Err(flow_like_types::anyhow!("no turns"))
        }
    }

    fn crew_member(id: &str, name: &str) -> CrewMember {
        CrewMember {
            id: id.into(),
            name: name.into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_bot_with_a_crew_can_ask_it_and_core_answers_ask_bot() {
        let host: Arc<dyn BotTalkHost> = Arc::new(FixedCrew(vec![crew_member(
            "bot_digest",
            "Research digest",
        )]));
        let scout = Arc::new(bot(&[BotCapability::Internet]));
        let talk = Arc::new(BotTalk::for_run(host.clone(), &scout, BotLocation::Desktop));
        assert!(talk.interactive());
        let inner = Arc::new(RecordingBridge::default());

        let turn = prepare_bot_turn(
            scout.clone(),
            "Signed-in user: Ada.",
            false,
            inner.clone(),
            Some(&talk),
        )
        .await;

        let offered: Vec<&str> = turn.tool_specs.iter().map(|spec| spec.name).collect();
        assert!(offered.contains(&"ask_bot") && offered.contains(&"ask_user"));
        assert!(turn.system_prompt.contains("## YOUR CREW"));
        assert!(
            turn.system_prompt
                .contains("Research digest (id: bot_digest)")
        );
        assert!(turn.system_prompt.ends_with("Signed-in user: Ada."));
        let answer = turn
            .bridge
            .call("ask_bot", json!({ "bot": "Payroll", "message": "Hi?" }))
            .await;
        assert_eq!(code(&answer), "bot_not_found");
        assert!(inner.calls.lock().unwrap().is_empty());

        let alone = prepare_bot_turn(scout.clone(), "", false, inner.clone(), None).await;
        assert!(alone.tool_specs.iter().all(|spec| spec.name != "ask_bot"));
        assert!(!alone.system_prompt.contains("YOUR CREW"));
        let no_crew: Arc<dyn BotTalkHost> = Arc::new(FixedCrew(Vec::new()));
        let lonely = Arc::new(BotTalk::for_run(no_crew, &scout, BotLocation::Desktop));
        let lonely = prepare_bot_turn(scout, "", false, inner, Some(&lonely)).await;
        assert!(lonely.tool_specs.iter().all(|spec| spec.name != "ask_bot"));
        assert!(!lonely.system_prompt.contains("YOUR CREW"));
    }

    #[tokio::test]
    async fn a_turn_another_bot_asked_for_never_runs_on_a_chat_host() {
        let host: Arc<dyn BotTalkHost> = Arc::new(FixedCrew(vec![
            crew_member("bot_digest", "Research digest"),
            crew_member("bot_ledger", "Ledger"),
        ]));
        let ledger = Arc::new(bot(&[BotCapability::Internet, BotCapability::SelectedApps]));
        assert_eq!(ledger.tool_host, BotToolHost::Chat);
        let scout = CrewMember {
            id: "bot_scout".into(),
            name: "Scout".into(),
            ..Default::default()
        };
        let nested = Arc::new(BotTalk::new(host.clone(), scout.clone(), true).nested(
            &CrewMember {
                id: ledger.bot_id.clone(),
                name: "Ledger".into(),
                ..Default::default()
            },
            Default::default(),
        ));
        assert!(nested.is_nested() && !nested.interactive());

        let turn = prepare_bot_turn(
            ledger.clone(),
            "",
            false,
            Arc::new(RecordingBridge::default()),
            Some(&nested),
        )
        .await;

        let offered: Vec<&str> = turn.tool_specs.iter().map(|spec| spec.name).collect();
        for chat_only in CHAT_ONLY_TOOLS {
            assert!(!offered.contains(&chat_only), "{chat_only} offered");
        }
        assert!(offered.contains(&"ask_bot") && offered.contains(&"list_apps"));
        assert!(
            turn.system_prompt
                .contains("## THIS TURN\nScout, another of your owner's bots")
        );
        assert!(turn.system_prompt.contains("No one is watching this turn"));

        let deepest = BotTalk::new(host.clone(), scout.clone(), true);
        let mut chain = Arc::new(deepest);
        for hop in 0..crate::flow::copilot::bot_talk::MAX_ASK_HOPS {
            let callee = CrewMember {
                id: format!("bot_hop_{hop}"),
                ..Default::default()
            };
            chain = Arc::new(chain.nested(&callee, Default::default()));
        }
        let limit = prepare_bot_turn(
            ledger,
            "",
            false,
            Arc::new(RecordingBridge::default()),
            Some(&chain),
        )
        .await;
        assert!(limit.tool_specs.iter().all(|spec| spec.name != "ask_bot"));
    }

    fn memory_tool_names(bot: &BotRunContext) -> Vec<&'static str> {
        names(bot, false)
            .into_iter()
            .filter(|name| flow_like_bot_memory::host::MemoryToolbox::handles(name))
            .collect()
    }

    #[tokio::test]
    async fn memory_tools_need_a_bot_that_remembers_and_a_host_that_opened_its_memory() {
        let handle = super::super::memory_tools::test_handle().await;
        let with_memory = |bot: BotRunContext| BotRunContext {
            host_tools: HostTools::none().with_memory(handle.clone()),
            ..bot
        };

        assert!(memory_tool_names(&with_memory(bot(&[]))).is_empty());
        assert_eq!(
            memory_tool_names(&with_memory(bot(&[BotCapability::Memory]))),
            [
                "memory_search",
                "history_search",
                "history_read",
                "message_get",
                "summary_expand",
                "memory_save",
                "memory_forget"
            ]
        );
        let flowpilot = BotRunContext {
            toolset: BotToolset::FlowPilot,
            ..bot(&[BotCapability::Flowpilot])
        };
        assert_eq!(memory_tool_names(&with_memory(flowpilot)).len(), 7);
        let core_only = BotRunContext {
            tool_host: BotToolHost::CoreOnly,
            ..bot(&[BotCapability::Memory])
        };
        assert_eq!(memory_tool_names(&with_memory(core_only)).len(), 7);
        assert!(memory_tool_names(&bot(&[BotCapability::Memory])).is_empty());
    }

    #[tokio::test]
    async fn the_memory_bridge_answers_memory_tools_itself_and_holds_writes_after_other_tools() {
        let handle = super::super::memory_tools::test_handle().await;
        let remembering = Arc::new(BotRunContext {
            host_tools: HostTools::none().with_memory(handle),
            ..bot(&[BotCapability::Memory, BotCapability::SelectedApps])
        });
        let host = Arc::new(RecordingBridge {
            output: json!({ "status": "ok" }).to_string(),
            ..Default::default()
        });
        let turn = prepare_bot_turn(remembering, "", false, host.clone(), None).await;
        let status = |answer: &str| {
            serde_json::from_str::<Value>(answer).unwrap()["status"]
                .as_str()
                .unwrap()
                .to_string()
        };

        let saved = turn
            .bridge
            .call(
                "memory_save",
                json!({ "text": "The user's cat is called Mia.", "kind": "profile" }),
            )
            .await;
        assert_eq!(status(&saved), "ok", "{saved}");
        assert!(host.calls.lock().unwrap().is_empty());

        turn.bridge.call("list_apps", json!({})).await;
        let held = turn
            .bridge
            .call("memory_save", json!({ "text": "The user likes tea." }))
            .await;
        assert_eq!(status(&held), "needs_confirmation", "{held}");

        let quiet = Arc::new(bot(&[BotCapability::Memory]));
        let plain = prepare_bot_turn(quiet, "", false, host.clone(), None).await;
        assert!(
            plain
                .tool_specs
                .iter()
                .all(|spec| spec.name != "memory_save")
        );
    }
}
