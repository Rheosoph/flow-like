use std::{collections::HashMap, time::SystemTime};

use flow_like_storage::{Path, object_store};
use flow_like_types::{FromProto, ToProto, create_id, proto};
use futures::{StreamExt, TryStreamExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    app::App,
    state::FlowLikeState,
    utils::compression::{compress_to_file, compress_to_file_create, from_compressed},
};

use super::{
    board::VersionType, compiled::prerun::PrerunPageExecution, pin::PinType, variable::Variable,
};

/// Simplified input pin metadata for events (used when board can't be fetched)
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct EventInput {
    pub id: String,
    pub name: String,
    pub friendly_name: String,
    pub description: String,
    pub data_type: String,
    pub value_type: String,
    pub schema: Option<String>,
    pub default_value: Option<Vec<u8>>,
    pub index: u16,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub enum ReleaseNotes {
    NOTES(String),
    URL(String),
}

/// Where a single event runs. Unlike `Board::ExecutionMode`, events are never
/// Hybrid — an event is always bound to exactly one environment so its
/// configuration (URLs, credentials, tutorials) can be unambiguous.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EventExecutionMode {
    /// Runs on the user's device (desktop app / synced into local sqlite).
    #[default]
    Local,
    /// Runs on the server (cloud endpoint, cron worker, sink service).
    Remote,
}

impl EventExecutionMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventExecutionMode::Local => "Local",
            EventExecutionMode::Remote => "Remote",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "Remote" | "remote" | "REMOTE" => EventExecutionMode::Remote,
            _ => EventExecutionMode::Local,
        }
    }
}

/// Where an event is reachable from. `Public` events with a REST/MCP surface
/// are served on the public inbound routers with their configured auth.
/// `Internal` events are only callable by connected apps through the
/// app-connection proxy (gated by the connection role) and are never exposed
/// publicly — so a public secret can never be bypassed via the proxy.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventExposure {
    /// Public HTTP surface (REST/MCP), protected by the event's own auth.
    #[default]
    Public,
    /// Reachable only via app connections, never publicly.
    Internal,
}

impl EventExposure {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventExposure::Public => "PUBLIC",
            EventExposure::Internal => "INTERNAL",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "Internal" | "internal" | "INTERNAL" => EventExposure::Internal,
            _ => EventExposure::Public,
        }
    }

    pub fn is_internal(&self) -> bool {
        matches!(self, EventExposure::Internal)
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct CanaryEvent {
    pub weight: f32,
    pub variables: HashMap<String, Variable>,
    pub board_id: String,
    pub board_version: Option<(u32, u32, u32)>,
    pub node_id: String,
    pub created_at: std::time::SystemTime,
    pub updated_at: std::time::SystemTime,
}

/// How a variant handles its share of traffic: `Live` replaces the primary
/// target for `weight` of triggers, `Shadow` additionally runs the variant for
/// `sample_rate` of triggers and discards the result.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub enum EventVariantMode {
    Live { weight: f32 },
    Shadow { sample_rate: f32 },
}

impl EventVariantMode {
    /// The traffic share, regardless of mode. Raw as stored — callers that
    /// serve traffic read through [`Event::variant_set`], which clamps.
    pub fn share(&self) -> f32 {
        match self {
            EventVariantMode::Live { weight } => *weight,
            EventVariantMode::Shadow { sample_rate } => *sample_rate,
        }
    }

    fn clamp(&mut self) {
        match self {
            EventVariantMode::Live { weight } => *weight = clamp_unit(*weight),
            EventVariantMode::Shadow { sample_rate } => *sample_rate = clamp_unit(*sample_rate),
        }
    }

    /// Bitwise share comparison so a stored NaN reads as unchanged (and stays
    /// grandfathered) instead of tripping validation on every save.
    fn content_equal(&self, other: &EventVariantMode) -> bool {
        match (self, other) {
            (EventVariantMode::Live { weight: a }, EventVariantMode::Live { weight: b }) => {
                a.to_bits() == b.to_bits()
            }
            (
                EventVariantMode::Shadow { sample_rate: a },
                EventVariantMode::Shadow { sample_rate: b },
            ) => a.to_bits() == b.to_bits(),
            _ => false,
        }
    }
}

fn clamp_unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// An alternate dispatch target for an event: a canary (`Live`) or shadow
/// deployment of another board/version/node. Read through
/// [`Event::variant_set`], which falls back to the legacy `canary` field.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct EventVariant {
    /// Unique per event, `[a-z0-9-]`; "canary" is the conventional name.
    pub name: String,
    pub board_id: String,
    /// `None` floats on latest (the etag is bound at dispatch); the `u32::MAX`
    /// sentinel is reserved and refused at upsert.
    pub board_version: Option<(u32, u32, u32)>,
    pub node_id: String,
    /// Merged over the event's variables per key, variant wins.
    pub variables: HashMap<String, Variable>,
    /// Page-event variants only: repoints what a matched route renders.
    pub default_page_id: Option<String>,
    pub mode: EventVariantMode,
    pub created_at: std::time::SystemTime,
    pub updated_at: std::time::SystemTime,
}

impl EventVariant {
    /// Equality minus bookkeeping (timestamps): the "did this variant change
    /// in this upsert" predicate that gates validation — see
    /// [`Event::validate_variants`].
    fn content_equal(&self, other: &EventVariant) -> bool {
        self.name == other.name
            && self.board_id == other.board_id
            && self.board_version == other.board_version
            && self.node_id == other.node_id
            && self.variables == other.variables
            && self.default_page_id == other.default_page_id
            && self.mode.content_equal(&other.mode)
    }

    fn is_live(&self) -> bool {
        matches!(self.mode, EventVariantMode::Live { .. })
    }
}

fn valid_variant_name(name: &str) -> bool {
    // "stable" addresses the PRIMARY target on every variant-aware surface
    // (setup, inbound pins, registrations) — a user variant by that name
    // would be unreachable everywhere.
    !name.is_empty()
        && name != "stable"
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Event {
    pub id: String,
    pub name: String,
    pub description: String,
    pub board_id: String,
    pub board_version: Option<(u32, u32, u32)>,
    pub node_id: String,
    pub variables: HashMap<String, Variable>,
    pub config: Vec<u8>,
    pub active: bool,

    pub canary: Option<CanaryEvent>,

    /// Canary/shadow targets. Read through [`Event::variant_set`]: when this
    /// is empty a legacy `canary` still counts as a single Live variant.
    #[serde(default)]
    pub variants: Vec<EventVariant>,

    pub priority: u32,
    pub event_type: String,
    pub notes: Option<ReleaseNotes>,
    pub event_version: (u32, u32, u32),
    pub created_at: std::time::SystemTime,
    pub updated_at: std::time::SystemTime,

    // A2UI: default page to render for this event
    pub default_page_id: Option<String>,

    /// Input pins copied from the node (populated at upsert time)
    #[serde(default)]
    pub inputs: Vec<EventInput>,

    /// URL route path that maps to this event (e.g., "/", "/dashboard")
    #[serde(default)]
    pub route: Option<String>,

    /// Whether this is the default event/route for the app (shown at "/")
    #[serde(default)]
    pub is_default: bool,

    /// Where this event runs (Local/offline vs Remote/server). Inherited from
    /// the board's `execution_mode` when that mode is not Hybrid.
    #[serde(default)]
    pub execution_mode: EventExecutionMode,

    /// Whether the event is publicly reachable or only callable by connected
    /// apps via the app-connection proxy. Only meaningful for REST/MCP events.
    #[serde(default)]
    pub exposure: EventExposure,

    /// Process-mining case-key mappings: business key name → dot-path into
    /// the invocation payload (e.g. `order_id` → `order.id`). Extracted on
    /// every run so cases group by business object automatically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_mappings: Option<std::collections::HashMap<String, String>>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ChatVoiceParameters {
    /// Capture mode: "disabled" | "stt" (frontend transcript) | "record" (audio).
    pub mode: Option<String>,
    /// Invoke mode: "manual" | "hold" | "auto".
    pub invoke: Option<String>,
    /// Visual style: "conservative" | "waveform" | "orb" | "vortex" | "shader".
    pub variant: Option<String>,
    /// Element size: "sm" | "md" | "lg".
    pub size: Option<String>,
    /// Base accent color (CSS color string).
    pub color: Option<String>,
    /// Accent color while recording (CSS color string).
    pub recording_color: Option<String>,
    /// Answer playback: "text" | "audio" | "both".
    pub playback: Option<String>,
    pub max_duration: Option<u32>,
    pub auto_stop: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ChatEventParameters {
    pub history_elements: Option<u32>,
    pub allow_file_upload: Option<bool>,
    pub allow_voice_input: Option<bool>,
    pub allow_voice_output: Option<bool>,
    pub allow_voice_mode: Option<bool>,
    pub voice: Option<ChatVoiceParameters>,
    pub tools: Option<Vec<String>>,
    pub default_tools: Option<Vec<String>>,
    pub example_messages: Option<Vec<String>>,
    /// Attach PNG snapshots of the latest assistant message's embedded widgets
    /// to the outgoing user turn so vision-capable models see the rendered UI.
    /// Defaults to enabled.
    pub attach_widget_snapshots: Option<bool>,
    /// Custom CSS scoped to the chat interface.
    pub custom_css: Option<String>,
    /// Background image URL or app storage path for the chat interface.
    pub background_image: Option<String>,
    /// Mark shown on the empty chat: "none" | "planet" | "bubble" | "image".
    /// Defaults to "planet".
    pub placeholder_visual: Option<String>,
    /// Which orb state the bubble placeholder rests in:
    /// "idle" | "ready" | "thinking" | "working".
    pub placeholder_bubble_state: Option<String>,
    /// Image URL or app storage path for the "image" placeholder.
    pub placeholder_image: Option<String>,
    /// Let the placeholder mark react while the user types: it leans toward the
    /// composer and stirs in proportion to how fast they write. Applies to the
    /// "planet" and "bubble" marks. Defaults to disabled.
    pub placeholder_typing_motion: Option<bool>,
    /// Preferred chat color scheme: "system" | "light" | "dark".
    pub color_scheme: Option<String>,
    /// User-facing disclosure that the conversation is with an AI.
    pub ai_disclosure: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct EmailEventParameters {
    pub mail: Option<String>,
    pub sender_name: Option<String>,
    pub smtp_server: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_username: Option<String>,
    pub secret_smtp_password: Option<String>,
    pub imap_server: Option<String>,
    pub imap_port: Option<u16>,
    pub imap_username: Option<String>,
    pub secret_imap_password: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ApiEventParameters {
    pub path_suffix: Option<String>,
    pub method: Option<String>,
    pub public_endpoint: Option<bool>,
}

#[allow(clippy::large_enum_variant)]
// schema + deserialisation target only; no runtime variant construction, so the gap never materialises
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
#[serde(untagged)]
pub enum EventPayload {
    ChatEvent(ChatEventParameters),
    MailEvent(EmailEventParameters),
    ApiEvent(ApiEventParameters),
    AnyEvent(HashMap<String, flow_like_types::Value>),
    QuickAction,
}

/// Whether a failed [`Event::load`] means the object is genuinely absent, as
/// opposed to unreadable (transient store error, truncated/corrupt bytes).
/// Only absence may fall through to the create branch of [`Event::upsert`] —
/// anything else must abort the write, or one flaky read forks the event into
/// a duplicate under a fresh id.
fn load_error_is_not_found(error: &flow_like_types::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<object_store::Error>(),
            Some(object_store::Error::NotFound { .. })
        )
    })
}

/// Filter out secret variable values from an event.
/// Secret variables will have their `default_value` set to `None`.
/// This should be used when returning events to clients, as secrets
/// are only used server-side during execution.
pub fn filter_event_secrets(mut event: Event) -> Event {
    blank_secret_values(&mut event.variables);

    if let Some(ref mut canary) = event.canary {
        blank_secret_values(&mut canary.variables);
    }
    for variant in event.variants.iter_mut() {
        blank_secret_values(&mut variant.variables);
    }

    event
}

fn blank_secret_values(variables: &mut HashMap<String, Variable>) {
    for var in variables.values_mut() {
        if var.secret {
            var.default_value = None;
        }
    }
}

/// Restore secret variable values the client had no way to send back.
///
/// `filter_event_secrets` blanks secret `default_value`s on the way out, so a client
/// round-tripping an event returns them as `None`. Without this merge every save from
/// the events UI would erase the stored secret. A secret the client does send is a
/// deliberate change and wins.
pub fn preserve_event_secrets(incoming: &mut Event, existing: &Event) {
    restore_secret_values(&mut incoming.variables, &existing.variables);

    if let (Some(canary), Some(existing_canary)) =
        (incoming.canary.as_mut(), existing.canary.as_ref())
    {
        restore_secret_values(&mut canary.variables, &existing_canary.variables);
    }

    // Matching by name through `variant_set` covers the migration save where
    // the stored event still carries only the legacy canary but the client
    // sends it back as a variant named "canary".
    let existing_variants = existing.variant_set();
    for variant in incoming.variants.iter_mut() {
        if let Some(existing_variant) = existing_variants
            .iter()
            .find(|candidate| candidate.name == variant.name)
        {
            restore_secret_values(&mut variant.variables, &existing_variant.variables);
        }
    }
}

fn restore_secret_values(
    incoming: &mut HashMap<String, Variable>,
    existing: &HashMap<String, Variable>,
) {
    for (variable_id, variable) in incoming.iter_mut() {
        if !variable.secret || variable.default_value.is_some() {
            continue;
        }

        if let Some(stored) = existing.get(variable_id) {
            variable.default_value = stored.default_value.clone();
        }
    }
}

pub fn canary_equal(a: &Option<CanaryEvent>, b: &Option<CanaryEvent>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            a.board_id == b.board_id
                && a.board_version == b.board_version
                && a.node_id == b.node_id
                && a.weight == b.weight
                && a.variables == b.variables
        }
        (None, None) => true,
        _ => false,
    }
}

