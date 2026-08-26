use super::{
    execution::LogLevel,
    node::{Node, NodeLogic},
    pin::Pin,
    variable::{Variable, VariableType},
};
use crate::{
    a2ui::{
        id_refs::IdRef,
        page_remap::{IdTranslators, remap_page_refs},
        widget::Page,
    },
    app::App,
    state::{FlowLikeState, FlowNodeRegistry},
    utils::compression::{
        ConditionalRead, compress_to_file, compress_to_file_create, compress_to_file_update,
        from_compressed, from_compressed_if_changed, from_compressed_json,
        from_compressed_with_meta,
    },
};
use commands::GenericCommand;
use commands::nodes::update_node::UpdateNodeCommand;
use dirty::{DirtyIndex, Touched};
use flow_like_storage::object_store::{self, ObjectStore, PutResult, UpdateVersion, path::Path};
use flow_like_types::proto;
use flow_like_types::{FromProto, ToProto, create_id, sync::Mutex};
use futures::StreamExt;
use highway::{HighwayHash, HighwayHasher};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Weak},
    time::SystemTime,
};
use tracing::instrument;

pub mod cleanup;
pub mod commands;
pub mod dirty;
pub mod summary;
pub mod sync;

/// Reserved board-ref namespace for host bookkeeping that must be persisted atomically with a
/// board mutation but must never participate in FlowScript, semantic fingerprints, or user-facing
/// context. Values under this prefix are opaque to the workflow engine.
pub const INTERNAL_BOARD_REF_PREFIX: &str = "__flow_like_internal_v1/";

/// How many times a node may be re-derived before the sweep gives up.
///
/// A pass exists because one node's `on_update` can retype a pin its neighbour reads, so
/// derivations settle by iteration. The scoped sweep spends the same total budget, just spread over
/// a queue instead of whole-board passes.
const MAX_UPDATE_PASSES: usize = 10;

/// How many patch slots a publication may skip before it gives up.
///
/// Slots are skipped when another publisher owns them, which is rare and bounded in practice.
/// Every attempt writes a full board plus its pages, so an unbounded scan turns a systematic
/// validation failure into thousands of orphan snapshots instead of one error.
const MAX_PATCH_SLOT_SCAN: u32 = 64;

/// How often a publication re-attempts after a racing draft save.
///
/// Every attempt writes a full board plus its pages, so this stays small: the
/// retry exists to survive one editor save landing mid-publication, not to
/// grind against a draft that is being typed into.
const MAX_PUBLISH_RACE_RETRIES: usize = 2;

pub fn is_internal_board_ref(key: &str) -> bool {
    key.starts_with(INTERNAL_BOARD_REF_PREFIX)
}

/// A publication that wrote its immutable objects but lost the final check
/// against the floating draft: another writer saved the draft while the
/// snapshot was being copied.
///
/// The written version is an inert orphan - nothing points at it - so the
/// operation is safe to retry from the reloaded draft at a fresh patch. Typed
/// rather than a bare message so callers can tell this transient race apart
/// from a genuine storage failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardDraftChanged {
    pub version: (u32, u32, u32),
}

impl std::fmt::Display for BoardDraftChanged {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Board draft changed while publishing immutable version {}.{}.{}",
            self.version.0, self.version.1, self.version.2
        )
    }
}

impl std::error::Error for BoardDraftChanged {}

/// Whether `error` is the retryable [`BoardDraftChanged`] publication race,
/// including when it was propagated through a patch-slot scan.
pub fn is_board_draft_race(error: &flow_like_types::Error) -> bool {
    error.downcast_ref::<BoardDraftChanged>().is_some()
}

/// Whether the persisted draft demonstrably changed between two revision
/// readings. Missing revision tokens prove nothing, so they answer `false`:
/// re-publishing on a hunch writes another full snapshot.
fn draft_was_replaced(before: &Option<String>, after: &Option<String>) -> bool {
    matches!((before, after), (Some(before), Some(after)) if before != after)
}

#[derive(Debug, Clone)]
pub enum BoardParent {
    App(Weak<Mutex<App>>),
}

/// An immutable board snapshot that has been fully written but has not yet
/// been made the floating draft's current version.
///
/// Callers that publish other metadata referring to the snapshot should commit
/// that metadata first, then call [`Board::commit_prepared_snapshot`]. Keeping
/// these two phases separate prevents a failed metadata write from advancing
/// the user's draft and losing the fact that it still differs from the
/// previously published action implementation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PreparedBoardSnapshot {
    board_id: String,
    version: (u32, u32, u32),
}

impl PreparedBoardSnapshot {
    pub fn board_id(&self) -> &str {
        &self.board_id
    }

    pub fn version(&self) -> (u32, u32, u32) {
        self.version
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub enum ExecutionStage {
    Dev,
    Int,
    QA,
    PreProd,
    Prod,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Default, Debug, PartialEq, Eq)]
pub enum ExecutionMode {
    #[default]
    Hybrid,
    Remote,
    Local,
}
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub enum LayerType {
    Function,
    Macro,
    Collapsed,
    /// Purely organizational grouping — a "virtual file" for flow organization. Has no
    /// boundary pins, no cache and no runtime effect. May only be nested inside another
    /// `Module`.
    Module,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub enum VersionType {
    Major,
    Minor,
    Patch,
}

/// Who a layer's cached results belong to. Mirrors the scopes the flow cache backends
/// understand; kept here so the layer settings do not have to depend on the catalog.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayerCacheScope {
    /// Shared by everyone who can execute in the app.
    #[default]
    App,
    /// Private to the user who triggered the run.
    User,
}

impl LayerCacheScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::User => "user",
        }
    }
}

/// Result caching for a layer invoked as a function.
///
/// A hit replaces the whole call: the function body never runs, so its side effects do
/// not happen either. Only turn this on for layers whose outputs are a function of their
/// inputs.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerCache {
    #[serde(default)]
    pub enabled: bool,
    /// Namespace every entry for this layer is written under, so one layer's cache can be
    /// invalidated without touching the rest of the app's.
    #[serde(default)]
    pub prefix: String,
    /// Lifetime of an entry in seconds. `None` or `0` keeps it until it is invalidated.
    #[serde(default)]
    pub ttl_seconds: Option<u64>,
    #[serde(default)]
    pub scope: LayerCacheScope,
}

impl LayerCache {
    pub fn is_active(&self) -> bool {
        self.enabled
    }

