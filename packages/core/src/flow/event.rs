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
        if let Some(mut old_event) = old_event {
            if old_event.node_id != self.node_id
                || old_event.board_id != self.board_id
                || !canary_equal(&old_event.canary, &self.canary)
                || version_type.is_some()
            {
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

    pub async fn get_versions(&self, app: &App) -> flow_like_types::Result<Vec<(u32, u32, u32)>> {
        let storage_root = Path::from("apps").child(app.id.clone()).child("events");
        let app_state = app
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        let store = FlowLikeState::project_meta_store(&app_state)
            .await?
            .as_generic();

        let versions_path = storage_root.child("versions").child(self.id.clone());
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

        Ok(())
    }

    pub async fn load(
        id: &str,
        app: &App,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<Event> {
        let storage_root = Path::from("apps").child(app.id.clone()).child("events");
        let app_state = app
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        let store = FlowLikeState::project_meta_store(&app_state)
            .await?
            .as_generic();

        let event_path = match version {
            Some(version) => storage_root
                .child("versions")
                .child(id)
                .child(format!("{}.{}.{}", version.0, version.1, version.2)),
            None => storage_root.child(format!("{}.event", id)),
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
        let storage_root = Path::from("apps").child(app.id.clone()).child("events");
        let state = app
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        let store = FlowLikeState::project_meta_store(&state)
            .await?
            .as_generic();

        let proto = self.to_proto();
        let Some(version) = version else {
            let event_path = storage_root.child(format!("{}.event", self.id));
            compress_to_file(store, event_path, &proto).await?;
            return Ok(());
        };

        // Archived versions are immutable, like board snapshots: a create-only
        // write means two racing publishers cannot destroy each other's slot.
        // An identical occupant is success — it is also the crash-recovery path
        // where an earlier upsert archived this version but died before writing
        // the live object; without it the event would wedge unsaveable.
        let event_path = storage_root
            .child("versions")
            .child(self.id.clone())
            .child(format!("{}.{}.{}", version.0, version.1, version.2));
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
        let event_dir = Path::from("apps")
            .child(app.id.clone())
            .child("events")
            .child(format!("{}.event", self.id));

        let state = app
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        let store = FlowLikeState::project_meta_store(&state)
            .await?
            .as_generic();
        store.delete(&event_dir).await?;

        // Remove all versions of the event
        let versions_path = Path::from("apps")
            .child(app.id.clone())
            .child("events")
            .child("versions")
            .child(self.id.clone());

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
    use super::{ChatEventParameters, Event, EventPayload, load_error_is_not_found};
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