/// How [`Event::plan_restore`] treats the fields a restore does not blindly
/// copy. Every flag defaults to `false` — the safe direction: routing stays
/// live, the snapshot's canary is kept, and a blank secret blocks.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Default)]
pub struct RestoreOptions {
    pub restore_route: bool,
    pub drop_canary: bool,
    pub accept_blank_secrets: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestoreIssueSeverity {
    Blocking,
    Warning,
}

/// `RouteConflict` and `CronScheduleUnchanged` are emitted by the API layer
/// (they need the Postgres row); they live here so one enum describes every
/// issue a restore plan can carry.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestoreIssueCode {
    BoardMissing,
    BoardVersionMissing,
    NodeMissing,
    PageMissing,
    EventTypeChanged,
    TargetKindChanged,
    FloatingBoard,
    SecretUnrecoverable,
    RouteConflict,
    CronScheduleUnchanged,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct RestoreIssue {
    pub code: RestoreIssueCode,
    pub severity: RestoreIssueSeverity,
    pub message: String,
    pub subject: Option<String>,
}

/// One display-level field difference between the live event and the plan's
/// `restored`. `from`/`to` are human-readable state strings and never carry a
/// variable value — secrets must not leak through the plan.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct RestoreFieldChange {
    pub field: String,
    pub from: String,
    pub to: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct RestorePlan {
    pub restored: Event,
    pub diff: Vec<RestoreFieldChange>,
    pub not_restored: Vec<String>,
    pub issues: Vec<RestoreIssue>,
}

fn display_board_version(version: &Option<(u32, u32, u32)>) -> String {
    match version {
        Some((major, minor, patch)) => format!("{major}.{minor}.{patch}"),
        None => "latest".to_string(),
    }
}

fn display_notes_kind(notes: &Option<ReleaseNotes>) -> String {
    match notes {
        Some(ReleaseNotes::NOTES(_)) => "notes",
        Some(ReleaseNotes::URL(_)) => "url",
        None => "none",
    }
    .to_string()
}

fn push_field_change(diff: &mut Vec<RestoreFieldChange>, field: &str, from: String, to: String) {
    if from != to {
        diff.push(RestoreFieldChange {
            field: field.to_string(),
            from,
            to,
        });
    }
}

fn collect_blank_secret_ids(variables: &HashMap<String, Variable>, out: &mut Vec<String>) {
    for (variable_id, variable) in variables {
        if variable.secret && variable.default_value.is_none() {
            out.push(variable_id.clone());
        }
    }
}

fn restore_diff(live: &Event, restored: &Event) -> Vec<RestoreFieldChange> {
    let mut diff = Vec::new();
    push_field_change(&mut diff, "name", live.name.clone(), restored.name.clone());
    push_field_change(
        &mut diff,
        "description",
        live.description.clone(),
        restored.description.clone(),
    );
    push_field_change(
        &mut diff,
        "board_id",
        live.board_id.clone(),
        restored.board_id.clone(),
    );
    push_field_change(
        &mut diff,
        "board_version",
        display_board_version(&live.board_version),
        display_board_version(&restored.board_version),
    );
    push_field_change(
        &mut diff,
        "node_id",
        live.node_id.clone(),
        restored.node_id.clone(),
    );
    push_field_change(
        &mut diff,
        "event_type",
        live.event_type.clone(),
        restored.event_type.clone(),
    );
    push_field_change(
        &mut diff,
        "active",
        live.active.to_string(),
        restored.active.to_string(),
    );
    push_field_change(
        &mut diff,
        "priority",
        live.priority.to_string(),
        restored.priority.to_string(),
    );
    push_field_change(
        &mut diff,
        "default_page_id",
        live.default_page_id
            .clone()
            .unwrap_or_else(|| "none".to_string()),
        restored
            .default_page_id
            .clone()
            .unwrap_or_else(|| "none".to_string()),
    );
    push_field_change(
        &mut diff,
        "notes",
        display_notes_kind(&live.notes),
        display_notes_kind(&restored.notes),
    );
    // Config bytes may carry auth material, so the diff only says that they
    // differ, never what they hold.
    if live.config != restored.config {
        diff.push(RestoreFieldChange {
            field: "config".to_string(),
            from: "current".to_string(),
            to: "changed".to_string(),
        });
    }
    // Variables diff by id and state only — values (secret or not) never
    // appear in the plan.
    let mut variable_ids: Vec<&String> = live
        .variables
        .keys()
        .chain(restored.variables.keys())
        .collect();
    variable_ids.sort();
    variable_ids.dedup();
    for variable_id in variable_ids {
        let field = format!("variables.{variable_id}");
        match (
            live.variables.get(variable_id),
            restored.variables.get(variable_id),
        ) {
            (None, Some(_)) => diff.push(RestoreFieldChange {
                field,
                from: "absent".to_string(),
                to: "present".to_string(),
            }),
            (Some(_), None) => diff.push(RestoreFieldChange {
                field,
                from: "present".to_string(),
                to: "absent".to_string(),
            }),
            (Some(live_variable), Some(restored_variable))
                if live_variable != restored_variable =>
            {
                diff.push(RestoreFieldChange {
                    field,
                    from: "live definition".to_string(),
                    to: "snapshot definition".to_string(),
                })
            }
            _ => {}
        }
    }
    diff
}

/// Mirror of [`Event::validate_event_references`], downgraded from `Err` to
/// plan issues: a target that vanished since the snapshot was cut is exactly
/// what a restore plan exists to report.
async fn push_target_resolution_issues(
    app: &App,
    restored: &Event,
    issues: &mut Vec<RestoreIssue>,
) {
    let board = match app
        .open_board(
            restored.board_id.clone(),
            Some(false),
            restored.board_version,
        )
        .await
    {
        Ok(board) => board,
        Err(_) => {
            // The unversioned open tells a missing board apart from a board
            // whose archived version is gone.
            let code = if restored.board_version.is_some()
                && app
                    .open_board(restored.board_id.clone(), Some(false), None)
                    .await
                    .is_ok()
            {
                RestoreIssueCode::BoardVersionMissing
            } else {
                RestoreIssueCode::BoardMissing
            };
            let message = match code {
                RestoreIssueCode::BoardVersionMissing => format!(
                    "board {} exists, but snapshot version {} is gone from its archive",
                    restored.board_id,
                    display_board_version(&restored.board_version)
                ),
                _ => format!(
                    "board {} referenced by the snapshot cannot be opened",
                    restored.board_id
                ),
            };
            issues.push(RestoreIssue {
                code,
                severity: RestoreIssueSeverity::Blocking,
                message,
                subject: Some(restored.board_id.clone()),
            });
            return;
        }
    };
    let board = board.lock().await;
    if let Some(page_id) = restored.default_page_id.as_deref() {
        if !board.page_ids.iter().any(|candidate| candidate == page_id) {
            issues.push(RestoreIssue {
                code: RestoreIssueCode::PageMissing,
                severity: RestoreIssueSeverity::Blocking,
                message: format!(
                    "page {} is not listed by board {}{}",
                    page_id,
                    restored.board_id,
                    restored
                        .board_version
                        .map(|version| format!(" at {}.{}.{}", version.0, version.1, version.2))
                        .unwrap_or_default()
                ),
                subject: Some(page_id.to_string()),
            });
        }
    } else if !board.nodes.contains_key(&restored.node_id) {
        issues.push(RestoreIssue {
            code: RestoreIssueCode::NodeMissing,
            severity: RestoreIssueSeverity::Blocking,
            message: format!(
                "node {} not found in board {}",
                restored.node_id, restored.board_id
            ),
            subject: Some(restored.node_id.clone()),
        });
    }
}

impl Event {
    /// Populate the inputs field from the board's node pins
    pub async fn populate_inputs(&mut self, app: &App) -> flow_like_types::Result<()> {
        let board = app
            .open_board(self.board_id.clone(), Some(true), self.board_version)
            .await?;

        let board_guard = board.lock().await;

        if let Some(node) = board_guard.nodes.get(&self.node_id) {
            // For page-target events (A2UI/generic form), we need Input pins (what user provides)
            // For regular events, we need Output pins (what the event produces)
            let target_pin_type = if self.default_page_id.is_some() {
                PinType::Input
            } else {
                PinType::Output
            };

            let mut inputs: Vec<EventInput> = node
                .pins
                .values()
                .filter(|pin| {
                    pin.pin_type == target_pin_type
                        && pin.data_type != super::variable::VariableType::Execution
                })
                .map(|pin| EventInput {
                    id: pin.id.clone(),
                    name: pin.name.clone(),
                    friendly_name: pin.friendly_name.clone(),
                    description: pin.description.clone(),
                    data_type: format!("{:?}", pin.data_type),
                    value_type: format!("{:?}", pin.value_type),
                    schema: pin.schema.clone(),
                    default_value: pin.default_value.clone(),
                    index: pin.index,
                })
                .collect();
            inputs.sort_by_key(|i| i.index);
            self.inputs = inputs;
        }

        Ok(())
    }