    /// The TTL handed to the cache backend. Layer settings define both omission and `0` as
    /// "never expires", so normalize both to explicit `Some(0)`. At the remote cache boundary,
    /// `None` instead means "use the deployment default" and would violate that layer contract.
    pub fn ttl(&self) -> Option<u64> {
        Some(self.ttl_seconds.unwrap_or(0))
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Layer {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    /// Folder the layer is filed under in the sidebar, nested with forward slashes.
    /// Empty means the top level. Purely organizational, it does not affect execution.
    #[serde(default)]
    pub category: Option<String>,
    pub r#type: LayerType,
    pub nodes: HashMap<String, Node>,
    pub variables: HashMap<String, Variable>,
    pub comments: HashMap<String, Comment>,
    pub coordinates: (f32, f32, f32),
    pub in_coordinates: Option<(f32, f32, f32)>,
    pub out_coordinates: Option<(f32, f32, f32)>,
    pub pins: HashMap<String, Pin>,
    pub comment: Option<String>,
    pub error: Option<String>,
    pub color: Option<String>,
    #[serde(default)]
    pub cache: Option<LayerCache>,
    pub hash: Option<u64>,
}

impl Layer {
    pub fn new(id: String, name: String, r#type: LayerType) -> Self {
        Layer {
            id,
            parent_id: None,
            name,
            category: None,
            r#type,
            nodes: HashMap::new(),
            variables: HashMap::new(),
            comments: HashMap::new(),
            coordinates: (0.0, 0.0, 0.0),
            in_coordinates: None,
            out_coordinates: None,
            pins: HashMap::new(),
            comment: None,
            error: None,
            color: None,
            cache: None,
            hash: None,
        }
    }

    pub fn hash(&mut self) {
        let mut hasher = HighwayHasher::new(highway::Key([
            0x0123456789abcdfe,
            0xfedcba9876543210,
            0x0011223344556677,
            0x8899aabbccddeeff,
        ]));

        hasher.append(self.id.as_bytes());
        hasher.append(self.name.as_bytes());
        hasher.append(format!("{:?}", self.r#type).as_bytes());

        if let Some(category) = &self.category {
            hasher.append(category.as_bytes());
        }

        if let Some(parent_id) = &self.parent_id {
            hasher.append(parent_id.as_bytes());
        }

        let mut sorted_nodes: Vec<_> = self.nodes.iter().collect();
        sorted_nodes.sort_by_key(|(id, _)| *id);
        for (id, node) in sorted_nodes {
            hasher.append(id.as_bytes());
            hasher.append(node.id.as_bytes());
        }

        let mut sorted_variables: Vec<_> = self.variables.iter().collect();
        sorted_variables.sort_by_key(|(id, _)| *id);
        for (id, variable) in sorted_variables {
            hasher.append(id.as_bytes());
            hasher.append(variable.id.as_bytes());
        }

        let mut sorted_comments: Vec<_> = self.comments.iter().collect();
        sorted_comments.sort_by_key(|(id, _)| *id);
        for (id, comment) in sorted_comments {
            hasher.append(id.as_bytes());
            hasher.append(comment.id.as_bytes());
        }

        let mut sorted_pins: Vec<_> = self.pins.iter().collect();
        sorted_pins.sort_by_key(|(id, _)| *id);
        for (_id, pin) in sorted_pins {
            pin.hash_into(&mut hasher);
        }

        hasher.append(&self.coordinates.0.to_le_bytes());
        hasher.append(&self.coordinates.1.to_le_bytes());
        hasher.append(&self.coordinates.2.to_le_bytes());

        if let Some(comment) = &self.comment {
            hasher.append(comment.as_bytes());
        }

        if let Some(color) = &self.color {
            hasher.append(color.as_bytes());
        }

        if let Some(cache) = &self.cache {
            hasher.append(&[cache.enabled as u8]);
            hasher.append(cache.prefix.as_bytes());
            hasher.append(&cache.ttl_seconds.unwrap_or_default().to_le_bytes());
            hasher.append(cache.scope.as_str().as_bytes());
        }

        self.hash = Some(hasher.finalize64());
    }
}

/// A page the board lists but whose payload could not be read on this host.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct UnreadablePage {
    pub page_id: String,
    pub reason: String,
}

/// The readable pages of a board plus the ids it lists that could not be read.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct LoadedPages {
    pub pages: Vec<Page>,
    pub unreadable: Vec<UnreadablePage>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct Board {
    pub id: String,
    pub name: String,
    pub description: String,
    pub nodes: HashMap<String, Node>,
    pub variables: HashMap<String, Variable>,
    pub comments: HashMap<String, Comment>,
    pub viewport: (f32, f32, f32),
    pub version: (u32, u32, u32),
    pub stage: ExecutionStage,
    pub log_level: LogLevel,
    pub execution_mode: ExecutionMode,
    pub refs: HashMap<String, String>,
    /// Persisted host bookkeeping, intentionally excluded from Board JSON and all semantic
    /// workflow surfaces. External crates can only access this map through the prefix-validating
    /// methods on `Board`.
    #[serde(skip)]
    pub(crate) internal_refs: HashMap<String, String>,
    pub layers: HashMap<String, Layer>,
    pub page_ids: Vec<String>,
    pub hash: Option<u64>,

    pub created_at: SystemTime,
    pub updated_at: SystemTime,

    #[serde(skip)]
    pub parent: Option<BoardParent>,

    #[serde(skip)]
    pub board_dir: Path,

    #[serde(skip)]
    pub logic_nodes: HashMap<String, Arc<dyn NodeLogic>>,

    #[serde(skip)]
    pub app_state: Option<Arc<FlowLikeState>>,

    /// Pin id to owning container, populated only while [`Board::node_updates`] runs.
    ///
    /// `on_update` receives an immutable board, so within a pass only the node currently being
    /// updated can change its pins; the index is refreshed as each node is written back. Outside
    /// `node_updates` this stays `None` and [`Board::get_pin_by_id`] scans, so no mutation path
    /// has to maintain it.
    #[serde(skip)]
    pub(crate) pin_index: Option<HashMap<String, PinOwner>>,
}

/// Which container owns a pin, as recorded by [`Board::pin_index`].
#[derive(Clone)]
pub(crate) enum PinOwner {
    Node(String),
    LayerPin(String),
    LayerNode { layer: String, node: String },
}

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct BoardUndoRedoStack {
    pub undo_stack: Vec<String>,
    pub redo_stack: Vec<String>,
}

fn append_optional_u64(hasher: &mut HighwayHasher, value: Option<u64>) {
    match value {
        Some(value) => {
            hasher.append(&[1]);
            hasher.append(&value.to_le_bytes());
        }
        None => hasher.append(&[0]),
    }
}

fn stage_marker(stage: &ExecutionStage) -> u8 {
    match stage {
        ExecutionStage::Dev => 0,
        ExecutionStage::Int => 1,
        ExecutionStage::QA => 2,
        ExecutionStage::PreProd => 3,
        ExecutionStage::Prod => 4,
    }
}

fn execution_mode_marker(mode: &ExecutionMode) -> u8 {
    match mode {
        ExecutionMode::Hybrid => 0,
        ExecutionMode::Remote => 1,
        ExecutionMode::Local => 2,
    }
}

/// Pin ids are the only part of a node another machine cannot re-derive: everything else
/// `on_update` produces is a deterministic function of the board it already replayed.
fn pin_ids_match(current: &Node, previous: &Node) -> bool {
    current.pins.len() == previous.pins.len()
        && current.pins.keys().all(|id| previous.pins.contains_key(id))
}

impl Board {
    /// The board's variables with secret values removed, plus only the `refs` entries their
    /// schemas reach. This is what a configuration surface needs and nothing a full board
    /// transfer would add.
    pub fn public_variables(&self) -> (HashMap<String, Variable>, HashMap<String, String>) {
        let mut variables = self.variables.clone();
        let mut refs = HashMap::new();
        for variable in variables.values_mut() {
            if variable.secret {
                variable.default_value = None;
            }
            // Refs may chain (ref → ref → schema); follow every hop the client would.
            let mut key = variable.schema.clone();
            while let Some(current) = key.take() {
                let Some(resolved) = self.refs.get(&current) else {
                    break;
                };
                if refs.insert(current, resolved.clone()).is_some() {
                    break;
                }
                key = Some(resolved.clone());
            }
        }
        (variables, refs)
    }

    /// Create a new board with a unique ID
    /// The board is created in the base directory appended with the ID
    pub fn new(id: Option<String>, base_dir: Path, app_state: Arc<FlowLikeState>) -> Self {
        let mut board = Self::new_detached(id, base_dir);
        board.app_state = Some(app_state);
        board
    }

    /// Create a board without runtime state. Deterministic transforms, importers, and fixtures can
    /// use this constructor and attach credentials before calling storage or execution methods.
    pub fn new_detached(id: Option<String>, base_dir: Path) -> Self {
        let id = id.unwrap_or(create_id());
        let board_dir = base_dir;

        let mut board = Board {
            id,
            name: "New Board".to_string(),
            description: "Your new Workflow!".to_string(),
            nodes: HashMap::new(),
            variables: HashMap::new(),
            comments: HashMap::new(),
            log_level: LogLevel::Info,
            stage: ExecutionStage::Dev,
            execution_mode: ExecutionMode::Hybrid,
            viewport: (0.0, 0.0, 0.0),
            version: (0, 0, 1),
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            layers: HashMap::new(),
            page_ids: Vec::new(),
            hash: None,
            refs: HashMap::new(),
            internal_refs: HashMap::new(),
            parent: None,
            board_dir,
            logic_nodes: HashMap::new(),
            app_state: None,
            pin_index: None,
        };
        board.hash();
        board
    }

    pub fn mark_changed(&mut self) {
        self.updated_at = SystemTime::now();
        self.hash();
    }

    /// Read one host-owned board reference. Public workflow refs are deliberately inaccessible
    /// through this API, and a non-reserved key can never alias into the internal namespace.
    pub fn internal_ref(&self, key: &str) -> Option<&str> {
        is_internal_board_ref(key)
            .then(|| self.internal_refs.get(key).map(String::as_str))
            .flatten()
    }

    /// Persist one host-owned board reference after enforcing the reserved namespace boundary.
    pub fn insert_internal_ref(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> flow_like_types::Result<Option<String>> {
        let key = key.into();
        if !is_internal_board_ref(&key) {
            return Err(flow_like_types::anyhow!(
                "internal board reference keys must start with '{INTERNAL_BOARD_REF_PREFIX}'"
            ));
        }
        Ok(self.internal_refs.insert(key, value.into()))
    }

    /// Remove one host-owned board reference. Non-reserved keys are never removed by this API.
    pub fn remove_internal_ref(&mut self, key: &str) -> Option<String> {
        is_internal_board_ref(key)
            .then(|| self.internal_refs.remove(key))
            .flatten()
    }

    /// Iterate over one reserved sub-namespace without exposing the backing map for mutation.
    pub fn internal_refs_with_prefix<'a>(
        &'a self,
        prefix: &'a str,
    ) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
        let valid_prefix = is_internal_board_ref(prefix);
        self.internal_refs.iter().filter_map(move |(key, value)| {
            (valid_prefix && key.starts_with(prefix)).then_some((key.as_str(), value.as_str()))
        })
    }

    /// Retain selected values in one reserved sub-namespace while leaving every other host-owned
    /// namespace untouched.
    pub fn retain_internal_refs_with_prefix<F>(
        &mut self,
        prefix: &str,
        mut retain: F,
    ) -> flow_like_types::Result<()>
    where
        F: FnMut(&str, &str) -> bool,
    {
        if !is_internal_board_ref(prefix) {
            return Err(flow_like_types::anyhow!(
                "internal board reference prefixes must start with '{INTERNAL_BOARD_REF_PREFIX}'"
            ));
        }
        self.internal_refs
            .retain(|key, value| !key.starts_with(prefix) || retain(key.as_str(), value.as_str()));
        self.internal_refs.shrink_to_fit();
        Ok(())
    }

    /// Remove all host bookkeeping before a board is copied into a semantic artifact such as a
    /// template, immutable published version, or fork.
    pub fn clear_internal_refs(&mut self) {
        self.internal_refs.clear();
    }

    /// Derives a governed ontology action's parameter schema from the start
    /// node's `parameters` struct pin.
    ///
    /// This is the authoritative source: the schema is read from the pinned,
    /// published board that actually executes, so a governed action can never
    /// advertise a contract its implementation board does not honor. Returns
    /// `None` when the start node has no typed `parameters` pin, which callers
    /// treat as "accepts any object payload". A present but malformed schema
    /// is an error: silently treating it as absent would widen the governed
    /// action contract.
    pub fn action_parameter_schema(
        &self,
        start_node_id: &str,
    ) -> flow_like_types::Result<Option<flow_like_types::Value>> {
        let node = self.nodes.get(start_node_id).ok_or_else(|| {
            flow_like_types::anyhow!(
                "Start node '{}' does not exist in the action implementation board",
                start_node_id
            )
        })?;
        let Some(pin) = node.pins.values().find(|pin| {
            pin.name == "parameters"
                && pin.data_type == VariableType::Struct
                && pin.schema.is_some()
        }) else {
            return Ok(None);
        };
        let raw_schema = pin
            .schema
            .as_deref()
            .expect("the selected parameters pin has a schema");
        let parsed: flow_like_types::Value =
            flow_like_types::json::from_str(raw_schema).map_err(|error| {
                flow_like_types::anyhow!(
                    "The parameters schema on start node '{}' is invalid JSON: {error}",
                    start_node_id
                )
            })?;
        if !parsed.is_object() {
            return Err(flow_like_types::anyhow!(
                "The parameters schema on start node '{}' must be a JSON object",
                start_node_id
            ));
        }
        Ok(Some(parsed))
    }

    pub fn hash(&mut self) {
        for node in self.nodes.values_mut() {
            node.hash();
        }
        for variable in self.variables.values_mut() {
            variable.hash();
        }
        for comment in self.comments.values_mut() {
            comment.hash();
        }
        for layer in self.layers.values_mut() {
            for node in layer.nodes.values_mut() {
                node.hash();
            }
            for variable in layer.variables.values_mut() {
                variable.hash();
            }
            for comment in layer.comments.values_mut() {
                comment.hash();
            }
            layer.hash();
        }

        let mut hasher = HighwayHasher::new(highway::Key([
            0x0123456789abcdfe,
            0xfedcba9876543210,
            0x0011223344556677,
            0x8899aabbccddeeff,
        ]));

        hasher.append(self.id.as_bytes());
        hasher.append(self.name.as_bytes());
        hasher.append(self.description.as_bytes());
        hasher.append(&self.version.0.to_le_bytes());
        hasher.append(&self.version.1.to_le_bytes());
        hasher.append(&self.version.2.to_le_bytes());
        hasher.append(&self.viewport.0.to_le_bytes());
        hasher.append(&self.viewport.1.to_le_bytes());
        hasher.append(&self.viewport.2.to_le_bytes());
        hasher.append(&[stage_marker(&self.stage)]);
        hasher.append(&[self.log_level.to_u8()]);
        hasher.append(&[execution_mode_marker(&self.execution_mode)]);

        let mut refs = self
            .refs
            .iter()
            .filter(|(key, _)| !is_internal_board_ref(key))
            .collect::<Vec<_>>();
        refs.sort_by_key(|(key, _)| *key);
        for (key, value) in refs {
            hasher.append(key.as_bytes());
            hasher.append(value.as_bytes());
        }

        for page_id in &self.page_ids {
            hasher.append(page_id.as_bytes());
        }

        let mut nodes = self.nodes.iter().collect::<Vec<_>>();
        nodes.sort_by_key(|(id, _)| *id);
        for (id, node) in nodes {
            hasher.append(id.as_bytes());
            append_optional_u64(&mut hasher, node.hash);
        }

        let mut variables = self.variables.iter().collect::<Vec<_>>();
        variables.sort_by_key(|(id, _)| *id);
        for (id, variable) in variables {
            hasher.append(id.as_bytes());
            append_optional_u64(&mut hasher, variable.hash);
        }

        let mut comments = self.comments.iter().collect::<Vec<_>>();
        comments.sort_by_key(|(id, _)| *id);
        for (id, comment) in comments {
            hasher.append(id.as_bytes());
            append_optional_u64(&mut hasher, comment.hash);
        }

        let mut layers = self.layers.iter().collect::<Vec<_>>();
        layers.sort_by_key(|(id, _)| *id);
        for (id, layer) in layers {
            hasher.append(id.as_bytes());
            append_optional_u64(&mut hasher, layer.hash);

            let mut layer_nodes = layer.nodes.iter().collect::<Vec<_>>();
            layer_nodes.sort_by_key(|(id, _)| *id);
            for (id, node) in layer_nodes {
                hasher.append(id.as_bytes());
                append_optional_u64(&mut hasher, node.hash);
            }

            let mut layer_variables = layer.variables.iter().collect::<Vec<_>>();
            layer_variables.sort_by_key(|(id, _)| *id);
            for (id, variable) in layer_variables {
                hasher.append(id.as_bytes());
                append_optional_u64(&mut hasher, variable.hash);
            }

            let mut layer_comments = layer.comments.iter().collect::<Vec<_>>();
            layer_comments.sort_by_key(|(id, _)| *id);
            for (id, comment) in layer_comments {
                hasher.append(id.as_bytes());
                append_optional_u64(&mut hasher, comment.hash);
            }
        }

        self.hash = Some(hasher.finalize64());
    }

    /// Recompute catalog-derived node schemas for an already-open board.
    ///
    /// Hosts call this when their node/widget registry becomes available or is
    /// replaced after the board was loaded. The refresh is deliberately
    /// in-memory only: it must not make the board look user-edited or persist
    /// schema-only changes behind the user's back.
    pub async fn refresh_node_definitions(&mut self, state: Arc<FlowLikeState>) {
        let updated_at = self.updated_at;
        let hash = self.hash;

        // A registry replacement can also replace NodeLogic implementations.
        // Do not let the board's per-node logic cache pin it to the old one.
        self.logic_nodes.clear();
        self.node_updates(state).await;
        self.cleanup();

        self.updated_at = updated_at;
        self.hash = hash;
    }

    async fn node_updates(&mut self, state: Arc<FlowLikeState>) {
        self.node_updates_scoped(state, None).await;
    }

    /// Re-derive nodes through their `on_update` until the board settles.
    ///
    /// `dirty` narrows the sweep to what a command batch could have reached. `None` re-derives
    /// every node, which is what board load, undo/redo and a registry swap need — and what the
    /// narrowed sweep is checked against. See [`dirty`] for the propagation channels and why
    /// restricting the sweep is sound.
    async fn node_updates_scoped(&mut self, state: Arc<FlowLikeState>, dirty: Option<&Touched>) {
        let registry = state.node_registry().clone();
        let registry = registry.read().await;

        // First, sync node schemas for any version mismatches
        // This runs BEFORE on_update so dynamic nodes can still add their pins
        cleanup::sync_node_schema::sync_board_node_schemas(self, &registry.node_registry).await;

        // The schema sync above expands compact schema refs onto every pin, and `Node::hash`
        // covers `pin.schema`. Without re-baselining, roughly half the nodes on a real board
        // report "changed" in pass 1 purely from that bookkeeping, which forces a second full
        // `on_update` pass over every node on every load. Rebase the hashes now so pass 1's
        // change detector only sees what `on_update` itself did.
        for node in self.nodes.values_mut() {
            node.hash();
        }
        for layer in self.layers.values_mut() {
            for node in layer.nodes.values_mut() {
                node.hash();
            }
        }

        // `get_pin_by_id` scans the whole board, and the `on_update` of variable, struct and widget
        // nodes calls it once per connection — the dominant cost of this sweep on large boards.
        // Index the pins for its duration: `on_update` takes an immutable board, so only the node
        // being updated can change, and refreshing its entries on write-back keeps the index exact.
        self.pin_index = Some(self.build_pin_index());

        match dirty.and_then(|touched| self.dirty_queue(touched)) {
            Some((index, queue)) => self.settle_dirty_nodes(&registry, &index, queue).await,
            None => self.settle_every_node(&registry).await,
        }

        self.pin_index = None;

        for layer in self.layers.values_mut() {
            layer.hash();
        }

        for variable in self.variables.values_mut() {
            variable.hash();
        }

        for comment in self.comments.values_mut() {
            comment.hash();
        }
    }

    /// Re-run every node until a pass changes nothing.
    ///
    /// This is the reference behaviour a dirty sweep is measured against, so it stays exhaustive:
    /// any node whose `on_update` moved forces another pass over the whole board.
    async fn settle_every_node(&mut self, registry: &FlowNodeRegistry) {
        for _ in 0..MAX_UPDATE_PASSES {
            let mut changed = false;

            let node_ids: Vec<String> = self.nodes.keys().cloned().collect();
            for node_id in node_ids {
                let Some(node) = self.nodes.remove(&node_id) else {
                    continue;
                };
                let owner = PinOwner::Node(node_id.clone());
                let (node, node_changed) = self.update_node(registry, node, owner).await;
                changed |= node_changed;
                self.nodes.insert(node_id, node);
            }

            let layer_ids: Vec<String> = self.layers.keys().cloned().collect();
            for layer_id in layer_ids {
                let layer_node_ids: Vec<String> = match self.layers.get(&layer_id) {
                    Some(layer) => layer.nodes.keys().cloned().collect(),
                    None => continue,
                };

                for node_id in layer_node_ids {
                    let Some(node) = self
                        .layers
                        .get_mut(&layer_id)
                        .and_then(|layer| layer.nodes.remove(&node_id))
                    else {
                        continue;
                    };
                    let owner = PinOwner::LayerNode {
                        layer: layer_id.clone(),
                        node: node_id.clone(),
                    };
                    let (node, node_changed) = self.update_node(registry, node, owner).await;
                    changed |= node_changed;
                    if let Some(layer) = self.layers.get_mut(&layer_id) {
                        layer.nodes.insert(node_id, node);
                    }
                }
            }

            if !changed {
                break;
            }
        }
    }

    /// Re-run only the queued nodes, following each change to whatever reads it.
    ///
    /// A node re-enters the queue only when its own `on_update` moved it, so the work is bounded by
    /// the edit instead of by the board. Anything a command changed *without* running `on_update`
    /// is already accounted for: [`DirtyIndex::seed`] queues the wired neighbours of every node the
    /// batch wrote.
    async fn settle_dirty_nodes(
        &mut self,
        registry: &FlowNodeRegistry,
        index: &DirtyIndex,
        mut queue: VecDeque<String>,
    ) {
        let mut queued: HashSet<String> = queue.iter().cloned().collect();
        // The same budget the full sweep has, so a pathological chain degrades to the old cost
        // rather than spinning.
        let budget = self.nodes.len().saturating_mul(MAX_UPDATE_PASSES);
        let mut visits = 0usize;

        while let Some(node_id) = queue.pop_front() {
            queued.remove(&node_id);
            visits += 1;
            if visits > budget {
                tracing::warn!(
                    board = %self.id,
                    "scoped node update did not settle within its budget; falling back to a full sweep"
                );
                self.settle_every_node(registry).await;
                return;
            }

            let Some(node) = self.nodes.remove(&node_id) else {
                continue;
            };
            let owner = PinOwner::Node(node_id.clone());
            let (node, changed) = self.update_node(registry, node, owner).await;
            self.nodes.insert(node_id.clone(), node);

            if !changed {
                continue;
            }
            let mut dependents = HashSet::new();
            index.wired_neighbours(self, &node_id, &mut dependents);
            for dependent in dependents {
                if queued.insert(dependent.clone()) {
                    queue.push_back(dependent);
                }
            }
        }
    }

    /// Run one node's `on_update`, returning it with whether it changed itself.
    ///
    /// The node is detached from the board across the call because `on_update` receives the board
    /// immutably, so a lookup of its own pins answers `None` while it runs — the same answer the
    /// pre-index scan gave for a detached node.
    async fn update_node(
        &mut self,
        registry: &FlowNodeRegistry,
        mut node: Node,
        owner: PinOwner,
    ) -> (Node, bool) {
        let old_hash = node.hash;
        let Some(node_logic) = self.node_logic(registry, &node) else {
            return (node, false);
        };
        let previous_pins: Vec<String> = node.pins.keys().cloned().collect();
        node_logic.on_update(&mut node, self).await;

        node.hash();
        let changed = node.hash != old_hash;
        if changed {
            self.reindex_node_pins(owner, &previous_pins, &node);
        }
        (node, changed)
    }

    /// The logic for a node's type, instantiated once and memoized on the board.
    fn node_logic(
        &mut self,
        registry: &FlowNodeRegistry,
        node: &Node,
    ) -> Option<Arc<dyn NodeLogic>> {
        if let Some(logic) = self.logic_nodes.get(&node.name) {
            return Some(Arc::clone(logic));
        }
        let logic = registry.instantiate(node).ok()?;
        self.logic_nodes
            .insert(node.name.clone(), Arc::clone(&logic));
        Some(logic)
    }

    /// The index and starting queue for a scoped sweep, or `None` when it cannot be bounded.
    fn dirty_queue(&self, touched: &Touched) -> Option<(DirtyIndex, VecDeque<String>)> {
        // Nodes parked inside a layer are a legacy shape none of the propagation channels describe.
        // Every board written since keeps its nodes in `self.nodes` with a `layer` tag, so bounding
        // this case would buy nothing and risk a great deal.
        if self.layers.values().any(|layer| !layer.nodes.is_empty()) {
            return None;
        }
        let index = DirtyIndex::build(self);
        let queue = index.seed(self, touched).into_iter().collect();
        Some((index, queue))
    }

    /// The container that owns `pin_id`, while [`Self::pin_index`] is live.
    pub(crate) fn pin_owner(&self, pin_id: &str) -> Option<&PinOwner> {
        self.pin_index.as_ref()?.get(pin_id)
    }

    async fn rollback_commands(
        &mut self,
        commands: &mut [GenericCommand],
        state: Arc<FlowLikeState>,
    ) -> Vec<String> {
        let mut recovery_errors = Vec::new();
        for command in commands.iter_mut().rev() {
            if let Err(error) = command.undo(self, state.clone()).await {
                recovery_errors.push(format!("rollback undo failed: {error}"));
            }
        }
        recovery_errors
    }

    async fn reapply_commands(
        &mut self,
        commands: &mut [GenericCommand],
        state: Arc<FlowLikeState>,
    ) -> Vec<String> {
        let mut recovery_errors = Vec::new();
        for command in commands.iter_mut() {
            if let Err(error) = command.execute(self, state.clone()).await {
                recovery_errors.push(format!("rollback execute failed: {error}"));
            }
        }
        recovery_errors
    }

    pub async fn execute_command(
        &mut self,
        command: GenericCommand,
        state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<GenericCommand> {
        let mut command = command;
        if tracing::enabled!(tracing::Level::DEBUG) {
            let cmd_json = serde_json::to_string(&command).unwrap_or_default();
            tracing::debug!(command = %cmd_json, "Executing board command");
        }
        command.validate(self, state.clone()).await?;
        if let Err(e) = command.execute(self, state.clone()).await {
            let mut recovery_errors = Vec::new();
            if let Err(rollback_error) = command.undo(self, state.clone()).await {
                recovery_errors.push(format!(
                    "failed to rollback current command after execute error: {rollback_error}"
                ));
            }
            tracing::error!(error = ?e, "Board command execution failed");
            let primary_error = e.to_string();
            let error = if recovery_errors.is_empty() {
                e
            } else {
                flow_like_types::anyhow!(
                    "Command execution failed: {primary_error}; recovery errors: {}",
                    recovery_errors.join(" | ")
                )
            };
            return Err(error);
        }
        tracing::debug!("Board command executed successfully");
        let mut touched = Touched::default();
        command.touched(&mut touched);
        self.node_updates_scoped(state, Some(&touched)).await;
        self.cleanup();
        self.mark_changed();
        Ok(command)
    }

    /// Restate every node whose pin identities `on_update` derived rather than the batch itself.
    ///
    /// Pins minted inside `on_update` — function-call mirrors, `string_format` placeholders — are
    /// allocated with `create_id()`, so any machine that re-derives them gets *different* ids. The
    /// returned batch is not only local undo history: the desktop ships it to the Hub and replays it
    /// there verbatim. A `ConnectPin` that targets such a pin therefore only resolves if the batch
    /// also carries the node state that owns it.
    ///
    /// `on_update` implementations reconcile mirrored pins by name, so replaying explicit node state
    /// makes the replayer adopt these ids instead of minting a second set.
    fn derived_node_state_commands(
        &self,
        before: &HashMap<String, Node>,
        executed: &[GenericCommand],
    ) -> Vec<GenericCommand> {
        let mut described = HashMap::<&str, &Node>::new();
        for command in executed {
            match command {
                GenericCommand::AddNode(command) => {
                    described.insert(command.node.id.as_str(), &command.node);
                }
                GenericCommand::UpdateNode(command) => {
                    described.insert(command.node.id.as_str(), &command.node);
                }
                GenericCommand::CopyPaste(command) => {
                    for node in &command.new_nodes {
                        described.insert(node.id.as_str(), node);
                    }
                }
                GenericCommand::UpsertLayer(command) => {
                    for node in command.layer.nodes.values() {
                        described.insert(node.id.as_str(), node);
                    }
                }
                _ => {}
            }
        }

        let mut restated = self
            .nodes
            .iter()
            .filter_map(|(node_id, current)| {
                // `node_updates` runs `on_update` on every node, so a layer edit can re-mint pins on
                // a node no command in this batch mentions.
                let previous = described
                    .get(node_id.as_str())
                    .copied()
                    .or_else(|| before.get(node_id))?;
                if pin_ids_match(current, previous) {
                    return None;
                }
                Some((
                    node_id.clone(),
                    GenericCommand::UpdateNode(UpdateNodeCommand {
                        node: current.clone(),
                        old_node: Some(previous.clone()),
                    }),
                ))
            })
            .collect::<Vec<_>>();
        // Map iteration order is randomized, and the remote retry identity is a digest of the exact
        // payload — an unstable order would turn every retry into an idempotency conflict.
        restated.sort_by(|(left, _), (right, _)| left.cmp(right));
        restated.into_iter().map(|(_, command)| command).collect()
    }

    pub async fn execute_commands(
        &mut self,
        commands: Vec<GenericCommand>,
        state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<Vec<GenericCommand>> {
        let mut commands = commands;
        let nodes_before = self.nodes.clone();
        for index in 0..commands.len() {
            if let Err(error) = commands[index].validate(self, state.clone()).await {
                let recovery_errors = self
                    .rollback_commands(&mut commands[..index], state.clone())
                    .await;
                return Err(if recovery_errors.is_empty() {
                    error
                } else {
                    flow_like_types::anyhow!(
                        "Command batch validation failed at index {index}: {error}; recovery errors: {}",
                        recovery_errors.join(" | ")
                    )
                });
            }

            if let Err(error) = commands[index].execute(self, state.clone()).await {
                let mut recovery_errors = Vec::new();
                if let Err(rollback_error) = commands[index].undo(self, state.clone()).await {
                    recovery_errors.push(format!(
                        "failed to rollback current command at index {index}: {rollback_error}"
                    ));
                }
                recovery_errors.extend(
                    self.rollback_commands(&mut commands[..index], state.clone())
                        .await,
                );
                return Err(if recovery_errors.is_empty() {
                    error
                } else {
                    flow_like_types::anyhow!(
                        "Command batch execution failed at index {index}: {error}; recovery errors: {}",
                        recovery_errors.join(" | ")
                    )
                });
            }
        }
        let mut touched = Touched::default();
        for command in &commands {
            command.touched(&mut touched);
        }
        self.node_updates_scoped(state, Some(&touched)).await;
        self.cleanup();
        self.mark_changed();
        let derived = self.derived_node_state_commands(&nodes_before, &commands);
        commands.extend(derived);
        Ok(commands)
    }

    pub async fn undo(
        &mut self,
        commands: Vec<GenericCommand>,
        state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        let mut commands = commands;
        for index in (0..commands.len()).rev() {
            if let Err(error) = commands[index].undo(self, state.clone()).await {
                let mut recovery_errors = Vec::new();
                if let Err(rollback_error) = commands[index].execute(self, state.clone()).await {
                    recovery_errors.push(format!(
                        "failed to restore current command at index {index}: {rollback_error}"
                    ));
                }
                recovery_errors.extend(
                    self.reapply_commands(&mut commands[index + 1..], state.clone())
                        .await,
                );
                return Err(if recovery_errors.is_empty() {
                    error
                } else {
                    flow_like_types::anyhow!(
                        "Undo failed at index {index}: {error}; recovery errors: {}",
                        recovery_errors.join(" | ")
                    )
                });
            }
        }
        self.node_updates(state).await;
        self.cleanup();
        self.mark_changed();
        Ok(())
    }

    pub async fn redo(
        &mut self,
        commands: Vec<GenericCommand>,
        state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        let mut commands = commands;
        for index in 0..commands.len() {
            if let Err(error) = commands[index].validate(self, state.clone()).await {
                let recovery_errors = self
                    .rollback_commands(&mut commands[..index], state.clone())
                    .await;
                return Err(if recovery_errors.is_empty() {
                    error
                } else {
                    flow_like_types::anyhow!(
                        "Redo validation failed at index {index}: {error}; recovery errors: {}",
                        recovery_errors.join(" | ")
                    )
                });
            }

            if let Err(error) = commands[index].execute(self, state.clone()).await {
                let mut recovery_errors = Vec::new();
                if let Err(rollback_error) = commands[index].undo(self, state.clone()).await {
                    recovery_errors.push(format!(
                        "failed to rollback current redo command at index {index}: {rollback_error}"
                    ));
                }
                recovery_errors.extend(
                    self.rollback_commands(&mut commands[..index], state.clone())
                        .await,
                );
                return Err(if recovery_errors.is_empty() {
                    error
                } else {
                    flow_like_types::anyhow!(
                        "Redo failed at index {index}: {error}; recovery errors: {}",
                        recovery_errors.join(" | ")
                    )
                });
            }
        }
        self.node_updates(state).await;
        self.cleanup();
        self.mark_changed();
        Ok(())
    }

    pub fn get_pin_by_id(&self, pin_id: &str) -> Option<&Pin> {
        if self.pin_index.is_some() {
            return self.indexed_pin(pin_id);
        }

        for node in self.nodes.values() {
            if let Some(pin) = node.pins.get(pin_id) {
                return Some(pin);
            }
        }

        for layer in self.layers.values() {
            if let Some(pin) = layer.pins.get(pin_id) {
                return Some(pin);
            }
            for node in layer.nodes.values() {
                if let Some(pin) = node.pins.get(pin_id) {
                    return Some(pin);
                }
            }
        }

        None
    }

    /// A node is removed from its map while its own `on_update` runs, so an index entry that no
    /// longer resolves yields `None` — the same answer the scan gives for a detached node.
    fn indexed_pin(&self, pin_id: &str) -> Option<&Pin> {
        match self.pin_index.as_ref()?.get(pin_id)? {
            PinOwner::Node(node) => self.nodes.get(node)?.pins.get(pin_id),
            PinOwner::LayerPin(layer) => self.layers.get(layer)?.pins.get(pin_id),
            PinOwner::LayerNode { layer, node } => {
                self.layers.get(layer)?.nodes.get(node)?.pins.get(pin_id)
            }
        }
    }

    /// Layer entries are written first so a pin id present in both a node and a layer resolves to
    /// the node, matching the scan order of the fallback in [`Self::get_pin_by_id`].
    fn build_pin_index(&self) -> HashMap<String, PinOwner> {
        let mut index = HashMap::new();
        for (layer_id, layer) in &self.layers {
            for pin_id in layer.pins.keys() {
                index.insert(pin_id.clone(), PinOwner::LayerPin(layer_id.clone()));
            }
            for (node_id, node) in &layer.nodes {
                for pin_id in node.pins.keys() {
                    index.insert(
                        pin_id.clone(),
                        PinOwner::LayerNode {
                            layer: layer_id.clone(),
                            node: node_id.clone(),
                        },
                    );
                }
            }
        }
        for (node_id, node) in &self.nodes {
            for pin_id in node.pins.keys() {
                index.insert(pin_id.clone(), PinOwner::Node(node_id.clone()));
            }
        }
        index
    }

    /// Re-point the index at `node`'s pins after its `on_update`, dropping the ids it gave up.
    fn reindex_node_pins(&mut self, owner: PinOwner, previous: &[String], node: &Node) {
        let Some(index) = self.pin_index.as_mut() else {
            return;
        };
        for pin_id in previous {
            if !node.pins.contains_key(pin_id) {
                index.remove(pin_id);
            }
        }
        for pin_id in node.pins.keys() {
            index.insert(pin_id.clone(), owner.clone());
        }
    }

    pub fn get_dependent_nodes(&self, node_id: &str) -> Vec<&Node> {
        let mut dependent_nodes = HashMap::new();
        for node in self.nodes.values() {
            for pin in node.pins.values() {
                if pin.depends_on.contains(node_id) {
                    dependent_nodes.insert(&node.id, node);
                }
            }
        }

        dependent_nodes.values().cloned().collect()
    }

    pub fn get_connected_nodes(&self, node_id: &str) -> Vec<&Node> {
        let mut connected_nodes = HashMap::new();
        for node in self.nodes.values() {
            for pin in node.pins.values() {
                if pin.connected_to.contains(node_id) {
                    connected_nodes.insert(&node.id, node);
                }
            }
        }

        connected_nodes.values().cloned().collect()
    }

    pub fn get_variable(&self, variable_id: &str) -> Option<&Variable> {
        self.variables.get(variable_id)
    }

    /// Search for a variable in board globals AND all layer-scoped variables.
    pub fn get_any_variable(&self, variable_id: &str) -> Option<Variable> {
        if let Some(var) = self.variables.get(variable_id) {
            return Some(var.clone());
        }
        for layer in self.layers.values() {
            if let Some(var) = layer.variables.get(variable_id) {
                return Some(var.clone());
            }
        }
        None
    }

    /// Writes an immutable snapshot of the current board (and its pages) at the
    /// given version without changing the working `version` or touching the
    /// floating "latest" board. Used to publish the exact version a governed
    /// ontology action pins, so it can be validated and invoked reproducibly.
    pub async fn snapshot_at_version(
        &self,
        version: (u32, u32, u32),
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<()> {
        let store = self.get_store(store).await?;

        // Always serialize a board whose embedded version agrees with the
        // immutable path. In particular, the two-phase action publisher calls
        // this while the floating draft still points at its previous version.
        let mut published = self.clone();
        published.version = version;
        published.clear_internal_refs();
        published.hash();

        let board_version_path = self
            .board_dir
            .child("versions")
            .child(self.id.clone())
            .child(format!("{}_{}_{}.board", version.0, version.1, version.2));

        match store.head(&board_version_path).await {
            Ok(_) => {
                if published
                    .snapshot_matches_current(version, Some(store.clone()))
                    .await?
                    && published
                        .snapshot_matches_persisted_draft(version, Some(store.clone()))
                        .await?
                {
                    // A previous attempt may have committed the immutable
                    // snapshot and lost its response. Identical retries are
                    // safe, provided the persisted floating draft still names
                    // that content. A stale process-local board must not make
                    // an old implementation look current.
                    return Ok(());
                }
                return Err(Self::immutable_snapshot_conflict(version));
            }
            Err(object_store::Error::NotFound { .. }) => {}
            Err(error) => return Err(error.into()),
        }

        // Copy pages first and atomically create the board last. The board file
        // is the snapshot's existence marker; once present it can never be
        // replaced by a later draft or a racing publisher.
        for page_id in &self.page_ids {
            let src_path = self.page_path(page_id);
            let dst_path = self.versioned_page_path(version, page_id);
            let page_proto: proto::Page = from_compressed(store.clone(), src_path).await?;
            match store.head(&dst_path).await {
                Ok(_) => {
                    let existing: proto::Page =
                        from_compressed(store.clone(), dst_path.clone()).await?;
                    if existing != page_proto {
                        return Err(Self::immutable_snapshot_conflict(version));
                    }
                }
                Err(object_store::Error::NotFound { .. }) => {
                    if let Err(create_error) =
                        compress_to_file_create(store.clone(), dst_path.clone(), &page_proto).await
                    {
                        // A racing identical publisher is success. A racing
                        // different publisher owns this version slot and must
                        // not be overwritten.
                        let raced: flow_like_types::Result<proto::Page> =
                            from_compressed(store.clone(), dst_path).await;
                        if !matches!(raced, Ok(ref existing) if existing == &page_proto) {
                            return Err(create_error);
                        }
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }

        let board = published.to_proto();
        if let Err(create_error) =
            compress_to_file_create(store.clone(), board_version_path, &board).await
            && !published
                .snapshot_matches_current(version, Some(store.clone()))
                .await
                .unwrap_or(false)
        {
            return Err(create_error);
        }

        // The board object is the publication marker, so do one final read of
        // the authoritative floating draft (and all current page objects)
        // before returning a snapshot that callers may reference from other
        // metadata. This detects a board/page edit that raced the page-first
        // copy and prevents a torn immutable snapshot from being installed as
        // an action implementation. The already-created version remains an
        // inert immutable orphan and a later attempt advances to a fresh patch.
        if !published
            .snapshot_matches_persisted_draft(version, Some(store))
            .await?
        {
            return Err(BoardDraftChanged { version }.into());
        }

        Ok(())
    }

    fn immutable_snapshot_conflict(version: (u32, u32, u32)) -> flow_like_types::Error {
        flow_like_types::anyhow!(
            "Board version {}.{}.{} already contains different immutable snapshot data",
            version.0,
            version.1,
            version.2
        )
    }

    /// Return whether a version slot is empty or contains only immutable
    /// artifacts identical to this draft. A board-last publication can leave
    /// page objects behind when interrupted; identical retries resume them,
    /// while different drafts skip the occupied patch rather than poisoning
    /// all future retries.
    pub async fn snapshot_version_slot_is_compatible(
        &self,
        version: (u32, u32, u32),
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<bool> {
        let store = self.get_store(store).await?;
        self.snapshot_version_slot_is_compatible_with_store(version, store)
            .await
    }

    async fn snapshot_version_slot_is_compatible_with_store(
        &self,
        version: (u32, u32, u32),
        store: Arc<dyn ObjectStore>,
    ) -> flow_like_types::Result<bool> {
        let board_path = Self::proto_path(&self.board_dir, &self.id, Some(version));
        match store.head(&board_path).await {
            Ok(_) => {
                return self.snapshot_matches_current(version, Some(store)).await;
            }
            Err(object_store::Error::NotFound { .. }) => {}
            Err(error) => return Err(error.into()),
        }

        for page_id in &self.page_ids {
            let path = self.versioned_page_path(version, page_id);
            match store.head(&path).await {
                Ok(_) => {
                    let current: proto::Page =
                        from_compressed(store.clone(), self.page_path(page_id)).await?;
                    let existing: proto::Page = from_compressed(store.clone(), path).await?;
                    if current != existing {
                        return Ok(false);
                    }
                }
                Err(object_store::Error::NotFound { .. }) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(true)
    }

    /// Recompute the deterministic board hash instead of trusting a possibly
    /// stale serialized `hash` field.
    pub fn content_hash(&self) -> u64 {
        let mut board = self.clone();
        board.hash();
        board.hash.expect("Board::hash always sets a hash")
    }

    /// Return whether an existing immutable snapshot contains the same board
    /// and page content as the current draft when assigned `version`.
    /// Normalizing the draft version makes this useful between the prepare and
    /// commit phases, while the floating board still carries its old version.
    pub async fn snapshot_matches_current(
        &self,
        version: (u32, u32, u32),
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<bool> {
        let store = self.get_store(store).await?;
        let proto: proto::Board = from_compressed(
            store.clone(),
            Self::proto_path(&self.board_dir, &self.id, Some(version)),
        )
        .await?;
        let snapshot = Self::from_proto(proto);
        // Compare what persistence keeps, not what the draft holds in memory. Protobuf stores
        // several `Option<bool>` / `Option<f64>` fields as bare proto3 scalars, so an explicit
        // `Some(false)` or `Some(0.0)` (a2ui element pins carry `enforce_schema: Some(false)`)
        // is indistinguishable from unset and reads back as `None`. Hashing the live draft
        // directly would report every such board as different from the snapshot just written
        // from it, and the publisher would never recognize its own output.
        let mut current = Self::from_proto(self.to_proto());
        current.version = version;
        if snapshot.content_hash() != current.content_hash() {
            return Ok(false);
        }
        for page_id in &self.page_ids {
            let current: proto::Page =
                from_compressed(store.clone(), self.page_path(page_id)).await?;
            let published: proto::Page =
                from_compressed(store.clone(), self.versioned_page_path(version, page_id)).await?;
            if current != published {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Compare an immutable version with the authoritative floating board
    /// object and its current page objects, bypassing any process-local board
    /// registry entry. If no floating object exists (only possible for a
    /// not-yet-saved in-memory board), fall back to the caller's draft.
    pub async fn snapshot_matches_persisted_draft(
        &self,
        version: (u32, u32, u32),
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<bool> {
        let store = self.get_store(store).await?;
        let floating_path = Self::proto_path(&self.board_dir, &self.id, None);
        match store.head(&floating_path).await {
            Ok(_) => {}
            Err(object_store::Error::NotFound { .. }) => {
                return self.snapshot_matches_current(version, Some(store)).await;
            }
            Err(error) => return Err(error.into()),
        }

        // Read the draft exactly as it is stored. Re-deriving it through `from_loaded_proto`
        // would run a full `node_updates` sweep, which settles schema propagation further than
        // the scoped sweep an interactive edit performs — an unedited board would then look
        // "changed" on every publish. The question here is only whether another writer replaced
        // the draft, and that shows up in the stored bytes.
        let proto: proto::Board = from_compressed(store.clone(), floating_path).await?;
        let mut floating = Self::from_proto(proto);
        floating.board_dir = self.board_dir.clone();
        floating.app_state = self.app_state.clone();
        floating
            .snapshot_matches_current(version, Some(store))
            .await
    }

    /// Storage revision of the persisted floating draft.
    ///
    /// Publishers compare this across a failed publication to tell a racing
    /// writer apart from a snapshot comparison that can never succeed. `None`
    /// when the draft is absent or the backend offers no revision token, which
    /// callers must read as "cannot prove a race".
    pub async fn persisted_draft_revision(
        &self,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<Option<String>> {
        let store = self.get_store(store).await?;
        let floating_path = Self::proto_path(&self.board_dir, &self.id, None);
        match store.head(&floating_path).await {
            Ok(meta) => Ok(meta.e_tag.clone().or_else(|| meta.version.clone())),
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Read the authoritative floating draft as its own board, exactly as
    /// stored - `from_loaded_proto` would re-derive it and settle schema
    /// propagation further than an interactive edit, which is the difference
    /// that makes an unedited board look changed.
    async fn reloaded_persisted_draft(
        &self,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<Self> {
        let store = self.get_store(store).await?;
        let floating_path = Self::proto_path(&self.board_dir, &self.id, None);
        let proto: proto::Board = from_compressed(store, floating_path).await?;
        let mut floating = Self::from_proto(proto);
        floating.board_dir = self.board_dir.clone();
        floating.app_state = self.app_state.clone();
        Ok(floating)
    }

    /// Prepare a fresh immutable patch snapshot, recovering from an editor save
    /// that lands mid-publication.
    ///
    /// Only a publication the draft actually moved under is retried: the
    /// persisted revision is compared across the failure, so a
    /// [`BoardDraftChanged`] that no writer caused - a snapshot comparison that
    /// cannot succeed for this board - fails immediately instead of writing one
    /// orphan snapshot per attempt.
    pub async fn prepare_snapshot_recovering_from_races(
        &self,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<PreparedBoardSnapshot> {
        let store = self.get_store(store).await?;
        let mut reloaded: Option<Self> = None;
        let mut attempt = 0;
        loop {
            let (result, before, after) = {
                let publisher = reloaded.as_ref().unwrap_or(self);
                let before = publisher
                    .persisted_draft_revision(Some(store.clone()))
                    .await?;
                let result = publisher
                    .prepare_snapshot_at_fresh_patch_version(Some(store.clone()))
                    .await;
                let after = match result {
                    Ok(_) => None,
                    Err(_) => {
                        publisher
                            .persisted_draft_revision(Some(store.clone()))
                            .await?
                    }
                };
                (result, before, after)
            };
            let error = match result {
                Ok(prepared) => return Ok(prepared),
                Err(error) => error,
            };
            if attempt >= MAX_PUBLISH_RACE_RETRIES
                || !is_board_draft_race(&error)
                || !draft_was_replaced(&before, &after)
            {
                return Err(error);
            }
            reloaded = Some(self.reloaded_persisted_draft(Some(store.clone())).await?);
            attempt += 1;
        }
    }

    /// Publish the draft into `version`'s slot, moving to a fresh patch when an
    /// editor save invalidates it mid-publication. `None` means the requested
    /// version was published; `Some` carries the fresh snapshot the publication
    /// had to move to, which the caller must re-pin.
    pub async fn snapshot_at_version_recovering_from_races(
        &self,
        version: (u32, u32, u32),
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<Option<PreparedBoardSnapshot>> {
        let store = self.get_store(store).await?;
        let before = self.persisted_draft_revision(Some(store.clone())).await?;
        let error = match self.snapshot_at_version(version, Some(store.clone())).await {
            Ok(()) => return Ok(None),
            Err(error) => error,
        };
        if !is_board_draft_race(&error) {
            return Err(error);
        }
        let after = self.persisted_draft_revision(Some(store.clone())).await?;
        if !draft_was_replaced(&before, &after) {
            return Err(error);
        }
        self.reloaded_persisted_draft(Some(store.clone()))
            .await?
            .prepare_snapshot_recovering_from_races(Some(store))
            .await
            .map(Some)
    }

    /// Validate an already-created snapshot as the current draft's prepared
    /// publication. This is used to finish a commit whose floating-board save
    /// previously failed after the referring ontology was persisted.
    pub async fn prepared_snapshot_at_version(
        &self,
        version: (u32, u32, u32),
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<PreparedBoardSnapshot> {
        if !self.snapshot_matches_current(version, store).await? {
            return Err(Self::immutable_snapshot_conflict(version));
        }
        Ok(PreparedBoardSnapshot {
            board_id: self.id.clone(),
            version,
        })
    }

    /// Prepare a fresh immutable patch snapshot without changing or saving the
    /// floating board. Interrupted page-first attempts are resumed when their
    /// content matches and skipped when another draft owns the candidate slot.
    pub async fn prepare_snapshot_at_fresh_patch_version(
        &self,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<PreparedBoardSnapshot> {
        let store = self.get_store(store).await?;
        let first_patch = self
            .version
            .2
            .checked_add(1)
            .ok_or_else(|| flow_like_types::anyhow!("Board patch version overflow"))?;
        let mut next = (self.version.0, self.version.1, first_patch);

        for _ in 0..MAX_PATCH_SLOT_SCAN {
            if self
                .snapshot_version_slot_is_compatible_with_store(next, store.clone())
                .await?
            {
                match self.snapshot_at_version(next, Some(store.clone())).await {
                    Ok(()) => {
                        return Ok(PreparedBoardSnapshot {
                            board_id: self.id.clone(),
                            version: next,
                        });
                    }
                    Err(error) => {
                        // If a different publisher won the race, advance to a
                        // new patch. Preserve unrelated storage failures.
                        if self
                            .snapshot_version_slot_is_compatible_with_store(next, store.clone())
                            .await?
                        {
                            return Err(error);
                        }
                    }
                }
            }
            next.2 = next
                .2
                .checked_add(1)
                .ok_or_else(|| flow_like_types::anyhow!("Board patch version overflow"))?;
        }

        Err(flow_like_types::anyhow!(
            "Board {} found no free patch slot in {} attempts starting at {}.{}.{}; every attempt \
             published an immutable snapshot that then failed to validate, so the scan was \
             stopped instead of filling the version store",
            self.id,
            MAX_PATCH_SLOT_SCAN,
            self.version.0,
            self.version.1,
            first_patch
        ))
    }

    /// Advance the floating board to a prepared immutable snapshot after the
    /// metadata that refers to it has committed. If the user edited the draft
    /// in the meantime, leave it untouched; a subsequent publication will
    /// create another patch from that newer content.
    pub async fn commit_prepared_snapshot(
        &mut self,
        prepared: &PreparedBoardSnapshot,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<bool> {
        if prepared.board_id != self.id || prepared.version < self.version {
            return Ok(false);
        }
        let store = self.get_store(store).await?;
        // Protect unsaved in-process edits first.
        if !self
            .snapshot_matches_current(prepared.version, Some(store.clone()))
            .await?
        {
            return Ok(false);
        }

        // Then compare and conditionally update the authoritative floating
        // object. `open_board` may return a process-local cached board, so an
        // unconditional save here could otherwise overwrite another process's
        // edit made after the immutable snapshot was prepared.
        let floating_path = self.board_dir.child(format!("{}.board", self.id));
        let (floating_proto, floating_meta): (proto::Board, _) =
            match from_compressed_with_meta(store.clone(), floating_path.clone()).await {
                Ok(loaded) => loaded,
                Err(load_error) => match store.head(&floating_path).await {
                    Err(object_store::Error::NotFound { .. }) => {
                        let mut floating = self.clone();
                        floating.version = prepared.version;
                        floating.mark_changed();
                        compress_to_file_create(store, floating_path, &floating.to_proto()).await?;
                        self.version = prepared.version;
                        self.updated_at = floating.updated_at;
                        self.hash();
                        return Ok(true);
                    }
                    _ => return Err(load_error),
                },
            };
        let mut floating = Self::from_proto(floating_proto);
        floating.board_dir = self.board_dir.clone();
        if floating.version > prepared.version
            || !floating
                .snapshot_matches_current(prepared.version, Some(store.clone()))
                .await?
        {
            return Ok(false);
        }
        if floating.version == prepared.version {
            self.version = prepared.version;
            self.hash();
            return Ok(true);
        }

        floating.version = prepared.version;
        floating.mark_changed();
        if floating_meta.e_tag.is_none() && floating_meta.version.is_none() {
            // This backend cannot offer a compare-and-swap token. Leaving the
            // pointer behind is safe because reconciliation recognizes the
            // prepared patch; an unconditional write could lose another
            // writer's draft.
            return Ok(false);
        }
        compress_to_file_update(
            store,
            floating_path,
            &floating.to_proto(),
            UpdateVersion {
                e_tag: floating_meta.e_tag,
                version: floating_meta.version,
            },
        )
        .await?;
        self.version = prepared.version;
        self.updated_at = floating.updated_at;
        self.hash();
        Ok(true)
    }

    /// Publish the current draft under a fresh patch version without touching
    /// any existing versioned object. This is used when an ontology action's
    /// current pin already names an older immutable snapshot but the working
    /// board has since changed.
    pub async fn snapshot_at_fresh_patch_version(
        &mut self,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<(u32, u32, u32)> {
        let store = self.get_store(store).await?;
        let prepared = self
            .prepare_snapshot_at_fresh_patch_version(Some(store.clone()))
            .await?;
        let version = prepared.version();
        if !self
            .commit_prepared_snapshot(&prepared, Some(store))
            .await?
        {
            return Err(flow_like_types::anyhow!(
                "Board draft changed while publishing version {}.{}.{}",
                version.0,
                version.1,
                version.2
            ));
        }
        Ok(version)
    }

    pub async fn create_version(
        &mut self,
        version_type: VersionType,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<(u32, u32, u32)> {
        self.create_version_returning_published(version_type, store)
            .await
            .map(|(new_version, _published)| new_version)
    }

    /// Like [`Self::create_version`], but also returns the version the
    /// immutable snapshot was published under (the pre-bump version). Callers
    /// that derive per-version artifacts (e.g. compiled-board warm-up) need
    /// the snapshot's version, not the bumped draft version.
    pub async fn create_version_returning_published(
        &mut self,
        version_type: VersionType,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<((u32, u32, u32), (u32, u32, u32))> {
        let store = self.get_store(store).await?;
        let existing = self.get_versions(Some(store.clone())).await?;
        let mut published = self.version;
        if existing.contains(&published) {
            if !self
                .snapshot_matches_current(published, Some(store.clone()))
                .await?
            {
                published = self
                    .snapshot_at_fresh_patch_version(Some(store.clone()))
                    .await?;
            }
        } else {
            self.snapshot_at_version(published, Some(store.clone()))
                .await?;
        }

        let bump =
            |version: (u32, u32, u32)| -> flow_like_types::Result<_> {
                Ok(match &version_type {
                    VersionType::Major => (
                        version.0.checked_add(1).ok_or_else(|| {
                            flow_like_types::anyhow!("Board major version overflow")
                        })?,
                        0,
                        0,
                    ),
                    VersionType::Minor => (
                        version.0,
                        version.1.checked_add(1).ok_or_else(|| {
                            flow_like_types::anyhow!("Board minor version overflow")
                        })?,
                        0,
                    ),
                    VersionType::Patch => (
                        version.0,
                        version.1,
                        version.2.checked_add(1).ok_or_else(|| {
                            flow_like_types::anyhow!("Board patch version overflow")
                        })?,
                    ),
                })
            };
        let existing = self.get_versions(Some(store.clone())).await?;
        let mut new_version = bump(published)?;
        while existing.contains(&new_version) {
            new_version = bump(new_version)?;
        }

        self.version = new_version;
        self.mark_changed();
        self.save(Some(store)).await?;
        Ok((new_version, published))
    }

    pub async fn get_versions(
        &self,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<Vec<(u32, u32, u32)>> {
        let versions_dir = self
            .board_dir
            .clone()
            .child("versions")
            .child(self.id.clone());

        let store = match store {
            Some(store) => store,
            None => self
                .app_state
                .as_ref()
                .expect("app_state should always be set")
                .config
                .read()
                .await
                .stores
                .app_meta_store
                .clone()
                .ok_or(flow_like_types::anyhow!("Project store not found"))?
                .as_generic(),
        };

        let mut versions = store.list(Some(&versions_dir));
        let mut version_list = Vec::new();

        while let Some(meta) = versions.next().await {
            let meta = meta?;
            let file_name = match meta.location.filename() {
                Some(name) => name,
                None => continue,
            };
            if !file_name.ends_with(".board") {
                continue;
            }
            let version = file_name.strip_suffix(".board").unwrap_or(file_name);
            if version == "latest" {
                continue;
            }
            let version = version.strip_prefix("v").unwrap_or(version);
            let version = version.split("_").collect::<Vec<&str>>();

            if version.len() < 3 {
                continue;
            }

            let version = (
                version[0].parse::<u32>().unwrap_or(0),
                version[1].parse::<u32>().unwrap_or(0),
                version[2].parse::<u32>().unwrap_or(0),
            );

            version_list.push(version);
        }
        Ok(version_list)
    }

    /// Resolve the on-disk storage path for a board's compressed proto.
    /// `board_dir` is the per-app root (e.g. `apps/{app_id}`). When `version`
    /// is `None` this returns the floating "latest" path; otherwise the
    /// immutable per-version path.
    pub fn proto_path(board_dir: &Path, id: &str, version: Option<(u32, u32, u32)>) -> Path {
        match version {
            Some((maj, min, pat)) => board_dir
                .child("versions")
                .child(id.to_string())
                .child(format!("{}_{}_{}.board", maj, min, pat)),
            None => board_dir.child(format!("{}.board", id)),
        }
    }

    /// Fetch and decompress the board's proto representation. This step is
    /// independent of any `FlowLikeState` and produces no per-request data,
    /// so the result is safe to share across users (cf. executor's proto cache).
    #[instrument(name = "Board::load_proto", skip(store), level = "debug")]
    pub async fn load_proto(
        store: Arc<dyn ObjectStore>,
        board_dir: &Path,
        id: &str,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<flow_like_types::proto::Board> {
        let path = Self::proto_path(board_dir, id, version);
        from_compressed(store, path).await
    }

    /// Like [`Self::load_proto`] but additionally returns the storage
    /// [`ObjectMeta`] (e_tag / last_modified). Use this when caching the proto
    /// — a subsequent HEAD against the same path lets you detect mutations
    /// without re-downloading the body.
    #[instrument(name = "Board::load_proto_with_meta", skip(store), level = "debug")]
    pub async fn load_proto_with_meta(
        store: Arc<dyn ObjectStore>,
        board_dir: &Path,
        id: &str,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<(
        flow_like_types::proto::Board,
        flow_like_storage::object_store::ObjectMeta,
    )> {
        let path = Self::proto_path(board_dir, id, version);
        from_compressed_with_meta(store, path).await
    }

    /// Conditional [`Self::load_proto_with_meta`]: given the `e_tag` of a cached copy, one
    /// `If-None-Match` GET either confirms the cache (`NotModified`, no body) or delivers the
    /// changed proto in the same round trip. See [`ConditionalRead`].
    #[instrument(
        name = "Board::load_proto_if_changed",
        skip(store, e_tag),
        level = "debug"
    )]
    pub async fn load_proto_if_changed(
        store: Arc<dyn ObjectStore>,
        board_dir: &Path,
        id: &str,
        version: Option<(u32, u32, u32)>,
        e_tag: Option<&str>,
    ) -> flow_like_types::Result<ConditionalRead<flow_like_types::proto::Board>> {
        let path = Self::proto_path(board_dir, id, version);
        from_compressed_if_changed(store, path, e_tag).await
    }

    /// The object store the board's `.board` file lives in.
    pub async fn meta_store(
        app_state: &FlowLikeState,
    ) -> flow_like_types::Result<Arc<dyn ObjectStore>> {
        Ok(app_state
            .config
            .read()
            .await
            .stores
            .app_meta_store
            .clone()
            .ok_or_else(|| flow_like_types::anyhow!("Project store not found"))?
            .as_generic())
    }

    /// Build a fully-initialised `Board` from a previously-loaded proto.
    /// Runs `node_updates` so dynamic nodes/schema migrations apply against
    /// the caller's registry — this is per-request and must not be cached
    /// across users.
    pub async fn from_loaded_proto(
        proto: flow_like_types::proto::Board,
        board_dir: Path,
        app_state: Arc<FlowLikeState>,
    ) -> Self {
        let mut board = Board::from_proto(proto);
        board.board_dir = board_dir;
        board.app_state = Some(app_state.clone());
        board.logic_nodes = HashMap::new();

        board.node_updates(app_state).await;
        board.cleanup();
        // `node_updates` hashes nodes while their schema refs are expanded; `cleanup` then
        // compacts them again. Rehash so a loaded board carries the same node hashes as the
        // board `execute_commands` saves (which hashes after its own cleanup) — the client keys
        // its rendered-node cache on `node.hash`, and the sync protocol ships it, so a hash that
        // depends on the code path and not the content invalidates both for nothing.
        board.hash();

        board
    }

    #[instrument(name = "Board::load", skip(app_state, path), level = "debug")]
    pub async fn load(
        path: Path,
        id: &str,
        app_state: Arc<FlowLikeState>,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<Self> {
        let store = app_state
            .config
            .read()
            .await
            .stores
            .app_meta_store
            .clone()
            .ok_or_else(|| {
                tracing::error!("Project store not found while loading board: id={}", id);
                flow_like_types::anyhow!("Project store not found")
            })?
            .as_generic();

        let proto = Self::load_proto(store, &path, id, version).await?;
        Ok(Self::from_loaded_proto(proto, path, app_state).await)
    }

    /// Persist the floating draft. Returns the store's [`PutResult`] so a caller that keeps this
    /// board in memory can pin it to the exact object identity (`e_tag`) it just wrote — that is
    /// what lets its next read validate the cached copy instead of reloading it.
    pub async fn save(
        &self,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<PutResult> {
        let to = self.board_dir.child(format!("{}.board", self.id));
        let store = match store {
            Some(store) => store,
            None => {
                Self::meta_store(
                    self.app_state
                        .as_ref()
                        .expect("app_state should always be set"),
                )
                .await?
            }
        };

        let board = self.to_proto();
        compress_to_file(store, to, &board).await
    }

    // PAGE FUNCTIONS

    /// Where the pages of an arbitrary board on this app's store live.
    ///
    /// Template writers need this: by the time a board has been cloned into a template its `id` is
    /// already the template id, so the pages it is copying from can only be addressed by the id of
    /// the board they came from.
    fn board_pages_dir(&self, board_id: &str) -> Path {
        self.board_dir.child(format!("_{}", board_id))
    }

    fn pages_dir(&self) -> Path {
        self.board_pages_dir(&self.id)
    }

    fn page_path(&self, page_id: &str) -> Path {
        self.pages_dir().child(format!("{}.page", page_id))
    }

    fn versioned_pages_dir(&self, version: (u32, u32, u32)) -> Path {
        self.board_dir
            .child("versions")
            .child(self.id.clone())
            .child(format!("{}_{}_{}", version.0, version.1, version.2))
    }

    fn versioned_page_path(&self, version: (u32, u32, u32), page_id: &str) -> Path {
        self.versioned_pages_dir(version)
            .child(format!("{}.page", page_id))
    }

    /// `apps/{app_id}/_template_{template_id}` — where a template's page payloads live. Taken as
    /// a free-standing path so callers that never open the template board (`App::delete_template`,
    /// the fork's storage sweep) still get the layout from one place.
    pub fn template_pages_dir(board_dir: &Path, template_id: &str) -> Path {
        board_dir.child(format!("_template_{}", template_id))
    }

    fn template_page_path(&self, template_id: &str, page_id: &str) -> Path {
        Self::template_pages_dir(&self.board_dir, template_id).child(format!("{}.page", page_id))
    }

    /// Root of a template's version archive. Keyed on the **template** id, never the board the
    /// template was cut from: listing, versioned reads and template deletion all look here.
    fn versioned_template_dir(board_dir: &Path, template_id: &str) -> Path {
        board_dir
            .child("templates")
            .child("versions")
            .child(template_id)
    }

    fn versioned_template_path(
        board_dir: &Path,
        template_id: &str,
        version: (u32, u32, u32),
    ) -> Path {
        Self::versioned_template_dir(board_dir, template_id).child(format!(
            "{}_{}_{}.template",
            version.0, version.1, version.2
        ))
    }

    fn versioned_template_pages_dir(&self, template_id: &str, version: (u32, u32, u32)) -> Path {
        Self::versioned_template_dir(&self.board_dir, template_id)
            .child(format!("{}_{}_{}", version.0, version.1, version.2))
    }

    fn versioned_template_page_path(
        &self,
        template_id: &str,
        version: (u32, u32, u32),
        page_id: &str,
    ) -> Path {
        self.versioned_template_pages_dir(template_id, version)
            .child(format!("{}.page", page_id))
    }

    async fn get_store(
        &self,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<Arc<dyn ObjectStore>> {
        match store {
            Some(s) => Ok(s),
            None => self
                .app_state
                .as_ref()
                .ok_or_else(|| flow_like_types::anyhow!("app_state not set"))?
                .config
                .read()
                .await
                .stores
                .app_meta_store
                .clone()
                .ok_or_else(|| flow_like_types::anyhow!("Project store not found"))
                .map(|s| s.as_generic()),
        }
    }

    pub fn get_page_ids(&self) -> &[String] {
        &self.page_ids
    }

    pub fn page_count(&self) -> usize {
        self.page_ids.len()
    }

    pub async fn load_page(
        &self,
        page_id: &str,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<Page> {
        let store = self.get_store(store).await?;
        self.load_page_with_legacy_fallback(&store, page_id).await
    }

    /// Read every page the board lists.
    ///
    /// A board carries page ids, its pages are separate files, and the two can legitimately
    /// disagree: a board synced from a remote arrives before its payloads do. One unreadable
    /// page must therefore never cost the caller the rest of the board — the ids that failed
    /// are reported alongside the pages that loaded so callers can surface or repair them.
    /// Only a board-level storage failure is an error.
    pub async fn load_all_pages(
        &self,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<LoadedPages> {
        let store = self.get_store(store).await?;
        let mut loaded = LoadedPages {
            pages: Vec::with_capacity(self.page_ids.len()),
            unreadable: Vec::new(),
        };

        for page_id in &self.page_ids {
            match self.load_page_with_legacy_fallback(&store, page_id).await {
                Ok(page) => loaded.pages.push(page),
                Err(error) => loaded.unreadable.push(UnreadablePage {
                    page_id: page_id.clone(),
                    reason: error.to_string(),
                }),
            }
        }

        Ok(loaded)
    }

    /// Load a page from the canonical board-scoped binary-proto path,
    /// falling back to the legacy app-level JSON path written by the
    /// removed `App::save_page`. On a successful fallback the page is
    /// migrated in-place: the proto is written to the canonical path
    /// and the legacy file is removed, so subsequent reads short-circuit
    /// to the canonical lookup. Migration writes are best-effort —
    /// failures are logged and the loaded page is still returned, so a
    /// transient storage hiccup never turns a successful read into a
    /// hard error.
    async fn load_page_with_legacy_fallback(
        &self,
        store: &Arc<dyn ObjectStore>,
        page_id: &str,
    ) -> flow_like_types::Result<Page> {
        let canonical = self.page_path(page_id);
        match from_compressed::<proto::Page>(store.clone(), canonical.clone()).await {
            Ok(p) => Ok(p.into()),
            Err(canonical_err) => {
                let legacy = self.board_dir.child(format!("{}.page", page_id));
                match from_compressed_json::<Page>(store.clone(), legacy.clone()).await {
                    Ok(page) => {
                        let proto: proto::Page = page.clone().into();
                        if let Err(e) = compress_to_file(store.clone(), canonical, &proto).await {
                            tracing::warn!(
                                "page {} legacy→canonical migration write failed: {e}",
                                page_id
                            );
                        } else if let Err(e) = store.delete(&legacy).await {
                            tracing::warn!(
                                "page {} legacy→canonical migration: canonical written but legacy delete failed: {e}",
                                page_id
                            );
                        }
                        Ok(page)
                    }
                    Err(legacy_err) => Err(flow_like_types::anyhow!(
                        "page {} not found at canonical path ({}) or legacy app-level path ({})",
                        page_id,
                        canonical_err,
                        legacy_err
                    )),
                }
            }
        }
    }

    pub async fn load_versioned_page(
        &self,
        page_id: &str,
        version: (u32, u32, u32),
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<Page> {
        let store = self.get_store(store).await?;
        let path = self.versioned_page_path(version, page_id);
        let page_proto: proto::Page = from_compressed(store, path).await?;
        Ok(page_proto.into())
    }

    pub async fn save_page(
        &mut self,
        page: &Page,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<()> {
        let store = self.get_store(store).await?;
        let path = self.page_path(&page.id);
        let page_proto: proto::Page = page.clone().into();
        compress_to_file(store, path, &page_proto).await?;

        if !self.page_ids.contains(&page.id) {
            self.page_ids.push(page.id.clone());
        }
        self.mark_changed();
        Ok(())
    }

    pub async fn delete_page(
        &mut self,
        page_id: &str,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<()> {
        let store = self.get_store(store).await?;
        // Best-effort delete on the canonical path; missing files
        // (e.g. data only ever written via the legacy `App::save_page`)
        // shouldn't fail the call.
        let _ = store.delete(&self.page_path(page_id)).await;
        // Also evict any legacy app-level copy so a subsequent load
        // can't resurrect the page through the fallback reader.
        let legacy = self.board_dir.child(format!("{}.page", page_id));
        let _ = store.delete(&legacy).await;
        self.page_ids.retain(|id| id != page_id);
        self.mark_changed();
        Ok(())
    }

    pub fn get_required_element_ids(&self) -> std::collections::HashSet<String> {
        let mut required_ids = std::collections::HashSet::new();
        for node in self.nodes.values() {
            Self::extract_element_refs_from_node(node, &mut required_ids);
        }
        for layer in self.layers.values() {
            for node in layer.nodes.values() {
                Self::extract_element_refs_from_node(node, &mut required_ids);
            }
        }
        required_ids
    }

    fn extract_element_refs_from_node(
        node: &Node,
        required_ids: &mut std::collections::HashSet<String>,
    ) {
        for pin in node.pins.values() {
            if pin.name == "element_ref"
                && let Some(default_value) = &pin.default_value
                && let Ok(value) =
                    flow_like_types::json::from_slice::<flow_like_types::Value>(default_value)
                && let Some(id) = value.as_str()
                && !id.is_empty()
            {
                required_ids.insert(id.to_string());
            }
        }
    }

    pub async fn get_execution_elements(
        &self,
        page_id: &str,
        wildcard: bool,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<std::collections::HashMap<String, flow_like_types::Value>> {
        let mut elements = std::collections::HashMap::new();

        let page = match self.load_page(page_id, store).await {
            Ok(p) => p,
            Err(_) => return Ok(elements),
        };

        if wildcard {
            for component in &page.components {
                let full_id = format!("{}/{}", page_id, component.id);
                if let Ok(value) = flow_like_types::json::to_value(component) {
                    elements.insert(full_id, value);
                }
            }
        } else {
            let required_ids = self.get_required_element_ids();

            // A ref prefixed with another page's id still ships this page's component of the
            // same name, so flows written against one page resolve on every page that has
            // the element.
            for component in &page.components {
                let suffix = format!("/{}", component.id);
                let required = required_ids
                    .iter()
                    .any(|id| *id == component.id || id.ends_with(&suffix));
                if required && let Ok(value) = flow_like_types::json::to_value(component) {
                    elements.insert(format!("{}/{}", page_id, component.id), value);
                }
            }
        }

        Ok(elements)
    }

    // TEMPLATE FUNCTIONS

    /// Copy the listed pages verbatim from one page directory to another.
    ///
    /// A page a board lists can legitimately have no file behind it — a board synced from a remote
    /// arrives before its payloads do, and a template that only ever existed as a record has none
    /// at all. The record these pages belong to has already been written by the time this runs, so
    /// aborting on the first miss would throw away a template save the user believes happened for
    /// the sake of one page. Misses are logged and skipped, the same trade `load_all_pages` makes.
    /// A failed *write* still propagates: that is the destination store breaking, not missing data.
    async fn copy_pages_between(
        store: &Arc<dyn ObjectStore>,
        page_ids: &[String],
        src_dir: &Path,
        dst_dir: &Path,
    ) -> flow_like_types::Result<()> {
        for page_id in page_ids {
            let src_path = src_dir.child(format!("{}.page", page_id));
            let page_proto: proto::Page =
                match from_compressed(store.clone(), src_path.clone()).await {
                    Ok(page) => page,
                    Err(error) => {
                        tracing::warn!(
                            "skipping page {}: reading {} failed: {}",
                            page_id,
                            src_path,
                            error
                        );
                        continue;
                    }
                };
            let dst_path = dst_dir.child(format!("{}.page", page_id));
            compress_to_file(store.clone(), dst_path, &page_proto).await?;
        }
        Ok(())
    }

    /// Persist this board as the template `self.id`.
    ///
    /// `source_board_id` names the board whose page files back `self.page_ids`. It has to be
    /// passed in because `self.id` is already the *template* id by the time a caller gets here,
    /// so the pages are no longer reachable from this board's own paths. `None` marks a
    /// record-only write: a template cached from a remote carries page ids but no payloads on
    /// this store, and there is nothing to copy.
    pub async fn save_as_template(
        &self,
        source_board_id: Option<&str>,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<()> {
        let to = self.board_dir.child(format!("{}.template", self.id));
        let store = self.get_store(store).await?;

        let mut template = self.clone();
        template.clear_internal_refs();
        let board = template.to_proto();
        compress_to_file(store.clone(), to, &board).await?;

        if let Some(source_board_id) = source_board_id {
            let src_dir = self.board_pages_dir(source_board_id);
            let dst_dir = Self::template_pages_dir(&self.board_dir, &self.id);
            Self::copy_pages_between(&store, &self.page_ids, &src_dir, &dst_dir).await?;
        }

        Ok(())
    }

    /// Overwrite an already-published template version in place. `source_board_id` carries the
    /// same meaning as in [`Self::save_as_template`].
    pub async fn overwrite_template_version(
        &mut self,
        version: (u32, u32, u32),
        source_board_id: Option<&str>,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<()> {
        let store = self.get_store(store).await?;

        let to = Self::versioned_template_path(&self.board_dir, &self.id, version);

        let mut template = self.clone();
        template.clear_internal_refs();
        let board = template.to_proto();
        compress_to_file(store.clone(), to, &board).await?;

        if let Some(source_board_id) = source_board_id {
            let src_dir = self.board_pages_dir(source_board_id);
            let dst_dir = self.versioned_template_pages_dir(&self.id, version);
            Self::copy_pages_between(&store, &self.page_ids, &src_dir, &dst_dir).await?;
        }

        Ok(())
    }

    pub async fn create_template(
        &mut self,
        template_id: String,
        version_type: VersionType,
        old_template: Option<Board>,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<(u32, u32, u32)> {
        let store = self.get_store(store).await?;

        let version = old_template
            .as_ref()
            .map(|t| t.version)
            .unwrap_or((0, 0, 0));

        let mut new_version = (0, 0, 0);

        // The archive of the outgoing version is keyed on the template id, matching every reader:
        // `load_template`, `get_template_versions` and `App::delete_template`. Archives written
        // before this fix live under the source board's id instead and are unreachable — they were
        // already invisible to all three, so there is nothing to migrate, only orphans to ignore.
        if let Some(old_template) = &old_template {
            let to = Self::versioned_template_path(&self.board_dir, &template_id, version);
            let mut old_template = old_template.clone();
            old_template.clear_internal_refs();
            compress_to_file(store.clone(), to, &old_template.to_proto()).await?;

            let src_dir = Self::template_pages_dir(&old_template.board_dir, &old_template.id);
            let dst_dir = self.versioned_template_pages_dir(&template_id, version);
            Self::copy_pages_between(&store, &old_template.page_ids, &src_dir, &dst_dir).await?;

            new_version = match version_type {
                VersionType::Major => (version.0 + 1, 0, 0),
                VersionType::Minor => (version.0, version.1 + 1, 0),
                VersionType::Patch => (version.0, version.1, version.2 + 1),
            }
        }

        let source_board_id = self.id.clone();
        let mut template = self.clone();
        template.id = template_id;
        template.version = new_version;

        for variable in template.variables.values_mut() {
            if variable.secret {
                variable.default_value = None;
            }
        }

        template.mark_changed();
        template
            .save_as_template(Some(source_board_id.as_str()), Some(store))
            .await?;
        Ok(new_version)
    }

    pub async fn load_template(
        path: Path,
        template_id: &str,
        app_state: Arc<FlowLikeState>,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<Self> {
        let store = app_state
            .config
            .read()
            .await
            .stores
            .app_meta_store
            .clone()
            .ok_or(flow_like_types::anyhow!("Project store not found"))?
            .as_generic();

        let board_dir = path.clone();
        let path = match version {
            Some(version) => Self::versioned_template_path(&board_dir, template_id, version),
            None => path.child(format!("{}.template", template_id)),
        };

        let board: flow_like_types::proto::Board = from_compressed(store, path).await?;
        let mut board = Board::from_proto(board);
        board.board_dir = board_dir;
        board.app_state = Some(app_state.clone());
        board.logic_nodes = HashMap::new();

        // Sync node schemas on load to handle version migrations
        board.node_updates(app_state).await;
        board.cleanup();

        Ok(board)
    }

    fn template_page_source(
        &self,
        template_id: &str,
        version: Option<(u32, u32, u32)>,
        page_id: &str,
    ) -> Path {
        match version {
            Some(version) => self.versioned_template_page_path(template_id, version, page_id),
            None => self.template_page_path(template_id, page_id),
        }
    }

    async fn load_template_page_proto(
        &self,
        template_id: &str,
        page_id: &str,
        version: Option<(u32, u32, u32)>,
        store: &Arc<dyn ObjectStore>,
    ) -> flow_like_types::Result<proto::Page> {
        let path = self.template_page_source(template_id, version, page_id);
        from_compressed(store.clone(), path).await
    }

    pub async fn load_template_page(
        &self,
        template_id: &str,
        page_id: &str,
        version: Option<(u32, u32, u32)>,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<Page> {
        let store = self.get_store(store).await?;
        Ok(self
            .load_template_page_proto(template_id, page_id, version, &store)
            .await?
            .into())
    }

    /// Read every page this template lists. A page whose payload is missing is skipped with a
    /// warning rather than failing the read, so one absent file never hides the rest of the
    /// template — the same contract [`Self::load_all_pages`] holds for a board.
    pub async fn load_all_template_pages(
        &self,
        template_id: &str,
        version: Option<(u32, u32, u32)>,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<HashMap<String, Page>> {
        let store = self.get_store(store).await?;

        let mut pages = HashMap::with_capacity(self.page_ids.len());
        for page_id in &self.page_ids {
            match self
                .load_template_page_proto(template_id, page_id, version, &store)
                .await
            {
                Ok(page) => {
                    pages.insert(page_id.clone(), page.into());
                }
                Err(error) => tracing::warn!(
                    "skipping page {} of template {}: {}",
                    page_id,
                    template_id,
                    error
                ),
            }
        }
        Ok(pages)
    }

    /// Copy `template`'s pages onto this board under freshly minted ids.
    ///
    /// `Page.id` is a global primary key, so an instantiated page can never keep the template's
    /// id — the copy would collide with the original the moment either is persisted. Once the ids
    /// move, everything naming them has to move too: `node_translation` is the source→minted map
    /// the accompanying [`commands::nodes::copy_paste::CopyPasteCommand`] produced for the graph,
    /// and `app_translation` names the source and destination apps when the template came from a
    /// different one.
    ///
    /// A page the template lists but has no payload for is skipped with a warning: a template that
    /// crossed a serialization boundary carries ids without files, and one missing page must not
    /// cost the caller the whole board.
    pub async fn instantiate_template_pages(
        &mut self,
        template: &Board,
        node_translation: &HashMap<String, String>,
        app_translation: Option<(&str, &str)>,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<Vec<Page>> {
        if template.page_ids.is_empty() {
            return Ok(Vec::new());
        }

        let store = self.get_store(store).await?;
        let page_translation = template
            .page_ids
            .iter()
            .map(|page_id| (page_id.clone(), create_id()))
            .collect::<HashMap<String, String>>();

        let board_id = self.id.clone();
        let template_id = template.id.clone();
        let mut instantiated = Vec::with_capacity(template.page_ids.len());

        for page_id in &template.page_ids {
            let mut page_proto = match template
                .load_template_page_proto(&template_id, page_id, None, &store)
                .await
            {
                Ok(page) => page,
                Err(error) => {
                    tracing::warn!(
                        "skipping page {} of template {}: {}",
                        page_id,
                        template_id,
                        error
                    );
                    continue;
                }
            };
            let Some(new_page_id) = page_translation.get(page_id) else {
                continue;
            };

            let mut translate = |kind: IdRef, id: &str| match kind {
                IdRef::Node => node_translation.get(id).cloned(),
                IdRef::Page => page_translation.get(id).cloned(),
                IdRef::Board => (id == template_id.as_str()).then(|| board_id.clone()),
                IdRef::App => {
                    app_translation.and_then(|(from, to)| (id == from).then(|| to.to_string()))
                }
                IdRef::Widget | IdRef::Event => None,
            };
            // Payloads that decode to a bare literal — a prop default, a customization value —
            // carry no field name to key off, so every id this instantiation minted is matched
            // directly instead.
            let mut translate_literal = |id: &str| {
                node_translation
                    .get(id)
                    .or_else(|| page_translation.get(id))
                    .cloned()
                    .or_else(|| (id == template_id.as_str()).then(|| board_id.clone()))
                    .or_else(|| {
                        app_translation.and_then(|(from, to)| (id == from).then(|| to.to_string()))
                    })
                    // An element reference is composite — `{page_id}/{component_id}` — so
                    // whole-string matching never sees it. The fork translates these the same
                    // way; the two passes have to agree or a copy behaves differently
                    // depending on which one made it.
                    .or_else(|| {
                        let (page_id, component_id) = id.split_once('/')?;
                        if component_id.is_empty() {
                            return None;
                        }
                        Some(format!(
                            "{}/{}",
                            page_translation.get(page_id)?,
                            component_id
                        ))
                    })
            };

            Self::remap_instantiated_page(
                &mut page_proto,
                new_page_id,
                &board_id,
                &mut translate,
                &mut translate_literal,
            );

            // Written as a proto instead of through `save_page`: the round trip through the
            // in-memory `Page` drops `PageContent`'s grid placement and region and
            // `SurfaceComponent::event_relevant`, and a copy has no business losing them.
            let dst_path = self.page_path(new_page_id);
            compress_to_file(store.clone(), dst_path, &page_proto).await?;

            if !self.page_ids.contains(new_page_id) {
                self.page_ids.push(new_page_id.clone());
            }
            instantiated.push(Page::from(page_proto));
        }

        self.mark_changed();
        Ok(instantiated)
    }

    /// Move one page payload into this board's id space.
    ///
    /// Coverage lives in [`crate::a2ui::page_remap`], shared with the fork: both operations move a
    /// page across an id boundary and have to rewrite the same references, and keeping two
    /// inventories in step is what failed last time. This function owns only the policy — the
    /// page's new identity and which id maps to what.
    fn remap_instantiated_page(
        page: &mut proto::Page,
        new_page_id: &str,
        board_id: &str,
        translate: &mut dyn FnMut(IdRef, &str) -> Option<String>,
        translate_literal: &mut dyn FnMut(&str) -> Option<String>,
    ) {
        page.id = new_page_id.to_string();
        page.board_id = Some(board_id.to_string());

        let mut translators = IdTranslators {
            by_field: translate,
            by_literal: translate_literal,
        };
        let unrewritten = remap_page_refs(page, &mut translators);
        if !unrewritten.is_empty() {
            // The page is still written: a component whose JSON will not parse is already broken,
            // and dropping the whole page would take the working ones with it.
            tracing::warn!(
                page_id = %new_page_id,
                board_id = %board_id,
                "instantiated page kept {} payload(s) the template copy could not rewrite: {}",
                unrewritten.len(),
                unrewritten.join("; ")
            );
        }
    }

    pub async fn get_template_versions(
        &self,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<Vec<(u32, u32, u32)>> {
        let versions_dir = Self::versioned_template_dir(&self.board_dir, &self.id);

        let store = self.get_store(store).await?;

        let mut versions = store.list(Some(&versions_dir));
        let mut version_list = Vec::new();

        while let Some(Ok(meta)) = versions.next().await {
            let file_name = match meta.location.filename() {
                Some(name) => name,
                None => continue,
            };
            if !file_name.ends_with(".template") {
                continue;
            }
            let version = file_name.strip_suffix(".template").unwrap_or(file_name);
            if version == "latest" {
                continue;
            }
            let version = version.strip_prefix("v").unwrap_or(version);
            let version = version.split("_").collect::<Vec<&str>>();

            if version.len() < 3 {
                continue;
            }

            let version = (
                version[0].parse::<u32>().unwrap_or(0),
                version[1].parse::<u32>().unwrap_or(0),
                version[2].parse::<u32>().unwrap_or(0),
            );

            version_list.push(version);
        }
        Ok(version_list)
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub enum CommentType {
    Text,
    Image,
    Video,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Comment {
    pub id: String,
    pub author: Option<String>,
    pub content: String,
    pub comment_type: CommentType,
    pub timestamp: SystemTime,
    pub coordinates: (f32, f32, f32),
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub layer: Option<String>,
    pub color: Option<String>,
    pub z_index: Option<i32>,
    pub hash: Option<u64>,
    pub is_locked: Option<bool>,
    /// Soft reference to a board node this comment is attached to (e.g. the statement a
    /// FlowScript thread anchors on). Presentation metadata only — dangling references are
    /// legal (the node may be deleted later); consumers must treat a missing node as unanchored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
}

impl Comment {
    pub fn hash(&mut self) {
        let mut hasher = HighwayHasher::new(highway::Key([
            0x0123456789abcdfe,
            0xfedcba9876543210,
            0x0011223344556677,
            0x8899aabbccddeeff,
        ]));

        hasher.append(self.id.as_bytes());
        hasher.append(self.content.as_bytes());
        hasher.append(format!("{:?}", self.comment_type).as_bytes());

        if let Some(author) = &self.author {
            hasher.append(author.as_bytes());
        }

        hasher.append(&self.coordinates.0.to_le_bytes());
        hasher.append(&self.coordinates.1.to_le_bytes());
        hasher.append(&self.coordinates.2.to_le_bytes());

        if let Some(width) = self.width {
            hasher.append(&width.to_le_bytes());
        }

        if let Some(height) = self.height {
            hasher.append(&height.to_le_bytes());
        }

        if let Some(layer) = &self.layer {
            hasher.append(layer.as_bytes());
        }

        if let Some(color) = &self.color {
            hasher.append(color.as_bytes());
        }

        if let Some(z_index) = self.z_index {
            hasher.append(&z_index.to_le_bytes());
        }

        if let Some(is_locked) = self.is_locked {
            hasher.append(&[is_locked as u8]);
        }

        if let Some(node_id) = &self.node_id {
            hasher.append(node_id.as_bytes());
        }

        self.hash = Some(hasher.finalize64());
    }
}

#[cfg(test)]
mod tests {
    use crate::{state::FlowLikeConfig, utils::http::HTTPClient};
    use flow_like_storage::{
        files::store::FlowLikeStore,
        object_store::{self, path::Path},
    };
    use flow_like_types::{FromProto, ToProto};
    use flow_like_types::{Message, tokio};
    use std::sync::Arc;

    async fn flow_state() -> Arc<crate::state::FlowLikeState> {
        let mut config: FlowLikeConfig = FlowLikeConfig::new();
        config.register_app_meta_store(FlowLikeStore::Other(Arc::new(
            object_store::memory::InMemory::new(),
        )));
        let http_client = HTTPClient::new_without_refetch();
        let flow_like_state = crate::state::FlowLikeState::new(config, http_client);
        Arc::new(flow_like_state)
    }

    struct RefreshDefinitionLogic {
        label: &'static str,
    }

    #[flow_like_types::async_trait]
    impl crate::flow::node::NodeLogic for RefreshDefinitionLogic {
        fn get_node(&self) -> crate::flow::node::Node {
            crate::flow::node::Node::new("refresh_definition_test", self.label, "", "test")
        }

        async fn run(
            &self,
            _: &mut crate::flow::execution::context::ExecutionContext,
        ) -> flow_like_types::Result<()> {
            Ok(())
        }

        async fn on_update(&self, node: &mut crate::flow::node::Node, _: &super::Board) {
            node.friendly_name = self.label.to_string();
        }
    }

    #[tokio::test]
    async fn refresh_node_definitions_uses_current_registry_without_marking_board_dirty() {
        use crate::flow::node::NodeLogic;

        let state = flow_state().await;
        let old_logic: Arc<dyn NodeLogic> = Arc::new(RefreshDefinitionLogic { label: "Old logic" });
        let new_logic: Arc<dyn NodeLogic> = Arc::new(RefreshDefinitionLogic { label: "New logic" });

        let mut board = super::Board::new(None, Path::from("boards"), state.clone());
        let node = old_logic.get_node();
        let node_id = node.id.clone();
        board.nodes.insert(node_id.clone(), node);
        board
            .logic_nodes
            .insert("refresh_definition_test".to_string(), old_logic);

        state.node_registry().write().await.push_node(new_logic);
        let updated_at = board.updated_at;
        board.hash = Some(0xdead_beef);

        board.refresh_node_definitions(state).await;

        assert_eq!(board.nodes[&node_id].friendly_name, "New logic");
        assert_eq!(board.updated_at, updated_at);
        assert_eq!(board.hash, Some(0xdead_beef));
    }

    #[tokio::test]
    async fn pin_index_answers_exactly_like_the_scan() {
        use crate::flow::node::Node;
        use crate::flow::variable::VariableType;

        let state = flow_state().await;
        let mut board = super::Board::new(None, Path::from("boards"), state);

        let mut node = Node::new("indexed_node", "Indexed Node", "", "test");
        node.add_input_pin("in", "In", "", VariableType::String);
        node.add_output_pin("out", "Out", "", VariableType::String);
        let mut pin_ids: Vec<String> = node.pins.keys().cloned().collect();
        board.nodes.insert(node.id.clone(), node);

        let mut interface = Node::new("layer_interface", "Layer Interface", "", "test");
        interface.add_input_pin("layer_in", "Layer In", "", VariableType::String);
        pin_ids.extend(interface.pins.keys().cloned());

        let mut nested = Node::new("nested_node", "Nested Node", "", "test");
        nested.add_output_pin("nested_out", "Nested Out", "", VariableType::String);
        pin_ids.extend(nested.pins.keys().cloned());

        let mut layer = super::Layer::new(
            "layer-1".to_string(),
            "Layer".to_string(),
            super::LayerType::Function,
        );
        layer.pins = interface.pins.clone();
        layer.nodes.insert(nested.id.clone(), nested);
        board.layers.insert(layer.id.clone(), layer);

        pin_ids.push("pin-that-does-not-exist".to_string());

        let scanned: Vec<Option<String>> = pin_ids
            .iter()
            .map(|id| board.get_pin_by_id(id).map(|pin| pin.id.clone()))
            .collect();
        assert_eq!(
            scanned.iter().filter(|found| found.is_some()).count(),
            pin_ids.len() - 1
        );

        board.pin_index = Some(board.build_pin_index());
        let indexed: Vec<Option<String>> = pin_ids
            .iter()
            .map(|id| board.get_pin_by_id(id).map(|pin| pin.id.clone()))
            .collect();

        assert_eq!(scanned, indexed);
    }

    /// Mimics the `match_type` family: adopts the data type of whatever feeds its input, which is
    /// how a retyped pin travels along a chain of wires.
    struct TypeMirrorLogic;

    #[flow_like_types::async_trait]
    impl crate::flow::node::NodeLogic for TypeMirrorLogic {
        fn get_node(&self) -> crate::flow::node::Node {
            use crate::flow::variable::VariableType;
            let mut node =
                crate::flow::node::Node::new("type_mirror_test", "Type Mirror", "", "test");
            node.add_input_pin("in", "In", "", VariableType::Generic);
            node.add_output_pin("out", "Out", "", VariableType::Generic);
            node
        }

        async fn run(
            &self,
            _: &mut crate::flow::execution::context::ExecutionContext,
        ) -> flow_like_types::Result<()> {
            Ok(())
        }

        async fn on_update(&self, node: &mut crate::flow::node::Node, board: &super::Board) {
            let upstream = node
                .get_pin_by_name("in")
                .and_then(|pin| pin.depends_on.iter().next().cloned())
                .and_then(|pin_id| board.get_pin_by_id(&pin_id))
                .map(|pin| pin.data_type.clone());
            let Some(data_type) = upstream else {
                return;
            };
            for pin in node.pins.values_mut() {
                pin.data_type = data_type.clone();
            }
        }
    }

    /// A scoped sweep must leave exactly the board a full sweep would.
    ///
    /// This is the check that makes narrowing the sweep defensible: if a propagation channel is
    /// ever missed, a node keeps a stale derivation and the two boards diverge here. `node.error`
    /// is compared separately because `Node::hash` does not cover it, so a divergence in validation
    /// messages alone would otherwise pass unnoticed.
    async fn assert_sweeps_agree(
        board: &super::Board,
        state: Arc<crate::state::FlowLikeState>,
        commands: Vec<super::GenericCommand>,
    ) {
        use crate::flow::board::dirty::Touched;

        let mut full = board.clone();
        let mut scoped = board.clone();
        for target in [&mut full, &mut scoped] {
            for command in commands.clone().iter_mut() {
                command.execute(target, state.clone()).await.expect("apply");
            }
        }

        let mut touched = Touched::default();
        for command in &commands {
            command.touched(&mut touched);
        }

        full.node_updates_scoped(state.clone(), None).await;
        full.cleanup();
        full.mark_changed();

        scoped
            .node_updates_scoped(state.clone(), Some(&touched))
            .await;
        scoped.cleanup();
        scoped.mark_changed();

        for (node_id, expected) in &full.nodes {
            let actual = scoped.nodes.get(node_id).expect("node present after sweep");
            assert_eq!(
                expected.hash, actual.hash,
                "node {node_id} ({}) settled differently",
                expected.name
            );
            assert_eq!(
                expected.error, actual.error,
                "node {node_id} ({}) reports a different error",
                expected.name
            );
        }
        assert_eq!(full.nodes.len(), scoped.nodes.len());
        assert_eq!(full.hash, scoped.hash, "board hashes diverged");
    }

    /// A retyped pin has to travel the whole chain, not just to the first neighbour.
    #[tokio::test]
    async fn dirty_sweep_matches_full_sweep_along_a_wire_chain() {
        use crate::flow::node::{Node, NodeLogic};
        use crate::flow::variable::VariableType;

        let state = flow_state().await;
        let logic: Arc<dyn NodeLogic> = Arc::new(TypeMirrorLogic);
        state.node_registry().write().await.push_node(logic.clone());

        let mut board = super::Board::new(None, Path::from("boards"), state.clone());

        // source -> a -> b -> c, so a change at the head has three hops to travel.
        let mut source = Node::new("type_mirror_test", "Source", "", "test");
        source.add_output_pin("out", "Out", "", VariableType::String);
        let mut chain: Vec<Node> = vec![source];
        for index in 0..3 {
            let mut link = logic.get_node();
            link.friendly_name = format!("Link {index}");
            chain.push(link);
        }

        for window in 0..chain.len() - 1 {
            let out_pin = chain[window]
                .get_pin_by_name("out")
                .expect("output pin")
                .id
                .clone();
            let in_pin = chain[window + 1]
                .get_pin_by_name("in")
                .expect("input pin")
                .id
                .clone();
            chain[window]
                .get_pin_mut_by_name("out")
                .expect("output pin")
                .connected_to
                .insert(in_pin.clone());
            chain[window + 1]
                .get_pin_mut_by_name("in")
                .expect("input pin")
                .depends_on
                .insert(out_pin);
        }

        let head_id = chain[0].id.clone();
        for node in chain {
            board.nodes.insert(node.id.clone(), node);
        }
        board.node_updates(state.clone()).await;
        board.cleanup();
        board.mark_changed();

        // Retype the head. Every link downstream has to adopt it.
        let mut head = board.nodes.get(&head_id).expect("head node").clone();
        head.get_pin_mut_by_name("out")
            .expect("output pin")
            .data_type = VariableType::Integer;
        let command = super::GenericCommand::UpdateNode(
            crate::flow::board::commands::nodes::update_node::UpdateNodeCommand {
                node: head,
                old_node: board.nodes.get(&head_id).cloned(),
            },
        );

        assert_sweeps_agree(&board, state, vec![command]).await;
    }

    /// Where an apply actually spends its time on a large board, with a trivial `on_update` so the
    /// numbers are the fixed per-node overhead rather than any one node type's work. Ignored: it
    /// reports a measurement rather than asserting a threshold. Run with:
    ///   cargo test -p flow-like --lib apply_phase_breakdown -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn apply_phase_breakdown() {
        use crate::flow::node::NodeLogic;
        use crate::flow::variable::VariableType;
        use std::time::Instant;

        const NODES: usize = 1097;

        let state = flow_state().await;
        let logic: Arc<dyn NodeLogic> = Arc::new(RefreshDefinitionLogic { label: "Scaling" });
        state.node_registry().write().await.push_node(logic.clone());

        let mut board = super::Board::new(None, Path::from("boards"), state.clone());
        for _ in 0..NODES {
            let mut node = logic.get_node();
            node.add_input_pin("in", "In", "", VariableType::String);
            node.add_output_pin("out", "Out", "", VariableType::String);
            board.nodes.insert(node.id.clone(), node);
        }

        let started = Instant::now();
        board.node_updates(state.clone()).await;
        let node_updates = started.elapsed();

        let started = Instant::now();
        board.cleanup();
        let cleanup = started.elapsed();

        let started = Instant::now();
        board.mark_changed();
        let mark_changed = started.elapsed();

        let started = Instant::now();
        let _nodes_before = board.nodes.clone();
        let clone_nodes = started.elapsed();

        println!(
            "{NODES} nodes / {} pins: node_updates {node_updates:?}, cleanup {cleanup:?}, mark_changed {mark_changed:?}, nodes.clone() {clone_nodes:?}",
            board
                .nodes
                .values()
                .map(|node| node.pins.len())
                .sum::<usize>(),
        );
    }

    /// How the `node_updates` pin lookup scales with board size. Ignored: it reports a measurement
    /// rather than asserting a threshold. Run with:
    ///   cargo test -p flow-like --lib pin_lookup_scaling -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn pin_lookup_scaling() {
        use crate::flow::node::Node;
        use crate::flow::variable::VariableType;
        use std::time::Instant;

        const NODES: usize = 1097;

        let state = flow_state().await;
        let mut board = super::Board::new(None, Path::from("boards"), state);

        let mut pin_ids: Vec<String> = Vec::new();
        for index in 0..NODES {
            let mut node = Node::new("scaling_node", &format!("Node {index}"), "", "test");
            node.add_input_pin("in", "In", "", VariableType::String);
            node.add_output_pin("out", "Out", "", VariableType::String);
            pin_ids.extend(node.pins.keys().cloned());
            board.nodes.insert(node.id.clone(), node);
        }

        // Spread the sample over the map so neither mode wins on locality.
        let lookups: Vec<&String> = pin_ids.iter().step_by(3).collect();

        let started = Instant::now();
        let scanned = lookups
            .iter()
            .filter(|pin_id| board.get_pin_by_id(pin_id).is_some())
            .count();
        let scan = started.elapsed();

        board.pin_index = Some(board.build_pin_index());
        let started = Instant::now();
        let indexed_hits = lookups
            .iter()
            .filter(|pin_id| board.get_pin_by_id(pin_id).is_some())
            .count();
        let indexed = started.elapsed();

        assert_eq!(scanned, indexed_hits);
        println!(
            "{NODES} nodes / {} pins, {} lookups: scan {scan:?} ({:?} each), indexed {indexed:?} ({:?} each)",
            pin_ids.len(),
            lookups.len(),
            scan / lookups.len() as u32,
            indexed / lookups.len() as u32,
        );
    }

    /// The index is only exact while `node_updates` owns it; leaking it would let a later mutation
    /// answer `get_pin_by_id` from stale entries.
    #[tokio::test]
    async fn node_updates_clears_the_pin_index() {
        let state = flow_state().await;
        let mut board = super::Board::new(None, Path::from("boards"), state.clone());
        board.node_updates(state).await;
        assert!(board.pin_index.is_none());
    }

    #[tokio::test]
    async fn serialize_board() {
        let state = flow_state().await;
        let base_dir = Path::from("boards");
        let board = super::Board::new(None, base_dir, state);

        let mut buf = Vec::new();
        board.to_proto().encode(&mut buf).unwrap();
        let deser_board =
            super::Board::from_proto(flow_like_types::proto::Board::decode(&buf[..]).unwrap());

        assert_eq!(board.id, deser_board.id);
    }

    #[tokio::test]
    async fn internal_refs_roundtrip_in_proto_but_not_board_json_or_semantic_hash() {
        let state = flow_state().await;
        let mut board = super::Board::new(None, Path::from("boards"), state);
        let original_hash = board.content_hash();
        let key = format!("{}test-receipt", super::INTERNAL_BOARD_REF_PREFIX);
        board
            .insert_internal_ref(key.clone(), "opaque")
            .expect("reserved key");

        assert_eq!(board.content_hash(), original_hash);
        let json = serde_json::to_value(&board).expect("board JSON");
        assert!(json.get("internal_refs").is_none());
        assert!(
            json.get("refs")
                .and_then(serde_json::Value::as_object)
                .is_some_and(|refs| !refs.contains_key(&key))
        );

        let proto = board.to_proto();
        assert_eq!(
            proto.internal_refs.get(&key).map(String::as_str),
            Some("opaque")
        );
        let restored = super::Board::from_proto(proto);
        assert_eq!(restored.internal_ref(&key), Some("opaque"));
        assert!(!restored.refs.contains_key(&key));
    }

    #[tokio::test]
    async fn legacy_prefixed_refs_migrate_to_internal_proto_storage() {
        let state = flow_state().await;
        let board = super::Board::new(None, Path::from("boards"), state);
        let mut proto = board.to_proto();
        let key = format!("{}legacy", super::INTERNAL_BOARD_REF_PREFIX);
        proto.refs.insert(key.clone(), "receipt".to_string());

        let restored = super::Board::from_proto(proto);
        assert_eq!(restored.internal_ref(&key), Some("receipt"));
        assert!(!restored.refs.contains_key(&key));
        let migrated = restored.to_proto();
        assert!(!migrated.refs.contains_key(&key));
        assert_eq!(
            migrated.internal_refs.get(&key).map(String::as_str),
            Some("receipt")
        );
    }

    #[tokio::test]
    async fn governed_action_parameter_schema_fails_closed_when_authored_schema_is_malformed() {
        use crate::flow::{node::Node, variable::VariableType};

        let state = flow_state().await;
        let mut board = super::Board::new(None, Path::from("boards"), state);
        assert!(board.action_parameter_schema("missing-start").is_err());
        let mut start = Node::new("start", "Start", "", "events");
        let start_id = start.id.clone();
        let parameters_pin = start
            .add_output_pin("parameters", "Parameters", "", VariableType::Struct)
            .id
            .clone();
        start.pins.get_mut(&parameters_pin).unwrap().schema = Some("not-json".to_string());
        board.nodes.insert(start_id.clone(), start);

        assert!(board.action_parameter_schema(&start_id).is_err());
        board
            .nodes
            .get_mut(&start_id)
            .unwrap()
            .pins
            .get_mut(&parameters_pin)
            .unwrap()
            .schema = Some("[]".to_string());
        assert!(board.action_parameter_schema(&start_id).is_err());
    }

    #[tokio::test]
    async fn listing_pages_survives_one_unreadable_payload() {
        use crate::a2ui::widget::Page;

        let state = flow_state().await;
        let mut board = super::Board::new(None, Path::from("boards"), state);
        board
            .save_page(&Page::new("page-1", "First", "/"), None)
            .await
            .unwrap();
        board
            .save_page(&Page::new("page-2", "Second", "/second"), None)
            .await
            .unwrap();
        // A board synced from a remote knows page ids whose payloads never arrived.
        board.page_ids.push("page-missing".to_string());

        let loaded = board.load_all_pages(None).await.unwrap();

        assert_eq!(
            loaded
                .pages
                .iter()
                .map(|page| page.id.as_str())
                .collect::<Vec<_>>(),
            vec!["page-1", "page-2"],
            "an unreadable page must not cost the caller the rest of the board"
        );
        assert_eq!(loaded.unreadable.len(), 1);
        assert_eq!(loaded.unreadable[0].page_id, "page-missing");
        assert!(!loaded.unreadable[0].reason.is_empty());
    }

    #[tokio::test]
    async fn immutable_snapshot_rejects_overwrite_and_publishes_fresh_patch() {
        use crate::a2ui::widget::Page;

        let state = flow_state().await;
        let base_dir = Path::from("boards");
        let mut board = super::Board::new(None, base_dir.clone(), state.clone());
        let original_version = board.version;
        let original_name = board.name.clone();
        let mut page = Page::new("page-1", "Original page", "/");
        board.save_page(&page, None).await.unwrap();

        board
            .snapshot_at_version(original_version, None)
            .await
            .unwrap();
        board
            .snapshot_at_version(original_version, None)
            .await
            .expect("an identical retry must be idempotent");
        assert!(
            board
                .snapshot_matches_current(original_version, None)
                .await
                .unwrap()
        );
        page.name = "Edited page".to_string();
        board.save_page(&page, None).await.unwrap();
        assert!(
            !board
                .snapshot_matches_current(original_version, None)
                .await
                .unwrap(),
            "page-only edits must make the snapshot stale"
        );
        board.name = "Edited draft".to_string();
        board.mark_changed();

        assert!(
            board
                .snapshot_at_version(original_version, None)
                .await
                .is_err(),
            "an existing version must never be overwritten"
        );
        let original = super::Board::load(
            base_dir.clone(),
            &board.id,
            state.clone(),
            Some(original_version),
        )
        .await
        .unwrap();
        assert_eq!(original.name, original_name);
        let original_page = board
            .load_versioned_page("page-1", original_version, None)
            .await
            .unwrap();
        assert_eq!(original_page.name, "Original page");

        let fresh = board.snapshot_at_fresh_patch_version(None).await.unwrap();
        assert_eq!(
            fresh,
            (
                original_version.0,
                original_version.1,
                original_version.2 + 1
            )
        );
        let published = super::Board::load(base_dir, &board.id, state, Some(fresh))
            .await
            .unwrap();
        assert_eq!(published.name, "Edited draft");
        let published_page = board
            .load_versioned_page("page-1", fresh, None)
            .await
            .unwrap();
        assert_eq!(published_page.name, "Edited page");
    }

    #[tokio::test]
    async fn prepared_snapshot_advances_only_after_commit_and_never_overwrites_newer_draft() {
        let state = flow_state().await;
        let base_dir = Path::from("boards");
        let mut board = super::Board::new(None, base_dir.clone(), state.clone());
        let original_version = board.version;
        board.save(None).await.unwrap();
        board
            .snapshot_at_version(original_version, None)
            .await
            .unwrap();

        board.name = "Prepared action implementation".to_string();
        board.mark_changed();
        board.save(None).await.unwrap();
        let prepared = board
            .prepare_snapshot_at_fresh_patch_version(None)
            .await
            .unwrap();
        assert_eq!(board.version, original_version);
        assert_eq!(prepared.version(), (0, 0, 2));

        let floating = super::Board::load(base_dir.clone(), &board.id, state.clone(), None)
            .await
            .unwrap();
        assert_eq!(floating.version, original_version);
        assert_eq!(floating.name, "Prepared action implementation");

        assert!(
            board
                .commit_prepared_snapshot(&prepared, None)
                .await
                .unwrap()
        );
        assert_eq!(board.version, prepared.version());
        let committed = super::Board::load(base_dir.clone(), &board.id, state.clone(), None)
            .await
            .unwrap();
        assert_eq!(committed.version, prepared.version());

        committed
            .snapshot_at_version(committed.version, None)
            .await
            .unwrap();
        let mut newer_draft = committed;
        newer_draft.name = "Concurrent edit".to_string();
        newer_draft.mark_changed();
        newer_draft.save(None).await.unwrap();
        let second_prepared = newer_draft
            .prepare_snapshot_at_fresh_patch_version(None)
            .await
            .unwrap();
        newer_draft.name = "Edit after prepare".to_string();
        newer_draft.mark_changed();
        newer_draft.save(None).await.unwrap();
        assert!(
            !newer_draft
                .commit_prepared_snapshot(&second_prepared, None)
                .await
                .unwrap(),
            "a post-prepare edit must keep the floating draft at its current version"
        );
        assert_eq!(newer_draft.version, prepared.version());
    }

    #[tokio::test]
    async fn snapshot_publication_rejects_a_stale_cached_floating_board() {
        let state = flow_state().await;
        let base_dir = Path::from("boards");
        let mut persisted = super::Board::new(None, base_dir.clone(), state.clone());
        persisted.name = "Original draft".to_string();
        persisted.mark_changed();
        persisted.save(None).await.unwrap();

        // Simulate a second API process retaining an older in-memory board
        // after another process has committed a newer floating draft at the
        // same semantic version.
        let stale = super::Board::load(base_dir.clone(), &persisted.id, state.clone(), None)
            .await
            .unwrap();
        persisted.name = "Newer authoritative draft".to_string();
        persisted.mark_changed();
        persisted.save(None).await.unwrap();

        let candidate = (stale.version.0, stale.version.1, stale.version.2 + 1);
        let error = stale
            .snapshot_at_version(candidate, None)
            .await
            .expect_err("stale cached content must never become a publishable snapshot");
        assert!(error.to_string().contains("Board draft changed"));
        // Publishers retry this race instead of failing the whole save, which
        // only works while the error stays downcastable.
        assert!(super::is_board_draft_race(&error));
        assert!(
            !stale
                .snapshot_matches_persisted_draft(candidate, None)
                .await
                .unwrap(),
            "the immutable orphan must not be mistaken for the authoritative draft"
        );
    }

    /// A board carrying pin options that protobuf cannot distinguish from unset.
    ///
    /// `enforce_schema: Some(false)` is what the ~100 a2ui element nodes declare, and proto3
    /// writes it as the field default, so it reads back as `None`. Any publication check that
    /// hashes the live draft rather than its persisted projection reports such a board as
    /// different from the snapshot just written from it.
    fn board_with_protobuf_flattened_pin_options(
        state: Arc<crate::state::FlowLikeState>,
        base_dir: Path,
    ) -> super::Board {
        use crate::flow::{node::Node, pin::PinOptions, variable::VariableType};

        let mut board = super::Board::new(None, base_dir, state);
        let mut node = Node::new("a2ui_set_element_text", "Set Element Text", "", "test");
        node.add_input_pin("value_in", "Value", "", VariableType::Struct)
            .set_options(
                PinOptions::new()
                    .set_enforce_schema(false)
                    .set_enforce_generic_value_type(false)
                    .set_step(0.0)
                    .build(),
            );
        board.nodes.insert(node.id.clone(), node);
        board.mark_changed();
        board
    }

    #[tokio::test]
    async fn a_draft_recognizes_the_snapshot_written_from_it() {
        let state = flow_state().await;
        let base_dir = Path::from("boards");
        let board = board_with_protobuf_flattened_pin_options(state, base_dir);
        board.save(None).await.unwrap();
        let version = board.version;

        board.snapshot_at_version(version, None).await.unwrap();

        assert!(
            board.snapshot_matches_current(version, None).await.unwrap(),
            "a draft must recognize the snapshot written from it"
        );
        assert!(
            board
                .snapshot_matches_persisted_draft(version, None)
                .await
                .unwrap(),
            "an unedited draft must not read as changed during its own publication"
        );
    }

    #[tokio::test]
    async fn repeated_patch_publication_advances_one_slot_at_a_time() {
        let state = flow_state().await;
        let base_dir = Path::from("boards");
        let mut board = board_with_protobuf_flattened_pin_options(state, base_dir);
        board.save(None).await.unwrap();
        let first_published = board.version;

        let after_first = board
            .create_version(super::VersionType::Patch, None)
            .await
            .unwrap();
        let after_second = board
            .create_version(super::VersionType::Patch, None)
            .await
            .unwrap();

        assert_eq!(after_first, (first_published.0, first_published.1, 2));
        assert_eq!(after_second, (first_published.0, first_published.1, 3));

        let mut versions = board.get_versions(None).await.unwrap();
        versions.sort();
        assert_eq!(
            versions,
            vec![
                (first_published.0, first_published.1, 1),
                (first_published.0, first_published.1, 2)
            ],
            "each publication must occupy exactly one version slot"
        );
    }

    #[tokio::test]
    async fn interrupted_page_snapshot_resumes_or_skips_conflicting_patch() {
        use crate::{a2ui::widget::Page, utils::compression::compress_to_file_create};

        let state = flow_state().await;
        let base_dir = Path::from("boards");
        let mut board = super::Board::new(None, base_dir, state);
        let page = Page::new("page-1", "Current page", "/");
        board.save_page(&page, None).await.unwrap();
        board.save(None).await.unwrap();
        let store = board.get_store(None).await.unwrap();
        let first_candidate = (board.version.0, board.version.1, board.version.2 + 1);

        // Simulate a process that copied a page and stopped before writing the
        // board existence marker. The next attempt must finish that version.
        let page_proto: flow_like_types::proto::Page = page.clone().into();
        compress_to_file_create(
            store.clone(),
            board.versioned_page_path(first_candidate, "page-1"),
            &page_proto,
        )
        .await
        .unwrap();
        let resumed = board
            .prepare_snapshot_at_fresh_patch_version(Some(store.clone()))
            .await
            .unwrap();
        assert_eq!(resumed.version(), first_candidate);

        board
            .commit_prepared_snapshot(&resumed, Some(store.clone()))
            .await
            .unwrap();
        board.name = "Another draft".to_string();
        board.mark_changed();
        board.save(Some(store.clone())).await.unwrap();
        let conflicting_candidate = (board.version.0, board.version.1, board.version.2 + 1);
        let conflicting_page = Page::new("page-1", "Other publisher's page", "/");
        let conflicting_page_proto: flow_like_types::proto::Page = conflicting_page.into();
        compress_to_file_create(
            store.clone(),
            board.versioned_page_path(conflicting_candidate, "page-1"),
            &conflicting_page_proto,
        )
        .await
        .unwrap();

        let prepared = board
            .prepare_snapshot_at_fresh_patch_version(Some(store))
            .await
            .unwrap();
        assert_eq!(
            prepared.version(),
            (
                conflicting_candidate.0,
                conflicting_candidate.1,
                conflicting_candidate.2 + 1
            ),
            "a conflicting orphan page must not permanently poison publication"
        );
    }

    #[tokio::test]
    async fn undo_against_diverged_board_errors_and_restores_undone_tail() {
        use crate::flow::{
            board::{
                Comment, CommentType,
                commands::{
                    GenericCommand, comments::upsert_comment::UpsertCommentCommand,
                    pins::connect_pins::ConnectPinsCommand,
                },
            },
            node::Node,
            variable::VariableType,
        };
        use std::time::SystemTime;

        let state = flow_state().await;
        let base_dir = Path::from("boards");
        let mut board = super::Board::new(None, base_dir, state.clone());

        let mut from_node = Node::new("test_from", "From", "", "test");
        let from_pin = from_node
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        let mut to_node = Node::new("test_to", "To", "", "test");
        let to_pin = to_node
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let from_id = from_node.id.clone();
        let to_id = to_node.id.clone();
        board.nodes.insert(from_id.clone(), from_node);
        board.nodes.insert(to_id.clone(), to_node);

        let comment = Comment {
            id: "comment-1".to_string(),
            author: None,
            content: "recorded after the connect".to_string(),
            comment_type: CommentType::Text,
            timestamp: SystemTime::now(),
            coordinates: (0.0, 0.0, 0.0),
            width: None,
            height: None,
            layer: None,
            color: None,
            z_index: None,
            hash: None,
            is_locked: None,
            node_id: None,
        };

        let commands = vec![
            GenericCommand::ConnectPin(ConnectPinsCommand::new(
                from_id.clone(),
                to_id.clone(),
                from_pin.clone(),
                to_pin.clone(),
            )),
            GenericCommand::UpsertComment(UpsertCommentCommand::new(comment)),
        ];

        let executed = board
            .execute_commands(commands, state.clone())
            .await
            .unwrap();
        assert!(board.comments.contains_key("comment-1"));

        // Divergence: the board was rewritten underneath the recorded history —
        // the connection's source node no longer exists.
        board.nodes.remove(&from_id);

        let result = board.undo(executed, state.clone()).await;

        assert!(result.is_err(), "undo against a diverged board must fail");
        assert!(
            board.comments.contains_key("comment-1"),
            "commands undone before the failure must be re-applied so the board is not left partially rolled back"
        );
    }

    #[test]
    fn comment_node_id_roundtrips_serde_and_proto_and_changes_hash() {
        use super::{Comment, CommentType};
        use std::time::SystemTime;

        let mut comment = Comment {
            id: "comment-1".to_string(),
            author: None,
            content: "anchored".to_string(),
            comment_type: CommentType::Text,
            timestamp: SystemTime::UNIX_EPOCH,
            coordinates: (1.0, 2.0, 3.0),
            width: None,
            height: None,
            layer: None,
            color: None,
            z_index: None,
            hash: None,
            is_locked: None,
            node_id: Some("node-abc".to_string()),
        };

        let json = flow_like_types::json::to_string(&comment).unwrap();
        let deser: Comment = flow_like_types::json::from_str(&json).unwrap();
        assert_eq!(deser.node_id.as_deref(), Some("node-abc"));

        let mut buf = Vec::new();
        comment.to_proto().encode(&mut buf).unwrap();
        let from_proto =
            Comment::from_proto(flow_like_types::proto::Comment::decode(&buf[..]).unwrap());
        assert_eq!(from_proto.node_id.as_deref(), Some("node-abc"));

        comment.hash();
        let anchored_hash = comment.hash;
        comment.node_id = None;
        comment.hash();
        assert_ne!(
            anchored_hash, comment.hash,
            "hash must change when node_id changes so edits sync"
        );

        let unanchored_json = flow_like_types::json::to_string(&comment).unwrap();
        assert!(
            !unanchored_json.contains("node_id"),
            "None node_id must be skipped so old payloads stay byte-identical"
        );
        let legacy: Comment = flow_like_types::json::from_str(&unanchored_json).unwrap();
        assert_eq!(legacy.node_id, None);
    }

    #[tokio::test]
    async fn upsert_comment_preserves_node_id_on_board() {
        use crate::flow::board::{
            Comment, CommentType,
            commands::{GenericCommand, comments::upsert_comment::UpsertCommentCommand},
        };
        use std::time::SystemTime;

        let state = flow_state().await;
        let mut board = super::Board::new(None, Path::from("boards"), state.clone());

        let comment = Comment {
            id: "comment-anchored".to_string(),
            author: None,
            content: "bound to a statement".to_string(),
            comment_type: CommentType::Text,
            timestamp: SystemTime::now(),
            coordinates: (0.0, 0.0, 0.0),
            width: None,
            height: None,
            layer: None,
            color: None,
            z_index: None,
            hash: None,
            is_locked: None,
            node_id: Some("node-xyz".to_string()),
        };

        board
            .execute_commands(
                vec![GenericCommand::UpsertComment(UpsertCommentCommand::new(
                    comment,
                ))],
                state,
            )
            .await
            .unwrap();

        let stored = board.comments.get("comment-anchored").unwrap();
        assert_eq!(stored.node_id.as_deref(), Some("node-xyz"));
        assert!(stored.hash.is_some());
    }
}