    /// The stored variants as written, falling back to a single Live variant
    /// named "canary" synthesized from the legacy `canary` field. Shares stay
    /// raw so [`Event::validate_variants`] can grandfather stored values.
    fn raw_variant_set(&self) -> Vec<EventVariant> {
        if !self.variants.is_empty() {
            return self.variants.clone();
        }
        self.canary
            .as_ref()
            .map(|canary| {
                vec![EventVariant {
                    name: "canary".to_string(),
                    board_id: canary.board_id.clone(),
                    board_version: canary.board_version,
                    node_id: canary.node_id.clone(),
                    variables: canary.variables.clone(),
                    default_page_id: None,
                    mode: EventVariantMode::Live {
                        weight: canary.weight,
                    },
                    created_at: canary.created_at,
                    updated_at: canary.updated_at,
                }]
            })
            .unwrap_or_default()
    }

    /// The one read model for dispatch-time variant resolution: the stored
    /// variants when non-empty, else the legacy `canary` as a Live variant.
    /// Weights and sample rates are clamped to `[0, 1]` (non-finite → 0.0) so
    /// a stored out-of-range value can never over-serve a variant.
    pub fn variant_set(&self) -> Vec<EventVariant> {
        let mut variants = self.raw_variant_set();
        for variant in variants.iter_mut() {
            variant.mode.clamp();
        }
        variants
    }

    /// Upsert-time variant validation, grandfathered: a variant that is
    /// content-identical to the stored variant of the same name is accepted
    /// as-is (clamp-on-read covers it), so a pre-existing invalid value never
    /// wedges the event — including the save that would clear it. Name
    /// uniqueness always holds, since names are the grandfathering key.
    fn validate_variants(&self, old_event: Option<&Event>) -> flow_like_types::Result<()> {
        if self.variants.is_empty() {
            return Ok(());
        }

        let old_variants = old_event.map(Event::raw_variant_set).unwrap_or_default();
        let unchanged = |variant: &EventVariant| {
            old_variants
                .iter()
                .any(|old| old.name == variant.name && old.content_equal(variant))
        };

        let mut seen = std::collections::HashSet::new();
        for variant in &self.variants {
            if !seen.insert(variant.name.as_str()) {
                return Err(flow_like_types::anyhow!(
                    "variant name '{}' is used more than once on event {}",
                    variant.name,
                    self.id
                ));
            }

            if unchanged(variant) {
                continue;
            }

            if !valid_variant_name(&variant.name) {
                return Err(flow_like_types::anyhow!(
                    "variant name '{}' is invalid: only lowercase letters, digits and '-' are allowed",
                    variant.name
                ));
            }
            let share = variant.mode.share();
            if !share.is_finite() || !(0.0..=1.0).contains(&share) {
                return Err(flow_like_types::anyhow!(
                    "variant '{}' has a traffic share of {}; it must be between 0 and 1",
                    variant.name,
                    share
                ));
            }
            if variant.board_id == self.board_id
                && variant.board_version == self.board_version
                && variant.node_id == self.node_id
                && variant.default_page_id == self.default_page_id
            {
                return Err(flow_like_types::anyhow!(
                    "variant '{}' targets exactly the primary of event {}; a variant must differ somewhere",
                    variant.name,
                    self.id
                ));
            }
            if self.default_page_id.is_some() {
                match variant.mode {
                    // Page-event canaries resolve at bootstrap: the sealed
                    // page_execution claims then pin the whole session to the
                    // resolved variant. A page variant must itself be a page
                    // target — the sealed contract has nothing to render
                    // otherwise.
                    EventVariantMode::Live { .. } => {
                        if variant.default_page_id.is_none() {
                            return Err(flow_like_types::anyhow!(
                                "variant '{}': a variant on a page event must name its own page (default_page_id)",
                                variant.name
                            ));
                        }
                        // The sealed claims bind exactly (page, board, version):
                        // two page targets sharing that triple are
                        // indistinguishable at validation time, so the triple
                        // must be unique across the primary and every variant.
                        let triple =
                            |page: &Option<String>,
                             board: &str,
                             version: &Option<(u32, u32, u32)>| {
                                (page.clone(), board.to_string(), *version)
                            };
                        let mine = triple(
                            &variant.default_page_id,
                            &variant.board_id,
                            &variant.board_version,
                        );
                        if mine
                            == triple(&self.default_page_id, &self.board_id, &self.board_version)
                        {
                            return Err(flow_like_types::anyhow!(
                                "variant '{}': a page variant must differ from the primary in page, board or version — node or variable changes alone are invisible to the page contract",
                                variant.name
                            ));
                        }
                        if let Some(clash) = self.variants.iter().find(|other| {
                            other.name != variant.name
                                && matches!(other.mode, EventVariantMode::Live { .. })
                                && triple(
                                    &other.default_page_id,
                                    &other.board_id,
                                    &other.board_version,
                                ) == mine
                        }) {
                            return Err(flow_like_types::anyhow!(
                                "variants '{}' and '{}' resolve to the same page/board/version; page variants must be distinguishable",
                                variant.name,
                                clash.name
                            ));
                        }
                    }
                    // Page actions mutate page state — doubling interactions is
                    // wrong by construction, so this stays refused.
                    EventVariantMode::Shadow { .. } => {
                        return Err(flow_like_types::anyhow!(
                            "variant '{}': shadow variants are not supported on page events",
                            variant.name
                        ));
                    }
                }
            }
        }

        // Grandfathered like the per-variant checks: only a changed Live set
        // can be rejected, so an over-full stored set still saves untouched.
        let live: Vec<&EventVariant> = self
            .variants
            .iter()
            .filter(|variant| variant.is_live())
            .collect();
        let live_unchanged = live.iter().all(|variant| unchanged(variant));
        if live.len() > 2 && !live_unchanged {
            return Err(flow_like_types::anyhow!(
                "event {} declares {} live variants; at most 2 are allowed",
                self.id,
                live.len()
            ));
        }

        Ok(())
    }

    pub async fn upsert(
        &mut self,
        app: &App,
        version_type: Option<VersionType>,
        enforce_id: bool,
    ) -> flow_like_types::Result<Self> {
        if self.board_version == Some(flow_like_types::dispatch::ETAG_BOUND_LATEST_VERSION_SENTINEL)
            || self.canary.as_ref().is_some_and(|canary| {
                canary.board_version
                    == Some(flow_like_types::dispatch::ETAG_BOUND_LATEST_VERSION_SENTINEL)
            })
            || self.variants.iter().any(|variant| {
                variant.board_version
                    == Some(flow_like_types::dispatch::ETAG_BOUND_LATEST_VERSION_SENTINEL)
            })
        {
            return Err(flow_like_types::anyhow!(
                "the selected board version is reserved for ETag-bound Latest dispatch"
            ));
        }
        if self.id.is_empty() {
            self.id = create_id();
        }

        // If we set an event as deactivated, we do not have to validate the nodes and boards
        if self.active {
            self.validate_event_references(app).await?;
        }

        self.reconcile_execution_mode_with_board(app).await?;

        // Populate inputs from the board before saving
        if let Err(e) = self.populate_inputs(app).await {
            tracing::warn!("Failed to populate event inputs during upsert: {}", e);
        }

        let old_event = match Event::load(&self.id, app, None).await {
            Ok(event) => Some(event),
            Err(error) if load_error_is_not_found(&error) => None,
            Err(error) => {
                return Err(error.context(format!(
                    "loading existing event {} failed during upsert; refusing to recreate it under a fresh id",
                    self.id
                )));
            }
        };

        self.validate_variants(old_event.as_ref())?;

        if let Some(mut old_event) = old_event {
            if !old_event.content_equal(self) || version_type.is_some() {
                let version_type = version_type.unwrap_or(VersionType::Patch);
                old_event.save(app, Some(old_event.event_version)).await?;
                old_event.event_version = match version_type {
                    VersionType::Major => (old_event.event_version.0 + 1, 0, 0),
                    VersionType::Minor => {
                        (old_event.event_version.0, old_event.event_version.1 + 1, 0)
                    }
                    VersionType::Patch => (
                        old_event.event_version.0,
                        old_event.event_version.1,
                        old_event.event_version.2 + 1,
                    ),
                };
            }

            let updated_event = Event {
                id: old_event.id,
                event_version: old_event.event_version,
                created_at: old_event.created_at,
                updated_at: SystemTime::now(),
                ..self.clone()
            };

            updated_event.save(app, None).await?;
            return Ok(updated_event.clone());
        }

        if !enforce_id {
            self.id = create_id();
        }
        self.event_version = (0, 0, 0);
        self.created_at = SystemTime::now();
        self.updated_at = SystemTime::now();
        self.save(app, None).await?;
        Ok(self.clone())
    }

    /// Content equality on the persisted proto projection: two events are
    /// equal when saving one over the other changes nothing a reader can see.
    /// Identity and bookkeeping fields (`id`, `event_version`, `created_at`,
    /// `updated_at`, the canary's and each variant's timestamps) are excluded —
    /// and so are the canary's traffic `weight` and each variant's
    /// weight/sample_rate: a traffic-share change must never cut a version,
    /// because a bump rewrites the `(appId, eventId, eventVersion)` key that
    /// inbound REST/MCP registrations route on. Comparing the proto projection
    /// rather than the in-memory struct normalizes anything the storage format
    /// flattens, matching the `Board::snapshot_matches_current` precedent.
    pub fn content_equal(&self, other: &Event) -> bool {
        // The common "changed" case exits before paying two full projections.
        if self.node_id != other.node_id
            || self.board_id != other.board_id
            || self.name != other.name
            || self.config != other.config
        {
            return false;
        }

        fn projection(event: &Event) -> proto::Event {
            let mut proto = event.to_proto();
            proto.id = String::new();
            proto.event_version = None;
            proto.created_at = None;
            proto.updated_at = None;
            if let Some(canary) = proto.canary.as_mut() {
                canary.weight = 0.0;
                canary.created_at = None;
                canary.updated_at = None;
            }
            // Variants participate except their traffic share and timestamps —
            // a slider drag must never cut a version, exactly like the canary
            // weight above. Target/mode/variable changes still count.
            for variant in proto.variants.iter_mut() {
                variant.created_at = None;
                variant.updated_at = None;
                match variant.mode.as_mut() {
                    Some(proto::event_variant::Mode::Live(live)) => live.weight = 0.0,
                    Some(proto::event_variant::Mode::Shadow(shadow)) => shadow.sample_rate = 0.0,
                    None => {}
                }
            }
            // Routing metadata never cuts a version: `route`/`is_default` are
            // owned by the route endpoints (a stale client copy must not bump
            // and, for rest/mcp, re-run setup), and `inputs` is board-derived —
            // populate_inputs recomputes it before this compare, so a board pin
            // change would otherwise version every untouched bound event.
            proto.route = None;
            proto.is_default = false;
            proto.inputs = Vec::new();
            proto
        }
        projection(self) == projection(other)
    }

    /// Delete the oldest archived versions beyond `keep`, never touching a
    /// version listed in `protect` (the API passes the parsed
    /// `last_setup_version`, which inbound REST/MCP may still be serving).
    /// Individual delete failures are logged and skipped so one flaky object
    /// cannot abort the save that triggered pruning. Desktop-local apps do not
    /// call this — history there is the user's disk.
    pub async fn prune_versions(
        &self,
        app: &App,
        keep: usize,
        protect: &[(u32, u32, u32)],
    ) -> flow_like_types::Result<usize> {
        let versions = self.get_versions(app).await?;
        if versions.len() <= keep {
            return Ok(0);
        }

        let state = app
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        let store = FlowLikeState::project_meta_store(&state)
            .await?
            .as_generic();

        let deleted = futures::stream::iter(
            versions
                .into_iter()
                .skip(keep)
                .filter(|version| !protect.contains(version)),
        )
        .map(|version| {
            let store = store.clone();
            let path = Self::version_path(&app.id, &self.id, version);
            let event_id = self.id.clone();
            async move {
                match store.delete(&path).await {
                    Ok(()) => 1usize,
                    Err(error) => {
                        tracing::warn!(
                            event_id = %event_id,
                            version = format!("{}.{}.{}", version.0, version.1, version.2),
                            %error,
                            "failed to prune archived event version; skipping"
                        );
                        0
                    }
                }
            }
        })
        .buffer_unordered(8)
        .fold(0usize, |acc, n| async move { acc + n })
        .await;
        Ok(deleted)
    }

    /// Build the forward-only restore plan for archived `version` of
    /// `event_id` against the currently live event. Never writes: the caller
    /// applies `plan.restored` through `App::upsert_event` so a NEW version is
    /// cut. Target-resolution failures are issues on the plan, never `Err` —
    /// only an unusable request (reserved sentinel version, already-live
    /// version, unreadable snapshot) aborts.
    pub async fn plan_restore(
        app: &App,
        event_id: &str,
        version: (u32, u32, u32),
        live: &Event,
        options: &RestoreOptions,
    ) -> flow_like_types::Result<RestorePlan> {
        if version == flow_like_types::dispatch::ETAG_BOUND_LATEST_VERSION_SENTINEL {
            return Err(flow_like_types::anyhow!(
                "cannot restore event {}: the requested version is reserved for ETag-bound Latest dispatch",
                event_id
            ));
        }
        if version == live.event_version {
            return Err(flow_like_types::anyhow!(
                "cannot restore event {} to {}.{}.{}: that revision is already live",
                event_id,
                version.0,
                version.1,
                version.2
            ));
        }

        let snapshot = Event::load(event_id, app, Some(version))
            .await
            .map_err(|error| {
                error.context(format!(
                    "loading archived version {}.{}.{} of event {} failed",
                    version.0, version.1, version.2, event_id
                ))
            })?;

        // Secrets three-way: blank the snapshot's secret values, then let the
        // live values win — a rotated secret is never un-rotated by a restore.
        let mut restored = filter_event_secrets(snapshot);
        restored.id = live.id.clone();
        restored.event_version = live.event_version;
        restored.created_at = live.created_at;
        restored.updated_at = live.updated_at;
        restored.inputs = live.inputs.clone();
        restored.execution_mode = live.execution_mode;
        if !options.restore_route {
            restored.route = live.route.clone();
            restored.is_default = live.is_default;
        }
        if options.drop_canary {
            restored.canary = None;
        }
        preserve_event_secrets(&mut restored, live);

        let mut issues = Vec::new();

        // A secret still blank after filter + preserve has no value in the
        // snapshot AND none live — restoring would blank it for good.
        let secret_severity = if options.accept_blank_secrets {
            RestoreIssueSeverity::Warning
        } else {
            RestoreIssueSeverity::Blocking
        };
        let mut blank_secrets = Vec::new();
        collect_blank_secret_ids(&restored.variables, &mut blank_secrets);
        if let Some(canary) = &restored.canary {
            collect_blank_secret_ids(&canary.variables, &mut blank_secrets);
        }
        for variant in &restored.variants {
            collect_blank_secret_ids(&variant.variables, &mut blank_secrets);
        }
        blank_secrets.sort();
        blank_secrets.dedup();
        for variable_id in blank_secrets {
            issues.push(RestoreIssue {
                code: RestoreIssueCode::SecretUnrecoverable,
                severity: secret_severity,
                message: format!(
                    "secret variable {variable_id} has no value in the snapshot or the live event; restoring leaves it blank"
                ),
                subject: Some(variable_id),
            });
        }

        if restored.event_type != live.event_type {
            issues.push(RestoreIssue {
                code: RestoreIssueCode::EventTypeChanged,
                severity: RestoreIssueSeverity::Blocking,
                message: format!(
                    "snapshot event type '{}' differs from live '{}'; restoring would rewrite the sink type and silently dismantle its schedule",
                    restored.event_type, live.event_type
                ),
                subject: Some(restored.event_type.clone()),
            });
        }
        if restored.default_page_id.is_some() != live.default_page_id.is_some() {
            issues.push(RestoreIssue {
                code: RestoreIssueCode::TargetKindChanged,
                severity: RestoreIssueSeverity::Blocking,
                message:
                    "snapshot and live event disagree on being a page event; restoring flips which pin direction inputs derive from"
                        .to_string(),
                subject: restored.default_page_id.clone(),
            });
        }
        if restored.board_version.is_none() {
            issues.push(RestoreIssue {
                code: RestoreIssueCode::FloatingBoard,
                severity: RestoreIssueSeverity::Warning,
                message: format!(
                    "snapshot floats on the latest version of board {}; restoring restores a pointer, not a flow",
                    restored.board_id
                ),
                subject: Some(restored.board_id.clone()),
            });
        }
        push_target_resolution_issues(app, &restored, &mut issues).await;

        let mut not_restored: Vec<String> = [
            "inputs",
            "execution_mode",
            "sink PAT",
            "sink OAuth tokens",
            "sink model profile",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        if !options.restore_route {
            not_restored.push("route".to_string());
            not_restored.push("is_default".to_string());
        }
        if options.drop_canary {
            not_restored.push("canary".to_string());
        }

        let diff = restore_diff(live, &restored);
        Ok(RestorePlan {
            restored,
            diff,
            not_restored,
            issues,
        })
    }

    /// `apps/{app_id}/events` — the storage root every event object lives
    /// under. All path construction goes through these helpers; the layout
    /// literal must exist exactly once.
    fn storage_root(app_id: &str) -> Path {
        Path::from("apps").child(app_id).child("events")
    }

    fn live_path(app_id: &str, event_id: &str) -> Path {
        Self::storage_root(app_id).child(format!("{event_id}.event"))
    }

    fn versions_root(app_id: &str, event_id: &str) -> Path {
        Self::storage_root(app_id).child("versions").child(event_id)
    }

    fn version_path(app_id: &str, event_id: &str, version: (u32, u32, u32)) -> Path {
        Self::versions_root(app_id, event_id)
            .child(format!("{}.{}.{}", version.0, version.1, version.2))
    }

    pub async fn get_versions(&self, app: &App) -> flow_like_types::Result<Vec<(u32, u32, u32)>> {
        let app_state = app
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        let store = FlowLikeState::project_meta_store(&app_state)
            .await?
            .as_generic();

        let versions_path = Self::versions_root(&app.id, &self.id);
        let mut list_stream = store
            .list(Some(&versions_path))
            .map_ok(|m| m.location)
            .boxed();

        let mut versions = Vec::new();
        while let Some(Ok(location)) = list_stream.next().await {
            if let Some(version_str) = location.filename() {
                let version = version_str.split('.').collect::<Vec<&str>>();
                let version = version.as_slice();
                if version.len() == 3
                    && let (Ok(major), Ok(minor), Ok(patch)) =
                        (version[0].parse(), version[1].parse(), version[2].parse())
                {
                    versions.push((major, minor, patch));
                }
            }
        }

        // Newest first, matching Board::get_versions. Beyond display order this
        // keeps the desktop's local and remote listings comparable: the hybrid
        // provider diffs them with an order-sensitive deep equal.
        versions.sort_unstable_by(|a, b| b.cmp(a));
        Ok(versions)
    }

    /// Force the event's `execution_mode` to match the board when the board
    /// is locked to `Local` or `Remote`. Events are never Hybrid — when the
    /// board allows either, whatever the caller supplied is kept.
    pub async fn reconcile_execution_mode_with_board(
        &mut self,
        app: &App,
    ) -> flow_like_types::Result<()> {
        if self.board_id.is_empty() {
            return Ok(());
        }

        let board = match app
            .open_board(self.board_id.clone(), Some(false), self.board_version)
            .await
        {
            Ok(b) => b,
            Err(_) => return Ok(()),
        };
        let board_mode = board.lock().await.execution_mode.clone();

        match board_mode {
            super::board::ExecutionMode::Local => {
                if self.execution_mode != EventExecutionMode::Local {
                    self.execution_mode = EventExecutionMode::Local;
                }
            }
            super::board::ExecutionMode::Remote => {
                if self.execution_mode != EventExecutionMode::Remote {
                    self.execution_mode = EventExecutionMode::Remote;
                }
            }
            super::board::ExecutionMode::Hybrid => {}
        }

        Ok(())
    }

    /// Resolve every variant's target: its board (at its pinned version), then
    /// its page when it carries one, else its node. Runs under the same
    /// active-only gating as the primary checks.
    async fn validate_variant_references(&self, app: &App) -> flow_like_types::Result<()> {
        for variant in &self.variants {
            let variant_board = app
                .open_board(variant.board_id.clone(), Some(false), variant.board_version)
                .await?;
            let variant_board = variant_board.lock().await;
            if let Some(page_id) = variant.default_page_id.as_deref() {
                if !variant_board
                    .page_ids
                    .iter()
                    .any(|candidate| candidate == page_id)
                {
                    return Err(flow_like_types::anyhow!(
                        "Page '{}' is not listed by board '{}' (variant '{}')",
                        page_id,
                        variant.board_id,
                        variant.name
                    ));
                }
            } else {
                variant_board.nodes.get(&variant.node_id).ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "Node with id {} not found in board {} (variant '{}')",
                        variant.node_id,
                        variant.board_id,
                        variant.name
                    )
                })?;
            }
        }
        Ok(())
    }

    /// Resolve the primary target plus every canary/variant target. Only ever
    /// called for `active` events (see [`Event::upsert`]) — an inactive event
    /// with dangling primary or variant targets saves unvalidated. Known and
    /// accepted: it is validated on the save that activates it.
    pub async fn validate_event_references(&self, app: &App) -> flow_like_types::Result<()> {
        if let Some(page_id) = self.default_page_id.as_deref() {
            if self.board_id.trim().is_empty() {
                return Err(flow_like_types::anyhow!(
                    "Page Event '{}' must identify its owning board",
                    self.id
                ));
            }
            // Page Events follow the same version selector as every other
            // Event. `None` means the current board and is validated against
            // the current Page contract by bootstrap/prerun/invoke.
            let version = self.board_version;
            let board = app
                .open_board(self.board_id.clone(), Some(false), version)
                .await?;
            let board = board.lock().await;
            if board.id != self.board_id
                || version.is_some_and(|expected| board.version != expected)
            {
                let expected = version.unwrap_or(board.version);
                return Err(flow_like_types::anyhow!(
                    "Page Event '{}' resolved board '{}' at {}.{}.{}, expected '{}' at {}.{}.{}",
                    self.id,
                    board.id,
                    board.version.0,
                    board.version.1,
                    board.version.2,
                    self.board_id,
                    expected.0,
                    expected.1,
                    expected.2
                ));
            }
            if !board.page_ids.iter().any(|candidate| candidate == page_id) {
                let suffix = version
                    .map(|version| format!(" at {}.{}.{}", version.0, version.1, version.2))
                    .unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "Page '{}' is not listed by board '{}'{}",
                    page_id,
                    board.id,
                    suffix
                ));
            }
            let page = match version {
                Some(version) => board.load_versioned_page(page_id, version, None).await?,
                None => board.load_page(page_id, None).await?,
            };
            if page.id != page_id
                || page
                    .board_id
                    .as_deref()
                    .is_some_and(|page_board_id| page_board_id != board.id)
            {
                return Err(flow_like_types::anyhow!(
                    "Page Event '{}' has an invalid Page/board binding",
                    self.id
                ));
            }
            PrerunPageExecution::from_page(&board, &page)?;
            drop(board);
            self.validate_variant_references(app).await?;
            return Ok(());
        }

        let board = app
            .open_board(self.board_id.clone(), Some(false), self.board_version)
            .await?;

        board.lock().await.nodes.get(&self.node_id).ok_or_else(|| {
            flow_like_types::anyhow!(
                "Node with id {} not found in board {}",
                self.node_id,
                self.board_id
            )
        })?;

        if let Some(canary) = &self.canary {
            let canary_board = app
                .open_board(canary.board_id.clone(), Some(false), canary.board_version)
                .await?;

            canary_board
                .lock()
                .await
                .nodes
                .get(&canary.node_id)
                .ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "Node with id {} not found in board {} (Canary)",
                        canary.node_id,
                        canary.board_id
                    )
                })?;
        }

        self.validate_variant_references(app).await?;

        Ok(())
    }

    pub async fn load(
        id: &str,
        app: &App,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<Event> {
        let app_state = app
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        let store = FlowLikeState::project_meta_store(&app_state)
            .await?
            .as_generic();

        let event_path = match version {
            Some(version) => Self::version_path(&app.id, id, version),
            None => Self::live_path(&app.id, id),
        };

        let event_proto: proto::Event = from_compressed(store, event_path).await?;
        let event = Event::from_proto(event_proto);

        Ok(event)
    }

    pub async fn save(
        &self,
        app: &App,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<()> {
        let state = app
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        let store = FlowLikeState::project_meta_store(&state)
            .await?
            .as_generic();

        let proto = self.to_proto();
        let Some(version) = version else {
            let event_path = Self::live_path(&app.id, &self.id);
            compress_to_file(store, event_path, &proto).await?;
            return Ok(());
        };

        // Archived versions are immutable, like board snapshots: a create-only
        // write means two racing publishers cannot destroy each other's slot.
        // An identical occupant is success — it is also the crash-recovery path
        // where an earlier upsert archived this version but died before writing
        // the live object; without it the event would wedge unsaveable.
        let event_path = Self::version_path(&app.id, &self.id, version);
        if let Err(create_error) =
            compress_to_file_create(store.clone(), event_path.clone(), &proto).await
        {
            let existing: flow_like_types::Result<proto::Event> =
                from_compressed(store, event_path).await;
            match existing {
                Ok(existing) if existing == proto => {}
                Ok(_) => {
                    return Err(flow_like_types::anyhow!(
                        "archived version {}.{}.{} of event {} already exists with different content; refusing to overwrite it",
                        version.0,
                        version.1,
                        version.2,
                        self.id
                    ));
                }
                Err(_) => return Err(create_error),
            }
        }
        Ok(())
    }

    pub async fn delete(&self, app: &App) -> flow_like_types::Result<()> {
        let event_dir = Self::live_path(&app.id, &self.id);

        let state = app
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        let store = FlowLikeState::project_meta_store(&state)
            .await?
            .as_generic();
        store.delete(&event_dir).await?;

        // Remove all versions of the event
        let versions_path = Self::versions_root(&app.id, &self.id);

        let locations = store
            .list(Some(&versions_path))
            .map_ok(|m| m.location)
            .boxed();

        store
            .delete_stream(locations)
            .try_collect::<Vec<Path>>()
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChatEventParameters, Event, EventPayload, EventVariant, EventVariantMode, RestoreIssueCode,
        RestoreIssueSeverity, RestoreOptions, load_error_is_not_found,
    };
    use crate::flow::{
        pin::ValueType,
        variable::{Variable, VariableType},
    };
    use crate::state::{FlowLikeConfig, FlowLikeState};
    use flow_like_storage::{
        Path,
        files::store::FlowLikeStore,
        object_store::{self, PutPayload},
    };
    use flow_like_types::tokio;
    use serde_json::json;
    use std::{collections::HashMap, sync::Arc, time::SystemTime};

    async fn test_app() -> crate::app::App {
        let mut config = FlowLikeConfig::new();
        config.register_app_meta_store(FlowLikeStore::Other(Arc::new(
            object_store::memory::InMemory::new(),
        )));
        config.register_app_storage_store(FlowLikeStore::Other(Arc::new(
            object_store::memory::InMemory::new(),
        )));
        let state = Arc::new(FlowLikeState::new(
            config,
            crate::utils::http::HTTPClient::new_without_refetch(),
        ));
        crate::app::App::new(None, crate::bit::Metadata::default(), vec![], state)
            .await
            .unwrap()
    }

    fn storage_event(id: &str) -> Event {
        Event {
            id: id.to_string(),
            name: "Storage Fixture".to_string(),
            description: String::new(),
            board_id: String::new(),
            board_version: None,
            node_id: String::new(),
            variables: HashMap::new(),
            config: Vec::new(),
            active: false,
            canary: None,
            variants: Vec::new(),
            priority: 0,
            event_type: "quick_action".to_string(),
            notes: None,
            event_version: (0, 0, 0),
            created_at: SystemTime::UNIX_EPOCH,
            updated_at: SystemTime::UNIX_EPOCH,
            default_page_id: None,
            inputs: Vec::new(),
            route: None,
            is_default: false,
            execution_mode: super::EventExecutionMode::Local,
            exposure: super::EventExposure::Public,
            correlation_mappings: None,
        }
    }

    fn test_variant(name: &str) -> EventVariant {
        EventVariant {
            name: name.to_string(),
            board_id: "board-b".to_string(),
            board_version: Some((1, 2, 3)),
            node_id: "node-b".to_string(),
            variables: HashMap::new(),
            default_page_id: None,
            mode: EventVariantMode::Live { weight: 0.25 },
            created_at: SystemTime::UNIX_EPOCH,
            updated_at: SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn load_error_classification_only_treats_missing_objects_as_absent() {
        let not_found: flow_like_types::Error = object_store::Error::NotFound {
            path: "apps/a/events/e.event".to_string(),
            source: "gone".into(),
        }
        .into();
        assert!(load_error_is_not_found(&not_found));
        assert!(load_error_is_not_found(
            &not_found.context("loading event failed")
        ));

        assert!(!load_error_is_not_found(&flow_like_types::anyhow!(
            "connection reset"
        )));
        let other_store_error: flow_like_types::Error = object_store::Error::Generic {
            store: "s3",
            source: "throttled".into(),
        }
        .into();
        assert!(!load_error_is_not_found(&other_store_error));
    }

    #[tokio::test]
    async fn archived_event_versions_are_immutable() {
        let app = test_app().await;
        let event = storage_event("evt-immutable");

        event.save(&app, Some((1, 0, 0))).await.unwrap();
        // The identical occupant is the crash-recovery path: archiving again
        // after a died live-write must not wedge the event.
        event.save(&app, Some((1, 0, 0))).await.unwrap();

        let mut changed = event.clone();
        changed.name = "Different Content".to_string();
        let error = changed.save(&app, Some((1, 0, 0))).await.unwrap_err();
        assert!(
            error.to_string().contains("refusing to overwrite"),
            "unexpected error: {error:#}"
        );

        // The live object stays last-write-wins.
        changed.save(&app, None).await.unwrap();
        event.save(&app, None).await.unwrap();
        let live = Event::load(&event.id, &app, None).await.unwrap();
        assert_eq!(live.name, event.name);
    }

    #[tokio::test]
    async fn upsert_creates_on_absence_but_refuses_unreadable_events() {
        let app = test_app().await;

        let mut fresh = storage_event("evt-upsert");
        let created = fresh.upsert(&app, None, true).await.unwrap();
        assert_eq!(created.id, "evt-upsert");
        assert_eq!(created.event_version, (0, 0, 0));

        // Corrupt the live object: the next upsert must surface the read
        // failure instead of forking the event under a fresh id.
        let store = FlowLikeState::project_meta_store(app.app_state.as_ref().unwrap())
            .await
            .unwrap()
            .as_generic();
        let live_path = Path::from("apps")
            .child(app.id.clone())
            .child("events")
            .child("evt-upsert.event");
        store
            .put(&live_path, PutPayload::from_static(b"not lz4 protobuf"))
            .await
            .unwrap();

        let mut update = storage_event("evt-upsert");
        update.name = "Update Attempt".to_string();
        let error = update.upsert(&app, None, true).await.unwrap_err();
        assert!(
            format!("{error:#}").contains("refusing to recreate"),
            "unexpected error: {error:#}"
        );
        assert_eq!(update.id, "evt-upsert");
    }

    #[test]
    fn content_equal_ignores_bookkeeping_and_canary_weight() {
        let base = storage_event("evt-eq");

        let mut bookkeeping = base.clone();
        bookkeeping.id = "different-id".to_string();
        bookkeeping.event_version = (9, 9, 9);
        bookkeeping.created_at = SystemTime::now();
        bookkeeping.updated_at = SystemTime::now();
        assert!(base.content_equal(&bookkeeping));

        let canary = super::CanaryEvent {
            weight: 0.1,
            variables: HashMap::new(),
            board_id: "board-b".to_string(),
            board_version: None,
            node_id: "node-b".to_string(),
            created_at: SystemTime::UNIX_EPOCH,
            updated_at: SystemTime::UNIX_EPOCH,
        };
        let mut with_canary = base.clone();
        with_canary.canary = Some(canary.clone());
        let mut reweighted = base.clone();
        reweighted.canary = Some(super::CanaryEvent {
            weight: 0.9,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            ..canary
        });
        // A slider drag must not read as a content change…
        assert!(with_canary.content_equal(&reweighted));
        // …but adding or removing the canary is one.
        assert!(!base.content_equal(&with_canary));

        let mut renamed = base.clone();
        renamed.name = "Different".to_string();
        assert!(!base.content_equal(&renamed));
        let mut reconfigured = base.clone();
        reconfigured.config = vec![1, 2, 3];
        assert!(!base.content_equal(&reconfigured));

        // Routing metadata and board-derived inputs never count as content:
        // a route change or a board pin rename must not cut a version.
        let mut rerouted = base.clone();
        rerouted.route = Some("/dashboard".to_string());
        rerouted.is_default = true;
        assert!(base.content_equal(&rerouted));
        let mut repinned = base.clone();
        repinned.inputs = vec![super::EventInput {
            id: "pin".to_string(),
            name: "renamed_pin".to_string(),
            friendly_name: "Renamed".to_string(),
            description: String::new(),
            data_type: "String".to_string(),
            value_type: "Normal".to_string(),
            schema: None,
            default_value: None,
            index: 0,
        }];
        assert!(base.content_equal(&repinned));
    }

    #[test]
    fn variants_round_trip_through_proto_including_mode() {
        use flow_like_types::{FromProto, ToProto};

        let mut event = storage_event("evt-variant-proto");
        let mut live = test_variant("canary");
        live.variables.insert(
            "var-secret".to_string(),
            secret_variable("var-secret", Some(b"value")),
        );
        let mut shadow = test_variant("mirror");
        shadow.board_version = None;
        shadow.default_page_id = Some("page-1".to_string());
        shadow.mode = EventVariantMode::Shadow { sample_rate: 0.5 };
        event.variants = vec![live, shadow];

        let round_tripped = Event::from_proto(event.to_proto());
        assert_eq!(round_tripped.variants.len(), 2);
        let live_rt = &round_tripped.variants[0];
        assert_eq!(live_rt.name, "canary");
        assert_eq!(live_rt.board_id, "board-b");
        assert_eq!(live_rt.board_version, Some((1, 2, 3)));
        assert_eq!(live_rt.node_id, "node-b");
        assert_eq!(
            live_rt.variables["var-secret"].default_value.as_deref(),
            Some(b"value".as_slice())
        );
        assert_eq!(live_rt.mode, EventVariantMode::Live { weight: 0.25 });
        let shadow_rt = &round_tripped.variants[1];
        assert_eq!(shadow_rt.board_version, None);
        assert_eq!(shadow_rt.default_page_id.as_deref(), Some("page-1"));
        assert_eq!(
            shadow_rt.mode,
            EventVariantMode::Shadow { sample_rate: 0.5 }
        );

        // A wire variant without a mode decodes to an inert zero-weight Live.
        let mut modeless = event.to_proto();
        modeless.variants[0].mode = None;
        let decoded = Event::from_proto(modeless);
        assert_eq!(
            decoded.variants[0].mode,
            EventVariantMode::Live { weight: 0.0 }
        );
    }

    #[test]
    fn variant_set_falls_back_to_legacy_canary_and_clamps() {
        let mut event = storage_event("evt-variant-legacy");
        event.canary = Some(super::CanaryEvent {
            weight: 1.5,
            variables: HashMap::new(),
            board_id: "board-c".to_string(),
            board_version: Some((2, 0, 0)),
            node_id: "node-c".to_string(),
            created_at: SystemTime::UNIX_EPOCH,
            updated_at: SystemTime::UNIX_EPOCH,
        });

        let set = event.variant_set();
        assert_eq!(set.len(), 1);
        assert_eq!(set[0].name, "canary");
        assert_eq!(set[0].board_id, "board-c");
        assert_eq!(set[0].board_version, Some((2, 0, 0)));
        assert_eq!(set[0].node_id, "node-c");
        assert!(set[0].default_page_id.is_none());
        // The legacy weight clamps on read like any variant share.
        assert_eq!(set[0].mode, EventVariantMode::Live { weight: 1.0 });

        // Explicit variants win over the legacy mirror.
        event.variants = vec![test_variant("replacement")];
        let set = event.variant_set();
        assert_eq!(set.len(), 1);
        assert_eq!(set[0].name, "replacement");

        assert!(storage_event("evt-variant-none").variant_set().is_empty());
    }

    #[test]
    fn variant_set_clamps_out_of_range_and_non_finite_shares() {
        let mut event = storage_event("evt-variant-clamp");
        let mut hot = test_variant("hot");
        hot.mode = EventVariantMode::Live { weight: 7.5 };
        let mut cold = test_variant("cold");
        cold.mode = EventVariantMode::Live { weight: -0.5 };
        let mut poisoned = test_variant("poisoned");
        poisoned.mode = EventVariantMode::Shadow {
            sample_rate: f32::NAN,
        };
        let mut wide = test_variant("wide");
        wide.mode = EventVariantMode::Shadow { sample_rate: 2.0 };
        event.variants = vec![hot, cold, poisoned, wide];

        let shares: Vec<f32> = event
            .variant_set()
            .iter()
            .map(|variant| variant.mode.share())
            .collect();
        assert_eq!(shares, vec![1.0, 0.0, 0.0, 1.0]);
        // Clamping is a read-side view; the stored values stay raw.
        assert_eq!(event.variants[0].mode.share(), 7.5);
    }

    #[tokio::test]
    async fn upsert_grandfathers_stored_out_of_range_shares_until_changed() {
        let app = test_app().await;
        let mut event = storage_event("evt-variant-grandfather");
        let mut variant = test_variant("canary");
        variant.mode = EventVariantMode::Live { weight: 1.5 };
        event.variants = vec![variant];
        // Seed storage directly — upsert would never let this value in.
        event.save(&app, None).await.unwrap();

        // The untouched out-of-range share saves fine.
        let mut untouched = event.clone();
        untouched.description = "still saves".to_string();
        untouched.upsert(&app, None, true).await.unwrap();

        // Changing the share fires validation on the new value...
        let mut changed = event.clone();
        changed.variants[0].mode = EventVariantMode::Live { weight: 1.4 };
        let error = changed.upsert(&app, None, true).await.unwrap_err();
        assert!(
            error.to_string().contains("between 0 and 1"),
            "unexpected error: {error:#}"
        );

        // ...and the save that clears it passes.
        let mut fixed = event.clone();
        fixed.variants[0].mode = EventVariantMode::Live { weight: 0.4 };
        fixed.upsert(&app, None, true).await.unwrap();
    }

    #[tokio::test]
    async fn upsert_rejects_invalid_new_variants() {
        let app = test_app().await;
        let mut base = storage_event("evt-variant-reject");
        base.board_id = "board-a".to_string();
        base.node_id = "node-a".to_string();

        let mut sentinel = base.clone();
        let mut variant = test_variant("canary");
        variant.board_version = Some(flow_like_types::dispatch::ETAG_BOUND_LATEST_VERSION_SENTINEL);
        sentinel.variants = vec![variant];
        let error = sentinel.upsert(&app, None, true).await.unwrap_err();
        assert!(
            error.to_string().contains("reserved for ETag-bound"),
            "unexpected error: {error:#}"
        );

        // A variant equal to the primary target is a no-op.
        let mut noop = base.clone();
        let mut variant = test_variant("canary");
        variant.board_id = "board-a".to_string();
        variant.board_version = None;
        variant.node_id = "node-a".to_string();
        noop.variants = vec![variant];
        let error = noop.upsert(&app, None, true).await.unwrap_err();
        assert!(
            error.to_string().contains("targets exactly the primary"),
            "unexpected error: {error:#}"
        );

        let mut named = base.clone();
        named.variants = vec![test_variant("Canary!")];
        let error = named.upsert(&app, None, true).await.unwrap_err();
        assert!(
            error.to_string().contains("is invalid"),
            "unexpected error: {error:#}"
        );

        let mut duplicated = base.clone();
        duplicated.variants = vec![test_variant("canary"), test_variant("canary")];
        let error = duplicated.upsert(&app, None, true).await.unwrap_err();
        assert!(
            error.to_string().contains("more than once"),
            "unexpected error: {error:#}"
        );

        let mut hot = base.clone();
        let mut variant = test_variant("canary");
        variant.mode = EventVariantMode::Live { weight: 1.01 };
        hot.variants = vec![variant];
        let error = hot.upsert(&app, None, true).await.unwrap_err();
        assert!(
            error.to_string().contains("between 0 and 1"),
            "unexpected error: {error:#}"
        );

        let mut crowded = base.clone();
        crowded.variants = vec![
            test_variant("first"),
            test_variant("second"),
            test_variant("third"),
        ];
        let error = crowded.upsert(&app, None, true).await.unwrap_err();
        assert!(
            error.to_string().contains("at most 2"),
            "unexpected error: {error:#}"
        );

        // Page events take no variants yet: live is WP6b, shadow is refused
        // outright.
        let mut page_live = base.clone();
        page_live.default_page_id = Some("page-1".to_string());
        // A page-event Live variant is legal once it names its own page
        // (bootstrap-time resolution); without one it has nothing to render.
        page_live.variants = vec![test_variant("canary")];
        let error = page_live.upsert(&app, None, true).await.unwrap_err();
        assert!(
            error.to_string().contains("must name its own page"),
            "unexpected error: {error:#}"
        );
        let mut paged = test_variant("canary");
        paged.default_page_id = Some("variant-page".to_string());
        page_live.variants = vec![paged];
        // Inactive fixture events skip reference validation, so the upsert
        // itself must accept the paged Live variant.
        page_live.upsert(&app, None, true).await.unwrap();

        // The sealed page contract binds (page, board, version): a variant that
        // only changes node/variables is indistinguishable from the primary…
        let mut same_triple = base.clone();
        same_triple.default_page_id = Some("page-1".to_string());
        let mut twin = test_variant("twin");
        twin.default_page_id = Some("page-1".to_string());
        twin.board_id = same_triple.board_id.clone();
        twin.board_version = same_triple.board_version;
        twin.node_id = "other-node".to_string();
        same_triple.variants = vec![twin];
        let error = same_triple.upsert(&app, None, true).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("must differ from the primary in page, board or version"),
            "unexpected error: {error:#}"
        );
        // …and two variants sharing a triple are indistinguishable from each other.
        let mut clash = base.clone();
        clash.default_page_id = Some("page-1".to_string());
        let mut a = test_variant("a");
        a.default_page_id = Some("variant-page".to_string());
        let mut b = test_variant("b");
        b.default_page_id = Some("variant-page".to_string());
        clash.variants = vec![a, b];
        let error = clash.upsert(&app, None, true).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("resolve to the same page/board/version"),
            "unexpected error: {error:#}"
        );

        let mut page_shadow = base.clone();
        page_shadow.default_page_id = Some("page-1".to_string());
        let mut variant = test_variant("mirror");
        variant.mode = EventVariantMode::Shadow { sample_rate: 0.1 };
        page_shadow.variants = vec![variant];
        let error = page_shadow.upsert(&app, None, true).await.unwrap_err();
        assert!(
            error.to_string().contains("not supported on page events"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn content_equal_ignores_variant_shares_but_sees_target_changes() {
        let mut base = storage_event("evt-variant-eq");
        base.variants = vec![test_variant("canary")];

        let mut reweighted = base.clone();
        reweighted.variants[0].mode = EventVariantMode::Live { weight: 0.9 };
        reweighted.variants[0].created_at = SystemTime::now();
        reweighted.variants[0].updated_at = SystemTime::now();
        assert!(base.content_equal(&reweighted));

        let mut shadow_base = base.clone();
        shadow_base.variants[0].mode = EventVariantMode::Shadow { sample_rate: 0.1 };
        let mut resampled = shadow_base.clone();
        resampled.variants[0].mode = EventVariantMode::Shadow { sample_rate: 0.8 };
        assert!(shadow_base.content_equal(&resampled));

        // A mode flip is a content change even with the shares zeroed out.
        assert!(!base.content_equal(&shadow_base));

        let mut retargeted = base.clone();
        retargeted.variants[0].node_id = "node-z".to_string();
        assert!(!base.content_equal(&retargeted));

        let mut repointed = base.clone();
        repointed.variants[0].board_version = None;
        assert!(!base.content_equal(&repointed));

        let mut added = base.clone();
        added.variants.push(test_variant("second"));
        assert!(!base.content_equal(&added));

        assert!(!base.content_equal(&storage_event("evt-variant-eq")));
    }

    #[test]
    fn event_secret_filters_cover_variant_variables() {
        let mut event = storage_event("evt-variant-secret");
        let mut variant = test_variant("canary");
        variant.variables.insert(
            "var-secret".to_string(),
            secret_variable("var-secret", Some(b"stored-value")),
        );
        event.variants = vec![variant];

        let filtered = super::filter_event_secrets(event.clone());
        assert!(
            filtered.variants[0].variables["var-secret"]
                .default_value
                .is_none()
        );

        // A blanked round-tripped secret restores from the stored variant...
        let mut incoming = filtered.clone();
        super::preserve_event_secrets(&mut incoming, &event);
        assert_eq!(
            incoming.variants[0].variables["var-secret"]
                .default_value
                .as_deref(),
            Some(b"stored-value".as_slice())
        );

        // ...including across the legacy-canary migration save.
        let mut legacy = storage_event("evt-variant-secret-legacy");
        legacy.canary = Some(super::CanaryEvent {
            weight: 0.1,
            variables: HashMap::from([(
                "var-secret".to_string(),
                secret_variable("var-secret", Some(b"canary-value")),
            )]),
            board_id: "board-b".to_string(),
            board_version: None,
            node_id: "node-b".to_string(),
            created_at: SystemTime::UNIX_EPOCH,
            updated_at: SystemTime::UNIX_EPOCH,
        });
        let mut migrated = filtered.clone();
        super::preserve_event_secrets(&mut migrated, &legacy);
        assert_eq!(
            migrated.variants[0].variables["var-secret"]
                .default_value
                .as_deref(),
            Some(b"canary-value".as_slice())
        );
    }

    #[test]
    fn correlation_mappings_round_trip_through_proto() {
        use flow_like_types::{FromProto, ToProto};

        let mut event = storage_event("evt-corr");
        let mut mappings = HashMap::new();
        mappings.insert("order_id".to_string(), "order.id".to_string());
        event.correlation_mappings = Some(mappings.clone());
        let round_tripped = Event::from_proto(event.to_proto());
        assert_eq!(round_tripped.correlation_mappings, Some(mappings));

        let mut none = storage_event("evt-corr-none");
        none.correlation_mappings = None;
        assert_eq!(
            Event::from_proto(none.to_proto()).correlation_mappings,
            None
        );

        // An empty map normalizes to None across the wire — documented, and
        // exactly why content_equal compares the proto projection.
        let mut empty = storage_event("evt-corr-empty");
        empty.correlation_mappings = Some(HashMap::new());
        assert_eq!(
            Event::from_proto(empty.to_proto()).correlation_mappings,
            None
        );
        assert!(empty.content_equal(&none));
    }

    #[tokio::test]
    async fn upsert_archives_on_content_change_and_skips_identical_saves() {
        let app = test_app().await;

        let mut event = storage_event("evt-bump");
        let created = event.upsert(&app, None, true).await.unwrap();
        assert_eq!(created.event_version, (0, 0, 0));

        // Identical re-save: no bump, no archive.
        let mut identical = storage_event("evt-bump");
        let saved = identical.upsert(&app, None, true).await.unwrap();
        assert_eq!(saved.event_version, (0, 0, 0));
        assert!(saved.get_versions(&app).await.unwrap().is_empty());

        // A config-only edit now bumps and archives the prior content.
        let mut edited = storage_event("evt-bump");
        edited.config = vec![42];
        let bumped = edited.upsert(&app, None, true).await.unwrap();
        assert_eq!(bumped.event_version, (0, 0, 1));
        assert_eq!(bumped.get_versions(&app).await.unwrap(), vec![(0, 0, 0)]);

        // An explicit version_type forces a bump even on identical content.
        let mut forced = storage_event("evt-bump");
        forced.config = vec![42];
        let major = forced
            .upsert(&app, Some(crate::flow::board::VersionType::Major), true)
            .await
            .unwrap();
        assert_eq!(major.event_version, (1, 0, 0));
    }

    #[tokio::test]
    async fn prune_versions_keeps_newest_and_protected() {
        let app = test_app().await;
        let event = storage_event("evt-prune");
        for patch in 0..6 {
            event.save(&app, Some((0, 0, patch))).await.unwrap();
        }

        let deleted = event.prune_versions(&app, 2, &[(0, 0, 1)]).await.unwrap();
        assert_eq!(deleted, 3);
        assert_eq!(
            event.get_versions(&app).await.unwrap(),
            vec![(0, 0, 5), (0, 0, 4), (0, 0, 1)]
        );

        // Under the cap: nothing to do.
        assert_eq!(event.prune_versions(&app, 10, &[]).await.unwrap(), 0);
    }

    fn secret_variable(id: &str, value: Option<&[u8]>) -> Variable {
        let mut variable = Variable::new("api_key", VariableType::String, ValueType::Normal);
        variable.id = id.to_string();
        variable.secret = true;
        variable.default_value = value.map(<[u8]>::to_vec);
        variable
    }

    #[tokio::test]
    async fn plan_restore_never_unrotates_a_live_secret() {
        let app = test_app().await;
        let mut snapshot = storage_event("evt-restore-secret");
        snapshot.variables.insert(
            "var-secret".to_string(),
            secret_variable("var-secret", Some(b"old-secret-value")),
        );
        snapshot.save(&app, Some((0, 0, 1))).await.unwrap();

        let mut live = storage_event("evt-restore-secret");
        live.event_version = (0, 0, 3);
        live.variables.insert(
            "var-secret".to_string(),
            secret_variable("var-secret", Some(b"rotated-secret-value")),
        );

        let plan = Event::plan_restore(
            &app,
            "evt-restore-secret",
            (0, 0, 1),
            &live,
            &RestoreOptions::default(),
        )
        .await
        .unwrap();

        assert_eq!(
            plan.restored.variables["var-secret"]
                .default_value
                .as_deref(),
            Some(b"rotated-secret-value".as_slice())
        );
        assert!(
            !plan
                .issues
                .iter()
                .any(|issue| issue.code == RestoreIssueCode::SecretUnrecoverable)
        );
        // Identity and bookkeeping always come from the live event.
        assert_eq!(plan.restored.id, live.id);
        assert_eq!(plan.restored.event_version, live.event_version);
        assert_eq!(plan.restored.created_at, live.created_at);
    }

    #[tokio::test]
    async fn plan_restore_flags_unrecoverable_secrets() {
        let app = test_app().await;
        let mut snapshot = storage_event("evt-restore-blank");
        snapshot.variables.insert(
            "var-secret".to_string(),
            secret_variable("var-secret", Some(b"archived-secret")),
        );
        snapshot.save(&app, Some((0, 0, 1))).await.unwrap();

        let mut live = storage_event("evt-restore-blank");
        live.event_version = (0, 0, 2);
        live.variables.insert(
            "var-secret".to_string(),
            secret_variable("var-secret", None),
        );

        let plan = Event::plan_restore(
            &app,
            "evt-restore-blank",
            (0, 0, 1),
            &live,
            &RestoreOptions::default(),
        )
        .await
        .unwrap();
        let issue = plan
            .issues
            .iter()
            .find(|issue| issue.code == RestoreIssueCode::SecretUnrecoverable)
            .unwrap();
        assert_eq!(issue.severity, RestoreIssueSeverity::Blocking);
        assert_eq!(issue.subject.as_deref(), Some("var-secret"));
        // The archived value is filtered out, never resurrected.
        assert!(
            plan.restored.variables["var-secret"]
                .default_value
                .is_none()
        );

        let plan = Event::plan_restore(
            &app,
            "evt-restore-blank",
            (0, 0, 1),
            &live,
            &RestoreOptions {
                accept_blank_secrets: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let issue = plan
            .issues
            .iter()
            .find(|issue| issue.code == RestoreIssueCode::SecretUnrecoverable)
            .unwrap();
        assert_eq!(issue.severity, RestoreIssueSeverity::Warning);
    }

    #[tokio::test]
    async fn plan_restore_reports_missing_board_targets_as_issues() {
        let mut app = test_app().await;
        let created = app.create_board(None, None).await.unwrap();

        let mut snapshot = storage_event("evt-restore-version");
        snapshot.board_id = created.board_id.clone();
        snapshot.board_version = Some((9, 9, 9));
        snapshot.node_id = "node-x".to_string();
        snapshot.save(&app, Some((0, 0, 1))).await.unwrap();

        let mut live = storage_event("evt-restore-version");
        live.event_version = (0, 0, 2);

        let plan = Event::plan_restore(
            &app,
            "evt-restore-version",
            (0, 0, 1),
            &live,
            &RestoreOptions::default(),
        )
        .await
        .unwrap();
        let issue = plan
            .issues
            .iter()
            .find(|issue| issue.code == RestoreIssueCode::BoardVersionMissing)
            .unwrap();
        assert_eq!(issue.severity, RestoreIssueSeverity::Blocking);
        assert_eq!(issue.subject.as_deref(), Some(created.board_id.as_str()));

        // A board that is gone entirely is BoardMissing, not BoardVersionMissing.
        let mut orphan = storage_event("evt-restore-orphan");
        orphan.board_id = "board-gone".to_string();
        orphan.board_version = Some((1, 0, 0));
        orphan.save(&app, Some((0, 0, 1))).await.unwrap();
        let mut orphan_live = storage_event("evt-restore-orphan");
        orphan_live.event_version = (0, 0, 2);

        let plan = Event::plan_restore(
            &app,
            "evt-restore-orphan",
            (0, 0, 1),
            &orphan_live,
            &RestoreOptions::default(),
        )
        .await
        .unwrap();
        assert!(
            plan.issues
                .iter()
                .any(|issue| issue.code == RestoreIssueCode::BoardMissing
                    && issue.severity == RestoreIssueSeverity::Blocking)
        );
    }

    #[tokio::test]
    async fn plan_restore_warns_on_floating_board_snapshot() {
        let mut app = test_app().await;
        let created = app.create_board(None, None).await.unwrap();

        let mut snapshot = storage_event("evt-restore-float");
        snapshot.board_id = created.board_id.clone();
        snapshot.board_version = None;
        snapshot.save(&app, Some((0, 0, 1))).await.unwrap();

        let mut live = storage_event("evt-restore-float");
        live.event_version = (0, 0, 2);

        let plan = Event::plan_restore(
            &app,
            "evt-restore-float",
            (0, 0, 1),
            &live,
            &RestoreOptions::default(),
        )
        .await
        .unwrap();
        let issue = plan
            .issues
            .iter()
            .find(|issue| issue.code == RestoreIssueCode::FloatingBoard)
            .unwrap();
        assert_eq!(issue.severity, RestoreIssueSeverity::Warning);
        assert_eq!(issue.subject.as_deref(), Some(created.board_id.as_str()));
    }

    #[tokio::test]
    async fn plan_restore_keeps_live_route_unless_asked() {
        let app = test_app().await;
        let mut snapshot = storage_event("evt-restore-route");
        snapshot.route = Some("/archived".to_string());
        snapshot.is_default = true;
        snapshot.save(&app, Some((0, 0, 1))).await.unwrap();

        let mut live = storage_event("evt-restore-route");
        live.event_version = (0, 0, 2);
        live.route = Some("/live".to_string());

        let plan = Event::plan_restore(
            &app,
            "evt-restore-route",
            (0, 0, 1),
            &live,
            &RestoreOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(plan.restored.route.as_deref(), Some("/live"));
        assert!(!plan.restored.is_default);
        assert!(plan.not_restored.iter().any(|field| field == "route"));
        assert!(plan.not_restored.iter().any(|field| field == "is_default"));

        let plan = Event::plan_restore(
            &app,
            "evt-restore-route",
            (0, 0, 1),
            &live,
            &RestoreOptions {
                restore_route: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(plan.restored.route.as_deref(), Some("/archived"));
        assert!(plan.restored.is_default);
        assert!(!plan.not_restored.iter().any(|field| field == "route"));
        // The always-excluded set stays either way.
        for field in [
            "inputs",
            "execution_mode",
            "sink PAT",
            "sink OAuth tokens",
            "sink model profile",
        ] {
            assert!(plan.not_restored.iter().any(|entry| entry == field));
        }
    }

    #[tokio::test]
    async fn plan_restore_refuses_the_live_revision_and_the_sentinel() {
        let app = test_app().await;
        let mut live = storage_event("evt-restore-guard");
        live.event_version = (0, 0, 2);

        let error = Event::plan_restore(
            &app,
            "evt-restore-guard",
            (0, 0, 2),
            &live,
            &RestoreOptions::default(),
        )
        .await
        .unwrap_err();
        assert!(
            error.to_string().contains("already live"),
            "unexpected error: {error:#}"
        );

        let error = Event::plan_restore(
            &app,
            "evt-restore-guard",
            (u32::MAX, u32::MAX, u32::MAX),
            &live,
            &RestoreOptions::default(),
        )
        .await
        .unwrap_err();
        assert!(
            error.to_string().contains("reserved"),
            "unexpected error: {error:#}"
        );
    }

    #[tokio::test]
    async fn plan_restore_diff_never_carries_secret_values() {
        let app = test_app().await;
        let mut snapshot = storage_event("evt-restore-diff");
        snapshot.name = "Archived Name".to_string();
        snapshot.variables.insert(
            "var-changed".to_string(),
            secret_variable("var-changed", Some(b"archived-secret-value")),
        );
        snapshot.variables.insert(
            "var-added".to_string(),
            secret_variable("var-added", Some(b"added-secret-value")),
        );
        snapshot.save(&app, Some((0, 0, 1))).await.unwrap();

        let mut live = storage_event("evt-restore-diff");
        live.event_version = (0, 0, 2);
        let mut rotated = secret_variable("var-changed", Some(b"live-secret-value"));
        rotated.name = "renamed".to_string();
        live.variables.insert("var-changed".to_string(), rotated);
        live.variables.insert(
            "var-removed".to_string(),
            secret_variable("var-removed", Some(b"removed-secret-value")),
        );

        let plan = Event::plan_restore(
            &app,
            "evt-restore-diff",
            (0, 0, 1),
            &live,
            &RestoreOptions {
                accept_blank_secrets: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let field = |name: &str| {
            plan.diff
                .iter()
                .find(|change| change.field == name)
                .unwrap_or_else(|| panic!("missing diff entry for {name}"))
        };
        assert_eq!(field("variables.var-added").to, "present");
        assert_eq!(field("variables.var-removed").to, "absent");
        assert_eq!(field("variables.var-changed").to, "snapshot definition");
        assert_eq!(field("name").to, "Archived Name");

        for change in &plan.diff {
            for text in [&change.from, &change.to] {
                assert!(
                    !text.contains("secret-value"),
                    "diff leaked a value: {} {} -> {}",
                    change.field,
                    change.from,
                    change.to
                );
            }
        }
    }

    #[test]
    fn chat_event_parameters_default_presentation_fields_to_none() {
        let parameters: ChatEventParameters = serde_json::from_value(json!({})).unwrap();

        assert!(parameters.custom_css.is_none());
        assert!(parameters.background_image.is_none());
        assert!(parameters.color_scheme.is_none());
        assert!(parameters.ai_disclosure.is_none());
        // Motion the interface never asked for must stay off for chats saved before the field
        // existed, so an upgrade never starts animating someone's placeholder.
        assert!(parameters.placeholder_typing_motion.is_none());
    }

    #[test]
    fn chat_event_payload_round_trips_placeholder_typing_motion() {
        let payload: EventPayload = serde_json::from_value(json!({
            "placeholder_visual": "bubble",
            "placeholder_typing_motion": true
        }))
        .unwrap();

        let EventPayload::ChatEvent(parameters) = &payload else {
            panic!("placeholder fields should deserialize as chat event parameters");
        };
        assert_eq!(parameters.placeholder_visual.as_deref(), Some("bubble"));
        assert_eq!(parameters.placeholder_typing_motion, Some(true));

        let serialized = serde_json::to_value(payload).unwrap();
        assert_eq!(serialized["placeholder_typing_motion"], json!(true));
    }

    #[test]
    fn chat_event_payload_round_trips_presentation_fields() {
        let payload: EventPayload = serde_json::from_value(json!({
            "custom_css": ".message { border-radius: 1rem; }",
            "background_image": "https://example.com/chat-background.webp",
            "color_scheme": "dark",
            "ai_disclosure": "Plot twist: you're chatting with an AI."
        }))
        .unwrap();

        let EventPayload::ChatEvent(parameters) = &payload else {
            panic!("presentation fields should deserialize as chat event parameters");
        };
        assert_eq!(
            parameters.custom_css.as_deref(),
            Some(".message { border-radius: 1rem; }")
        );
        assert_eq!(
            parameters.background_image.as_deref(),
            Some("https://example.com/chat-background.webp")
        );
        assert_eq!(parameters.color_scheme.as_deref(), Some("dark"));
        assert_eq!(
            parameters.ai_disclosure.as_deref(),
            Some("Plot twist: you're chatting with an AI.")
        );

        let serialized = serde_json::to_value(payload).unwrap();
        assert_eq!(
            serialized["custom_css"],
            json!(".message { border-radius: 1rem; }")
        );
        assert_eq!(
            serialized["background_image"],
            json!("https://example.com/chat-background.webp")
        );
        assert_eq!(serialized["color_scheme"], json!("dark"));
        assert_eq!(
            serialized["ai_disclosure"],
            json!("Plot twist: you're chatting with an AI.")
        );
    }
}
